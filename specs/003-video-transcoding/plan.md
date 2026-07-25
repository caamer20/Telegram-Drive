# Implementation Plan: Tối ưu video trước khi lưu trữ

**Branch**: `003-video-transcoding` | **Date**: 2026-07-24 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/003-video-transcoding/spec.md`

## Ngôn ngữ

**QUAN TRỌNG**: Toàn bộ nội dung plan này được viết bằng **Tiếng Việt**. Chỉ giữ tên công nghệ, thư viện, biến/hàm/lớp bằng English.

## Summary

Thêm một module media preparation độc lập trong Rust migration backend. Sau khi tải OneDrive và kiểm tra duplicate, worker gọi FFprobe để phân loại/đọc metadata, quyết định passthrough hoặc chạy FFmpeg tạo MP4 H.264/AAC tối đa khung 1920×1080. Module trả về RAII-style `PreparedUpload` để worker tải đúng tệp, báo progress phase `processing`, hỗ trợ cancel và luôn dọn đầu ra tạm.

## Technical Context

**Language/Version**: Rust 2021 trong Tauri v2; TypeScript/React chỉ nhận event hiện có

**Primary Dependencies**: Tokio process/fs, Serde/serde_json, FFmpeg + FFprobe có trong resource directory hoặc PATH, migration worker và upload adapter hiện có

**Storage**: Tệp `.part` và `.transcoded.mp4` trong local working directory; SQLite migration metadata hiện có, không thêm schema

**Testing**: `cargo test` cho decision/parser/command builder/cleanup; integration tests có điều kiện khi FFmpeg/FFprobe khả dụng; frontend regression hiện có

**Target Platform**: Desktop macOS, Windows và Linux; Android không thuộc phạm vi migration preprocessing

**Project Type**: Tauri desktop application với Rust background worker và React frontend

**Performance Goals**: Không đọc toàn bộ video vào RAM; một FFmpeg process cho mỗi item; progress cập nhật tối đa theo thay đổi phần trăm; passthrough hoàn tất probe mà không mã hóa lại

**Constraints**: Giữ nguyên tệp nguồn; output video H.264 `yuv420p`, audio AAC nếu có; scale giữ tỷ lệ, không upscale, kích thước chẵn, tối đa 1920×1080; hủy phải kill child; output lỗi/zero-byte không được upload

**Scale/Scope**: Một item tại một thời điểm theo migration worker hiện có; áp dụng cho manual và auto OneDrive migration dùng chung worker; không áp dụng upload thủ công/URL

## Constitution Check

*GATE: Đã đánh giá trước nghiên cứu và đánh giá lại sau thiết kế.*

- **I. Rust Backend / Actix**: PASS — đây là background processing nội bộ, được phép chạy Tokio task trong Tauri runtime; không thêm HTTP route.
- **II. Tauri IPC + React**: PASS — tái sử dụng event boundary hiện có; nếu thêm phase UI thì vẫn qua Tauri event.
- **III. Telegram MTProto**: PASS — vẫn dùng `TelegramState` và `upload_core`, không tạo Telegram client thứ hai.
- **IV. SQLite**: PASS — không thay đổi persistence; trạng thái item hiện có vẫn là nguồn sự thật.
- **V. Spec-Driven Development**: PASS — artifacts được tạo trước implementation và sẽ chạy analyze trước sửa code.
- **VI. Background Processing**: PASS — xử lý nằm trong Rust worker; feature chỉ cam kết khi Tauri process chạy, cleanup/retry dựa trên tệp nguồn.
- **Quality Gates**: PASS — lỗi trả `Result`, có tests; text/event mới phải có i18n nếu chạm UI.

**Post-design re-check**: PASS — thiết kế không thêm vi phạm hay ngoại lệ cần Complexity Tracking.

## Project Structure

### Documentation (this feature)

```text
specs/003-video-transcoding/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── media-preparation.md
└── tasks.md
```

### Source Code (repository root)

```text
app/src-tauri/src/
├── lib.rs
├── transcode.rs
└── migration/
    ├── mod.rs
    ├── media_processor.rs
    └── worker.rs

app/src/
├── types.ts
├── context/MigrationContext.tsx
└── i18n/locales/
    ├── en.json
    └── vi.json
```

**Structure Decision**: Tạo `migration/media_processor.rs` để cô lập probe, decision, command execution và cleanup khỏi `worker_loop_inner` vốn đã phức tạp. Tái sử dụng `TranscodeManager.ffmpeg_path`; mở rộng detection để lấy FFprobe cạnh FFmpeg hoặc từ PATH. Worker chỉ orchestration, event và mapping lỗi.

## Design Decisions

1. `probe_media()` chạy FFprobe JSON với `-show_streams -show_format`; kết quả hợp lệ không có video là passthrough. Nếu FFprobe từ chối tệp, phần mở rộng video đã biết được xem là media hỏng và trả lỗi, còn tệp không mang phần mở rộng video được passthrough; FFprobe vẫn là nguồn quyết định chính nên video hợp lệ không có/đổi phần mở rộng vẫn được nhận diện.
2. Rotation lấy từ `side_data_list.rotation` hoặc tag `rotate`; kích thước hiển thị hoán đổi cho 90°/270°.
3. Passthrough chỉ khi codec chuẩn hóa là `h264` và kích thước hiển thị không vượt 1920×1080.
4. Transcode dùng `scale=w='min(1920,iw)':h='min(1080,ih)':force_original_aspect_ratio=decrease:force_divisible_by=2`, `libx264`, `preset medium`, `crf 23`, `pix_fmt yuv420p`, `aac`, `movflags +faststart`.
5. Chỉ map video đầu tiên và audio đầu tiên nếu có; bỏ subtitle/data để tạo MP4 tương thích và hành vi xác định.
6. `PreparedUpload` sở hữu output tạm và dọn bằng `Drop` như lớp bảo vệ cuối, đồng thời có cleanup async rõ ràng trong worker.
7. Khi output transcode không nhỏ hơn nguồn, vẫn dùng output vì mục tiêu H.264/Full HD là bắt buộc; không xóa output dựa trên so sánh kích thước.
8. Quota được kiểm tra lại theo kích thước prepared upload trước khi upload; accounting thành công vẫn dùng source bytes cho snapshot progress và upload bytes thật cho bandwidth.

## Complexity Tracking

Không có vi phạm constitution cần biện minh.
