# Feature Specification: OneDrive Migration

**Feature Branch**: `001-onedrive-migration`

**Created**: 2026-07-23

**Status**: Draft

**Input**: Mô tả feature từ người dùng: "Tích hợp trực tiếp vào Telegram-Drive một tính năng cho phép người dùng chọn một thư mục OneDrive và migrate các file trong thư mục đó lên một Telegram destination."

## Ngôn ngữ

**QUAN TRỌNG**: Toàn bộ nội dung specification này được viết bằng **Tiếng Việt**. Chỉ giữ tên công nghệ, thư viện, biến/hàm/lớp bằng Tiếng Anh.

## Clarifications

### Session 2026-07-23

- Q: Migration job dùng danh sách file cố định hay tự động thêm file mới? → A: Snapshot migration — job dùng danh sách file tại thời điểm scan. Không tự thêm file mới phát sinh sau scan. Thuật ngữ nhất quán: "migration snapshot".
- Q: File OneDrive thay đổi sau khi scan thì xử lý thế nào? → A: Đánh dấu failed với error `source_changed`. Không tự thêm phiên bản mới của file đó.
- Q: Duplicate detection hoạt động ra sao? → A: Theo nội dung. Ưu tiên OneDrive fingerprint (nếu API cung cấp), fallback SHA-256 tính từ file local. Chỉ fingerprint của file upload **thành công** mới vào duplicate history. Dùng chung duplicate history giữa các job. File đầu tiên được upload, các file sau có cùng fingerprint → `skipped_duplicate`.
- Q: Phát hiện duplicate trước hay sau download? → A: Nếu OneDrive fingerprint có sẵn từ metadata → skip trước download, không cần tải file. Nếu phải tính SHA-256 từ nội dung → download về local rồi mới phát hiện trùng.
- Q: Khi nào coi là upload thành công? → A: Upload thành công khi logic upload hiện có của Telegram-Drive trả về success. Message ID từ Telegram là optional, không bắt buộc phải có.
- Q: Pause hoạt động thế nào? → A: Pause after current file — file đang xử lý được hoàn tất, sau đó dừng. Không dừng giữa chừng file.
- Q: Cancel hoạt động thế nào? → A: Không bắt đầu file mới. Giữ nguyên lịch sử các file đã completed. Không xóa dữ liệu đã upload lên Telegram. Trạng thái job chuyển thành `cancelled`.
- Q: Restart/resume sau crash hoặc đóng ứng dụng? → A: File đang downloading/uploading dở → `failed` hoặc `pending`. Resume thủ công (người dùng nhấn "Tiếp tục"). Không auto-start. MVP cung cấp at-least-once attempt, **không** đảm bảo exactly-once khi crash giữa upload và persist.
- Q: Retry hoạt động thế nào? → A: Tối đa 3 lần retry tự động cho lỗi tạm thời (mạng, timeout). Sau 3 lần → `failed`. Manual retry (người dùng nhấn) reset counter về 0.
- Q: Telegram cooldown/flood control xử lý ra sao? → A: Dừng upload, hiển thị trạng thái cooldown kèm thời gian chờ, không bypass. Tự động tiếp tục sau khi hết cooldown. Persist trạng thái cooldown để không mất khi restart.
- Q: File vượt giới hạn kích thước Telegram? → A: Không hardcode giới hạn kích thước. Nếu upload bị từ chối do file quá lớn (upload adapter trả `telegram_file_too_large`) → `failed`, không retry tự động.
- Q: Local working directory quản lý thế nào? → A: Một directory cho mỗi job. Nếu directory unavailable (không tồn tại, không writable) → dừng job, hiển thị lỗi, cho người dùng chọn lại.
- Q: File tạm local xóa khi nào? → A: Xóa sau upload thành công hoặc sau khi phát hiện duplicate (đã download về). Giữ lại để retry nếu file gặp lỗi.
- Q: Có thể chạy nhiều job cùng lúc không? → A: Chỉ một job `running` tại một thời điểm. Có thể lưu nhiều job trong history (completed, cancelled, failed).
- Q: Có tái tạo cây thư mục trên Telegram không? → A: Không. Tất cả file upload vào cùng Telegram destination đã chọn. Lưu source relative path trong migration history để tham khảo.

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Thiết lập và scan (Priority: P1)

Người dùng mở mục "OneDrive Migration" từ navigation của Telegram-Drive, kết nối tài khoản Microsoft, chọn một thư mục OneDrive làm nguồn, chọn Telegram destination để upload file đến, chọn một thư mục local làm nơi làm việc tạm, sau đó quét thư mục nguồn để xem danh sách thư mục, danh sách file và thống kê tổng quan.

**Why this priority**: Không có setup thì không thể chạy migration. Đây là bước tiên quyết, cung cấp cho người dùng cái nhìn tổng quan về dữ liệu sẽ được migrate và xác nhận mọi kết nối hoạt động.

**Independent Test**: Có thể kiểm tra độc lập bằng cách hoàn tất toàn bộ luồng setup và xác nhận danh sách file cùng thống kê hiển thị chính xác so với nội dung thực tế trên OneDrive. Không cần bắt đầu migration để xác minh.

**Acceptance Scenarios**:

1. **Given** người dùng chưa kết nối Microsoft, **When** nhấn nút kết nối và hoàn tất đăng nhập Microsoft, **Then** trạng thái kết nối hiển thị "Đã kết nối" kèm tên tài khoản.
2. **Given** đã kết nối Microsoft, **When** người dùng chọn một thư mục OneDrive, **Then** đường dẫn thư mục nguồn được hiển thị.
3. **Given** đã chọn thư mục nguồn, Telegram destination và thư mục local, **When** người dùng nhấn "Quét", **Then** hệ thống hiển thị danh sách thư mục con (kèm số file và tổng dung lượng), danh sách file (tên, đường dẫn tương đối, dung lượng), và thống kê tổng quan (tổng thư mục, tổng file, tổng dung lượng).
4. **Given** thư mục OneDrive nguồn rỗng, **When** quét, **Then** hiển thị thống kê với 0 file và thông báo phù hợp, không báo lỗi.
5. **Given** người dùng đã kết nối Microsoft, **When** nhấn "Ngắt kết nối", **Then** trạng thái chuyển về "Chưa kết nối" và thông tin thư mục nguồn được xóa.

---

### User Story 2 — Chạy migration (Priority: P1)

Người dùng sau khi scan xong nhấn "Bắt đầu", hệ thống xử lý tuần tự từng file: download từ OneDrive về thư mục local tạm, upload lên Telegram destination đã chọn, xóa file tạm, rồi chuyển sang file tiếp theo. Người dùng theo dõi được tiến trình: file đang xử lý, tiến độ download, tiến độ upload, trạng thái từng file.

**Why this priority**: Đây là chức năng cốt lõi của feature — thực hiện migration thực tế. Cùng P1 với setup vì không có migration thì setup không có giá trị.

**Independent Test**: Có thể kiểm tra bằng cách thiết lập một thư mục OneDrive nhỏ (vài file), bắt đầu migration và xác nhận từng file được download, upload thành công, file tạm bị xóa, và trạng thái file chuyển thành "completed".

**Acceptance Scenarios**:

1. **Given** đã scan và có danh sách file pending, **When** người dùng nhấn "Bắt đầu", **Then** hệ thống bắt đầu xử lý file đầu tiên: trạng thái chuyển từ pending → downloading → uploading → completed.
2. **Given** migration đang chạy, **When** một file upload thành công, **Then** file tạm local bị xóa, trạng thái file đó thành "completed", và file tiếp theo bắt đầu được xử lý.
3. **Given** migration đang chạy, **When** người dùng nhấn "Tạm dừng", **Then** hệ thống hoàn tất file đang xử lý rồi dừng, các file completed giữ nguyên trạng thái, file pending chưa xử lý vẫn là pending.
4. **Given** migration đang tạm dừng, **When** người dùng nhấn "Tiếp tục", **Then** hệ thống tiếp tục xử lý từ file pending tiếp theo.
5. **Given** migration đang chạy, **When** người dùng nhấn "Hủy", **Then** hệ thống không bắt đầu file mới, file đang xử lý được hoàn tất hoặc dừng, các file pending còn lại không được xử lý, job chuyển trạng thái `cancelled`, giữ nguyên lịch sử file completed và dữ liệu đã upload.
6. **Given** file vượt quá giới hạn kích thước của Telegram, **When** đến lượt xử lý file đó, **Then** hệ thống download và thử upload, khi upload adapter trả lỗi `telegram_file_too_large` → đánh dấu `failed`, không retry tự động. Hệ thống chuyển sang file tiếp theo.
7. **Given** file có OneDrive fingerprint trùng với duplicate history, **When** đến lượt xử lý file đó, **Then** file bị skip trước khi download, đánh dấu `skipped_duplicate`, không tải file về local.
8. **Given** file không có OneDrive fingerprint và phải tính SHA-256 sau download, **When** download xong và fingerprint trùng duplicate history, **Then** file bị skip upload, đánh dấu `skipped_duplicate`, file tạm local bị xóa.
9. **Given** Telegram trả về thời gian chờ (cooldown) trong lúc upload, **When** đang xử lý file, **Then** hệ thống dừng upload, hiển thị trạng thái cooldown kèm thời gian chờ, tự động tiếp tục upload file đó sau khi hết cooldown.
10. **Given** file OneDrive đã thay đổi (kích thước hoặc hash) so với lúc scan, **When** đến lượt download file đó, **Then** file được đánh dấu `failed` với error `source_changed`, không upload, chuyển sang file tiếp theo.
11. **Given** file đã upload thành công và trạng thái completed đã được persist, **When** kiểm tra thư mục local, **Then** file tạm tương ứng đã bị xóa.
12. **Given** migration đã hoàn tất hoặc bị hủy, **When** kiểm tra thư mục OneDrive nguồn, **Then** không có file nào bị xóa, đổi tên hoặc thay đổi nội dung.

---

### User Story 3 — Resume, retry và phát hiện trùng (Priority: P2)

Người dùng đóng ứng dụng khi migration đang chạy. Khi mở lại, job migration vẫn hiển thị với danh sách file và trạng thái cũ. File đã hoàn thành không bị upload lại. File pending tiếp tục được xử lý khi nhấn "Tiếp tục". Người dùng có thể retry file bị lỗi. Hệ thống tự động phát hiện file trùng nội dung và bỏ qua, không upload lại.

**Why this priority**: Quan trọng cho trải nghiệm thực tế (migration có thể mất nhiều giờ) nhưng không chặn luồng cơ bản. Người dùng vẫn có thể hoàn tất migration nhỏ mà không cần resume/retry.

**Independent Test**: Có thể kiểm tra bằng cách bắt đầu migration với vài file, đóng ứng dụng giữa chừng, mở lại và xác nhận trạng thái được giữ nguyên, nhấn "Tiếp tục" để hoàn tất các file còn lại.

**Acceptance Scenarios**:

1. **Given** migration job đang có file completed và file pending, **When** ứng dụng được khởi động lại, **Then** job vẫn hiển thị, file completed giữ nguyên trạng thái, file pending vẫn là pending, người dùng có thể nhấn "Tiếp tục".
2. **Given** có file failed trong job, **When** người dùng nhấn "Retry" trên file đó, **Then** file được đưa về trạng thái pending và được xử lý lại.
3. **Given** có nhiều file failed, **When** người dùng nhấn "Retry tất cả file lỗi", **Then** tất cả file failed được đưa về pending và xử lý lại tuần tự.
4. **Given** một file đã được upload thành công trước đó, **When** một file khác có cùng nội dung (cùng content fingerprint) đến lượt xử lý, **Then** file bị bỏ qua với trạng thái "skipped_duplicate", không upload lên Telegram.
5. **Given** hai file cùng tên nhưng khác nội dung, **When** đến lượt xử lý, **Then** cả hai đều được upload bình thường (không bị coi là trùng).
6. **Given** thư mục local không còn tồn tại hoặc không ghi được, **When** hệ thống chuẩn bị xử lý file tiếp theo, **Then** hiển thị lỗi rõ ràng, không tự chuyển sang thư mục khác, cho phép người dùng chọn lại thư mục.
7. **Given** file có OneDrive fingerprint trùng duplicate history, **When** quét hoặc chuẩn bị xử lý file đó, **Then** duplicate được phát hiện và skip trước download (không cần tải file).
8. **Given** file không có OneDrive fingerprint và phải tính SHA-256 sau download, **When** download xong và fingerprint trùng duplicate history, **Then** skip upload, đánh dấu `skipped_duplicate`, xóa file tạm.

---

### Edge Cases

- **Thư mục nguồn rỗng**: Hệ thống hiển thị thống kê 0 file, không có lỗi, nút "Bắt đầu" bị vô hiệu hóa hoặc hiển thị thông báo phù hợp.
- **Microsoft authentication hết hạn**: Khi token hết hạn trong lúc migration, hệ thống dừng xử lý file hiện tại, hiển thị lỗi xác thực, cho phép người dùng kết nối lại và tiếp tục.
- **Thư mục local không writable**: Kiểm tra trước khi bắt đầu mỗi file, nếu không ghi được thì dừng job, hiển thị lỗi, cho phép chọn thư mục khác.
- **Không đủ dung lượng local**: Trước khi download từng file, kiểm tra dung lượng trống. Nếu không đủ, dừng job, hiển thị cảnh báo.
- **File vượt giới hạn Telegram**: Không hardcode giới hạn kích thước. Nếu upload adapter trả lỗi `telegram_file_too_large` → `failed`, không retry tự động. Kèm thông báo rõ ràng.
- **File thay đổi sau scan (`source_changed`)**: Nếu file OneDrive thay đổi (kích thước, hash, thời gian sửa) so với lúc scan → đánh dấu `failed` với error `source_changed`. Không tự thêm phiên bản mới.
- **Download từ OneDrive thất bại**: File được đánh dấu "failed", lưu thông báo lỗi, hệ thống tiếp tục với file tiếp theo. Tự động retry tối đa 3 lần cho lỗi tạm thời; sau đó yêu cầu retry thủ công.
- **Upload lên Telegram thất bại**: Tương tự download thất bại — đánh dấu failed, lưu lỗi, auto-retry 3 lần cho lỗi tạm thời, tiếp tục file kế tiếp.
- **Telegram yêu cầu thời gian chờ (cooldown)**: Hệ thống dừng upload, hiển thị trạng thái cooldown kèm thời gian chờ, không bypass. Tự động tiếp tục sau cooldown. Persist trạng thái cooldown.
- **Ứng dụng restart giữa chừng khi đang download hoặc upload**: File đang xử lý dở → `failed` hoặc `pending`. File đã hoàn thành không bị ảnh hưởng. Người dùng nhấn "Tiếp tục" để xử lý tiếp. MVP cung cấp at-least-once; không đảm bảo exactly-once khi crash giữa upload và persist.
- **File trùng nội dung nhưng khác tên hoặc khác thư mục**: Ưu tiên OneDrive fingerprint nếu có → skip trước download. Nếu phải tính SHA-256 từ nội dung → download rồi phát hiện trùng. Đánh dấu `skipped_duplicate`. Chỉ fingerprint của file upload thành công mới vào duplicate history.
- **File cùng tên nhưng nội dung khác**: Hệ thống upload cả hai file bình thường, không coi là trùng.
- **File tạm local còn sót lại sau crash**: File tạm không được dọn kịp khi ứng dụng bị tắt đột ngột. Khi khởi động lại, hệ thống có thể nhận biết và dọn dẹp, hoặc ghi đè khi xử lý lại file đó.
- **Cây thư mục trên Telegram**: MVP không tái tạo cây thư mục OneDrive trên Telegram. Tất cả file upload vào cùng Telegram destination. Source relative path được lưu trong history.

## Requirements *(mandatory)*

### Functional Requirements

#### Kết nối và thiết lập

- **FR-001**: Hệ thống PHẢI cho phép người dùng kết nối một tài khoản Microsoft qua OAuth để truy cập OneDrive.
- **FR-002**: Hệ thống PHẢI hiển thị trạng thái kết nối Microsoft (đã kết nối / chưa kết nối) kèm tên tài khoản khi đã kết nối.
- **FR-003**: Hệ thống PHẢI cho phép người dùng ngắt kết nối tài khoản Microsoft, xóa thông tin thư mục nguồn đã chọn.
- **FR-004**: Hệ thống PHẢI cho phép người dùng chọn một thư mục OneDrive làm nguồn migration.
- **FR-005**: Hệ thống PHẢI cho phép người dùng chọn một Telegram destination để nhận file upload. Tất cả file được upload vào cùng một destination, không tái tạo cây thư mục OneDrive trên Telegram. Source relative path của mỗi file được lưu trong migration history.
- **FR-006**: Hệ thống PHẢI cho phép người dùng chọn một thư mục local làm nơi làm việc tạm (tải file từ OneDrive về, rồi upload lên Telegram).

#### Quét và hiển thị

- **FR-007**: Hệ thống PHẢI quét đệ quy thư mục OneDrive nguồn để lấy danh sách tất cả thư mục con và file.
- **FR-008**: Hệ thống PHẢI hiển thị danh sách thư mục với đường dẫn tương đối, số lượng file trong thư mục, và tổng dung lượng.
- **FR-009**: Hệ thống PHẢI hiển thị danh sách file với tên file, đường dẫn tương đối, dung lượng, và trạng thái migration.
- **FR-010**: Hệ thống PHẢI hiển thị thống kê tổng quan: tổng số thư mục, tổng số file, tổng dung lượng, số file theo từng trạng thái (pending, completed, skipped_duplicate, failed), và dung lượng đã migrate.

#### Xử lý migration

- **FR-011**: Hệ thống PHẢI xử lý migration tuần tự — mỗi thời điểm chỉ download hoặc upload một file.
- **FR-012**: Hệ thống PHẢI lưu bền vững trạng thái migration job và danh sách file, đảm bảo không mất dữ liệu sau khi ứng dụng restart.
- **FR-013**: Hệ thống PHẢI cho phép người dùng resume thủ công migration job sau khi ứng dụng được khởi động lại.
- **FR-014**: Hệ thống PHẢI phát hiện file trùng dựa trên content fingerprint: ưu tiên dùng hash từ OneDrive metadata (nếu API cung cấp), fallback SHA-256 tính từ file local sau download. Nếu OneDrive fingerprint có sẵn → skip trước download. Nếu phải tính SHA-256 → download rồi mới phát hiện.
- **FR-015**: Hệ thống PHẢI bỏ qua file có content fingerprint đã tồn tại trong duplicate history của các lần migration thành công trước đó (dùng chung giữa các job). Chỉ fingerprint của file upload thành công mới được thêm vào duplicate history. File đầu tiên có fingerprint mới được upload, các file sau có cùng fingerprint → `skipped_duplicate`.
- **FR-016**: Hệ thống PHẢI hiển thị tiến trình migration: tên file đang xử lý, tiến độ download, tiến độ upload, và trạng thái hiện tại.
- **FR-017**: Hệ thống PHẢI cho phép người dùng retry file lỗi — từng file riêng lẻ hoặc tất cả file failed cùng lúc.
- **FR-018**: Hệ thống PHẢI cho phép người dùng Pause (tạm dừng sau file hiện tại), Resume (tiếp tục), và Cancel (hủy) migration job.
- **FR-019**: Hệ thống PHẢI tôn trọng thời gian chờ từ Telegram (cooldown/flood control) — dừng upload, hiển thị trạng thái cooldown kèm thời gian chờ, tự động tiếp tục sau khi hết cooldown. Persist trạng thái cooldown.
- **FR-020**: Hệ thống PHẢI xóa file tạm local sau khi upload lên Telegram thành công hoặc sau khi phát hiện duplicate (đã download về). Giữ file tạm để retry nếu file gặp lỗi.
- **FR-021**: Hệ thống KHÔNG ĐƯỢC xóa hoặc thay đổi file nguồn trên OneDrive dưới bất kỳ hình thức nào.
- **FR-022**: Hệ thống PHẢI giới hạn chỉ một migration job chạy tại một thời điểm.
- **FR-023**: Hệ thống PHẢI kiểm tra thư mục local tồn tại và writable trước khi xử lý mỗi file. Nếu không đạt, dừng job và hiển thị lỗi.
- **FR-024**: Hệ thống PHẢI xử lý lỗi từng file độc lập — khi một file gặp lỗi, tự động retry tối đa 3 lần cho lỗi tạm thời (mạng, timeout). Sau 3 lần → đánh dấu `failed`, lưu thông báo lỗi, và tiếp tục với file tiếp theo. Manual retry (người dùng nhấn) reset counter về 0.
- **FR-025**: Hệ thống PHẢI lưu thông báo lỗi cho mỗi file failed để người dùng có thể xem và quyết định retry. Các loại lỗi bao gồm: `source_changed` (file thay đổi sau scan), lỗi mạng, lỗi xác thực, vượt giới hạn Telegram, và lỗi không xác định.
### Non-Functional Requirements

- **NFR-001 (Memory)**: Ứng dụng không được đọc toàn bộ file lớn vào RAM. Download và upload phải hoạt động theo stream hoặc cơ chế hiện có với mức sử dụng bộ nhớ bị giới hạn.
- **NFR-002 (Concurrency)**: Chỉ một file được download hoặc upload bởi migration tại một thời điểm.
- **NFR-003 (Persistence)**: Migration job, danh sách file, trạng thái từng file và duplicate history phải tồn tại sau khi ứng dụng restart.
- **NFR-004 (Completed-file safety)**: File đã `completed` hoặc `skipped_duplicate` không được tự động xử lý lại sau restart hoặc Resume.
- **NFR-005 (Security)**: Không được log Microsoft access token, Microsoft refresh token, Authorization header, Telegram session hoặc auth key.
- **NFR-006 (UI responsiveness)**: Quá trình scan, download và upload không được khóa main UI thread. Người dùng vẫn có thể xem progress và sử dụng các control của migration.
- **NFR-007 (Platform)**: MVP chỉ áp dụng cho desktop platforms hiện được Telegram-Drive hỗ trợ (Windows, macOS, Linux). Không bao gồm Android hoặc iOS.

### Key Entities

- **Migration Job**: Đại diện cho một lần migration. Chứa: trạng thái job (`draft`, `ready`, `running`, `paused`, `completed`, `failed`, `cancelled`), thư mục OneDrive nguồn, Telegram destination, thư mục local, migration snapshot (danh sách file cố định tại thời điểm scan), thời gian tạo, thời gian cập nhật. Mỗi thời điểm chỉ có tối đa một job ở trạng thái `running`. Có thể lưu nhiều job history (completed, cancelled, failed).
- **Migration File**: Một file trong job migration. Chứa: tên file, đường dẫn tương đối từ thư mục nguồn (source relative path), dung lượng, trạng thái (`pending`, `downloading`, `uploading`, `completed`, `skipped_duplicate`, `failed`), content fingerprint, loại lỗi (nếu failed: `source_changed`, `network`, `auth`, `telegram_file_too_large`, `unknown`), thông báo lỗi chi tiết, retry count, Telegram message identifier (optional, nếu upload thành công), thời gian hoàn thành.
- **Migration Folder**: Một thư mục trong cây thư mục nguồn. Chứa: đường dẫn tương đối, số lượng file, tổng dung lượng.
- **Microsoft Connection**: Trạng thái kết nối với tài khoản Microsoft. Chứa: trạng thái kết nối, tên tài khoản, thời gian kết nối.
- **Duplicate Record**: Lịch sử file đã migrate thành công. Chứa: content fingerprint, Telegram destination, thời gian migrate.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Người dùng có thể hoàn tất toàn bộ luồng thiết lập (kết nối Microsoft, chọn thư mục nguồn, chọn Telegram destination, chọn thư mục local) và quét danh sách file mà không cần sửa bất kỳ file cấu hình thủ công nào.
- **SC-002**: Tổng số file và tổng dung lượng hiển thị sau khi quét khớp chính xác với kết quả quét từ OneDrive (sai lệch 0%).
- **SC-003**: Một migration job có thể xử lý tuần tự ít nhất 100 file mà không crash hoặc mất dữ liệu trạng thái.
- **SC-004**: Sau khi ứng dụng restart, tất cả file đã "completed" không bị upload lại lần nữa.
- **SC-005**: Hai file nguồn có cùng nội dung (cùng content fingerprint) chỉ tạo ra đúng một file upload lên Telegram.
- **SC-006**: Người dùng có thể retry riêng một file failed mà không ảnh hưởng đến các file khác.
- **SC-007**: Khi tạm dừng migration, tất cả file đã "completed" giữ nguyên trạng thái và không bị mất.
- **SC-008**: File tạm local được xóa khỏi thư mục làm việc trong vòng 5 giây sau khi upload thành công.
- **SC-009**: Sau khi migration hoàn tất (hoặc bị hủy), không có file nào trên OneDrive nguồn bị xóa, đổi tên, hoặc thay đổi nội dung.
- **SC-010**: Giao diện người dùng vẫn phản hồi (có thể tương tác, không bị đơ quá 2 giây) trong suốt quá trình migration chạy.

## Assumptions

- Người dùng đã có tài khoản Microsoft với quyền truy cập OneDrive.
- Người dùng đã đăng nhập Telegram trong Telegram-Drive và có session hợp lệ.
- Telegram destination picker đã có sẵn trong Telegram-Drive và được tái sử dụng.
- Upload logic hiện tại của Telegram-Drive (Telegram MTProto) được tái sử dụng cho việc upload file trong migration.
- Database/persistence pattern hiện tại của Telegram-Drive (SQLite) được tái sử dụng. Migration job có thể dùng database riêng (`migration.db`) nếu cần.
- MVP chỉ hỗ trợ desktop (Windows, macOS, Linux). Mobile (Android/iOS) nằm ngoài phạm vi.
- Người dùng có kết nối Internet ổn định trong suốt quá trình migration.
- Một file được coi là "đã migrate thành công" khi upload lên Telegram hoàn tất và nhận được xác nhận từ Telegram API.
- Content fingerprint ưu tiên dùng hash do OneDrive cung cấp (nếu API trả về). Nếu không có, SHA-256 được tính từ file local sau khi download. Nếu OneDrive fingerprint có sẵn, duplicate được phát hiện trước download, tiết kiệm băng thông.
- Sau khi migration bắt đầu, danh sách file được cố định dựa trên migration snapshot lúc scan. Người dùng có thể scan lại để cập nhật danh sách trước khi bắt đầu, tạo snapshot mới.
- MVP cho phép tối đa 3 lần retry tự động cho lỗi mạng tạm thời. Manual retry reset counter. Các lỗi khác (source_changed, telegram_file_too_large, auth) không retry tự động.

## Scope

### In Scope

- Tích hợp vào Telegram-Drive desktop như một tính năng gốc trong navigation/sidebar
- Đăng nhập Microsoft qua OAuth
- Chọn một thư mục OneDrive làm nguồn
- Quét đệ quy thư mục nguồn
- Hiển thị danh sách thư mục và file dạng bảng/list cơ bản
- Hiển thị thống kê file và tổng dung lượng
- Chọn thư mục local làm nơi làm việc tạm qua native folder picker (một directory cho mỗi job)
- Download tuần tự từng file từ OneDrive
- Upload tuần tự từng file lên Telegram (tái sử dụng logic upload hiện có). Upload được coi là thành công khi logic hiện có trả về success; Telegram message ID là optional.
- Hiển thị tiến trình cơ bản (file hiện tại, download, upload, trạng thái)
- Lưu bền vững migration job và danh sách file
- Resume thủ công sau khi ứng dụng restart (không auto-start; MVP cung cấp at-least-once)
- Phát hiện file trùng theo content fingerprint (OneDrive fingerprint ưu tiên, fallback SHA-256)
- Bỏ qua file trùng (skip duplicate): nếu fingerprint có sẵn → skip trước download; nếu phải tính SHA-256 → download rồi phát hiện
- Retry file lỗi: tự động tối đa 3 lần cho lỗi tạm thời, sau đó thủ công
- Pause/Resume/Cancel migration
- Giới hạn một job chạy tại một thời điểm
- Giới hạn một file xử lý tại một thời điểm (không upload đồng thời)
- Không tái tạo cây thư mục OneDrive trên Telegram (tất cả file vào cùng destination, lưu source relative path)
- Tái sử dụng Telegram account/session, destination picker, upload logic, notification/toast, database pattern, UI component và design system hiện có

### Out of Scope

- Xóa file nguồn trên OneDrive
- Giám sát liên tục thư mục OneDrive (continuous monitoring)
- Tự động phát hiện file mới sau khi job đã bắt đầu
- Local backup lâu dài (file tạm bị xóa sau upload)
- Chạy như OS service hoặc sidecar worker độc lập
- Autostart cùng hệ điều hành
- System tray hoặc close-to-tray
- Hỗ trợ nhiều tài khoản Microsoft cùng lúc
- Nhiều migration job chạy đồng thời
- Nhiều file upload đồng thời
- Advanced adaptive flood guard (chỉ xử lý cooldown cơ bản)
- Exactly-once reconciliation phức tạp
- UI xử lý xung đột Telegram duplicate-message
- External drive volume identity hoặc auto remount detection
- Advanced metrics hoặc soak test 72 giờ
- Mobile (Android/iOS)
- SharePoint hoặc Microsoft Teams
- Các cloud provider khác (Google Drive, Dropbox, v.v.)
