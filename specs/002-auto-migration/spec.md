# Feature Specification: Automated Account Migration (Smart Auto-Sync)

**Feature Directory**: `specs/002-auto-migration`
**Created**: 2026-07-23
**Status**: Proposal / Design Review

---

## Executive Summary
Giao diện và cơ chế migration hiện tại còn quá nhiều thao tác thủ công (tạo job, chọn thư mục nguồn, chọn Telegram destination, chọn thư mục làm việc tạm, bấm scan, bấm start).

**Smart Auto-Migration (Tự Động Hóa 100%)** được thiết kế để biến quá trình chuyển đổi dữ liệu thành một trải nghiệm **Zero-Click / One-Click**:
1. Kết nối tài khoản Microsoft -> Hệ thống tự động thiết lập thông số mặc định (Default Destination, Default Temp Folder).
2. Công tắc duy nhất: **"Bật Tự Động Migrate & Đồng Bộ"** (Enable Auto-Migration).
3. Hệ thống tự động quét ngầm, lập danh sách snapshot, và xử lý chuyển dữ liệu liên tục theo tài khoản mà không cần người dùng thao tác từng bước.
4. **Bảo vệ chống Spam (Anti-Spam Guardrail)**: Tự động theo dõi dung lượng upload trong ngày. Tự động tạm dừng (Pause) quá trình upload nếu đạt ngưỡng **250GB/ngày** để bảo vệ tài khoản Telegram khỏi việc bị đánh dấu spam và khóa limit.
5. Lưu cấu hình tự động vào cơ sở dữ liệu (`migration.db`), đảm bảo tự động chạy ngầm mỗi khi ứng dụng được khởi động.

---

## Ngôn ngữ
**QUAN TRỌNG**: Toàn bộ nội dung specification này được viết bằng **Tiếng Việt**.

---

## User Scenarios & Testing

### User Story 1 — Kích hoạt Tự Động Migrate (One-Click Auto-Migration)
Người dùng sau khi bấm "Connect Microsoft" chỉ cần bật công tắc **"Tự Động Migrate"**. Hệ thống tự động nhận diện thư mục mặc định trên OneDrive và Telegram Destination mặc định, tự động quét và chạy ngầm mà người dùng không phải thiết lập job hay bấm các nút scan/start thủ công.

**Acceptance Scenarios**:
1. **Given** người dùng đã kết nối tài khoản Microsoft, **When** bật công tắc "Tự Động Migrate", **Then** hệ thống tự tạo Auto-Migration Profile trong database, tự chọn thư mục làm việc tạm mặc định và bắt đầu tự động quét + migrate ngầm.
2. **Given** Auto-Migration đang bật, **When** người dùng mở ứng dụng Telegram Drive ở các lần sau, **Then** hệ thống tự động kiểm tra tài khoản, khôi phục tiến trình tự động mà không yêu cầu tương tác.
3. **Given** Auto-Migration đang bật, **When** người dùng tắt công tắc, **Then** hệ thống tạm dừng các tiến trình ngầm và lưu trạng thái `paused`.

---

### User Story 2 — Giao Diện Đơn Giản Hóa (Smart Auto-Migration Dashboard)
Giao diện thay thế trang thiết lập phức tạp bằng một **Trung Tâm Tự Động Migrate (Auto-Migration Center)** tối giản:
- **Master Switch**: Công tắc lớn "Bật/Tắt Tự Động Migrate".
- **Daily Quota Status (Giới hạn 250GB/ngày)**: Hiển thị bộ đếm dung lượng đã upload trong ngày hôm nay. Cảnh báo nếu chạm mốc 250GB.
- **Account Cards**: Thẻ hiển thị tài khoản Microsoft đã kết nối kèm nhãn "Đang tự động đồng bộ ngầm".
- **Live Activity Stream & Real-time Progress**: Nhật ký hoạt động tối giản dạng dòng thời gian, kết hợp với **thanh tiến trình trực quan (Visual Progress Bar)** hiển thị rõ ràng phần trăm download từ OneDrive và upload lên Telegram của file đang được xử lý hiện tại. Giao diện phải mượt mà, dễ nhìn, không gây rối mắt nhưng vẫn đủ thông tin để theo dõi tiến độ.
- **Advanced Drawer (Tùy chọn nâng cao)**: Bảng trượt ẩn cho phép đổi kênh Telegram nhận file hoặc thư mục tạm nếu muốn.

**Acceptance Scenarios**:
1. **Given** người dùng ở giao diện Auto-Migration, **When** mở ứng dụng, **Then** thấy ngay thẻ tài khoản, trạng thái Master Switch và dòng nhật ký các file vừa được tự động xử lý.
2. **Given** người dùng muốn đổi kênh Telegram mặc định, **When** mở phần "Tùy chọn nâng cao", **Then** có thể chọn kênh khác và hệ thống tự động lưu vào database.

---

## Requirements

### Functional Requirements

- **FR-001**: Hệ thống PHẢI lưu cấu hình tự động migrate theo từng tài khoản (`auto_migration_profiles`) vào cơ sở dữ liệu `migration.db`.
- **FR-002**: Hệ thống PHẢI tự động chọn thư mục làm việc tạm mặc định (ví dụ: `[AppData]/TelegramDrive/temp_migration`) nếu người dùng không tự chọn.
- **FR-003**: Hệ thống PHẢI tự động chọn Telegram Destination mặc định (hoặc kênh/saved messages được dùng gần nhất) nếu người dùng không chỉ định.
- **FR-004**: Hệ thống PHẢI cung cấp công tắc Master Switch (Bật/Tắt Tự Động Migrate) trên giao diện frontend.
- **FR-005**: Khi Master Switch bật, backend PHẢI tự động tạo job, quét thư mục nguồn và kích hoạt migration worker hoàn toàn tự động mà không cần bấm nút Scan hay Start thủ công.
- **FR-006**: Hệ thống PHẢI duy trì chế độ tự động chạy ngầm mỗi khi ứng dụng Telegram Drive được mở lên.
- **FR-007**: Giao diện PHẢI hiển thị danh sách nhật ký hoạt động ngầm (Live Activity Stream) kết hợp với **tiến trình thời gian thực (Real-time Progress)** của file đang xử lý (tiến độ download, tiến độ upload), đảm bảo trực quan và dễ theo dõi.
- **FR-008 (Anti-Spam Limit)**: Backend PHẢI theo dõi tổng dung lượng upload lên Telegram trong ngày hiện tại. Nếu tổng dung lượng đạt **250GB**, hệ thống PHẢI tự động tạm dừng tiến trình migrate ngầm, và chỉ tiếp tục vào ngày tiếp theo để chống spam. Giao diện phải hiển thị mức sử dụng quota này.

### Non-Functional Requirements

- **NFR-001 (Zero Hassle)**: Số bước thao tác tối đa của người dùng để bắt đầu migrate từ tài khoản mới là **1 bước** (Bật công tắc).
- **NFR-002 (Silent Background Execution)**: Các tác vụ quét và migrate ngầm không làm đơ giao diện hay chắn thao tác của người dùng trên ứng dụng Desktop.
- **NFR-003 (Persistence)**: Cấu hình tự động và nhật ký hoạt động được lưu bền vững trong SQLite.

---

## Success Criteria

- **SC-001**: Người dùng bắt đầu quá trình migrate toàn bộ tài khoản OneDrive chỉ với 1 nhấp chuột duy nhất.
- **SC-002**: Khi khởi động lại ứng dụng, hệ thống tự động nhận diện tài khoản và tiếp tục tiến trình migrate ngầm mà không cần thao tác lại.
- **SC-003**: Giao diện trực quan, tối giản, loại bỏ 100% các nút bấm rườm rà (Create Job, Scan, Manual Start, Picker trùng lặp).
