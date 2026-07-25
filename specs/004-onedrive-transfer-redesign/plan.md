# Kế hoạch Triển khai (Implementation Plan) - Thiết kế lại tính năng Chuyển dữ liệu OneDrive

Kế hoạch này vạch ra kiến trúc pipeline song song phân tầng, chiến lược di chuyển cơ sở dữ liệu tương thích ngược và phương án phân rã các khối mã nguồn cồng kềnh.

---

## 1. Kiến trúc Bounded Multi-Stage Pipeline (Backend Rust)

*   **Runtime Threading**: Bộ điều phối chính (Worker) chạy trực tiếp trong tiến trình Tauri (Tauri process) bằng Tokio async runtime.
*   **Chia sẻ Telegram Session**: Worker dùng chung `TelegramState` sẵn có của ứng dụng thông qua Tauri State manager, **tuyệt đối không khởi tạo Telegram client thứ hai** để tránh xung đột session keys và giảm thiểu khả năng bị Telegram khóa tần suất.
*   **Cấu trúc Song song Phân tầng**:
    Worker được thiết kế dựa trên mô hình Actor sử dụng `tokio::sync::mpsc::channel` giữa các Stage:
    ```
    [OneDrive Queue]
           │
           ▼ (Stage 1: Downloading - Concurrency 2)
      [part_path -> Xóa part cũ khi crash & tải lại từ đầu]
           │
           ├──────────────────────────────┬─────────────────────────────┐
           ▼ (if Video)                   ▼ (if Image)                  ▼ (if Other)
    (Stage 2: FFmpeg - Concurrency 1)     │                             │
     [min(2, available_parallelism)]      │                             │
           │                              │                             │
           ▼ [artifact_file]              ▼ [artifact_file]             ▼ [artifact_file]
    (Stage 3 & 4: Uploading - Concurrency 1)                              (Stage 5: Local Commit)
     [Persisted random_id Idempotent]                                    [Atomic Rename]
           │                                                                    │
           ▼                                                                    ▼
      [Telegram]                                                         [OneDrive_Archive]
    ```

---

## 2. Kiểm soát Backpressure & Hạn mức Đĩa (Disk Budget Safety)

*   **Tính toán Hạn mức**:
    *   `disk_safety_reserve = max(5.000.000.000 bytes, 10% dung lượng filesystem)`
    *   `working_budget = min(50.000.000.000 bytes, disk_free_at_start - disk_safety_reserve)`
*   **Điều phối an toàn đĩa**:
    *   Trước khi bắt đầu bất kỳ Job nào, Backend kiểm tra nếu `working_budget` hiện tại không đủ chỗ cho kích thước tệp lớn nhất trong hàng đợi cộng với dung lượng đệm ước tính của FFmpeg transcode (headroom), Job sẽ không được phép chạy (`can_start = false`).
    *   Worker thực hiện giữ trước dung lượng đĩa (disk-byte reservation/permit) trước khi download và trước khi transcode. Việc kiểm tra dung lượng ổ đĩa qua filesystem chỉ đóng vai trò là lớp kiểm tra phụ.
    *   Dung lượng thư mục `.working` không bao giờ được phép vượt quá `working_budget`.

---

## 3. Khôi phục Upload Idempotent (Persisted random_id)

Để bảo đảm zero duplicate message khi gặp lỗi crash/mất điện giữa chừng:
1.  **Xác minh Grammers API**:
    *   Quá trình triển khai phải kiểm tra (validate) xem API cao cấp (high-level client) của thư viện `Grammers` có cho phép tùy biến hoặc tiêm (inject) tham số `random_id` (64-bit) hay không.
    *   Nếu không hỗ trợ, chúng ta PHẢI sử dụng trực tiếp các cuộc gọi MTProto cấp thấp hơn như `messages.sendMedia` thông qua adapter phù hợp.
2.  **Lưu trữ Random ID bền vững**:
    *   Trước khi gọi hàm gửi media, ghi `telegram_random_id` và `upload_attempt_id` vào SQLite.
    *   Khi tải lên, truyền trực tiếp `telegram_random_id` này.
    *   Nếu crash, khi chạy lại, worker tái sử dụng chính xác `telegram_random_id` này. Telegram sẽ tự động loại bỏ yêu cầu nếu đã nhận được tệp đó trước đó.
    *   Lưu ánh xạ (mapping) từ response Telegram sang `telegram_message_id`.
    *   Nếu adapter không thể đảm bảo an toàn idempotent (ví dụ gặp lỗi API không xác định), tệp tin chuyển sang trạng thái `reconciliation_required` và dừng lại để đối soát thủ công, không tự động tải lại.

---

## 4. Tương thích Ngược & Loại bỏ API Phá hủy (Pipeline Compatibility)

*   **Pipeline Versioning**: Bảng `migration_jobs` được bổ sung trường `pipeline_version`. Job hiện tại có version = 1; Job thiết kế mới có version = 2.
*   **Xử lý Job cũ**: Giữ nguyên lịch sử các Job v1. Không thực hiện mass-update route của các item cũ. Các Job v1 chưa hoàn thành phải được pause/archive an toàn và UI đề nghị tạo snapshot v2 mới.
*   **An toàn Dữ liệu**: **Loại bỏ hoàn toàn khả năng gọi `delete_onedrive_item` từ bất kỳ migration worker nào (cả v1 và v2)** để đảm bảo an toàn tuyệt đối cho tệp gốc OneDrive.

---

## 5. Kế hoạch Di chuyển Cơ sở dữ liệu (Database Migration Plan)

Bật chế độ WAL và cấu hình đồng bộ cao nhất:
```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = FULL; -- Tuyệt đối không dùng synchronous = NORMAL để bảo toàn dữ liệu khi crash
```

---

## 6. Kế hoạch Phân rã & Tái cấu trúc (Refactoring Plan)

### 6.1. Phân rã React `useMigration` Hook & State Owner
*   `MigrationContext` vẫn là **authoritative frontend state owner** duy nhất.
*   Các hooks nhỏ như `useMsAccount`, `useMigrationJob`, `useMigrationProgress`, `useMigrationLogs` chỉ đóng vai trò là các lớp phân tách logic nội bộ của `useMigration` để giảm số dòng mã nguồn, tuyệt đối không tự quản lý state độc lập gây lệch pha.
*   Mọi task triển khai các component React mới PHẢI đi kèm việc cài đặt đầy đủ i18n (vi/en) ngay trong task đó, không trì hoãn dịch thuật đến cuối dự án.

### 6.2. Phân rã React Component `AutoMigrationCenter.tsx`
Chia nhỏ thành:
*   `OneDriveTransferWizard.tsx`: Component hiển thị quy trình di chuyển 7 trạng thái.
*   `PlanSummaryView.tsx`: Hiển thị tóm tắt kế hoạch di chuyển (sử dụng các biến `local_final_bytes`, `working_peak_estimate_bytes`, `disk_safety_reserve_bytes`, `disk_free_bytes`, `can_start`, `blocking_reasons`).
*   `ActiveStagesPanel.tsx`: Bảng hiển thị tiến độ thời gian thực của các tệp đang chuyển.
*   `AdvancedDisclosureDrawer.tsx`: Drawer ẩn chứa log kỹ thuật và bảng duyệt hàng đợi file (chỉ đọc).
*   `DestinationPicker.tsx`: Component chọn destination Telegram có sẵn.
