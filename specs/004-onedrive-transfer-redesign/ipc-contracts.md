# Hợp đồng Giao tiếp IPC (IPC Contracts) - Chuyển dữ liệu OneDrive

Tài liệu này mô tả giao diện tương tác giữa Frontend (React/TypeScript) và Backend (Tauri/Rust) qua các Tauri Commands và Events.

---

## 1. Định nghĩa Giới hạn Kiểu dữ liệu (TypeScript Serialization Limit)

> [!WARNING]
> Tuyệt đối **KHÔNG** sử dụng kiểu dữ liệu `i64` của Rust trực tiếp trong TypeScript DTO dưới dạng chuỗi số lớn. Toàn bộ các định danh số (ID) và kích thước tệp (bytes) được chuyển đổi thành kiểu `number` trong TypeScript.
> Giới hạn an toàn của JavaScript `Number.MAX_SAFE_INTEGER` là $2^{53} - 1$ (tương đương $9.007.199.254.740.991$).
> Mọi giá trị vượt quá giới hạn này (như ID 64-bit ngẫu nhiên từ Telegram) PHẢI được serialize dưới dạng `string` ở lớp vận chuyển IPC DTO.

---

## 2. Tauri Commands (Frontend gọi Backend)

### 2.1. Khởi tạo Job chuyển dữ liệu dưới dạng Draft
*   **Command**: `cmd_migration_create_transfer`
*   **Tham số**:
    ```typescript
    interface CreateTransferConfig {
        onedrive_folder_id: string | null; // null đại diện cho root
        onedrive_folder_path: string;
    }
    ```
*   **Trả về**: `Result<number, String>` (Trả về `job_id` dạng an toàn).

### 2.2. Khởi động Quét OneDrive
*   **Command**: `cmd_migration_start_scan`
*   **Tham số**: `{ job_id: number }`
*   **Trả về**: `Result<(), String>`

### 2.3. Dừng Quét OneDrive
*   **Command**: `cmd_migration_stop_scan`
*   **Tham số**: `{ job_id: number }`
*   **Trả về**: `Result<(), String>`

### 2.4. Lấy Tóm tắt Kế hoạch Di chuyển (Plan Summary)
*   **Command**: `cmd_migration_get_plan_summary`
*   **Tham số**: `{ job_id: number }`
*   **Trả về**: `Result<PlanSummaryDTO, String>`

#### Cấu trúc `PlanSummaryDTO`:
```typescript
interface PlanSummaryDTO {
    job_id: number;
    video_count: number;
    video_bytes: number;
    image_count: number;
    image_bytes: number;
    other_count: number;
    other_bytes: number;
    duplicate_count: number;
    duplicate_bytes_saved: number;
    empty_folder_count: number;
    local_final_bytes: number;                  // Tổng dung lượng file cục bộ sau khi commit
    working_peak_estimate_bytes: number;        // Dự kiến dung lượng đệm tạm đỉnh của .working (bao gồm transcode headroom)
    disk_safety_reserve_bytes: number;          // Hạn mức an toàn bắt buộc của ổ đĩa
    disk_free_bytes: number;                    // Dung lượng trống hiện tại của ổ đĩa
    can_start: boolean;                         // True nếu đủ điều kiện dung lượng
    blocking_reasons: string[];                 // Lý do chặn nếu can_start là false (ví dụ: 'insufficient_disk_space')
}
```

### 2.5. Kích hoạt & Bắt đầu Job
*   **Command**: `cmd_migration_start_job`
*   **Tham số**:
    ```typescript
    interface StartJobConfig {
        job_id: number;
        local_dir: string;                      // Thư mục lưu cục bộ
        telegram_destination_id: number | null; // null đại diện cho Saved Messages
    }
    ```
*   **Trả về**: `Result<(), String>`

### 2.6. Điều khiển Job đang chạy
*   **Tạm dừng Job**: `cmd_migration_pause_job(job_id: number)` → `Result<(), String>`
*   **Tiếp tục Job**: `cmd_migration_resume_job(job_id: number)` → `Result<(), String>`
*   **Hủy Job**: `cmd_migration_cancel_job(job_id: number)` → `Result<(), String>`

---

## 3. Tauri Events (Backend bắn sang Frontend)

### 3.1. Sự kiện Trạng thái Job thay đổi
*   **Event**: `migration:job-state`
*   **Payload DTO**:
```typescript
interface JobStatePayload {
    job_id: number;
    state: 'draft' | 'scanning' | 'plan_review' | 'running' | 'pausing' | 'paused' | 'waiting_quota' | 'waiting_cooldown' | 'waiting_network' | 'completed' | 'completed_with_errors' | 'cancelled' | 'failed';
    pause_reason?: 'user' | 'daily_quota' | 'network_loss';
    completed_at?: number;
}
```

### 3.2. Sự kiện Tiến độ Chi tiết của Từng Stage
*   **Event**: `migration:stage-progress`
*   **Payload DTO**:
```typescript
interface StageProgressPayload {
    job_id: number;
    item_id: number;
    item_name: string;
    attempt: number;
    revision: number; // sequence_id tăng dần để tránh race condition
    event_id: string;
    timestamp: number;
    route_kind: 'video_to_td' | 'image_to_td' | 'other_to_local';
    stage: 'downloading' | 'transcoding' | 'uploading' | 'local_committing';
    percent: number;
    bytes_done: number;
    bytes_total: number;
    speed_bytes_per_sec: number;
}
```

### 3.3. Sự kiện Tệp tin Hoàn tất hoặc Lỗi
*   **Event**: `migration:item-complete`
*   **Payload DTO**:
```typescript
interface ItemCompletePayload {
    job_id: number;
    item_id: number;
    item_name: string;
    attempt: number;
    revision: number;
    event_id: string;
    timestamp: number;
    route_kind: 'video_to_td' | 'image_to_td' | 'other_to_local';
    status: 'completed' | 'skipped_duplicate' | 'reconciliation_required' | 'failed';
    telegram_message_id?: number;
    local_path?: string;
    error_message?: string;
}
```
