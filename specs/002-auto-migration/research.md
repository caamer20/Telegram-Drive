# Research: Automated Account Migration

## 1. Phạm vi nguồn

**Decision**: Dùng toàn bộ OneDrive root cho snapshot auto.
**Rationale**: Đáp ứng mục tiêu một chạm và migrate toàn bộ tài khoản.
**Alternatives considered**: Folder gần nhất; chọn folder lần đầu; nhiều root.

## 2. Snapshot và rescan

**Decision**: Tự scan đúng một lần khi chưa có snapshot; không polling. Nút “Quét lại” là trigger duy nhất sau đó; snapshot rỗng vẫn được persist.
**Rationale**: Người dùng xác nhận không thêm file trong quá trình; giữ queue ổn định và tránh scan lại vô hạn khi root rỗng.
**Alternatives considered**: Poll 15 phút; Graph delta; scan sau mỗi job.

## 3. Thứ tự xử lý

**Decision**: Sort theo `source_path`, sau đó `source_item_id`; persist `queue_position`.
**Rationale**: Thứ tự deterministic, độc lập với traversal order và row insertion.
**Alternatives considered**: ID tăng dần; tên file; thời gian sửa.

## 4. Microsoft session

**Decision**: Persist session trong app-private data ngoài repository, atomic write, permission hạn chế, xóa khi Disconnect.
**Rationale**: Người dùng ưu tiên tiện lợi và tự resume; chỉ yêu cầu credential không vào Git/log.
**Alternatives considered**: Memory-only; OS credential store; SQLite.

## 5. Telegram destination

**Decision**: Saved Messages khi profile không có destination tùy chỉnh.
**Rationale**: Luôn có sẵn và không thêm bước thiết lập.
**Alternatives considered**: Destination gần nhất; bắt buộc chọn; tự tạo channel.

## 6. Daily quota

**Decision**: 250 GiB theo ngày local; projected-size gate trước download; chỉ áp dụng job auto và cộng atomically với upload thành công.
**Rationale**: Không vượt ngưỡng, tránh download file chưa thể upload, không làm manual migration tiêu hao quota auto và không double-count khi crash.
**Alternatives considered**: Kiểm tra sau upload; UTC day; quota cấu hình.

## 7. Activity

**Decision**: Persist activity record ở backend và expose qua IPC; progress event mang ID/revision/attempt để chống duplicate và out-of-order.
**Rationale**: UI chỉ hiển thị sự kiện thật, lịch sử tồn tại sau restart và reconnect không làm sai phase hiện hành.
**Alternatives considered**: Dữ liệu mẫu; suy ra từ toast; chỉ giữ trong React state.

## 8. UI state

**Decision**: Connection gate ở cấp page/context; hai transfer list được derive từ authoritative phase bằng reducer/selectors thuần.
**Rationale**: Ngăn lộ snapshot trước connect, bảo đảm file không nằm ở hai list và cho phép test duplicate/out-of-order độc lập UI.
**Alternatives considered**: Render table disabled; một progress card có hai thanh.
