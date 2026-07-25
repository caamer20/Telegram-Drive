# Data Model: Tối ưu video trước khi lưu trữ

## MediaProbe

Kết quả đọc metadata của tệp local.

| Field | Type | Quy tắc |
|---|---|---|
| `has_video` | `bool` | `true` khi có ít nhất một video stream |
| `video_codec` | `Option<String>` | Chuẩn hóa lowercase, lấy stream video đầu tiên |
| `encoded_width` | `Option<u32>` | Phải > 0 khi có video |
| `encoded_height` | `Option<u32>` | Phải > 0 khi có video |
| `rotation_degrees` | `i32` | Chuẩn hóa về 0/90/180/270 |
| `display_width` | `Option<u32>` | Hoán đổi với height khi rotation 90/270 |
| `display_height` | `Option<u32>` | Hoán đổi với width khi rotation 90/270 |
| `duration_seconds` | `Option<f64>` | Dùng cho progress, không bắt buộc |
| `has_audio` | `bool` | Quyết định optional audio mapping |

## TranscodeDecision

| Variant | Dữ liệu | Điều kiện |
|---|---|---|
| `PassthroughNonVideo` | reason | Không có video stream |
| `PassthroughCompatible` | probe | Codec H.264 và display dimensions ≤ 1920×1080 |
| `Transcode` | probe, target box | Codec khác H.264 hoặc vượt bounding box |

## PreparedUpload

| Field | Type | Quy tắc |
|---|---|---|
| `path` | `PathBuf` | Source `.part` hoặc output `.transcoded.mp4` |
| `upload_name` | `String` | Giữ stem nguồn, đổi extension thành `.mp4` khi transcode |
| `mime_type` | `Option<String>` | `video/mp4` khi transcode |
| `size_bytes` | `u64` | Metadata thực của tệp được upload |
| `decision` | `TranscodeDecision` | Dùng cho log/test |
| `owned_temp_path` | `Option<PathBuf>` | Chỉ có khi module tạo output; cleanup đúng ownership |

## State Transitions

```text
downloaded
  ├─ probe non-video ───────────────> prepared(source)
  ├─ probe H.264 ≤ Full HD ─────────> prepared(source)
  ├─ probe requires transcode
  │    ├─ encoding ── success ──────> validate output ──> prepared(temp)
  │    ├─ encoding ── cancel ───────> cleanup temp ─────> cancelled/retry
  │    └─ encoding ── failure ──────> cleanup temp ─────> failed
  └─ probe failure ─────────────────> failed

prepared ── upload success/failure/cancel ──> cleanup owned temp + source part
```

## Validation Rules

- Không upload output nếu metadata không tồn tại, size bằng 0 hoặc probe sau encode không có video H.264.
- Kích thước hiển thị output không vượt 1920×1080.
- Không upscale: display width/height output không vượt display width/height input theo cùng orientation.
- Tệp source không thuộc ownership của `PreparedUpload`; worker giữ trách nhiệm dọn `.part`.
- Output temp luôn thuộc ownership của `PreparedUpload`.
