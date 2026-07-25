# Quickstart Validation: Tối ưu video trước khi lưu trữ

## Prerequisites

- Rust toolchain và Node dependencies của project.
- `ffmpeg` và `ffprobe` cùng khả dụng trong PATH hoặc resource directory của app.
- Tài khoản Microsoft và Telegram đã kết nối cho test end-to-end.

## Static and unit validation

```bash
cd app/src-tauri
cargo fmt -- --check
cargo test migration::media_processor
cargo test migration
```

## Frontend regression

```bash
cd app
npm test -- --run
npm run build
```

## End-to-end scenarios

### A. 4K landscape H.265

1. Đặt video 3840×2160 H.265 vào OneDrive source.
2. Chạy migration.
3. Kỳ vọng log có `analyzing` rồi `processing`.
4. Tải tệp từ Telegram Drive và kiểm tra:

```bash
ffprobe -v error -select_streams v:0 \
  -show_entries stream=codec_name,width,height \
  -of json downloaded.mp4
```

Kỳ vọng codec `h264`, width ≤ 1920, height ≤ 1080.

### B. 720p H.264 passthrough

1. Đặt video H.264 1280×720 vào source.
2. Chạy migration.
3. Kỳ vọng không có phase encode; SHA-256 tệp tải về trùng tệp nguồn.

### C. 720p VP9

1. Đặt video VP9 1280×720 vào source.
2. Chạy migration.
3. Kỳ vọng output H.264 và không có cạnh nào lớn hơn input.

### D. Video dọc có rotation

1. Đặt video quay điện thoại có rotation 90° và kích thước encoded 3840×2160.
2. Chạy migration.
3. Kỳ vọng display dimensions nằm trong bounding box Full HD và tỷ lệ được giữ.

### E. Non-video

1. Đặt PDF hoặc ZIP vào source.
2. Chạy migration.
3. Kỳ vọng nội dung tải lên không đổi và không chạy FFmpeg encode.

### F. Cancel và cleanup

1. Bắt đầu encode video dài.
2. Hủy job khi progress đang chạy.
3. Kỳ vọng process FFmpeg dừng, không còn `*.transcoded.mp4`, source OneDrive không bị xóa.

### G. Dependency/error

1. Chạy app trong môi trường không tìm thấy FFmpeg/FFprobe hoặc dùng video hỏng.
2. Kỳ vọng item failed với error code đúng, không upload file hỏng, không còn output temp.

## Cleanup audit

Sau mỗi scenario, local working directory không được còn output `mig_<job>_<item>.transcoded.mp4` của item đã kết thúc.

## Kết quả xác minh 2026-07-24

- Rust library: 24/24 tests pass, gồm FFmpeg integration 2560×1440 → H.264 ≤ 1920×1080 và startup cleanup chỉ xóa đúng output thuộc ownership của feature.
- Cancel/cleanup: fake long-running child bị kill, output Unicode được dọn.
- Corrupt video `.part` vẫn được nhận diện theo tên nguồn `.mp4` và trả probe/tool error.
- Frontend: 12/12 Vitest tests pass.
- TypeScript + Vite production build: pass; chỉ còn cảnh báo chunk-size/dynamic import đã tồn tại ngoài feature.
- `vi.json` khớp cấu trúc `en.json`; checker toàn repository vẫn fail vì các locale thứ ba đã thiếu toàn bộ nhóm migration từ trước, ngoài phạm vi constitution yêu cầu EN/VI.
- Full `cargo fmt` bị chặn bởi trailing whitespace có sẵn trong `app/src-tauri/src/lib.rs`; ba Rust file của feature đã được format trực tiếp bằng `rustfmt`.
- Scenario OneDrive → Telegram thực tế cần credential người dùng nên được giữ làm manual acceptance run; contract local của từng bước đã được kiểm tra tự động.
