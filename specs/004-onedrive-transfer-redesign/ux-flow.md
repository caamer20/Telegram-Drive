# Luồng Trải nghiệm Người dùng (UX Flow) - Chuyển dữ liệu OneDrive

Tài liệu này định nghĩa cấu trúc thông tin, các trạng thái màn hình và sơ đồ tương tác người dùng của tính năng "Chuyển dữ liệu từ OneDrive" được thiết kế lại.

---

## 1. Kiến trúc Thông tin (Information Architecture)

Trang chính: **Chuyển dữ liệu từ OneDrive** (`/migration`)
*   **Trạng thái 1: Kết nối (Connection)**
    *   Thẻ kết nối OneDrive (Nút: "Kết nối tài khoản Microsoft")
*   **Trạng thái 2: Chuẩn bị (Configuration)**
    *   Tài khoản OneDrive đang kết nối (Thông tin tên tài khoản, nút: "Đổi tài khoản")
    *   Đường dẫn lưu trữ:
        *   Một folder picker chính: **Thư mục lưu cục bộ** (Local Folder)
        *   Khối xem trước (Preview details - ẩn/hiện):
            *   Đường dẫn lưu file khác: `[Local Folder]/OneDrive_Archive/...`
            *   Đường dẫn vùng làm việc tạm: `[Local Folder]/.working/...`
    *   Nơi nhận file trên Telegram:
        *   Bộ chọn Destination (Destination Picker): Mặc định là **Saved Messages** (Tin nhắn đã lưu). Cho phép bấm để chọn các kênh Telegram sở hữu mà bot/user có quyền viết.
    *   Nút CTA chính: **"Quét dữ liệu & Lập kế hoạch"** (Vô hiệu hóa nếu chưa chọn Local Folder).
*   **Trạng thái 3: Quét & Lập kế hoạch (Scanning & Planning)**
    *   Tiến trình quét động: Hiển thị thanh tiến trình quét thư mục OneDrive, số lượng tệp và dung lượng đã phát hiện.
    *   Nút CTA: "Hủy quét".
*   **Trạng thái 4: Xem kế hoạch (Plan Review)**
    *   Bảng tóm tắt kết quả quét (Summary Dashboard):
        *   **Video**: X files (Y GB) → Tối ưu qua FFmpeg (remux hoặc transcode) → Upload lên Telegram Drive.
        *   **Ảnh**: X files (Y GB) → Upload nguyên bản trực tiếp lên Telegram Drive.
        *   **Tệp tin khác**: X files (Y GB) → Tải về máy cục bộ.
        *   **Trùng lặp dự kiến**: X files (Y GB) → Tiết kiệm bằng cơ chế liên kết.
        *   **Thư mục rỗng**: X thư mục → Tái tạo cấu trúc rỗng local.
    *   Thông số đĩa cứng:
        *   Dung lượng local cần thiết (`local_final_bytes`).
        *   Dung lượng đệm tạm dự kiến (`working_peak_estimate_bytes`).
        *   Hạn mức an toàn ổ đĩa (`disk_safety_reserve_bytes`).
        *   Dung lượng trống khả dụng (`disk_free_bytes`).
        *   Chỉ số bắt đầu: `can_start` (true/false) và lý do chặn: `blocking_reasons` (nếu có).
    *   Nút CTA chính: **"Bắt đầu chuyển ngay"** (Vô hiệu hóa nếu `can_start` là false). Nút phụ: "Quét lại".
*   **Trạng thái 5: Đang chuyển (Transferring)**
    *   Thanh tiến trình tổng thể (Overall Progress Bar): Phần trăm hoàn thành, số lượng file đã xong/tổng số file, tốc độ mạng trung bình, thời gian còn lại dự kiến.
    *   Khối trạng thái các Stage đang chạy (Active Stages):
        *   `[Tải xuống]`: Đang tải file A, B (Tốc độ, phần trăm).
        *   `[FFmpeg]`: Đang tối ưu video C (Phần trăm, tốc độ FPS).
        *   `[Upload]`: Đang đưa file D lên Telegram Drive (Tốc độ, phần trăm).
        *   `[Ghi cục bộ]`: Đang lưu file E vào thư mục đích.
    *   Khối thông số Quota: Hiển thị lượng dung lượng upload thực tế đã dùng trong ngày (ví dụ: `15.2 GB / 250.0 GB`), thanh tiến trình quota.
    *   Nút CTA chính: **"Tạm dừng"**.
    *   Khối chi tiết nâng cao (Advanced Disclosure - mặc định ẩn):
        *   Tab 1: Log kỹ thuật thời gian thực (Technical logs).
        *   Tab 2: Trình duyệt tệp tin đang chuyển (File Explorer - chỉ đọc, hiển thị danh sách hàng đợi và trạng thái chi tiết của từng tệp: pending, downloading, transcode, upload, completed, failed, skipped). Không cho phép sửa đổi tên/xóa OneDrive trên từng hàng.
*   **Trạng thái 6: Tạm dừng / Chờ (Paused / Waiting)**
    *   Hiển thị thông báo rõ ràng về nguyên nhân tạm dừng:
        *   *Người dùng chủ động tạm dừng*: "Đang tạm dừng chuyển dữ liệu. Bạn có thể tiếp tục bất cứ lúc nào." (Nút CTA: "Tiếp tục").
        *   *Vượt Quota ngày*: "Đã đạt giới hạn an toàn 250 GB upload trong ngày. Hệ thống sẽ tự động tiếp tục vào lúc 00:00 (sau X giờ Y phút)." (Không cung cấp nút bỏ qua).
        *   *Mất kết nối mạng*: "Mất kết nối mạng. Hệ thống đang tự động kết nối lại sau X giây..." (Nút CTA: "Thử lại ngay").
        *   *Chờ Cooldown Telegram*: "Đang tạm dừng do yêu cầu từ Telegram (FLOOD_WAIT). Sẽ tự động resume sau X giây..."
*   **Trạng thái 7: Hoàn tất (Completed / Completed with Errors)**
    *   Màn hình chúc mừng với các số liệu thống kê cuối cùng: tổng số file đã chuyển, dung lượng tiết kiệm được nhờ transcode và dedupe, tổng thời gian thực hiện.
    *   Trường hợp hoàn thành có lỗi: Hiển thị danh sách file lỗi, cho phép xuất file báo cáo manifest và cung cấp nút **"Thử lại các file lỗi"**.
    *   Nút CTA: "Chuyển tài khoản khác" hoặc "Trở lại Trang chủ".

---

## 2. Text Wireframes (Giao diện bằng văn bản)

### Màn hình Xem kế hoạch (Plan Review Screen)
```
+--------------------------------------------------------------------------+
| CHUYỂN DỮ LIỆU TỪ ONEDRIVE                                               |
+--------------------------------------------------------------------------+
|                                                                          |
|  [ Kế hoạch chuyển dữ liệu được tạo thành công! ]                        |
|  Tài khoản: user@outlook.com                                             |
|                                                                          |
|  Tóm tắt kế hoạch di chuyển:                                             |
|  ======================================================================  |
|  * Video (Transcode & Upload TD) : 142 tệp  (45.20 GB)                   |
|  * Hình ảnh (Upload TD nguyên bản): 854 tệp  (12.40 GB)                   |
|  * File khác (Chỉ lưu máy cục bộ): 1,205 tệp (18.10 GB)                  |
|  * Tệp trùng lặp (Bỏ qua & liên kết): 320 tệp  (8.50 GB - Tiết kiệm!)     |
|  * Thư mục rỗng (Tái tạo local)   : 12 thư mục                           |
|  ----------------------------------------------------------------------  |
|  Dung lượng cần lưu cục bộ:  93.80 GB                                    |
|  Dung lượng trống trên ổ đĩa: 245.50 GB [ Khả dụng - OK ]                |
|  ======================================================================  |
|                                                                          |
|  Thư mục đích lưu cục bộ: /Users/username/Downloads/OneDrive_Backup      |
|  Nơi lưu trên Telegram: Saved Messages (Mặc định)                        |
|                                                                          |
|  [ BẮT ĐẦU CHUYỂN NGAY ]                      [ Quét lại ]               |
+--------------------------------------------------------------------------+
```

### Màn hình Đang chuyển (Transferring Screen)
```
+--------------------------------------------------------------------------+
| CHUYỂN DỮ LIỆU TỪ ONEDRIVE                                               |
+--------------------------------------------------------------------------+
|                                                                          |
|  Trạng thái: Đang chuyển dữ liệu...                                      |
|  Tiến độ tổng thể: [██████████████░░░░░░░░░░░░░░░░░░░░░░░░░░] 35%         |
|  Đã hoàn thành: 342 / 2,201 tệp (18.4 GB / 75.7 GB)                      |
|  Tốc độ: 12.5 MB/s | Thời gian còn lại dự kiến: 01 giờ 12 phút           |
|                                                                          |
|  Các giai đoạn đang hoạt động:                                           |
|  ======================================================================  |
|  [Tải xuống] [1/2] video_sample_1.mkv (1.2 GB)    : [██████░░] 75%  5 MB/s |
|              [2/2] document_draft.pdf (4.5 MB)    : [████████] 100% (Xong)|
|  [Tối ưu FF] [1/1] holiday_movie.mp4 (450 MB)     : [████░░░░] 50%  24 fps |
|  [Upload TG] [1/1] family_photo.jpg (3.2 MB)      : [████████] 100% (Xong)|
|  ----------------------------------------------------------------------  |
|  Hạn mức upload Telegram hôm nay (Giới hạn an toàn):                     |
|  [████████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░] 18.4 GB / 250.0 GB    |
|                                                                          |
|  [ TẠM DỪNG ]                                                            |
|                                                                          |
|  > Chi tiết kỹ thuật & Nhật ký hoạt động (Bấm để mở rộng)                |
+--------------------------------------------------------------------------+
```

---

## 3. Các thành phần Thay đổi / Bỏ đi (UX Simplifications)

*   **LOẠI BỎ**:
    *   Hành động "Tiếp tục thủ công - Bỏ qua giới hạn an toàn" trên UI khi vượt quota ngày. Quota ngày là giới hạn cứng không thể bypass.
    *   Nút bấm xóa hoặc sửa tên tệp OneDrive trên từng dòng tệp hiển thị.
    *   Hành động chọn thủ công từng tệp để đồng bộ (người dùng di chuyển toàn bộ hoặc theo kế hoạch tự động phân loại).
    *   Cấu hình Telegram Destination ID bằng cách nhập số thô (thay bằng Destination Picker).
*   **THAY ĐỔI CẤU TRÚC**:
    *   Bảng quản lý tệp tin (File Explorer) phức tạp với các nút check chọn, rename, delete được ẩn đi, chỉ hiển thị trong phần **Advanced Disclosure** (Drawer nâng cao) phục vụ nhu cầu kiểm tra kỹ thuật (progressive disclosure).
    *   Hợp nhất 3 bảng Download List, Processing List, Upload List thành một khối hiển thị **Active Stages** gọn gàng, trực quan và chỉ thể hiện các tác vụ đang hoạt động thực tế.
    *   Gộp cấu hình "Thư mục làm việc tạm" (.working) và "Thư mục lưu file khác" thành duy nhất một lựa chọn **"Thư mục lưu cục bộ"** chính để tối giản thao tác.
