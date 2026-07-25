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

## 2. Tiêu chí Thành công (Success Criteria)

Hệ thống cam kết đạt được 6 tiêu chí thành công (SC) sau:

*   **SC-001**: Hình ảnh được chuyển trực tiếp nguyên bản (passthrough) lên Telegram Drive mà không bị FFmpeg nén hoặc thay đổi chất lượng/EXIF.
*   **SC-002**: Đảm bảo an toàn tuyệt đối cho OneDrive nguồn: không có cuộc gọi xóa hoặc đổi tên tệp nguồn OneDrive nào được kích hoạt trong suốt quá trình di chuyển.
*   **SC-003**: Chống tải lên trùng lặp bằng cơ chế "Idempotent send with persisted random_id", đảm bảo zero duplicate message khi khôi phục sau crash.
*   **SC-004**: Tiết kiệm tài nguyên nhờ Deduplication chéo theo artifact target key độc lập giữa local và Telegram.
*   **SC-005**: Kiểm soát an toàn đĩa cục bộ đảm bảo thư mục tạm không bao giờ vượt quá working budget khả dụng.
*   **SC-006**: Đảm bảo hiệu năng và tính bền vững của cơ sở dữ liệu khi chịu tải/crash bằng SQLite WAL Mode và synchronous = FULL.

---

## 3. Yêu cầu Chức năng (Functional Requirements)

### 3.1. Quản lý Giao dịch & Pipeline (Pipeline & Execution Management)
*   **FR-001**: Hệ thống PHẢI hỗ trợ cơ chế pipeline song song phân tầng (overlap): file N đang upload lên Telegram, file N+1 đang được FFmpeg xử lý, file N+2 đang được tải xuống từ OneDrive.
*   **FR-002**: Hệ thống PHẢI giới hạn concurrency tối đa mặc định cho các stage: download concurrency = 2, FFmpeg transcode concurrency = 1, Telegram upload concurrency = 1.
*   **FR-003**: Hệ thống PHẢI hỗ trợ điều phối dòng dữ liệu có giới hạn (Bounded Backpressure). Xem chi tiết tại Mục 5 (Backpressure).
*   **FR-004**: Hệ thống chỉ cho phép duy nhất MỘT Job di chuyển OneDrive hoạt động tại một thời điểm.
*   **FR-005**: Hệ thống PHẢI hỗ trợ tái tạo các thư mục OneDrive thực sự rỗng dưới thư mục lưu cục bộ `OneDrive_Archive` và ghi nhận các thư mục này trong manifest.

### 3.2. Quyết định Xử lý Video (Video Processing Decision Matrix)
*   **FR-036**: Không bắt buộc tất cả video tương thích đều phải remux. Mỗi video bắt buộc được kiểm tra bằng `ffprobe`, sau đó media processor chọn chính xác một trong ba quyết định:
    1.  `transcode`: Dùng khi codec, container, kích thước, bitrate hoặc khả năng phát trên Telegram Drive cần tối ưu. Không làm giảm chất lượng video chỉ để đạt một mức nén tùy tiện.
    2.  `remux_copy`: Dùng `ffmpeg -c copy` khi codec đã phù hợp nhưng container hoặc metadata cần chuẩn hóa.
    3.  `passthrough`: Dùng file gốc khi container và codec đã phù hợp, file không có vấn đề cấu trúc, remux/transcode không tạo lợi ích thực tế, file không vượt policy kích thước và passthrough không làm giảm khả năng phát hoặc độ ổn định.
*   **FR-037**: Trường hợp áp dụng `passthrough` vẫn phải được ghi rõ trong manifest và activity log là một quyết định xử lý có chủ đích, không phải bỏ qua xử lý.

### 3.3. Nhận diện & Chống trùng lặp (Deduplication)
*   **FR-006**: Vân tay tệp tin được phân loại theo các loại sau để khớp với code hiện tại: `onedrive_quickxor`, `onedrive_sha1`, `sha256`.
*   **FR-007**: Hệ thống PHẢI sử dụng `artifact_target_key` để phân định dedupe scope độc lập:
    *   Telegram: `telegram:<destination_id-or-saved>`
    *   Local: `local:<normalized-backup-root>`
*   **FR-008**: Tiến trình claim canonical item độc nhất dựa trên khóa tổ hợp:
    `fingerprint_type + fingerprint_value + source_size + artifact_target_key`
*   **FR-009**: Tuyệt đối không được phép skip tệp local chỉ vì đã có bản sao trên Telegram. Nếu cùng nội dung cần tồn tại ở cả Telegram và local, phải tạo hai artifact tương ứng.
*   **FR-010**: Thiết kế giao dịch đặt chỗ vân tay (Dedupe Claim/Reservation):
    *   Khi file bản gốc (canonical item) đang được xử lý, các bản sao (duplicate items) phải ở trạng thái chờ (`waiting_duplicate`).
    *   If bản gốc thành công, tất cả bản sao tự động chuyển sang `skipped_duplicate` và kế thừa liên kết message ID hoặc local path của bản gốc.
    *   If bản gốc thất bại hoàn toàn (sau 3 lần retry), hệ thống PHẢI tự động chọn một bản sao khác nâng cấp lên làm bản gốc mới (promote to canonical) để tiếp tục xử lý.
*   **FR-011**: Lịch sử trùng lặp (Dedupe history) PHẢI được lưu trữ toàn cục trong SQLite (`migrated_fingerprints`) để dùng chung giữa tất cả các Job.
*   **FR-012**: Đối với file chỉ lưu cục bộ (local-only files), hệ thống chỉ được phép bỏ qua (skip) nếu tệp bản gốc cục bộ đó thực sự tồn tại ở local path tương ứng. Nếu không, phải tải lại.

### 3.4. Telegram Safety Guard & Daily Budget
*   **FR-013**: Hệ thống PHẢI tuân thủ giới hạn quota an toàn nội bộ cứng: mặc định **250.000.000.000 bytes** (khoảng 250 GB) upload thành công mỗi ngày local.
    *   Không được mô tả đây là giới hạn chính thức của Telegram.
    *   Cho phép cấu hình giá trị thấp hơn trong tương lai, nhưng không được cấu hình cao hơn mức hard cap này và không cung cấp nút bypass trên UI.
*   **FR-014**: Quota ngày PHẢI được tính dựa trên dung lượng tệp tin thực tế được gửi lên Telegram (artifact size sau transcode/remux/passthrough), không dùng dung lượng tệp gốc trên OneDrive.
*   **FR-015**: Hệ thống PHẢI thực hiện giữ trước quota (atomic reservation) ngay trước khi bắt đầu upload file. Nếu upload thành công, commit quota; nếu thất bại, giải phóng quota đã giữ.
*   **FR-016**: Hệ thống PHẢI tự động áp dụng khoảng cách tối thiểu **3 giây** giữa hai lần gửi message thành công để pacing. Sau **100 file** gửi liên tiếp, tự động cooldown **120 giây**.
    *   Pacing state tối thiểu phải gồm: `last_success_timestamp`, `sent_count_since_cooldown`, `next_allowed_at`, `batch_cooldown_until`, `flood_wait_until` và `updated_at`. Các trạng thái này phải được persist vào DB.
*   **FR-017**: Hệ thống PHẢI tự động nhận diện cả lỗi `FLOOD_WAIT_X` và `FLOOD_PREMIUM_WAIT_X` từ Telegram và dùng chúng ghi đè hoàn toàn lên local pacing. `FLOOD_WAIT` luôn có ưu tiên cao nhất.

### 3.5. Khôi phục lỗi & Dừng an toàn (Stage Recovery & Safe Pause)
*   **FR-018**: Mỗi giai đoạn xử lý PHẢI có checkpoint ghi xuống SQLite.
*   **FR-019**: Khi hệ thống khởi động lại sau crash, worker PHẢI tự động khôi phục theo quy tắc:
    *   `downloading`: Xóa file `.part` dở dang do feature sở hữu, thực hiện tải lại file hiện tại từ đầu. Không sử dụng HTTP Range resume (tính năng này được ghi lại trong mục hardening tương lai).
    *   `transcoding`: Xóa file transcode tạm, thực hiện transcode lại từ đầu.
    *   `ready_upload` / `uploading`: Sử dụng cơ chế "Idempotent send with persisted random_id" được mô tả ở Mục 6.
*   **FR-020**: Khi người dùng nhấn nút Tạm dừng (Pause), hệ thống PHẢI dừng nhận các tệp mới vào pipeline, chờ các tệp đang tải/transcode/upload dở dang hoàn thành các checkpoint hiện tại rồi mới chuyển sang trạng thái dừng an toàn (Safe Pause).

---

## 4. Quản lý Đích đến & validate Quyền Telegram (Destination Permission Preflight)

*   **FR-021**: Mặc định kênh nhận file trên Telegram là **Saved Messages** (Tin nhắn đã lưu).
*   **FR-022**: Nếu người dùng chọn kênh Telegram tùy chỉnh, destination đó PHẢI được resolve và kiểm tra quyền gửi file (send message permission) trước khi Job bắt đầu chạy.
*   **FR-023**: Khi thực hiện preflight check, hệ thống **KHÔNG ĐƯỢC PHÉP** gửi test message nháp lên channel để tránh làm phiền người dùng. Phải sử dụng API check quyền sở hữu hoặc cấu hình chat của Telegram.

---

## 5. Kiểm soát Backpressure & Hạn mức Đĩa (Disk Working Budget)

*   `disk_safety_reserve = max(5.000.000.000 bytes, 10% dung lượng filesystem)`
*   `working_budget = min(50.000.000.000 bytes, disk_free_at_start - disk_safety_reserve)`
*   **FR-024**: Hệ thống **KHÔNG ĐƯỢC PHÉP** bắt đầu Job nếu `working_budget` hiện tại không đủ chỗ cho file lớn nhất trong hàng đợi cộng với dung lượng đệm ước tính của FFmpeg transcode (headroom).
*   **FR-025**: Hệ thống PHẢI thực hiện giữ trước dung lượng đĩa (disk-byte reservation/permit) ngay trước khi bắt đầu download và transcode tệp. Việc quét kiểm tra dung lượng ổ đĩa qua filesystem chỉ đóng vai trò là lớp kiểm tra phụ.
*   **FR-026**: Dung lượng thư mục `.working` không bao giờ được phép vượt quá `working_budget`.

---

## 6. Thiết kế Chống trùng lặp/Khôi phục Upload (Idempotency Recovery)

*   **FR-027**: Loại bỏ hoàn toàn cơ chế "quét 10 message gần nhất để so sánh SHA-256" làm cơ chế chính để khôi phục.
*   **FR-028**: Hệ thống PHẢI tạo và lưu trữ bền vững một số ngẫu nhiên 64-bit `telegram_random_id` và `upload_attempt_id` vào cơ sở dữ liệu trước khi gọi API gửi file lên Telegram.
*   **FR-029**: Hàm `upload_core` nhận `telegram_random_id` và truyền trực tiếp vào API Telegram.
    *   **Lưu ý kỹ thuật**: Thư viện Grammers high-level `Client::send_message` hiện tự động sinh `random_id` nội bộ nên không thể tiêm `random_id` trực tiếp qua API này.
    *   Để sử dụng persisted `random_id`, hệ thống phải dùng raw `Client::invoke(messages::SendMedia)` hoặc một wrapper/fork được kiểm soát. Đây là API không nằm trong stability guarantee mạnh của Grammers.
    *   Thiết kế persisted `random_id` được coi là hoàn tất khi capability spike biên dịch thành công, có test adapter và có chiến lược parse/reconcile response.
    *   Chỉ cam kết zero duplicate khi adapter chứng minh contract bằng test. Nếu không bảo đảm, tệp đó PHẢI được đánh dấu là `reconciliation_required` và dừng lại để đối soát thủ công, không tự động tải lên lại gây trùng lặp.
*   **FR-030**: Hệ thống PHẢI lưu trữ ánh xạ (mapping) từ `updateMessageID` hoặc response API Telegram thu được sang `telegram_message_id` trong SQLite.
*   **FR-031**: Cơ chế quét đối soát tin nhắn cũ chỉ được sử dụng làm fallback best-effort; không tuyên bố có SHA-256 từ Telegram message nếu chưa tự lưu metadata.

---

## 7. An toàn Đường dẫn & Chính sách Manifest (Path Safety & Manifest Policy)

*   **FR-033**: Hệ thống PHẢI tự động tạo và cập nhật atomic file manifest ở định dạng JSON và CSV ghi nhận toàn bộ thông tin nguồn và đích của quá trình di chuyển.
    *   **Quy định xuất manifest**: Hệ thống không yêu cầu người dùng chọn riêng manifest export path. Database là canonical record. Manifest hướng người dùng được tự động xuất tại:
        `[local_backup_dir]/_TelegramDrive_Backup/[job_id]/manifest.json`
        `[local_backup_dir]/_TelegramDrive_Backup/[job_id]/manifest.csv`
    *   Manifest phải được ghi vào file tạm thời (temporary file) rồi thực hiện atomic rename.
    *   Nếu local backup root tạm thời không khả dụng, dữ liệu Job trong SQLite vẫn được giữ nguyên, trạng thái xuất manifest được đặt là `export_pending` để thực hiện export lại khi resume/retry. Không đánh dấu toàn bộ Job thất bại chỉ vì lỗi CSV export.
*   **FR-034**: Mọi đường dẫn cục bộ (local destination) PHẢI nằm dưới thư mục backup đã được chuẩn hóa (`normalized backup root`).
*   **FR-035**: Hệ thống PHẢI thực hiện kiểm tra an toàn đường dẫn nghiêm ngặt bao gồm: chống tấn công path traversal (`../`), kiểm tra absolute paths, Windows drive/UNC paths, các ký tự/tên tệp đặc biệt của hệ điều hành, chuẩn hóa Unicode (Unicode normalization), xung đột case-insensitive, tên tệp trùng lặp và chặn symlink escape.
