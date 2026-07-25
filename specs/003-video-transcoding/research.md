# Research: Tối ưu video trước khi lưu trữ

## Quyết định 1: Phân tích metadata bằng FFprobe JSON

- **Decision**: Dùng FFprobe với JSON để xác định stream video, codec, width/height, rotation và duration. Khi probe thất bại, phần mở rộng chỉ là fallback để phân biệt media-candidate bị hỏng với non-media thông thường; probe thành công luôn có quyền quyết định cao hơn extension.
- **Rationale**: Dữ liệu có cấu trúc ổn định hơn parse stderr của FFmpeg, hỗ trợ codec/container đa dạng và cho phép unit-test parser độc lập.
- **Alternatives considered**:
  - Dựa vào phần mở rộng: sai với container đổi tên hoặc file giả.
  - Parse `ffmpeg -i` stderr: phụ thuộc format log và locale/version.
  - Thêm crate media parser cho mọi container: tăng dependency và vẫn không bao phủ codec/container như FFmpeg.

## Quyết định 2: MP4 + H.264/AAC là đầu ra chuẩn

- **Decision**: Tạo MP4 với `libx264`, `yuv420p`, AAC nếu có audio và `+faststart`.
- **Rationale**: Tương thích phát lại rộng, phù hợp hạ tầng MP4/HLS hiện có và đáp ứng yêu cầu H.264.
- **Alternatives considered**:
  - Giữ container nguồn: một số container không hỗ trợ tổ hợp codec/metadata mong muốn.
  - H.265/AV1: nén tốt hơn nhưng không đáp ứng yêu cầu H.264 và chi phí encode/compatibility cao hơn.
  - Copy audio mọi trường hợp: codec nguồn có thể không tương thích MP4.

## Quyết định 3: CRF 23, preset medium

- **Decision**: Dùng constant quality CRF 23 với preset `medium`, không ép video bitrate cố định.
- **Rationale**: CRF thích ứng độ phức tạp nội dung và thường tối ưu dung lượng/chất lượng tốt hơn bitrate cố định; `medium` cân bằng thời gian và compression.
- **Alternatives considered**:
  - Preset `veryfast`: nhanh hơn nhưng thường tạo file lớn hơn.
  - Bitrate cố định 5 Mbps: dễ dự đoán nhưng kém tối ưu với nội dung đơn giản/phức tạp.
  - Two-pass: hiệu quả cho target size nhưng tăng gấp đôi thời gian và không cần khi không có target bytes.

## Quyết định 4: Không upscale, giới hạn trong bounding box

- **Decision**: Scale giảm theo bounding box 1920×1080, giữ tỷ lệ, chia hết cho 2; video dọc được đánh giá sau rotation.
- **Rationale**: Tránh suy giảm/chèn pixel giả; xử lý đúng cả landscape và portrait; kích thước chẵn tương thích `yuv420p`.
- **Alternatives considered**:
  - Chỉ giới hạn height 1080: video siêu rộng có thể vượt width 1920.
  - Ép đúng 1920×1080: méo hình hoặc thêm padding không cần thiết.
  - Bỏ rotation metadata: quyết định sai với video quay từ điện thoại.

## Quyết định 5: Passthrough H.264 đã ≤ Full HD

- **Decision**: Bỏ encode nếu stream video đầu tiên là H.264 và display dimensions nằm trong giới hạn; non-video cũng passthrough.
- **Rationale**: Tránh generation loss và CPU/time không cần thiết, đúng lưu ý của người dùng.
- **Alternatives considered**:
  - Encode mọi video: bảo đảm container đồng nhất nhưng lãng phí và giảm chất lượng.
  - Bỏ mọi video ≤ Full HD bất kể codec: không đáp ứng yêu cầu chuẩn hóa H.264.

## Quyết định 6: Tận dụng FFmpeg detection hiện có

- **Decision**: Dùng `TranscodeManager.ffmpeg_path`; FFprobe được tìm cạnh binary FFmpeg trước, sau đó PATH.
- **Rationale**: Tránh detection/cache trùng lặp và giữ một nguồn cấu hình dependency trong Tauri app.
- **Alternatives considered**:
  - Detect mỗi item: tốn process spawn.
  - Hardcode `/usr/bin`: không portable.
  - Bundled sidecar ngay trong feature: repo hiện chưa bundle binary; packaging riêng cần artifact/license workflow ngoài scope.

## Quyết định 7: Hủy và cleanup theo ownership

- **Decision**: `tokio::select!` giữa FFmpeg completion và cancel token polling; child bật `kill_on_drop`, explicit kill/wait khi hủy; output path duy nhất theo job/item.
- **Rationale**: Không để process con/tệp tạm mồ côi và cho phép retry từ source `.part`.
- **Alternatives considered**:
  - Chỉ kiểm tra cancel sau encode: không đáp ứng hủy dài hạn.
  - Shell command string: khó quote path và tăng rủi ro injection.

## Quyết định 8: Test không phụ thuộc binary cho phần lớn logic

- **Decision**: Unit-test JSON parser, rotation, decision, scale/filter args, output validation và cleanup; integration test có điều kiện khi FFmpeg/FFprobe có trên máy.
- **Rationale**: Test CI ổn định nhưng vẫn có đường xác minh end-to-end tại môi trường đủ dependency.
- **Alternatives considered**:
  - Luôn yêu cầu FFmpeg trong test suite: dễ fail trên developer/CI chưa cài binary.
  - Chỉ mock toàn bộ: bỏ sót lỗi command thực.
