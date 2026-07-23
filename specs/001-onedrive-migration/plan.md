# Implementation Plan: OneDrive Migration (MVP)

**Branch**: `001-onedrive-migration` | **Ngày**: 2026-07-23 | **Spec**: [spec.md](./spec.md) | **Constitution**: v1.2.0

**Input**: Feature specification từ `/specs/001-onedrive-migration/spec.md`

## Ngôn ngữ

**QUAN TRỌNG**: Toàn bộ nội dung plan này được viết bằng **Tiếng Việt**. Chỉ giữ tên công nghệ, thư viện, biến/hàm/lớp bằng Tiếng Anh.

## Summary

Tích hợp tính năng OneDrive Migration vào Telegram-Drive desktop, cho phép người dùng chọn một thư mục OneDrive làm nguồn, quét danh sách file (snapshot cố định), rồi download tuần tự từng file về local và upload lên Telegram destination. Worker chạy in-process trong Tauri Rust backend dưới dạng Tokio task. Tái sử dụng logic upload hiện có qua shared internal function — manual upload command giữ nguyên policy hiện tại, migration worker có policy riêng. UI là một trang đơn (`OneDriveMigrationPage`) tích hợp vào sidebar navigation.

## Technical Context

**Language/Version**: Rust 1.75+ (backend), TypeScript 5.x (frontend)

**Primary Dependencies**:
- **Backend**: Tauri v2, grammers-client (Telegram MTProto), reqwest v0.12 (HTTP + SOCKS cho Microsoft Graph), serde (serialization), sha2 (SHA-256), sqlite (migration.db), tokio (async runtime), log + env_logger (logging)
- **Frontend**: React 19, TypeScript, Tailwind CSS v4, @tanstack/react-query, sonner (toast), @tauri-apps/api/core, @tauri-apps/api/event, @tauri-apps/plugin-dialog, react-i18next

**Storage**: SQLite (`migration.db`), tách biệt với `shares.db` hiện có. WAL mode, synchronous=FULL. 3 business tables: `migration_jobs`, `migration_items`, `migrated_fingerprints`.

**Testing**: Backend unit/integration tests dùng temporary SQLite + fake Microsoft/Telegram adapters. Frontend validation: type-check + production build + manual quickstart.

**Target Platform**: Desktop — Windows, macOS, Linux (Tauri desktop). Không hỗ trợ Android/iOS trong MVP.

**Project Type**: Desktop application — React frontend + Tauri Rust backend, in-process background worker.

**Performance Goals**:
- UI không bị đơ > 2 giây trong suốt quá trình migration
- Xử lý ít nhất 100 file tuần tự không crash, không mất trạng thái
- File tạm local bị xóa trong vòng 5 giây sau upload thành công

**Constraints**:
- Chỉ 1 migration job `running` tại một thời điểm
- Chỉ 1 file được xử lý tại một thời điểm (tuần tự)
- Không đọc toàn bộ file vào RAM (stream download/upload)
- Không được xóa/thay đổi file nguồn trên OneDrive
- Không log Microsoft access token, refresh token, Telegram session
- Microsoft token chỉ trong Rust process memory, không persist ra disk
- Không hardcode Telegram file size limit — nếu file quá lớn, upload adapter trả `telegram_file_too_large`, không auto-retry

**Scale/Scope**:
- Hỗ trợ thư mục OneDrive với hàng trăm đến hàng nghìn file
- Một tài khoản Microsoft, một Telegram destination mỗi job
- ~15 Tauri commands, 5 events, 1 frontend page, 3 business tables
- 5 phases implementation

## Constitution Check

| Nguyên tắc | Trạng thái | Ghi chú |
|-----------|-----------|--------|
| **I. Actix Web** | ✅ PASS | Worker dùng Tokio task. Được phép theo Amendment 1.2.0. |
| **II. Tauri IPC + React** | ✅ PASS | Frontend `invoke`/`listen`, React Context. |
| **III. Telegram MTProto** | ✅ PASS | Dùng chung `TelegramState`, shared internal upload function. |
| **IV. SQLite** | ✅ PASS | `migration.db` riêng, WAL+FULL, raw SQL. 3 business tables. |
| **V. Spec-Driven** | ✅ PASS | Spec đã clarify và hoàn thiện. |
| **VI. Background Processing (v1.2.0)** | ✅ PASS | MVP chỉ chạy khi Tauri process mở. Amendment 1.2.0: tray/autostart không bắt buộc. Manual Resume hợp constitution. |
| **i18n** | ✅ PASS | UI text qua i18n keys. |
| **Error handling** | ✅ PASS | Enum error cho migration module; map về String ở IPC boundary. |

**Gate Evaluation**: Tất cả nguyên tắc PASS. Không TENSION, không accepted violation.

## Project Structure

### Source Code

```text
app/
├── src/
│   ├── components/
│   │   ├── desktop/dashboard/
│   │   │   └── Sidebar.tsx           # ← Thêm nav item "OneDrive Migration"
│   │   └── migration/                # ← THƯ MỤC MỚI (4 components)
│   │       ├── OneDriveMigrationPage.tsx
│   │       ├── SetupSection.tsx       # Connect + chọn folder + scan
│   │       ├── ProgressPanel.tsx      # Summary + current progress + controls
│   │       └── FileTable.tsx          # Danh sách file + status + retry
│   ├── hooks/
│   │   └── useMigration.ts
│   ├── i18n/locales/{vi,en}.json
│   └── types.ts
│
└── src-tauri/
    ├── Cargo.toml                    # ← Thêm dep: sha2
    └── src/
        ├── lib.rs                    # ← Đăng ký MigrationState
        ├── commands/
        │   ├── fs.rs                 # ← Extract shared internal upload function
        │   └── utils.rs              # ← Extract parse_flood_wait_seconds()
        └── migration/                # ← THƯ MỤC MỚI (7 modules)
            ├── mod.rs                # MigrationState + orchestrator registration
            ├── models.rs             # Structs, enums, states, errors
            ├── db.rs                 # Schema, repos, transactions, recovery
            ├── microsoft.rs          # OAuth session, Graph API, scan, download
            ├── worker.rs             # Pipeline: validate→download→upload→persist→next
            ├── upload_adapter.rs     # Shared internal upload function seam
            └── commands.rs           # ~15 Tauri commands + 5 event emitters
```

**Structure Decision**: Backend 7 modules, frontend 1 page + 3 supporting components.

## Implementation Phases

### Phase 1 — OneDrive Connect & Folder Tree (Foundation)

**Goal**: Người dùng có thể kết nối tài khoản Microsoft và duyệt cây thư mục OneDrive. Đây là nền tảng cho toàn bộ feature.

**Code areas**:
- `migration/mod.rs` — MigrationState struct, module registration
- `migration/models.rs` — Structs cơ bản: MsAccountInfo, OneDriveFolder, OneDriveItem, error enums
- `migration/db.rs` — Schema creation (`migration.db`), bảng `migration_jobs` (cơ bản)
- `migration/microsoft.rs` — OAuth PKCE flow, token refresh, folder listing, recursive tree
- `migration/commands.rs` — Commands: `cmd_migration_ms_connect`, `cmd_migration_ms_disconnect`, `cmd_migration_ms_status`, `cmd_migration_list_onedrive_folders`
- `lib.rs` — Đăng ký MigrationState + commands
- `Cargo.toml` — Thêm dep: `sha2`
- `components/migration/OneDriveMigrationPage.tsx` — Page chính
- `components/migration/SetupSection.tsx` — Connect button + folder tree display
- `Sidebar.tsx` — Thêm nav item "OneDrive Migration"
- `hooks/useMigration.ts` — Hook cơ bản: connect/disconnect/status/list folders
- `types.ts` — TypeScript types cho migration
- `i18n/locales/{vi,en}.json` — i18n keys cho Phase 1

**Dependencies**: `reqwest` (đã có), `sqlite` (đã có), `tauri-plugin-opener` (đã có)

**Independent validation**:
- Manual: Mở app → click "OneDrive Migration" → kết nối Microsoft → thấy cây thư mục hiển thị
- Unit: OAuth PKCE code_verifier/challenge generation, token parsing
- Schema: `migration.db` tạo thành công với bảng `migration_jobs`

**Exit criteria**:
- ✅ Kết nối Microsoft OAuth thành công, hiển thị tên tài khoản
- ✅ Ngắt kết nối Microsoft hoạt động
- ✅ Hiển thị danh sách thư mục OneDrive (đệ quy) với tên, số file, dung lượng
- ✅ Navigate giữa các thư mục con
- ✅ migration.db tạo thành công
- ✅ i18n cho vi/en hoạt động
- ✅ Sidebar nav item hiển thị đúng

---

### Phase 2 — Scan Snapshot & Job Setup

**Goal**: Người dùng có thể tạo migration job, chọn nguồn/đích/thư mục local, scan snapshot đầy đủ.

**Code areas**:
- `migration/db.rs` — Bảng `migration_items`, batch insert, stats queries
- `migration/models.rs` — MigrationJob, MigrationItem, MigrationStats, FolderSummary structs đầy đủ
- `migration/microsoft.rs` — Recursive scan với pagination, metadata extraction (eTag, hashes)
- `migration/commands.rs` — Commands: `cmd_migration_create_job`, `cmd_migration_get_jobs`, `cmd_migration_get_job`, `cmd_migration_delete_job`, `cmd_migration_set_onedrive_folder`, `cmd_migration_set_telegram_destination`, `cmd_migration_set_local_dir`, `cmd_migration_scan`
- `components/migration/SetupSection.tsx` — Job creation, source/dest/local picker, scan trigger
- `components/migration/FileTable.tsx` — Danh sách file pending
- `hooks/useMigration.ts` — Thêm job management, scan

**Dependencies**: Phase 1

**Independent validation**:
- Manual: Tạo job → chọn thư mục OneDrive + Telegram dest + local dir → scan → xác nhận số file + tổng dung lượng khớp
- Unit: Batch insert items, stats calculation, one-active-job guard

**Exit criteria**:
- ✅ Scan totals chính xác (khớp OneDrive)
- ✅ Pagination hoạt động cho thư mục lớn
- ✅ Snapshot persisted trong migration.db
- ✅ Job state machine: draft → ready hoạt động

---

### Phase 3 — Sequential Migration Worker

**Goal**: Worker pipeline hoạt động: download → duplicate check → upload → persist → cleanup → next file.

**Code areas**:
- `migration/worker.rs` — Pipeline loop: validate → download → SHA-256 → duplicate check → upload → COMMIT → cleanup → pause/cancel check → next
- `migration/upload_adapter.rs` — Shared upload function seam, FloodWait handling
- `migration/microsoft.rs` — Stream download to .part file
- `migration/db.rs` — Bảng `migrated_fingerprints`, success transaction, recovery mapping
- `commands/fs.rs` — Extract `upload_core()` shared internal function
- `commands/utils.rs` — Extract `parse_flood_wait_seconds()`
- `migration/commands.rs` — Commands: `cmd_migration_start`, `cmd_migration_pause`, `cmd_migration_resume`, `cmd_migration_cancel`, `cmd_migration_retry_item`, `cmd_migration_retry_all_failed`

**Dependencies**: Phase 2

**Independent validation**:
- Unit: Duplicate logic (pre-download quickxor, post-download SHA-256), retry max 3, cooldown gate, recovery mapping, one-active-job enforcement
- Integration: Mock upload 5 files tuần tự, duplicate detected, cooldown respected

**Exit criteria**:
- ✅ 5 files migrated tuần tự thành công
- ✅ Duplicate detection hoạt động (pre-download + post-download)
- ✅ Cooldown respected, persist `cooldown_until`
- ✅ Pause/resume/cancel hoạt động
- ✅ Retry max 3, manual retry reset counter
- ✅ File tạm xóa sau success/duplicate

---

### Phase 4 — IPC Events & UI Progress

**Goal**: UI hoàn chỉnh với progress events, controls, file table.

**Code areas**:
- `migration/commands.rs` — 5 event emitters: `migration:job-state`, `migration:item-progress`, `migration:item-complete`, `migration:stats`, `migration:cooldown`
- `migration/mod.rs` — MigrationState registration đầy đủ
- `lib.rs` — Đăng ký tất cả commands + state
- `components/migration/ProgressPanel.tsx` — Summary stats + current file progress + controls (start/pause/resume/cancel/retry)
- `components/migration/FileTable.tsx` — File list với status, error display, retry button
- `hooks/useMigration.ts` — Event listeners, command wrappers đầy đủ
- `i18n/locales/{vi,en}.json` — i18n keys cho Phases 2-4

**Dependencies**: Phase 3

**Independent validation**:
- Manual: Full UI flow — setup → scan → start → watch progress → pause → resume → complete
- Progress bars hiển thị download/upload %
- All controls hoạt động từ UI

**Exit criteria**:
- ✅ UI hoàn chỉnh: setup → scan → migrate → complete
- ✅ Progress visible: download %, upload %, file name
- ✅ All controls: start/pause/resume/cancel/retry
- ✅ Toast notifications cho success/error/cooldown
- ✅ i18n vi/en cho toàn bộ UI

---

### Phase 5 — Tests & Hardening

**Goal**: Unit tests, integration tests, quickstart verification, edge case hardening.

**Code areas**:
- All `#[cfg(test)]` modules
- Fake Microsoft adapter (trait-based)
- Fake Telegram upload adapter
- Temporary SQLite cho test
- quickstart.md verification

**Dependencies**: Phase 4

**Independent validation**:
- All 7 quickstart scenarios pass
- Unit tests green
- Type-check + build pass

**Exit criteria**:
- ✅ Unit tests: state transitions, one-active-job, retry, duplicate logic, recovery mapping
- ✅ Integration tests: scan + pagination, download stream, upload success transaction, cooldown gate
- ✅ All 7 quickstart scenarios verified
- ✅ `tsc --noEmit` pass
- ✅ `npm run build` pass
- ✅ Gate: PASS cho `/speckit-converge`

---

## Duplicate Detection Strategy

Hai tầng kiểm tra, fingerprint types khác nhau:

1. **Pre-download**: OneDrive `quickXorHash` (type `onedrive_quickxor`) → check `migrated_fingerprints` với composite key `(fingerprint_type, fingerprint_value, file_size)`. Match → skip, không download.
2. **Post-download**: SHA-256 tính từ stream download (type `sha256`) → check `migrated_fingerprints`. Match → skip upload, xóa file tạm.

**Quy tắc**: Không so sánh hash khác thuật toán. Không dùng filename/path/mtime. File size phải khớp.

## Upload Seam

**Shared core** (`upload_core`): Nhận raw dependencies, trả `Result<UploadResult, UploadError>`. Không tự retry/sleep.

- **Manual adapter**: Giữ nguyên retry/sleep policy hiện tại.
- **Migration adapter**: Nhận `FloodWait{seconds}`, persist `cooldown_until`, không upload mới trước expiry, tự tiếp tục khi hết.

## OAuth Flow

- Authorization Code + PKCE (S256), public-client app registration
- System browser via `tauri-plugin-opener`
- Redirect URI: `http://localhost` (loopback, đã đăng ký trong app registration)
- Callback server bind `127.0.0.1` only
- `state` parameter cho CSRF protection
- Timeout 120 giây
- Không log code/token

## Token Handling

- Access/refresh token chỉ trong Rust process memory
- Không persist ra SQLite
- Sau app restart: cần reconnect Microsoft → Manual Resume
- Job/snapshot/progress vẫn persist bình thường

## Recovery Mapping

| Persisted state | Recovery state | Ghi chú |
|---|---|---|
| `pending` | `pending` | Giữ nguyên |
| `downloading` | `pending` + `recovery_interrupted` | Cleanup `.part` |
| `uploading` | `pending` + `recovery_interrupted` | Cleanup `.part` |
| `completed` | `completed` | Giữ nguyên |
| `skipped_duplicate` | `skipped_duplicate` | Giữ nguyên |
| `failed` | `failed` | Giữ nguyên |

Không tăng `attempt_count` chỉ vì restart. Không auto-start.

## Test Strategy

### Fakeable Boundaries

- **Microsoft**: Trait cho list folders, scan snapshot, get metadata, download item
- **Telegram**: Fakeable upload adapter
- **Database**: Temporary SQLite (`":memory:"` hoặc temp file)

### Rust Tests

- State/control transitions
- One active job
- Retry max 3, manual retry reset
- Completed/duplicate not reselected
- Provider fingerprint exact-type matching
- Hash type mismatch not duplicate
- Same content/different path = duplicate
- Same name/different content = not duplicate
- Post-download SHA-256 duplicate
- Temp cleanup
- Recovery mapping

### Frontend

- Type-check (`tsc --noEmit`)
- Production build
- Manual quickstart

## Complexity Tracking

| Vi phạm | Lý do cần | Tại sao không dùng cách đơn giản hơn |
|----------|-----------|-------------------------------------|
| Database riêng | Tránh rủi ro corruption cho shares.db | Merge tăng phức tạp migration script, risk dữ liệu hiện có |
| Worker ownership (Arc<AtomicBool>) | Ngăn multiple concurrent workers | DB guard là lớp thứ hai; atomic bool đơn giản nhất |
| Extract internal upload function | Worker cần gọi upload không qua Tauri IPC | REST localhost thêm latency, phức tạp port; copy-paste vi phạm DRY |
