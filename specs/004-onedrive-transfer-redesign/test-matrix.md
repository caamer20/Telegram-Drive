# Ma trận Kiểm thử (Test Matrix) - Chuyển dữ liệu OneDrive

Tài liệu này mô tả chi tiết các kịch bản kiểm thử (test cases) cần thiết để đảm bảo tính ổn định, an toàn dữ liệu và khả năng chịu lỗi của tính năng di chuyển OneDrive mới.

---

## 1. Kiểm thử Tích hợp Rust (Rust Integration Tests)

Các bài test tích hợp sẽ chạy trong môi trường test của Cargo (`src-tauri/tests/`) sử dụng cơ sở dữ liệu SQLite test tạm thời.

| ID | Nhóm kiểm thử | Kịch bản kiểm thử (Test Case) | Kết quả mong đợi (Expected Output) |
| :--- | :--- | :--- | :--- |
| **RT-001** | Routing | Đưa vào hàng đợi: 1 video (tương thích/không tương thích), 1 ảnh, 1 tài liệu pdf, 1 thư mục rỗng. | Video tương thích vẫn qua FFmpeg remux đổi container (`-c copy`), video không tương thích được transcode. Ảnh upload trực tiếp nguyên bản. Tài liệu pdf chỉ lưu local và giữ cấu trúc thư mục tương đối. Thư mục rỗng được tạo cục bộ và lưu trong manifest. |
| **RT-002** | Disk Full | Giả lập đĩa cứng bị đầy (`no space left on device`) ở Stage 1 (Tải xuống) hoặc Stage 2 (Transcode). | Pipeline dừng an toàn, ghi nhận lỗi `working_directory_unavailable` hoặc `insufficient_disk`, không làm đơ ứng dụng. |
| **RT-003** | Local Path | Cấu hình thư mục lưu cục bộ trỏ tới một ổ đĩa USB, sau đó rút USB ra giữa chừng. | Giai đoạn `local_committing` báo lỗi `working_directory_unavailable`, dừng pipeline và giữ các file tạm trong `.working`. |
| **RT-004** | Safe Pause | Bấm nút Pause khi đang có 2 file đang tải và 1 file đang upload. | Pipeline chờ các checkpoint SQLite hiện tại lưu an toàn rồi mới chuyển trạng thái Job sang `paused`. |

---

## 2. Kiểm thử An toàn Đường dẫn (Path Safety Validation Tests)

| ID | Kịch bản kiểm thử (Path Safety Case) | Kết quả mong đợi (Expected Output) |
| :--- | :--- | :--- |
| **PT-001** | Tấn công Path Traversal (`../file.txt`) | Hệ thống phát hiện, chặn download và ghi nhận lỗi `working_directory_unavailable` hoặc lỗi bảo mật đường dẫn. |
| **PT-002** | Đường dẫn Absolute hoặc Windows drive/UNC paths (`C:\windows\system32\...` hoặc `\\server\share\...`) | Chặn xử lý và báo lỗi bảo mật đường dẫn. Mọi đích đến bắt buộc phải nằm dưới normalized backup root. |
| **PT-003** | Tên tệp tin chứa ký tự đặc biệt hoặc Unicode chưa chuẩn hóa | Thực hiện chuẩn hóa Unicode (Unicode normalization), loại bỏ/thay thế ký tự cấm, tránh xung đột case-insensitive và chặn đứng symlink escape. |

---

## 3. Kiểm thử Khả năng Phục hồi khi Crash (Crash/Restart Recovery Tests)

Kiểm thử bằng cách dừng đột ngột tiến trình worker (kill process) tại các thời điểm nhạy cảm và mở lại.

| ID | Stage xảy ra crash | Trạng thái ghi trong DB | Hành vi sau khi mở lại ứng dụng & Resume |
| :--- | :--- | :--- | :--- |
| **CR-001** | Downloading | `downloading` | Xóa file `.part` dở dang, thực hiện tải lại file hiện tại từ đầu (không dùng HTTP Range resume dở dang). |
| **CR-002** | Transcoding | `transcoding` | Xóa file transcode `.transcoded.mp4` tạm, chạy lại FFmpeg cho file đó từ đầu. |
| **CR-003** | Uploading (Sau khi gửi message thành công lên Telegram nhưng trước khi commit DB) | `uploading` | Hệ thống tái sử dụng chính xác persisted `telegram_random_id` khi retry. Telegram phát hiện yêu cầu trùng lặp và phản hồi lại message ID cũ mà không upload lại tệp (Idempotent send). |
| **CR-004** | Local Commit | `local_committing` | Kiểm tra file đích trên ổ đĩa. Nếu chưa có hoặc kích thước sai -> thực hiện di chuyển/ghi đè lại từ `.working` bằng atomic rename. |

---

## 4. Kiểm thử Xung đột Trùng lặp (Dedupe Race Condition Tests)

Giả lập các tình huống trùng lặp phức tạp.

| ID | Nhóm kiểm thử | Kịch bản kiểm thử (Test Case) | Kết quả mong đợi (Expected Output) |
| :--- | :--- | :--- | :--- |
| **DR-001** | Pre-download Dedupe | Snapshot có 3 tệp trùng vân tay (`onedrive_quickxor` hoặc `onedrive_sha1` + size). Chạy song song. | 1 tệp được chọn làm canonical, 2 tệp còn lại trỏ `duplicate_of_item_id`. 2 tệp bản sao chờ tệp canonical chạy. Khi tệp canonical thành công, 2 bản sao tự động chuyển `skipped_duplicate` không cần tải xuống. |
| **DR-002** | Canonical Promotion | Tệp canonical được chọn bị lỗi tải xuống liên tục 3 lần (thất bại hoàn toàn). | Hệ thống tự động chọn tệp bản sao thứ nhất nâng cấp làm canonical mới, reset trạng thái về `pending` và tiếp tục xử lý. Tệp bản sao thứ hai trỏ về canonical mới này. |
| **DR-003** | Local Artifact Missing | Tệp trùng khớp vân tay với một file đã tải về local từ job trước, nhưng file local thực tế đã bị người dùng xóa trên ổ đĩa. | Hệ thống phát hiện file local không tồn tại ở `local_dest_path` → bỏ qua việc skip, tiến hành tải lại file đó. |
| **DR-004** | Target Key Split | Cùng một file trùng vân tay nhưng một bản cần đưa lên Telegram và một bản cần lưu local. | Không được phép skip. Hệ thống phải tạo hai artifact tương ứng (một bản Telegram, một bản local). |

---

## 5. Kiểm thử Giới hạn Quota & Cooldown (Quota & Cooldown Tests)

| ID | Nhóm kiểm thử | Kịch bản kiểm thử (Test Case) | Kết quả mong đợi (Expected Output) |
| :--- | :--- | :--- | :--- |
| **QT-001** | Quota Boundary | Quota còn lại là 500 MB. File tiếp theo có kích thước artifact sau transcode là 600 MB. | Hệ thống chặn trước khi upload, release reservation, chuyển trạng thái Job sang `waiting_quota`. Không có cách nào bypass giới hạn này từ UI. |
| **QT-002** | Quota Reset | Hệ thống đang dừng ở `waiting_quota`. Chờ đến 00:00 (giờ máy local). | Bộ đếm quota reset về 0, hệ thống tự động khởi động lại pipeline và tiếp tục xử lý file. |
| **QT-003** | Flood Wait | Telegram API trả về lỗi `FLOOD_WAIT_300`. | Worker bắt được lỗi, tính toán thời điểm cooldown, ghi vào DB, chuyển Job sang trạng thái `waiting_cooldown` và tự động resume sau 300 giây. |
| **QT-004** | Premium Flood | Telegram API trả về lỗi `FLOOD_PREMIUM_WAIT_90`. | Bộ phân tích lỗi nhận diện chính xác mã lỗi Premium, chuyển Job sang `waiting_cooldown` trong 90 giây và tự động resume. |

---

## 6. Kiểm thử Giao diện Người dùng (Vitest & UI Tests)

Kiểm thử các component frontend React.

| ID | Component / Trạng thái | Kịch bản kiểm thử (Test Case) | Kết quả kiểm tra (Expected Assertion) |
| :--- | :--- | :--- | :--- |
| **UT-001** | Chưa kết nối MS | Render trang di chuyển OneDrive. | Không hiển thị bất kỳ khối thông tin tiến trình hay danh sách tệp nào. Chỉ hiển thị nút "Kết nối tài khoản Microsoft". |
| **UT-002** | Plan Summary | Quét xong, render màn hình kế hoạch. | Hiển thị chính xác số lượng/dung lượng của từng nhóm định tuyến, tổng dung lượng cần thiết, dung lượng trống khả dụng và nút CTA "Bắt đầu". |
| **UT-003** | Active Stages | Có file đang transcode và upload. | Thanh tiến trình cho transcode và upload chạy độc lập, hiển thị tốc độ FPS (transcode) và MB/s (upload). |
| **UT-004** | Hết Quota | Trạng thái Job bị chuyển sang `waiting_quota`. | Hiển thị thông báo hạn mức quota đã hết, thời gian tự động reset rõ ràng. Tuyệt đối không hiển thị nút bypass. |

---

## 7. Kiểm thử Tích hợp Quy trình (CI/CD Pipeline Validation)

Mỗi lần commit code, hệ thống CI/CD cần chạy các bước kiểm tra sau để đảm bảo chất lượng:

1.  **Type Check**: Chạy `npm run type-check` (hoặc `tsc --noEmit`) để đảm bảo không có lỗi TypeScript ở Frontend.
2.  **Vitest**: Chạy `npm run test` để chạy toàn bộ unit tests frontend.
3.  **Cargo Test**: Chạy `cargo test` để chạy các unit/integration tests Rust ở Backend.
4.  **Production Build**: Chạy `npm run build` và `cargo tauri build` để kiểm tra khả năng đóng gói ứng dụng trên macOS.
