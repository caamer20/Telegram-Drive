# Contract: Migration Media Preparation

## Rust service contract

```rust
pub async fn prepare_upload(
    ffmpeg_path: &Path,
    source_path: &Path,
    source_name: &str,
    output_path: &Path,
    cancel_token: &AtomicBool,
    on_progress: impl Fn(MediaProgress),
) -> Result<PreparedUpload, MediaProcessError>;
```

### Guarantees

- Không sửa hoặc rename `source_path`.
- Video hợp lệ được nhận diện theo probe dù extension không thuộc danh sách; extension chỉ quyết định fallback khi probe không thể đọc tệp.
- `PreparedUpload.path == source_path` khi passthrough.
- `PreparedUpload.path == output_path` khi transcode.
- Khi trả `Err`, `output_path` không tồn tại hoặc đã được dọn.
- Tên upload kết thúc `.mp4` khi transcode.
- Callback progress chỉ phát phase `analyzing` hoặc `processing`; phần trăm đơn điệu trong 0..=100 bên trong từng phase và được phép reset khi đổi phase.

## Event contract

Tái sử dụng `migration:item-progress`:

```json
{
  "job_id": 42,
  "item_id": 7,
  "item_name": "clip.mov",
  "phase": "processing",
  "event_id": "42:7:1:processing:3",
  "attempt": 1,
  "revision": 3,
  "percent": 55,
  "bytes_done": 0,
  "bytes_total": 0,
  "speed_bytes_per_sec": 0,
  "timestamp": 1784827609000
}
```

`phase` bổ sung hai giá trị:

- `analyzing`: đọc metadata và ra quyết định.
- `processing`: FFmpeg đang tạo video tối ưu.

## Error contract

| Persisted code | Message prefix | Ý nghĩa | Retry |
|---|---|---|---|
| `unknown` | `[media_tool]` | Thiếu FFmpeg hoặc FFprobe | Không tự retry cho tới khi môi trường thay đổi |
| `unknown` | `[media_probe]` | Không đọc được metadata | Có thể retry theo attempt policy hiện có |
| `unknown` | `[media_transcode]` / `[media_output]` | FFmpeg exit lỗi hoặc output không hợp lệ | Có thể retry theo attempt policy hiện có |
| Không persist failure | `[media_cancelled]` | Job/item bị hủy khi xử lý | Trả item về pending và để control flow cancel hiện có kết thúc job |
| `insufficient_disk` | `[disk]` | Không đủ chỗ cho output dự kiến | Sau khi giải phóng dung lượng |

Schema SQLite hiện có giới hạn enum `last_error_code`; prefix có cấu trúc trong message giữ stage chẩn đoán mà không yêu cầu rebuild bảng.

## FFmpeg command contract

Argument vector phải dùng process API, không qua shell:

```text
-y -i <source>
-map 0:v:0 -map 0:a:0? -sn -dn
-vf scale=w='min(1920,iw)':h='min(1080,ih)':force_original_aspect_ratio=decrease:force_divisible_by=2
-c:v libx264 -preset medium -crf 23 -pix_fmt yuv420p
-c:a aac -b:a 128k
-movflags +faststart
<output.mp4>
```

Implementation có thể thêm `-progress pipe:2 -nostats` để parse progress nhưng không được thay đổi các invariant trên.
