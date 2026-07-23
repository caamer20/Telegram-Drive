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
3. Khi chưa có snapshot, hệ thống tự động quét OneDrive root một lần, thêm tất cả file vào danh sách theo thứ tự ổn định và xử lý tuần tự. Khi đã có danh sách, hệ thống không tự động quét lại.
4. **Bảo vệ chống Spam (Anti-Spam Guardrail)**: Tự động theo dõi dung lượng upload trong ngày. Tự động tạm dừng (Pause) quá trình upload nếu đạt ngưỡng **250GB/ngày** để bảo vệ tài khoản Telegram khỏi việc bị đánh dấu spam và khóa limit.
5. Lưu bền vững cấu hình tự động trong vùng dữ liệu riêng của ứng dụng, đảm bảo có thể khôi phục mỗi khi ứng dụng được khởi động.

---

## Clarifications

### Session 2026-07-23

- Q: Auto Migration sử dụng phạm vi nguồn OneDrive nào? → A: Toàn bộ OneDrive root của tài khoản Microsoft đang kết nối.
- Q: Auto Sync có tự động quét lại để thêm file mới không? → A: Không. Hệ thống dùng snapshot hiện có và xử lý tuần tự; người dùng chủ động nhấn "Quét lại" khi muốn tạo danh sách mới.
- Q: Microsoft token được xử lý thế nào sau khi ứng dụng restart? → A: Persist token vào vùng dữ liệu riêng của ứng dụng trên disk để tự khôi phục phiên; token không được nằm trong repository, không được commit lên Git và bị xóa khi người dùng Disconnect.
- Q: Daily quota 250GB được tính và reset như thế nào? → A: Chỉ tính file Auto Migration upload thành công; kiểm tra tổng dự kiến trước mỗi file, pause trước file làm vượt ngưỡng và reset lúc 00:00 theo giờ local của máy.
- Q: Telegram destination mặc định là gì? → A: Saved Messages; người dùng chỉ cần mở Advanced Settings khi muốn chọn destination khác.

---

## Ngôn ngữ
**QUAN TRỌNG**: Toàn bộ nội dung specification này được viết bằng **Tiếng Việt**.

---

## User Scenarios & Testing

### User Story 1 — Kích hoạt Tự Động Migrate (One-Click Auto-Migration)
Người dùng sau khi bấm "Connect Microsoft" chỉ cần bật công tắc **"Tự Động Migrate"**. Hệ thống sử dụng toàn bộ OneDrive root của tài khoản đang kết nối làm nguồn, tự động chọn Telegram Destination mặc định, quét và chạy ngầm mà người dùng không phải thiết lập job hay bấm các nút scan/start thủ công.

**Acceptance Scenarios**:
1. **Given** người dùng đã kết nối tài khoản Microsoft, **When** bật công tắc "Tự Động Migrate", **Then** hệ thống tự tạo và lưu bền vững Auto-Migration Profile, tự chọn thư mục làm việc tạm mặc định và bắt đầu tự động quét + migrate ngầm.
2. **Given** Auto-Migration đang bật và Microsoft token còn hợp lệ hoặc có thể refresh, **When** người dùng mở ứng dụng Telegram Drive ở các lần sau, **Then** hệ thống tự động khôi phục phiên Microsoft và tiếp tục snapshot hiện có mà không yêu cầu kết nối lại hoặc tự động quét lại.
3. **Given** Auto-Migration đang bật, **When** người dùng tắt công tắc, **Then** hệ thống tạm dừng các tiến trình ngầm và lưu trạng thái `paused`.
4. **Given** người dùng chưa kết nối tài khoản Microsoft, **When** mở trang Auto-Migration, **Then** hệ thống chỉ hiển thị trạng thái chưa kết nối và hành động kết nối tài khoản; không hiển thị danh sách file, nhật ký hoạt động hoặc các danh sách transfer.
5. **Given** Auto Migration chưa có snapshot, **When** người dùng bật Master Switch, **Then** hệ thống quét OneDrive root một lần, thêm tất cả file vào danh sách theo thứ tự ổn định và bắt đầu xử lý tuần tự.
6. **Given** đã có snapshot hoặc hàng đợi đang được xử lý, **When** OneDrive xuất hiện file mới, **Then** hệ thống không tự động quét hoặc thêm file mới vào danh sách hiện tại.
7. **Given** migration không ở trạng thái `running`, **When** người dùng nhấn "Quét lại", **Then** hệ thống chủ động tạo snapshot mới từ OneDrive root và cập nhật danh sách theo thứ tự.
8. **Given** người dùng vừa kết nối thành công một tài khoản Microsoft mới, **When** hệ thống chưa có snapshot của tài khoản đó, **Then** hệ thống tự tạo snapshot đầu tiên, bật Auto Migration mặc định và bắt đầu xử lý tuần tự mà không yêu cầu thêm thao tác.

---

### User Story 2 — Giao Diện Đơn Giản Hóa (Smart Auto-Migration Dashboard)
Giao diện thay thế trang thiết lập phức tạp bằng một **Trung Tâm Tự Động Migrate (Auto-Migration Center)** tối giản:
- **Master Switch**: Công tắc lớn "Bật/Tắt Tự Động Migrate".
- **Manual Rescan**: Nút "Quét lại" cho phép người dùng chủ động tạo snapshot mới; nút bị vô hiệu hóa trong lúc migration đang chạy.
- **Daily Quota Status (Giới hạn 250GB/ngày)**: Hiển thị bộ đếm dung lượng đã upload trong ngày hôm nay. Cảnh báo nếu chạm mốc 250GB.
- **Account Cards**: Thẻ hiển thị tài khoản Microsoft đã kết nối kèm nhãn "Đang tự động đồng bộ ngầm".
- **OneDrive Download List**: Danh sách riêng chỉ hiển thị các file thực tế đang ở giai đoạn tải dữ liệu từ OneDrive, kèm tiến độ download hiện tại.
- **Telegram Drive Upload List**: Danh sách riêng chỉ hiển thị các file thực tế đang ở giai đoạn upload lên Telegram Drive, kèm tiến độ upload hiện tại.
- **Live Activity Stream**: Nhật ký hoạt động tối giản dạng dòng thời gian, chỉ được tạo từ các file và sự kiện migration thực tế. Không hiển thị dữ liệu mẫu, placeholder hoặc file chưa thuộc một migration job đang hoạt động.
- **Real-time Progress**: Thanh tiến trình của từng file phải xuất hiện trong đúng danh sách tương ứng với phase hiện tại. Một file không được đồng thời xuất hiện trong cả Download List và Upload List.
- **Advanced Drawer (Tùy chọn nâng cao)**: Bảng trượt ẩn cho phép đổi kênh Telegram nhận file hoặc thư mục tạm nếu muốn.

**Acceptance Scenarios**:
1. **Given** người dùng chưa kết nối Microsoft, **When** mở giao diện Auto-Migration, **Then** không hiển thị Download List, Upload List hoặc Live Activity Stream.
2. **Given** người dùng đã kết nối Microsoft nhưng chưa có migration đang hoạt động, **When** mở giao diện Auto-Migration, **Then** các khu vực transfer có thể hiển thị trạng thái rỗng nhưng không được tạo dữ liệu file giả.
3. **Given** một file thực tế đang được tải từ OneDrive, **When** hệ thống báo phase `downloading`, **Then** file chỉ xuất hiện trong OneDrive Download List với tiến độ download tương ứng.
4. **Given** file đã tải xong và bắt đầu upload lên Telegram Drive, **When** hệ thống báo phase `uploading`, **Then** file được loại khỏi OneDrive Download List và chỉ xuất hiện trong Telegram Drive Upload List với tiến độ upload tương ứng.
5. **Given** migration phát sinh sự kiện thực tế cho một file, **When** giao diện cập nhật Live Activity Stream, **Then** mục nhật ký phải tham chiếu đúng file, phase, trạng thái và thời điểm của sự kiện đó.
6. **Given** người dùng muốn đổi kênh Telegram mặc định, **When** mở phần "Tùy chọn nâng cao", **Then** có thể chọn kênh khác và hệ thống tự động lưu bền vững lựa chọn đó.
7. **Given** tài khoản Microsoft đã kết nối và hệ thống đang lấy cây thư mục OneDrive, **When** snapshot chưa hoàn tất, **Then** vùng danh sách file hiển thị trạng thái loading trực quan và không hiển thị thông báo rỗng gây hiểu nhầm.
8. **Given** tài khoản Microsoft đã kết nối, **When** người dùng chọn "Đổi tài khoản", **Then** phiên hiện tại bị ngắt, dữ liệu hiển thị của tài khoản cũ được xóa và luồng xác thực tài khoản mới được mở.

---

## Requirements

### Functional Requirements

- **FR-001**: Hệ thống PHẢI lưu bền vững một Auto-Migration Profile cho tài khoản Microsoft đang kết nối.
- **FR-002**: Hệ thống PHẢI tự động chọn thư mục làm việc tạm mặc định (ví dụ: `[AppData]/TelegramDrive/temp_migration`) nếu người dùng không tự chọn.
- **FR-003**: Hệ thống PHẢI dùng Saved Messages làm Telegram destination mặc định nếu người dùng không chỉ định destination khác trong Advanced Settings.
- **FR-004**: Hệ thống PHẢI cung cấp công tắc Master Switch (Bật/Tắt Tự Động Migrate) trên giao diện frontend.
- **FR-005**: Khi Master Switch bật, hệ thống PHẢI tự động tạo job, quét thư mục nguồn và kích hoạt migration hoàn toàn tự động mà không cần bấm nút Scan hay Start thủ công.
- **FR-006**: Hệ thống PHẢI duy trì chế độ tự động chạy ngầm mỗi khi ứng dụng Telegram Drive được mở lên.
- **FR-007**: Giao diện PHẢI hiển thị tiến trình thời gian thực của các file thực tế thuộc migration job đang hoạt động; không được hiển thị dữ liệu mẫu, placeholder hoặc file không tồn tại trong migration state.
- **FR-008 (Anti-Spam Limit)**: Hệ thống PHẢI theo dõi tổng dung lượng của các file Auto Migration đã upload thành công trong ngày local hiện tại. Trước khi bắt đầu file tiếp theo, hệ thống PHẢI cộng kích thước file đó vào tổng dự kiến; nếu tổng dự kiến vượt **250GB**, hệ thống phải pause trước file đó. Quota reset lúc 00:00 theo timezone hiện tại của máy và giao diện phải hiển thị dung lượng đã dùng, dung lượng còn lại cùng thời điểm reset.
- **FR-009 (Connection Gate)**: Khi chưa kết nối tài khoản Microsoft, giao diện PHẢI ẩn danh sách file, OneDrive Download List, Telegram Drive Upload List và Live Activity Stream; chỉ hiển thị trạng thái chưa kết nối và hành động kết nối tài khoản.
- **FR-010 (OneDrive Download List)**: Giao diện PHẢI có một danh sách riêng cho các file đang ở phase `downloading` từ OneDrive. Mỗi mục PHẢI tham chiếu một migration item thực tế và hiển thị ít nhất tên file, dung lượng, tiến độ download và trạng thái hiện tại.
- **FR-011 (Telegram Drive Upload List)**: Giao diện PHẢI có một danh sách riêng cho các file đang ở phase `uploading` lên Telegram Drive. Mỗi mục PHẢI tham chiếu một migration item thực tế và hiển thị ít nhất tên file, dung lượng, tiến độ upload và trạng thái hiện tại.
- **FR-012 (Activity Integrity)**: Live Activity Stream PHẢI chỉ chứa sự kiện phát sinh từ quá trình migration thực tế, bao gồm file liên quan, phase, trạng thái và thời điểm. Không được dựng activity từ dữ liệu tĩnh hoặc dữ liệu mô phỏng.
- **FR-013 (Exclusive Transfer Phase)**: Tại cùng một thời điểm, một file chỉ được xuất hiện trong một trong hai danh sách transfer. Khi phase chuyển từ `downloading` sang `uploading`, giao diện PHẢI chuyển file sang danh sách tương ứng và không tạo mục trùng lặp.
- **FR-014 (Source Scope)**: Auto Migration PHẢI dùng toàn bộ OneDrive root của tài khoản Microsoft đang kết nối làm nguồn. Luồng mặc định không yêu cầu người dùng chọn thư mục OneDrive.
- **FR-015 (Ordered Snapshot)**: Khi chưa có snapshot, hệ thống PHẢI quét OneDrive root một lần và thêm toàn bộ file tìm thấy vào danh sách theo thứ tự ổn định. Worker PHẢI xử lý danh sách tuần tự, mỗi thời điểm chỉ có một file ở phase download hoặc upload.
- **FR-016 (No Automatic Rescan)**: Khi đã tồn tại snapshot, hệ thống KHÔNG ĐƯỢC tự động quét lại hoặc tự thêm file mới từ OneDrive vào danh sách hiện tại.
- **FR-017 (Manual Rescan)**: Giao diện PHẢI cung cấp nút "Quét lại" để người dùng chủ động tạo snapshot mới. Nút này chỉ khả dụng khi đã kết nối Microsoft và không có migration job đang `running`.
- **FR-018 (Microsoft Session Persistence)**: Hệ thống PHẢI lưu Microsoft token vào vùng dữ liệu riêng của ứng dụng trên máy người dùng để có thể tự khôi phục và refresh phiên sau khi restart. Token KHÔNG ĐƯỢC lưu trong repository, source tree hoặc file có thể được commit lên Git; KHÔNG ĐƯỢC xuất hiện trong log; và PHẢI bị xóa khỏi disk khi người dùng Disconnect.
- **FR-019 (Resume Without Rescan)**: Sau khi khôi phục phiên Microsoft lúc app khởi động, hệ thống PHẢI resume snapshot chưa hoàn tất đang tồn tại và KHÔNG tự động quét OneDrive lại.
- **FR-020 (Default Telegram Destination)**: Auto Migration PHẢI có thể bắt đầu bằng Saved Messages mà không yêu cầu người dùng chọn Telegram destination. Destination tùy chỉnh chỉ thay thế mặc định sau khi người dùng lưu lựa chọn trong Advanced Settings.
- **FR-021 (Initial Snapshot After Connect)**: Ngay sau khi kết nối thành công một tài khoản Microsoft chưa có snapshot, hệ thống PHẢI tự động lấy toàn bộ cây thư mục OneDrive root, tạo snapshot đầu tiên và bật Auto Migration mặc định.
- **FR-022 (Snapshot Loading State)**: Trong toàn bộ thời gian lấy cây thư mục và tạo snapshot, vùng danh sách file PHẢI hiển thị trạng thái loading rõ ràng; trạng thái rỗng chỉ được hiển thị sau khi thao tác kết thúc mà không có dữ liệu.
- **FR-023 (Switch Microsoft Account)**: Thẻ tài khoản OneDrive PHẢI cung cấp hành động "Đổi tài khoản". Khi kích hoạt, hệ thống PHẢI xóa phiên và dữ liệu UI của tài khoản cũ trước khi mở xác thực tài khoản mới; hành động bị vô hiệu hóa khi migration đang chạy.

### Non-Functional Requirements

- **NFR-001 (Zero Hassle)**: Số bước thao tác tối đa của người dùng để bắt đầu migrate từ tài khoản mới là **1 bước** (Bật công tắc).
- **NFR-002 (Silent Background Execution)**: Các tác vụ quét và migrate ngầm không làm đơ giao diện hay chắn thao tác của người dùng trên ứng dụng Desktop.
- **NFR-003 (Persistence)**: Cấu hình tự động, snapshot và nhật ký hoạt động PHẢI tồn tại sau khi ứng dụng restart.
- **NFR-004 (UI Data Integrity)**: Trạng thái, tiến độ và vị trí danh sách của mỗi file trên giao diện phải phản ánh migration state thực tế; giao diện không được tự suy diễn phase hoặc tạo transfer item không có nguồn dữ liệu thật.

---

## Success Criteria

- **SC-001**: Người dùng bắt đầu quá trình migrate toàn bộ tài khoản OneDrive chỉ với 1 nhấp chuột duy nhất.
- **SC-002**: Khi khởi động lại ứng dụng với token còn sử dụng được, hệ thống tự động khôi phục tài khoản và tiếp tục snapshot hiện có mà không yêu cầu thao tác lại hoặc tạo snapshot mới.
- **SC-003**: Giao diện trực quan, tối giản, loại bỏ 100% các nút bấm rườm rà (Create Job, Scan, Manual Start, Picker trùng lặp).
- **SC-004**: Trong 100% lần mở trang khi chưa kết nối Microsoft, không có danh sách file, transfer list hoặc activity stream nào được hiển thị.
- **SC-005**: 100% mục hiển thị trong Download List, Upload List và Live Activity Stream ánh xạ tới một migration item hoặc migration event thực tế.
- **SC-006**: Khi một file chuyển phase giữa download và upload, giao diện chuyển file sang đúng danh sách trong vòng 2 giây và không hiển thị file đồng thời ở cả hai danh sách.
- **SC-007**: Trong suốt quá trình xử lý một snapshot, 100% file được thực hiện theo thứ tự của danh sách và không có hai file được download hoặc upload đồng thời.
- **SC-008**: Sau khi snapshot đã được tạo, không có file mới nào được tự động thêm vào danh sách cho đến khi người dùng chủ động nhấn "Quét lại".
- **SC-009**: Token Microsoft không xuất hiện trong bất kỳ file nào thuộc repository hoặc log ứng dụng; sau Disconnect, phiên không thể được tự động khôi phục ở lần mở app tiếp theo.
- **SC-010**: Không file nào được bắt đầu nếu kích thước file đó làm tổng dung lượng Auto Migration upload thành công trong ngày vượt 250GB; file chờ được tiếp tục sau lần reset quota kế tiếp.
- **SC-011**: Trong 100% lần kết nối tài khoản mới, người dùng nhìn thấy trạng thái loading từ lúc bắt đầu lấy dữ liệu đến khi danh sách file hoặc lỗi được hiển thị; không có khoảng thời gian màn hình danh sách trống không giải thích.
- **SC-012**: Người dùng có thể bắt đầu đăng nhập một tài khoản Microsoft khác bằng một hành động từ thẻ tài khoản OneDrive và không nhìn thấy file của tài khoản cũ trong quá trình chuyển đổi.

---

## Edge Cases

- **Chưa từng kết nối Microsoft**: Chỉ hiển thị màn hình kết nối; không render vùng dữ liệu migration.
- **Microsoft bị ngắt kết nối khi đang xem trang**: Chuyển giao diện về trạng thái cần kết nối lại và ẩn các danh sách transfer cho đến khi kết nối được khôi phục.
- **Đã kết nối nhưng chưa có job**: Hiển thị trạng thái rỗng có chủ đích, không tạo file hoặc activity giả.
- **Không có file ở một phase**: Danh sách tương ứng hiển thị trạng thái rỗng độc lập; danh sách còn lại vẫn tiếp tục cập nhật.
- **Event đến sai thứ tự hoặc bị lặp**: Giao diện dùng định danh migration item để cập nhật mục hiện có, không tạo bản sao của cùng một file.
- **File thất bại khi download**: File rời Download List và được ghi vào Live Activity Stream với trạng thái failed; không xuất hiện trong Upload List.
- **File hoàn tất upload**: File rời Upload List và được ghi vào Live Activity Stream với trạng thái completed.
- **Không truy cập được OneDrive root**: Không tạo migration job; hiển thị lỗi kết nối nguồn và giữ Auto Migration ở trạng thái chờ khôi phục.
- **Có file mới sau khi snapshot được tạo**: File mới không được tự động thêm vào hàng đợi hiện tại; chỉ xuất hiện sau khi người dùng chủ động "Quét lại".
- **Người dùng yêu cầu quét khi migration đang chạy**: Nút "Quét lại" bị vô hiệu hóa để bảo toàn thứ tự và tính ổn định của snapshot.
- **Token hết hạn nhưng refresh token còn hợp lệ**: Hệ thống tự refresh token và tiếp tục snapshot hiện có mà không quét lại.
- **Token không thể refresh**: Dừng trước khi bắt đầu file tiếp theo, hiển thị trạng thái cần kết nối lại và giữ nguyên snapshot để resume sau khi xác thực thành công.
- **Người dùng Disconnect**: Xóa token đã persist; ẩn các transfer list theo connection gate và không tự khôi phục phiên ở lần mở app sau.
- **Người dùng đổi tài khoản khi OAuth mới bị hủy**: Tài khoản cũ vẫn ở trạng thái đã ngắt kết nối và dữ liệu của tài khoản cũ không được khôi phục lên giao diện.
- **Lấy cây thư mục thất bại sau khi kết nối**: Kết thúc trạng thái loading, giữ tài khoản ở trạng thái đã kết nối, hiển thị lỗi và cho phép người dùng chủ động nhấn "Quét lại".
- **File lớn hơn quota còn lại**: Pause trước khi download file đó, giữ file ở trạng thái chờ quota và không thay đổi thứ tự snapshot.
- **Ứng dụng chạy qua nửa đêm**: Reset quota theo timezone local hiện tại và tự cho phép worker tiếp tục file đang chờ nếu Master Switch vẫn bật.
- **Timezone của máy thay đổi**: Lần tính quota tiếp theo dùng ngày local hiện tại; không được cộng hoặc trừ lại các upload đã ghi nhận trước đó.
- **Saved Messages không thể resolve**: Không bắt đầu upload file tiếp theo; hiển thị lỗi destination và cho phép người dùng chọn destination khác trong Advanced Settings.

## Key Entities

- **Download Transfer Item**: Đại diện cho một migration item thực tế đang tải từ OneDrive, gồm định danh file, tên, dung lượng, số byte đã tải, tổng số byte và trạng thái.
- **Upload Transfer Item**: Đại diện cho một migration item thực tế đang upload lên Telegram Drive, gồm định danh file, tên, dung lượng, số byte đã upload, tổng số byte và trạng thái.
- **Activity Entry**: Một sự kiện thực tế của migration item, gồm định danh file, phase, trạng thái, thời điểm và thông báo liên quan.
- **Daily Migration Quota**: Tổng số byte của các file Auto Migration upload thành công trong một ngày local, kèm ngày áp dụng và thời điểm reset kế tiếp.

## Assumptions

- Hai transfer list phản ánh phase hiện tại, không phải toàn bộ snapshot hoặc lịch sử file.
- File đã hoàn tất hoặc thất bại được loại khỏi transfer list và có thể xuất hiện trong Live Activity Stream.
- Do worker migration hiện xử lý tuần tự, mỗi transfer list có thể chỉ chứa tối đa một file đang hoạt động; cấu trúc danh sách vẫn được giữ để hỗ trợ theo dõi rõ ràng và khả năng mở rộng sau này.
- OneDrive root là phạm vi nguồn duy nhất của luồng Auto Migration mặc định.
- Thứ tự file trong snapshot phải ổn định giữa các lần đọc cùng một snapshot; file mới chỉ được nhận vào khi người dùng tạo snapshot mới bằng nút "Quét lại".
- Token Microsoft được lưu trong vùng dữ liệu riêng của ứng dụng ngoài repository để ưu tiên trải nghiệm tự khôi phục; repository và log không chứa credential.
- Saved Messages là Telegram destination mặc định; cấu hình tùy chỉnh là tùy chọn.
- Người dùng đã đăng nhập Telegram và có một Telegram session hợp lệ trước khi bật Auto Migration.
- Mỗi thời điểm chỉ có một tài khoản Microsoft active và một snapshot được xử lý.

## Scope

### In Scope

- Toàn bộ OneDrive root của một tài khoản Microsoft active.
- Một snapshot có thứ tự ổn định và xử lý tuần tự.
- Tự tạo snapshot đầu tiên khi chưa có dữ liệu.
- Nút "Quét lại" do người dùng chủ động kích hoạt.
- Saved Messages mặc định và destination tùy chỉnh trong Advanced Settings.
- Persist Microsoft session, profile, snapshot, activity và daily quota.
- Hai transfer list riêng cùng Live Activity Stream từ dữ liệu thực.

### Out of Scope

- Tự động quét định kỳ hoặc tự thêm file mới sau khi snapshot đã được tạo.
- Download hoặc upload nhiều file song song.
- Nhiều tài khoản Microsoft active đồng thời.
- Xóa, đổi tên hoặc sửa file nguồn trên OneDrive.
- Tiếp tục chạy khi toàn bộ Tauri process đã bị terminate.
- System tray, close-to-tray, autostart hoặc OS service.
