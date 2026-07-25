# Đặc tả Kỹ thuật (Specification) - Thiết kế lại tính năng Chuyển dữ liệu OneDrive

Đặc tả này định nghĩa các yêu cầu chức năng, phi chức năng, các trường hợp biên và tiêu chí thành công cho tính năng Chuyển dữ liệu OneDrive mới.

---

## 1. User Stories

### Story 1: Chuyển dữ liệu OneDrive an toàn (Người dùng cá nhân)
*   **Là** một người dùng Telegram-Drive,
*   **Tôi muốn** chuyển toàn bộ dữ liệu từ OneDrive cá nhân sang Telegram Drive và ổ đĩa cục bộ chỉ với một lần thiết lập và chạy,
*   **Để tôi** có thể lưu trữ toàn bộ dữ liệu cũ của mình một cách an toàn mà không sợ bị sửa đổi hay xóa dữ liệu gốc trên OneDrive.

#### Kịch bản chấp nhận (Acceptance Scenarios):
1.  **Given** tài khoản Microsoft đã kết nối, **When** tôi bắt đầu cấu hình di chuyển, **Then** hệ thống yêu cầu tôi chọn thư mục lưu cục bộ chính (Local Folder) và hiển thị thông tin chi tiết về không gian đĩa trống so với nhu cầu sử dụng thực tế.
2.  **Given** quá trình chuyển đang chạy, **When** ứng dụng bị tắt đột ngột hoặc mất kết nối mạng, **Then** hệ thống sẽ tạm dừng và có thể tiếp tục (resume) chính xác từ vị trí dở dang khi mở lại ứng dụng bằng cách xóa tệp dở dang và tải lại từ đầu.
3.  **Given** một file bất kỳ, **When** quá trình chuyển hoàn tất thành công, **Then** tệp gốc trên OneDrive phải giữ nguyên cấu trúc, thuộc tính và tuyệt đối không bị xóa hoặc đổi tên.

---

### Story 2: Tự động phân loại và tối ưu hóa tài nguyên (Hệ thống)
*   **Là** một hệ thống xử lý dữ liệu thông minh,
*   **Tôi muốn** tự động phân loại tệp tin theo định dạng và xử lý chúng qua các pipeline chuyên biệt (remux hoặc transcode video qua FFmpeg, giữ nguyên ảnh gốc, tải cục bộ file khác bao gồm cả thư mục rỗng),
*   **Để tối ưu** dung lượng lưu trữ trên Telegram Drive và băng thông mạng.

---

## 2. Yêu cầu Chức năng (Functional Requirements)

### 2.1. Quản lý Giao dịch & Pipeline (Pipeline & Execution Management)
*   **FR-001**: Hệ thống PHẢI hỗ trợ cơ chế pipeline song song phân tầng (overlap): file N đang upload lên Telegram, file N+1 đang được FFmpeg xử lý, file N+2 đang được tải xuống từ OneDrive.
*   **FR-002**: Hệ thống PHẢI giới hạn concurrency tối đa mặc định cho các stage: download concurrency = 2, FFmpeg transcode concurrency = 1, Telegram upload concurrency = 1.
*   **FR-003**: Hệ thống PHẢI hỗ trợ điều phối dòng dữ liệu có giới hạn (Bounded Backpressure). Xem chi tiết tại Mục 4 (Backpressure).
*   **FR-004**: Hệ thống chỉ cho phép duy nhất MỘT Job di chuyển OneDrive hoạt động tại một thời điểm.
*   **FR-005**: Hệ thống PHẢI hỗ trợ tái tạo các thư mục OneDrive thực sự rỗng dưới thư mục lưu cục bộ `OneDrive_Archive` và ghi nhận các thư mục này trong manifest.

### 2.2. Nhận diện & Chống trùng lặp (Deduplication)
*   **FR-006**: Vân tay tệp tin được phân loại theo các loại sau để khớp với code hiện tại: `onedrive_quickxor`, `onedrive_sha1`, `sha256`.
*   **FR-007**: Hệ thống PHẢI sử dụng `artifact_target_key` để phân định dedupe scope độc lập:
    *   Telegram: `telegram:<destination_id-or-saved>`
    *   Local: `local:<normalized-backup-root>`
*   **FR-008**: Tiến trình claim canonical item độc nhất dựa trên khóa tổ hợp:
    `fingerprint_type + fingerprint_value + source_size + artifact_target_key`
*   **FR-009**: Tuyệt đối không được phép skip tệp local chỉ vì đã có bản sao trên Telegram. Nếu cùng nội dung cần tồn tại ở cả Telegram và local, phải tạo hai artifact tương ứng.
*   **FR-010**: Thiết kế giao dịch đặt chỗ vân tay (Dedupe Claim/Reservation):
    *   Khi file bản gốc (canonical item) đang được xử lý, các bản sao (duplicate items) phải ở trạng thái chờ (`waiting_duplicate`).
    *   Nếu bản gốc thành công, tất cả bản sao tự động chuyển sang `skipped_duplicate` và kế thừa liên kết message ID hoặc local path của bản gốc.
    *   Nếu bản gốc thất bại hoàn toàn (sau 3 lần retry), hệ thống PHẢI tự động chọn một bản sao khác nâng cấp lên làm bản gốc mới (promote to canonical) để tiếp tục xử lý.
*   **FR-011**: Lịch sử trùng lặp (Dedupe history) PHẢI được lưu trữ toàn cục trong SQLite (`migrated_fingerprints`) để dùng chung giữa tất cả các Job.
*   **FR-012**: Đối với file chỉ lưu cục bộ (local-only files), hệ thống chỉ được phép bỏ qua (skip) nếu tệp bản gốc cục bộ đó thực sự tồn tại ở local path tương ứng. Nếu không, phải tải lại.

### 2.3. Telegram Safety Guard & Rate Limits
*   **FR-013**: Hệ thống PHẢI tuân thủ giới hạn quota an toàn nội bộ cứng: tối đa **250.000.000.000 bytes** (250 GB thực tế) upload thành công mỗi ngày local. Không cung cấp bất kỳ nút, command hoặc setting nào cho phép người dùng bỏ qua giới hạn này.
*   **FR-014**: Quota ngày PHẢI được tính dựa trên dung lượng tệp tin thực tế sau khi xử lý (artifact size sau transcode/remux), không dùng dung lượng tệp gốc trên OneDrive.
*   **FR-015**: Hệ thống PHẢI thực hiện giữ trước quota (atomic reservation) ngay trước khi bắt đầu upload file. Nếu upload thành công, commit quota; nếu thất bại, giải phóng quota đã giữ.
*   **FR-016**: Hệ thống PHẢI tự động áp dụng khoảng cách tối thiểu **3 giây** giữa hai lần gửi message thành công để pacing. Sau **100 file** gửi liên tiếp, tự động cooldown **120 giây**. Các trạng thái pacing này phải được persist.
*   **FR-017**: Hệ thống PHẢI tự động nhận diện cả lỗi `FLOOD_WAIT_X` và `FLOOD_PREMIUM_WAIT_X` từ Telegram và dùng chúng ghi đè hoàn toàn lên local pacing. Không tuyên bố đây là giới hạn chính thức của Telegram.

### 2.4. Khôi phục lỗi & Dừng an toàn (Stage Recovery & Safe Pause)
*   **FR-018**: Mỗi giai đoạn xử lý PHẢI có checkpoint ghi xuống SQLite.
*   **FR-019**: Khi hệ thống khởi động lại sau crash, worker PHẢI tự động khôi phục theo quy tắc:
    *   `downloading`: Xóa file `.part` dở dang do feature sở hữu, thực hiện tải lại file hiện tại từ đầu. Không sử dụng HTTP Range resume (tính năng này được ghi lại trong mục hardening tương lai).
    *   `transcoding`: Xóa file transcode tạm, thực hiện transcode lại từ đầu.
    *   `ready_upload` / `uploading`: Sử dụng cơ chế idempotent gửi tệp với persisted `random_id` được mô tả ở Mục 5.
*   **FR-020**: Khi người dùng nhấn nút Tạm dừng (Pause), hệ thống PHẢI dừng nhận các tệp mới vào pipeline, chờ các tệp đang tải/transcode/upload dở dang hoàn thành các checkpoint hiện tại rồi mới chuyển sang trạng thái dừng an toàn (Safe Pause).

---

## 3. Quản lý Đích đến & validate Quyền Telegram (Destination Permission Preflight)

*   **FR-021**: Mặc định kênh nhận file trên Telegram là **Saved Messages** (Tin nhắn đã lưu).
*   **FR-022**: Nếu người dùng chọn kênh Telegram tùy chỉnh, destination đó PHẢI được resolve và kiểm tra quyền gửi file (send message permission) trước khi Job bắt đầu chạy.
*   **FR-023**: Khi thực hiện preflight check, hệ thống **KHÔNG ĐƯỢC PHÉP** gửi test message nháp lên channel để tránh làm phiền người dùng. Phải sử dụng API check quyền sở hữu hoặc cấu hình chat của Telegram.

---

## 4. Kiểm soát Backpressure & Hạn mức Đĩa (Disk Working Budget)

*   `disk_safety_reserve = max(5.000.000.000 bytes, 10% dung lượng filesystem)`
*   `working_budget = min(50.000.000.000 bytes, disk_free_at_start - disk_safety_reserve)`
*   **FR-024**: Hệ thống **KHÔNG ĐƯỢC PHÉP** bắt đầu Job nếu `working_budget` hiện tại không đủ chỗ cho file lớn nhất trong hàng đợi cộng với dung lượng đệm ước tính của FFmpeg transcode (headroom).
*   **FR-025**: Hệ thống PHẢI thực hiện giữ trước dung lượng đĩa (disk-byte reservation/permit) ngay trước khi bắt đầu download và transcode tệp. Việc quét kiểm tra dung lượng ổ đĩa qua filesystem chỉ đóng vai trò là lớp kiểm tra phụ.
*   **FR-026**: Dung lượng thư mục `.working` không bao giờ được phép vượt quá `working_budget`.

---

## 5. Thiết kế Chống trùng lặp/Khôi phục Upload (Idempotency Recovery)

*   **FR-027**: Loại bỏ hoàn toàn cơ chế "quét 10 message gần nhất để so sánh SHA-256" làm cơ chế chính để khôi phục.
*   **FR-028**: Hệ thống PHẢI tạo và lưu trữ bền vững một số ngẫu nhiên 64-bit `telegram_random_id` và `upload_attempt_id` vào cơ sở dữ liệu trước khi gọi API gửi file lên Telegram.
*   **FR-029**: Hàm `upload_core` nhận `telegram_random_id` và truyền trực tiếp vào API Telegram (MTProto hoặc Grammers). Khi thực hiện retry hoặc phục hồi sau crash, hệ thống PHẢI tái sử dụng chính xác `telegram_random_id` này cho cùng tệp và cùng destination.
*   **FR-030**: Hệ thống PHẢI lưu trữ ánh xạ (mapping) từ `updateMessageID` hoặc response API Telegram thu được sang `telegram_message_id` trong SQLite.
*   **FR-031**: Cơ chế quét đối soát tin nhắn cũ chỉ được sử dụng làm fallback best-effort; không tuyên bố có SHA-256 từ Telegram message nếu chưa tự lưu metadata.
*   **FR-032**: Chuyển thuật ngữ "Exactly-Once" thành "Idempotent send with persisted random_id". Nếu adapter không thể bảo đảm gửi idempotent (ví dụ: do API không hỗ trợ truyền random_id), tệp đó PHẢI được đánh dấu là `reconciliation_required` và dừng lại để đối soát, không được tự động tải lên lại gây trùng lặp.

---

## 6. An toàn Đường dẫn & Manifest (Path Safety & Manifest Validation)

*   **FR-033**: Hệ thống PHẢI tự động tạo và cập nhật atomic file manifest ở định dạng JSON và CSV ghi nhận toàn bộ thông tin nguồn và đích của quá trình chuyển đổi.
*   **FR-034**: Mọi đường dẫn cục bộ (local destination) PHẢI nằm dưới thư mục backup đã được chuẩn hóa (`normalized backup root`).
*   **FR-035**: Hệ thống PHẢI thực hiện kiểm tra an toàn đường dẫn nghiêm ngặt bao gồm: chống tấn công path traversal (`../`), kiểm tra absolute paths, Windows drive/UNC paths, các ký tự/tên tệp đặc biệt của hệ điều hành, chuẩn hóa Unicode (Unicode normalization), xung đột case-insensitive, tên tệp trùng lặp và chặn symlink escape.
