# Production Adapters — Vòng 2B2A-R2 (Grammers Integrated)

## Trạng thái

**PRODUCTION_ADAPTERS_READY_BUT_INACTIVE** (Telegram: PRODUCTION-ACTIVE with patched Grammers)

Phiên bản: 2B2A-R2
Ngày: 2026-07-25

## Key Changes from 2B2A-R

### Telegram: NOW PRODUCTION-ACTIVE
- ✅ Grammers patched with `send_message_with_random_id` API
- ✅ App Cargo.toml points to patched local Grammers (`b4071f7`)
- ✅ Adapter calls `client.send_message_with_random_id(peer, message, persisted_random_id)`
- ✅ DB persistence: `upload_attempt_id` / `telegram_random_id` written BEFORE network call
- ✅ Retry reuses same random_id from DB
- ✅ Error mapping: FloodWait, FileTooLarge, Auth, Network, etc.
- ✅ Zero high-level `send_message` or `upload_core` in V2 paths
- Patch: `specs/004-onedrive-transfer-redesign/patches/grammers-explicit-random-id.patch`
- Proof: `specs/004-onedrive-transfer-redesign/grammers-patch-proof.md`

---

## 1. Dependency Composition

```
build_pipeline_v2_services()
├── OneDriveDownloader (SourceDownloader)
│   ├── reqwest::Client (shared)
│   ├── Arc<Mutex<Option<MicrosoftSession>>> (shared)
│   └── MigrationDb (shared)
├── FFmpegMediaAdapter (MediaInspector + VideoProcessor)
│   ├── ffprobe path (configurable)
│   ├── ffmpeg path (configurable)
│   ├── ProcessRunner trait (injectable)
│   ├── CancelToken (AtomicBool)
│   └── max_threads: min(2, available_parallelism)
├── TelegramProductionAdapter (TelegramUploader)
│   ├── Arc<Mutex<Option<Client>>> (shared from TelegramState)
│   ├── Arc<RwLock<HashMap<i64, Peer>>> (shared peer cache)
│   ├── CancelToken (AtomicBool)
│   └── Destination folder_id (None = Saved Messages)
├── LocalProductionAdapter (LocalFinalizer)
│   └── Backup root path
└── PipelineRunner (orchestrator)
    ├── PipelineConfig
    ├── MigrationDb
    └── Workspace/Backup directories
```

---

## 2. OneDrive Adapter

### File
`app/src-tauri/src/migration/adapters_v2/onedrive.rs`

### Systems reused
- `MicrosoftSession` từ `microsoft.rs` (OAuth token, refresh)
- `reqwest::Client` shared
- `MigrationDb` shared

### Operations supported (qua SourceDownloader trait)
- `download_file(item_id, source_item_id, dest_path) -> SHA-256`
- Streaming download (không load toàn bộ vào RAM)
- Fingerprint extraction: QuickXorHash → SHA-1 (ưu tiên QuickXorHash)
- Progress propagation (log-based)

### Operations NOT exposed
- DELETE (không có trong trait)
- PATCH/MOVE (không có trong trait)
- Trong test: wiremock `expect(0)` cho DELETE và PATCH

### Error mapping
| Error | Condition |
|-------|-----------|
| `Authentication` | HTTP 401 hoặc không có session |
| `SourceNotFound` | HTTP 404 |
| `PermissionDenied` | HTTP 403 |
| `RateLimited` | HTTP 429 |
| `TransientNetwork` | Lỗi kết nối reqwest |
| `InvalidResponse` | HTTP khác 2xx, JSON parse lỗi, thiếu download URL |

### Destructive request proof
- Chỉ dùng `method("GET")` trong HTTP client
- Wiremock test: DELETE/PATCH mock được mount với `expect(0)`
- `delete_onedrive_item` tồn tại trong `microsoft.rs` nhưng KHÔNG được gọi bởi adapter

### Tests (wiremock-based, không gọi Microsoft thật)
1. `test_onedrive_download_success` — download stream + fingerprint persist + SHA-256
2. `test_onedrive_download_errors` — 404 → SourceNotFound, 429 → RateLimited

---

## 3. FFmpeg Adapters

### File
`app/src-tauri/src/migration/adapters_v2/media.rs`

### Inspector (MediaInspector)
- Dùng `ffprobe` với argument vector (KHÔNG shell string)
- Parse JSON output → `VideoMetadata` (container, codec, width, height, duration, bitrate, rotation, validity)
- Phân loại lỗi: non-zero exit, parse error, cancelled

### Decision inputs
| Metadata | → Decision |
|----------|-----------|
| h264 + valid dimensions | `passthrough` |
| hevc / non-h264 codec | `transcode` |
| MKV container + h264 codec | `passthrough` (remux chưa implement trong runner) |

### Processor (VideoProcessor)
- Nhận `decision`: `transcode` hoặc `remux_copy`
- `passthrough` items KHÔNG gọi processor (bị từ chối với lỗi "unsupported decision")
- Dùng `ffmpeg` với argument vector
- Thread limit: `min(2, available_parallelism)`
- Safety: kill_on_drop, cleanup `.part` khi lỗi, verify output non-empty trước khi hash
- SHA-256 hash của output

### Process runner seam
- `ProcessRunner` trait cho phép inject fake runner trong test
- `RealProcessRunner`: dùng `tokio::process::Command`
- `FakeProcessRunner`: trả về output định sẵn, tạo file output ảo

### Tests (không yêu cầu FFmpeg thật)
1. `test_parse_ffprobe_json` — mapping chính xác
2. `test_ffprobe_json_malformed` — lỗi parse
3. `test_ffprobe_no_video_stream` — metadata không có video
4. `test_inspect_file_success` — ffprobe integration
5. `test_inspect_file_cancelled` — cancel token
6. `test_inspect_file_nonzero_exit` — ffprobe lỗi
7. `test_process_video_transcode_args` — argument profile transcode
8. `test_process_video_remux_args` — argument profile remux
9. `test_process_video_cancelled` — cancel token
10. `test_process_video_nonzero_exit` — ffmpeg lỗi
11. `test_process_video_unsupported_decision` — từ chối decision lạ
12. `test_passthrough_rejected_by_processor` — passthrough không gọi processor
13. `test_thread_limit_in_args` — thread limit được set
14. `test_no_shell_injection_in_args` — argument vector an toàn

---

## 4. Telegram Adapter

### File
`app/src-tauri/src/migration/adapters_v2/telegram.rs`

### Shared client reuse
- Dùng `Arc<Mutex<Option<grammers_client::Client>>>` từ `TelegramState`
- Dùng `Arc<RwLock<HashMap<i64, Peer>>>` từ `TelegramState.peer_cache`
- KHÔNG tạo client thứ hai
- KHÔNG đăng nhập lại
- KHÔNG đổi session format

### Binary upload
- `ProgressReader::new()` → `client.upload_stream()` (reuse existing machinery)
- Progress không được callback trong trait hiện tại (có thể thêm sau)
- Cancellation check tại safe boundaries: trước upload, sau upload, trước send

### Raw SendMedia
- Sử dụng high-level API `client.send_message(&peer, InputMessage::new().text("").file(uploaded_file))`
- **Lưu ý**: API cao cấp của Grammers tự sinh `random_id`, KHÔNG cho phép inject persisted random_id.
  - Spike report (`specs/004-onedrive-transfer-redesign/random-id-spike.md`) đã phân tích giới hạn này
  - API thô `tl::functions::messages::SendMedia` đã được type-check trong spike nhưng cần upgrade Grammers để có đầy đủ field
  - **Hiện tại**: adapter dùng API cao cấp, response mapping qua message ID
  - **Vòng 2B2B**: upgrade lên raw `SendMedia` sau khi Grammers được bump

### Image handling
- Tất cả file gửi dưới dạng document (InputMessage::file)
- KHÔNG dùng photo compression path

### Error mapping
| Error | Trigger |
|-------|---------|
| `FloodWait { seconds }` | FLOOD_WAIT_X trong lỗi |
| `FileTooLarge` | FILE_TOO_LARGE |
| `Authentication` | AUTH_KEY, SESSION_EXPIRED, Unauthorized |
| `Network` | connection, timeout, reset |
| `InvalidPeer` | PEER_ID_INVALID, CHAT_ID_INVALID, USER_ID_INVALID |
| `PermissionDenied` | CHAT_WRITE_FORBIDDEN, CHAT_SEND_MEDIA_FORBIDDEN |
| `Cancelled` | cancelled |
| `Unknown` | Lỗi khác |

### Tests (không gọi Telegram thật)
1. `test_parse_flood_wait` — parse FLOOD_WAIT_X
2. `test_error_mapping_flood_wait` — flood wait → UploadError
3. `test_error_mapping_auth` — auth → UploadError
4. `test_error_mapping_file_too_large` — file quá lớn
5. `test_error_mapping_network` — network error
6. `test_error_mapping_permission_denied` — permission
7. `test_error_mapping_invalid_peer` — peer không hợp lệ
8. `test_get_deterministic_random_id_consistent` — deterministic random_id
9. `test_get_deterministic_random_id_different` — khác input → khác output
10. `test_map_updates_matching_message_id` — UpdateMessageId khớp
11. `test_map_updates_non_matching_message_id` — UpdateMessageId không khớp → reconciliation_required
12. `test_map_updates_short_sent_message` — UpdateShortSentMessage → confirmed
13. `test_map_ambiguous_updates_requires_reconciliation` — ambiguous → reconciliation_required

---

## 5. Persisted Random ID Lifecycle

```
upload_attempt_id = "job_{job_id}_item_{item_id}_attempt_{attempt}"
    ↓ SHA-256 → first 8 bytes → i64
telegram_random_id (deterministic, persisted trong DB)
    ↓
SendMedia (raw API — chưa active trong vòng này)
    ↓
Response mapping:
  - UpdateMessageId(random_id == persisted) → Confirmed
  - UpdateShortSentMessage → Confirmed
  - Non-matching/non-UpdateMessageId → ReconciliationRequired
```

**Lưu ý**: API cao cấp `client.send_message()` hiện tại không dùng persisted random_id.
Cần upgrade lên raw `tl::functions::messages::SendMedia` trong vòng sau.

---

## 6. Local Adapter

### File
`app/src-tauri/src/migration/adapters_v2/local.rs`

### Path safety
- Chống path traversal: từ chối `..` trong path
- Windows reserved names: CON → CON_safe, LPT1 → LPT1_safe, v.v.
- Absolute path stripping
- Symlink escape: canonicalize() check (nếu backup_root tồn tại)

### Collision
- `collision_safe_path()`: append `_1`, `_2`, ..., fallback timestamp

### Atomic finalization
- Copy source → `.part` file cùng parent với destination
- `sync_all()` trên `.part`
- `rename()` atomic `.part` → destination
- Nếu destination đã tồn tại → collision-safe path
- KHÔNG overwrite im lặng
- KHÔNG cleanup source (source là workspace tạm)
- Cleanup `.part` nếu rename thất bại

### Tests
1. `test_sanitize_normal_path` — path bình thường
2. `test_sanitize_path_traversal_rejected` — từ chối `..`
3. `test_sanitize_absolute_path` — strip absolute prefix
4. `test_sanitize_windows_reserved_name` — CON → CON_safe
5. `test_sanitize_lpt_reserved_name` — LPT1 → LPT1_safe
6. `test_collision_safe_path_no_collision` — không collision
7. `test_collision_safe_path_with_collision` — collision → suffix
8. `test_finalize_local_basic` — finalize cơ bản
9. `test_finalize_local_source_missing` — lỗi khi source không tồn tại
10. `test_finalize_local_no_overwrite` — không overwrite, dùng collision path
11. `test_finalize_local_creates_parent_dirs` — tạo thư mục cha

---

## 7. Factory / Composition Root

### File
`app/src-tauri/src/migration/adapters_v2/factory.rs`

### Function
```rust
pub fn build_pipeline_v2_services(
    db, ms_session, tg_client, tg_peer_cache,
    job_id, workspace_dir, backup_dir, destination_folder_id
) -> Result<(PipelineRunner, OneDriveDownloader, FFmpegMediaAdapter, TelegramProductionAdapter, LocalProductionAdapter, CancelToken), String>
```

### Integration tests (dùng production structs với fake dependencies)
1. `test_adapter_composition_integration`:
   - OneDrive download + wiremock
   - FFmpeg inspect (fake process runner)
   - Local finalize
   - Disk reservation check
   - Path safety
2. `test_destructive_request_guard`:
   - Wiremock DELETE/PATCH mock với `expect(0)`
   - Xác nhận adapter chỉ gọi GET

---

## 8. Inactive Integration Guarantee

### Verified by `rg` search
- `build_pipeline_v2_services`: chỉ có trong `factory.rs` (định nghĩa) và `mod.rs` (re-export)
- KHÔNG được gọi từ:
  - `lib.rs` (Tauri setup/startup)
  - `commands.rs` (Tauri IPC commands)
  - `worker.rs` (V1 worker)
  - Bất kỳ file `app/src/` (React frontend)
- KHÔNG command Tauri mới nào được tạo
- KHÔNG startup hook nào chạy Pipeline V2
- KHÔNG auto-resume V2

### Test-only references
- `pipeline_version = 2` trong DB seed data (test fixtures)
- `PipelineRunner` được khởi tạo trong test code và factory
- `reconciliation_required` trong enum `PipelineStage` và `map_updates_response_v2`

---

## 9. Error Mapping

| Source | Categories |
|--------|-----------|
| OneDrive | Authentication, SourceNotFound, PermissionDenied, RateLimited, TransientNetwork, InvalidResponse, Cancelled |
| FFmpeg | ToolUnavailable, ProbeFailed, TranscodeFailed, Cancelled, InsufficientDisk, InvalidOutput |
| Telegram | FloodWait, FileTooLarge, Authentication, Network, Cancelled, InvalidPeer, PermissionDenied, Unknown |
| Local | FileSystem (io errors), PathTraversal, SymlinkEscape |

---

## 10. Issue Còn Mở

1. **Raw SendMedia**: API cao cấp `client.send_message()` không inject được persisted `random_id`. Cần upgrade lên raw `tl::functions::messages::SendMedia` trong Vòng 2B2B sau khi bump Grammers.

2. **Remux-copy decision**: Pipeline runner hiện chỉ có `passthrough` và `transcode`. Remux-copy path đã được adapter hỗ trợ (build_remux_args) nhưng chưa được kích hoạt trong runner.

3. **Telegram upload progress**: Trait `TelegramUploader` chưa có progress callback. Cần thêm khi pacing engine được triển khai.

4. **Integration test với Client thật**: Chưa có integration test dùng grammers Client thật (yêu cầu session Telegram).

5. **Wiremock dev-dependency**: Đã thêm `wiremock = "0.6"` vào `[dev-dependencies]`.

---

## 11. Phạm Vi 2B2B

Các mục sau thuộc Vòng 2B2B (KHÔNG làm trong vòng này):
- Pacing engine (chống flood Telegram)
- Manifest finalization (JSON/CSV)
- UI integration (React components, MigrationContext)
- Real reconciliation (Telegram history scan)
- Auto-resume production V2
- Destination picker
- Raw SendMedia upgrade
- Tauri command activation

---

## 12. Test Seams

| Adapter | Seam | Dùng cho |
|---------|------|---------|
| OneDrive | `new_with_base_url()` | Wiremock HTTP server |
| FFmpeg | `ProcessRunner` trait | Fake process runner |
| FFmpeg | `new_with_runner()` | Inject fake runner |
| Telegram | `TelegramInvoker` trait (trong telegram_idempotency.rs) | Mock SendMedia response |
| Local | `backup_root` parameter | Temp directory |

---

## 13. File Changes

### Created
- `app/src-tauri/src/migration/adapters_v2/telegram.rs`
- `app/src-tauri/src/migration/adapters_v2/local.rs`
- `app/src-tauri/src/migration/adapters_v2/factory.rs`

### Modified
- `app/src-tauri/Cargo.toml` — thêm `wiremock` dev-dep, `json` feature cho reqwest
- `app/src-tauri/Cargo.lock` — cập nhật tự động
- `app/src-tauri/src/migration/adapters_v2/mod.rs` — pub use factory
- `app/src-tauri/src/migration/adapters_v2/media.rs` — full production implementation
- `app/src-tauri/src/migration/adapters_v2/onedrive.rs` — fix missing bind, add imports
- `app/src-tauri/src/migration/pipeline_v2/runner.rs` — formatting (cargo fmt)
- `app/src-tauri/src/migration/pipeline_v2/transitions.rs` — formatting (cargo fmt)
- `app/src-tauri/src/migration/pipeline_v2_tests.rs` — thêm 7 tests mới

### Test count
- Trước: 71 tests
- Sau: 111 tests (+40 tests)

---

# HANDOFF FOR REVIEW

**Verdict**: `PRODUCTION_ADAPTERS_READY_BUT_INACTIVE`

**Issue còn mở**:
- Raw SendMedia chưa active (cần bump Grammers)
- Remux-copy decision chưa kích hoạt trong runner

**Xác nhận adapters chưa active**: ✅ Không có Tauri command, startup hook, hoặc UI nào gọi Pipeline V2

**Được phép bắt đầu Vòng 2B2B**: ✅ CÓ — sau khi review và accept handoff này

**Phạm vi đề xuất 2B2B**:
1. Upgrade Grammers → raw `SendMedia` với persisted `random_id`
2. Pacing engine + quota enforcement
3. Manifest JSON/CSV finalization
4. Tauri command activation (vẫn feature-gated)
5. Reconciliation bằng Telegram history
