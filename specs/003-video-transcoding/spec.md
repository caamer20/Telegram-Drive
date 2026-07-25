# Feature Specification: Tối ưu video trước khi lưu trữ

**Feature Branch**: `003-video-transcoding`

**Created**: 2026-07-24

**Status**: Draft

**Input**: User description: "Chèn bước nén video về tối đa Full HD, dùng H.264 để giảm dung lượng và bỏ qua video có chất lượng thấp hơn."

## Ngôn ngữ

**QUAN TRỌNG**: Toàn bộ nội dung specification này được viết bằng **Tiếng Việt**. Chỉ giữ tên công nghệ, thư viện, biến/hàm/lớp bằng English.

## User Scenarios & Testing

### User Story 1 - Tự động tối ưu video độ phân giải cao (Priority: P1)

Là người dùng đồng bộ dữ liệu từ OneDrive sang Telegram Drive, tôi muốn video có độ phân giải cao hơn Full HD được tự động nén về tối đa Full HD và mã hóa H.264 trước khi tải lên để giảm dung lượng lưu trữ mà vẫn giữ chất lượng xem tốt.

**Why this priority**: Đây là giá trị cốt lõi của tính năng và trực tiếp giảm dung lượng truyền/lưu trữ.

**Independent Test**: Đồng bộ một video 4K hợp lệ và xác nhận bản được lưu có kích thước khung hình không vượt quá 1920×1080, video dùng H.264, có âm thanh tương thích và phát được.

**Acceptance Scenarios**:

1. **Given** một video có kích thước khung hình vượt 1920×1080, **When** hệ thống chuẩn bị tải tệp lên, **Then** hệ thống tạo bản tối ưu có kích thước nằm trong khung 1920×1080, giữ đúng tỷ lệ, không phóng đại và dùng H.264.
2. **Given** một video dọc vượt giới hạn tương ứng, **When** hệ thống tối ưu, **Then** bản kết quả nằm trong khung 1920×1080 theo hướng hiển thị thực tế và giữ đúng tỷ lệ.
3. **Given** bản tối ưu được tạo thành công, **When** hệ thống tải lên, **Then** chỉ bản tối ưu được tải lên và tệp nguồn không bị sửa đổi.

---

### User Story 2 - Bỏ qua video không cần xử lý (Priority: P2)

Là người dùng, tôi muốn video đã có độ phân giải bằng hoặc thấp hơn Full HD và đã ở định dạng H.264 tương thích được tải lên nguyên trạng để tránh mất thời gian và suy giảm chất lượng không cần thiết.

**Why this priority**: Tránh xử lý thừa, bảo toàn chất lượng và rút ngắn thời gian đồng bộ.

**Independent Test**: Đồng bộ một video H.264 1280×720 và xác nhận tệp được tải lên nguyên trạng mà không tạo bản mã hóa lại.

**Acceptance Scenarios**:

1. **Given** video H.264 có kích thước không vượt khung Full HD, **When** hệ thống kiểm tra tệp, **Then** hệ thống bỏ qua mã hóa lại và tải tệp nguồn lên.
2. **Given** video có độ phân giải thấp hơn Full HD nhưng codec không phải H.264, **When** hệ thống kiểm tra tệp, **Then** hệ thống chuyển mã sang H.264 mà không tăng độ phân giải.
3. **Given** tệp không phải video, **When** hệ thống kiểm tra tệp, **Then** tệp tiếp tục luồng tải lên hiện tại mà không bị xử lý media.

---

### User Story 3 - Phục hồi an toàn khi tối ưu thất bại (Priority: P3)

Là người dùng, tôi muốn lỗi nhận diện hoặc chuyển mã video được báo rõ và không làm hỏng tệp nguồn hay để lại dữ liệu tạm vô hạn.

**Why this priority**: Bảo đảm tính ổn định của luồng đồng bộ dài hạn và tránh thất thoát dữ liệu.

**Independent Test**: Đồng bộ một tệp video hỏng, một tệp thiếu luồng video và mô phỏng lỗi công cụ xử lý; xác nhận item thất bại có thông báo cụ thể, tệp nguồn còn nguyên và dữ liệu tạm được dọn.

**Acceptance Scenarios**:

1. **Given** tệp được nhận diện là video nhưng metadata không đọc được, **When** bước kiểm tra thất bại, **Then** item được đánh dấu thất bại với lỗi có thể chẩn đoán và không tải tệp không xác định lên.
2. **Given** quá trình chuyển mã bị hủy hoặc thất bại, **When** worker kết thúc xử lý item, **Then** tệp nguồn không thay đổi và tệp đầu ra dở dang được dọn dẹp.
3. **Given** ứng dụng được khởi động lại sau khi gián đoạn, **When** item được phục hồi, **Then** hệ thống có thể xử lý lại từ tệp nguồn mà không phụ thuộc vào đầu ra tạm cũ.

### Edge Cases

- Video có metadata xoay 90°/270° phải được đánh giá theo hướng hiển thị thực tế.
- Video có cạnh đúng giới hạn nhưng cạnh còn lại nhỏ hơn vẫn không được phóng đại.
- Kích thước đầu ra phải là số chẵn để tương thích H.264 phổ biến.
- Tệp có phần mở rộng video nhưng không có luồng video phải được xử lý như lỗi media có thể chẩn đoán.
- Video không có âm thanh vẫn phải chuyển mã thành công; nhiều luồng âm thanh/phụ đề không làm hỏng luồng video chính.
- Không đủ dung lượng đĩa cho đầu ra tạm phải dừng sớm nếu có thể và trả lỗi rõ ràng.
- Tên tệp có Unicode, khoảng trắng hoặc ký tự đặc biệt không được làm sai câu lệnh xử lý.
- Hủy job trong lúc chuyển mã phải kết thúc tiến trình con và dọn tệp tạm.

## Requirements

### Functional Requirements

- **FR-001**: Hệ thống MUST áp dụng bước kiểm tra media cho các tệp trong luồng đồng bộ OneDrive trước khi tải lên Telegram Drive.
- **FR-002**: Hệ thống MUST xác định tệp có luồng video bằng nội dung/metadata media thay vì chỉ dựa vào phần mở rộng.
- **FR-003**: Hệ thống MUST giới hạn video đầu ra trong khung 1920×1080 theo hướng hiển thị thực tế, giữ nguyên tỷ lệ và không tăng độ phân giải.
- **FR-004**: Hệ thống MUST mã hóa luồng video cần chuyển đổi sang H.264 với định dạng điểm ảnh tương thích phát lại phổ biến.
- **FR-005**: Hệ thống MUST giữ video H.264 đã nằm trong khung Full HD ở nguyên trạng.
- **FR-006**: Hệ thống MUST chuyển video không phải H.264 sang H.264 ngay cả khi độ phân giải đã thấp hơn Full HD, nhưng MUST không phóng đại hình ảnh.
- **FR-007**: Hệ thống MUST tạo đầu ra tối ưu dưới dạng tệp tạm riêng và MUST không sửa đổi tệp nguồn.
- **FR-008**: Hệ thống MUST tải bản tối ưu lên khi chuyển mã thành công và dùng tệp nguồn khi quyết định bỏ qua.
- **FR-009**: Hệ thống MUST dọn tệp đầu ra tạm sau thành công, lỗi, hủy hoặc phục hồi sau gián đoạn.
- **FR-010**: Hệ thống MUST báo trạng thái đang phân tích/đang tối ưu và lỗi xử lý đủ chi tiết để người dùng chẩn đoán.
- **FR-011**: Hệ thống MUST giữ nguyên luồng hiện tại đối với tệp không phải video.
- **FR-012**: Hệ thống MUST hỗ trợ hủy tiến trình chuyển mã cùng với cơ chế hủy item/job hiện có.
- **FR-013**: Hệ thống MUST từ chối tải bản chuyển mã khi đầu ra không có luồng video hợp lệ hoặc có kích thước bằng 0.
- **FR-014**: Hệ thống MUST dùng chính sách chất lượng cân bằng giữa giảm dung lượng, thời gian xử lý và chất lượng hình ảnh; các tham số cụ thể được quyết định trong plan.

### Key Entities

- **MediaProbe**: Kết quả phân tích một tệp, gồm loại media, codec video, kích thước mã hóa, hướng hiển thị, thời lượng và sự hiện diện của âm thanh.
- **TranscodeDecision**: Quyết định bỏ qua hoặc chuyển mã, lý do quyết định và kích thước đầu ra mục tiêu.
- **PreparedUpload**: Tệp cuối cùng dùng để tải lên, tên hiển thị, MIME type, kích thước và thông tin quyền sở hữu tệp tạm.

## Success Criteria

### Measurable Outcomes

- **SC-001**: 100% video thử nghiệm vượt Full HD được lưu với kích thước hiển thị không vượt khung 1920×1080 và giữ sai lệch tỷ lệ hình dưới 1%.
- **SC-002**: 100% video đầu vào không phải H.264 trong bộ kiểm thử được lưu với luồng video H.264 mà không bị tăng độ phân giải.
- **SC-003**: 100% video H.264 bằng hoặc thấp hơn Full HD trong bộ kiểm thử được bỏ qua mã hóa lại.
- **SC-004**: 100% tệp không phải video trong bộ kiểm thử tiếp tục tải lên mà nội dung không bị thay đổi.
- **SC-005**: Sau mọi trường hợp thành công, lỗi và hủy trong bộ kiểm thử, không còn tệp đầu ra tạm thuộc item đã kết thúc.
- **SC-006**: Mọi lỗi phân tích/chuyển mã đều tạo thông báo chứa giai đoạn thất bại và nguyên nhân có thể hành động.

## Assumptions

- Phạm vi ban đầu là luồng OneDrive migration chạy trong Rust background worker; upload thủ công và upload URL nằm ngoài phạm vi phiên bản này.
- Full HD được hiểu là khung tối đa 1920×1080 theo hướng hiển thị, không ép tất cả video thành đúng 1920×1080.
- Video H.264 đã nằm trong giới hạn được xem là đủ tối ưu để bỏ qua dù container có thể khác nhau, miễn luồng upload hiện tại hỗ trợ tệp đó.
- Âm thanh sẽ được giữ tương thích với container đầu ra; chi tiết codec, bitrate và xử lý stream phụ được quyết định trong plan sau nghiên cứu.
- Công cụ xử lý media phải có mặt trong môi trường desktop; chiến lược đóng gói và thông báo khi thiếu dependency được quyết định trong plan.
- Tính năng chỉ cam kết hoạt động khi Tauri process đang chạy; không mở rộng yêu cầu unattended operation.
