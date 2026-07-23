# Tasks: OneDrive Migration

## Ngôn ngữ

**QUAN TRỌNG**: Toàn bộ nội dung task list này được viết bằng **Tiếng Việt**. Chỉ giữ tên công nghệ, thư viện, biến/hàm/lớp bằng Tiếng Anh.

**Input**: Design documents from `/specs/001-onedrive-migration/`

**Prerequisites**: plan.md (required), spec.md (required), research.md, data-model.md, contracts/ipc-contracts.md, quickstart.md

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Có thể chạy song song (khác file, không phụ thuộc nhau)
- **[Story]**: User story tương ứng (US1, US2, US3)
- Bao gồm đường dẫn file chính xác trong mô tả

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Khởi tạo module structure, dependencies, database schema cơ bản

- [x] T001 Thêm dependency `sha2` vào `app/src-tauri/Cargo.toml`
- [x] T002 Tạo thư mục `app/src-tauri/src/migration/` và file `app/src-tauri/src/migration/mod.rs` với `MigrationState` struct cơ bản (chứa `Arc<Mutex<sqlite::Connection>>`)
- [x] T003 [P] Tạo `app/src-tauri/src/migration/models.rs` với error enums (`MigrationError`), `MsAccountInfo` struct, `OneDriveFolder` struct, `OneDriveItem` struct
- [x] T004 [P] Tạo `app/src-tauri/src/migration/db.rs` với hàm `init_migration_db()` — tạo `migration.db`, bảng `migration_jobs` (schema từ data-model.md), WAL mode, synchronous=FULL
- [x] T005 Đăng ký module `migration` trong `app/src-tauri/src/lib.rs` — thêm `mod migration;`, tạo và manage `MigrationState`

---

## Phase 2: Foundational — OneDrive Connect & Folder Tree (Blocking Prerequisites)

**Purpose**: Microsoft OAuth + OneDrive folder browsing — nền tảng cho toàn bộ feature

**⚠️ CRITICAL**: Đây là phase cốt lõi theo yêu cầu Phase 1 của người dùng — kết nối OneDrive và hiển thị cây thư mục

### Backend — Microsoft OAuth

- [x] T006 Tạo `app/src-tauri/src/migration/microsoft.rs` với struct `MicrosoftSession` — lưu access_token, refresh_token, account info trong process memory (KHÔNG persist ra disk). Trait `MicrosoftApi` cho testability
- [x] T007 Implement OAuth PKCE flow trong `app/src-tauri/src/migration/microsoft.rs` — function `start_oauth_flow()`: generate code_verifier (S256), state parameter, authorization URL với scopes `Files.Read offline_access user.read`, mở browser qua `tauri-plugin-opener`
- [x] T008 Implement local callback server trong `app/src-tauri/src/migration/microsoft.rs` — bind `127.0.0.1` (loopback only), parse authorization code, exchange code for tokens via `POST /common/oauth2/v2.0/token`, timeout 120 giây
- [x] T009 Implement token refresh logic trong `app/src-tauri/src/migration/microsoft.rs` — function `refresh_access_token()` dùng refresh_token, auto-refresh khi access_token hết hạn
- [x] T010 Implement `get_user_profile()` trong `app/src-tauri/src/migration/microsoft.rs` — gọi `GET /me` trả về tên + email

### Backend — OneDrive Folder Browsing

- [x] T011 Implement `list_children()` trong `app/src-tauri/src/migration/microsoft.rs` — gọi `GET /me/drive/items/{item-id}/children` hoặc `GET /me/drive/root/children` cho root. Pagination qua `@odata.nextLink`. Trả về `Vec<OneDriveItem>` (folders + files)
- [x] T012 Implement `list_folder_tree()` trong `app/src-tauri/src/migration/microsoft.rs` — recursive listing cho cây thư mục, trả về `Vec<OneDriveFolder>` với tên, đường dẫn tương đối, số file, tổng dung lượng mỗi thư mục

### Backend — Tauri Commands (Phase 1)

- [x] T013 Tạo `app/src-tauri/src/migration/commands.rs` với command `cmd_migration_ms_connect` — gọi OAuth flow, lưu session, trả `Result<MsAccountInfo, String>`
- [x] T014 [P] Implement command `cmd_migration_ms_disconnect` trong `app/src-tauri/src/migration/commands.rs` — xóa token khỏi memory, trả `Result<(), String>`
- [x] T015 [P] Implement command `cmd_migration_ms_status` trong `app/src-tauri/src/migration/commands.rs` — trả `Result<Option<MsAccountInfo>, String>`
- [x] T016 Implement command `cmd_migration_list_onedrive_folders` trong `app/src-tauri/src/migration/commands.rs` — nhận `parent_id: Option<String>` (None = root), trả `Result<Vec<OneDriveItem>, String>` danh sách children (folders + files)
- [x] T017 Đăng ký tất cả commands Phase 1 trong `app/src-tauri/src/lib.rs` — thêm `cmd_migration_ms_connect`, `cmd_migration_ms_disconnect`, `cmd_migration_ms_status`, `cmd_migration_list_onedrive_folders` vào Tauri builder

### Frontend — UI Components (Phase 1)

- [x] T018 Tạo `app/src/components/migration/OneDriveMigrationPage.tsx` — layout page chính với header "OneDrive Migration", connection status section, folder tree section. Dùng design system hiện có (Tailwind v4)
- [x] T019 Tạo `app/src/components/migration/SetupSection.tsx` — Microsoft connect/disconnect button, account info display, folder tree browser component (lazy loading: gọi `listOneDriveFolders(parentId)` khi mở thư mục, hiển thị tree dạng expandable list)
- [x] T020 Tạo `app/src/hooks/useMigration.ts` — custom hook: `connectMicrosoft()`, `disconnectMicrosoft()`, `getMsStatus()`, `listOneDriveFolders(parentId?)`. State management cho connection status và folder list
- [x] T021 Tạo TypeScript types trong `app/src/types.ts` — interfaces: `MsAccountInfo`, `OneDriveItem`, `OneDriveFolder`, `MigrationJob`, `MigrationItem`, `MigrationStats`
- [x] T022 Thêm nav item "OneDrive Migration" vào `app/src/components/desktop/dashboard/Sidebar.tsx` — icon, label, onClick navigation tới `OneDriveMigrationPage`
- [x] T023 Tích hợp `OneDriveMigrationPage` vào routing/view logic trong `app/src/components/desktop/DesktopDashboard.tsx` — conditional rendering khi chọn migration nav item

### i18n

- [x] T024 [P] Thêm i18n keys cho Phase 1 vào `app/src/i18n/locales/vi.json` — keys cho: kết nối/ngắt kết nối Microsoft, trạng thái kết nối, duyệt thư mục, tên page, labels
- [x] T025 [P] Thêm i18n keys cho Phase 1 vào `app/src/i18n/locales/en.json` — tương ứng với vi.json

**Checkpoint**: Kết nối Microsoft OAuth thành công, hiển thị cây thư mục OneDrive, navigate thư mục con. Phase 1 của người dùng hoàn tất tại đây.

---

## Phase 3: User Story 1 — Thiết lập và scan (Priority: P1) 🎯 MVP

**Goal**: Người dùng tạo migration job, chọn nguồn/đích/local dir, scan snapshot đầy đủ, xem danh sách file + thống kê

**Independent Test**: Hoàn tất setup → scan → xác nhận số file + dung lượng khớp OneDrive

### Implementation

- [x] T026 [US1] Mở rộng `app/src-tauri/src/migration/models.rs` — thêm `MigrationJob`, `MigrationItem`, `MigrationStats`, `FolderSummary`, `JobState` enum, `ItemState` enum đầy đủ theo data-model.md
- [x] T027 [US1] Mở rộng `app/src-tauri/src/migration/db.rs` — thêm bảng `migration_items` (schema từ data-model.md), indexes, hàm `create_job()`, `get_job()`, `get_jobs()`, `delete_job()`, `update_job_config()`, `batch_insert_items()`, `get_items_by_job()`, `get_stats()`
- [x] T028 [US1] Implement `scan_folder_recursive()` trong `app/src-tauri/src/migration/microsoft.rs` — recursive listing với pagination (`@odata.nextLink`), extract metadata (eTag, hashes.quickXorHash, size, lastModifiedDateTime), trả `Vec<OneDriveItem>` flat list
- [x] T029 [US1] Implement commands job management trong `app/src-tauri/src/migration/commands.rs` — `cmd_migration_create_job`, `cmd_migration_get_jobs`, `cmd_migration_get_job`, `cmd_migration_delete_job`
- [x] T030 [US1] Implement commands configuration trong `app/src-tauri/src/migration/commands.rs` — `cmd_migration_set_onedrive_folder`, `cmd_migration_set_telegram_destination`, `cmd_migration_set_local_dir`
- [x] T031 [US1] Implement command `cmd_migration_scan` trong `app/src-tauri/src/migration/commands.rs` — gọi `scan_folder_recursive()`, xóa snapshot cũ, batch insert items, cập nhật stats, chuyển job state `draft`→`ready`
- [x] T032 [US1] Đăng ký commands Phase 3 trong `app/src-tauri/src/lib.rs`
- [x] T033 [US1] Mở rộng `app/src/components/migration/SetupSection.tsx` — thêm job creation form, Telegram destination picker (reuse existing), local folder picker (native dialog), scan button, stats display
- [x] T034 [US1] Tạo `app/src/components/migration/FileTable.tsx` — bảng file list hiển thị: tên, đường dẫn, dung lượng, trạng thái. Virtual scrolling cho danh sách lớn
- [x] T034b [US1] Thêm folder summary display vào `app/src/components/migration/SetupSection.tsx` — sau scan hiển thị danh sách thư mục con (tên, đường dẫn tương đối, số file, tổng dung lượng) theo FR-008. Dữ liệu từ `FolderSummary` trong response `cmd_migration_get_job`
- [x] T035 [US1] Mở rộng `app/src/hooks/useMigration.ts` — thêm functions: `createJob()`, `getJobs()`, `getJob()`, `deleteJob()`, `setOneDriveFolder()`, `setTelegramDest()`, `setLocalDir()`, `scan()`
- [x] T036 [US1] Thêm i18n keys cho Phase 3 vào `app/src/i18n/locales/{vi,en}.json` — keys cho: tạo job, chọn nguồn/đích, scan, thống kê, danh sách file

**Checkpoint**: Tạo job → chọn nguồn/đích/local → scan → xem danh sách file + thống kê. User Story 1 hoàn tất.

---

## Phase 4: User Story 2 — Chạy migration (Priority: P1)

**Goal**: Migration pipeline hoạt động: download → duplicate check → upload → persist → cleanup. Có controls và progress.

**Independent Test**: Migrate 5 file nhỏ → tất cả completed → file tạm xóa → OneDrive không thay đổi

### Implementation

- [x] T037 [US2] Tạo `app/src-tauri/src/migration/upload_adapter.rs` — extract `upload_core()` shared internal function từ `app/src-tauri/src/commands/fs.rs`. Struct `UploadResult { message_id: Option<i32>, file_name: String, file_size: i64 }` và enum `UploadError { FloodWait{seconds}, TelegramLimit, Network, Auth, Cancelled, Unknown }`
- [x] T038 [US2] Extract `parse_flood_wait_seconds()` từ `app/src-tauri/src/commands/utils.rs` — parse error string `FLOOD_WAIT_{seconds}` thành i64
- [x] T039 [US2] Mở rộng `app/src-tauri/src/migration/db.rs` — thêm bảng `migrated_fingerprints`, hàm `check_fingerprint()`, `insert_fingerprint()`, success transaction (atomic: mark item completed + insert fingerprints + update job counters), recovery mapping khi startup
- [x] T040 [US2] Implement stream download trong `app/src-tauri/src/migration/microsoft.rs` — function `download_item()`: stream từ `@microsoft.graph.downloadUrl` vào `.part` file, tính SHA-256 trong stream, validate source eTag/size không đổi trước download
- [x] T041 [US2] Tạo `app/src-tauri/src/migration/worker.rs` — pipeline loop: select pending item → validate job/dir/cooldown → pre-download duplicate check (quickXorHash) → validate source metadata → disk space check → stream download → post-download SHA-256 duplicate check → upload qua `upload_core()` → COMMIT transaction → cleanup temp → check pause/cancel → next item. Nếu upload adapter trả `telegram_file_too_large` → đánh dấu `failed`, không retry (không hardcode size limit, dùng runtime error từ Telegram)
- [x] T042 [US2] Implement control flow trong `app/src-tauri/src/migration/worker.rs` — pause (set flag, hoàn tất file hiện tại rồi dừng), cancel (set flag, không bắt đầu file mới), cooldown gate (check `cooldown_until` trước mỗi upload)
- [x] T043 [US2] Implement retry logic trong `app/src-tauri/src/migration/worker.rs` — auto retry max 3 cho lỗi tạm thời (network, timeout), không retry cho `source_changed`/`telegram_file_too_large`/`auth`
- [x] T044 [US2] Implement commands control trong `app/src-tauri/src/migration/commands.rs` — `cmd_migration_start` (spawn Tokio task), `cmd_migration_pause`, `cmd_migration_resume`, `cmd_migration_cancel`, `cmd_migration_retry_item`, `cmd_migration_retry_all_failed`
- [x] T045 [US2] Implement 5 event emitters trong `app/src-tauri/src/migration/commands.rs` — `migration:job-state`, `migration:item-progress`, `migration:item-complete`, `migration:stats`, `migration:cooldown` (payloads theo ipc-contracts.md)
- [x] T046 [US2] Đăng ký commands Phase 4 trong `app/src-tauri/src/lib.rs`
- [x] T047 [US2] Tạo `app/src/components/migration/ProgressPanel.tsx` — summary stats (total/completed/failed/skipped), current file progress (download %, upload %), controls (start/pause/resume/cancel/retry all failed), cooldown display
- [x] T048 [US2] Mở rộng `app/src/components/migration/FileTable.tsx` — thêm cột trạng thái với color coding, error display, retry button cho từng file failed
- [x] T049 [US2] Mở rộng `app/src/hooks/useMigration.ts` — thêm event listeners cho 5 events, functions: `startMigration()`, `pause()`, `resume()`, `cancel()`, `retryItem()`, `retryAllFailed()`
- [x] T050 [US2] Thêm i18n keys cho Phase 4 vào `app/src/i18n/locales/{vi,en}.json` — keys cho: controls, progress, status labels, error messages, cooldown display

**Checkpoint**: Migration chạy end-to-end: start → download → upload → complete. Controls hoạt động.

---

## Phase 5: User Story 3 — Resume, retry và phát hiện trùng (Priority: P2)

**Goal**: Resume sau restart, manual retry, duplicate detection cross-job

**Independent Test**: Migrate vài file → đóng app → mở lại → file completed giữ nguyên → nhấn Resume → hoàn tất

### Implementation

- [x] T051 [US3] Implement startup recovery trong `app/src-tauri/src/migration/db.rs` — khi init: reset `downloading`/`uploading` → `pending` + `recovery_interrupted`, cleanup `.part` files, KHÔNG tăng attempt_count, KHÔNG auto-start job
- [x] T052 [US3] Mở rộng `app/src/components/migration/OneDriveMigrationPage.tsx` — hiển thị job history, resume button khi có job paused/running (sau restart), Microsoft reconnect prompt nếu cần
- [x] T053 [US3] Mở rộng `app/src/hooks/useMigration.ts` — thêm startup check: load existing jobs, detect Microsoft connection state, show reconnect prompt if needed
- [x] T054 [US3] Thêm i18n keys cho Phase 5 vào `app/src/i18n/locales/{vi,en}.json` — keys cho: resume, recovery, reconnect Microsoft

**Checkpoint**: Resume sau restart hoạt động. File completed không bị upload lại. Duplicate cross-job detection hoạt động.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Tests, hardening, quickstart verification

- [x] T055 [P] Tạo unit tests cho state transitions trong `app/src-tauri/src/migration/db.rs` — test: job state machine (draft→ready→running→paused→completed/cancelled/failed), one-active-job guard
- [x] T056 [P] Tạo unit tests cho duplicate logic trong `app/src-tauri/src/migration/worker.rs` — test: pre-download quickxor match, post-download SHA-256 match, hash type mismatch ≠ duplicate, same content/different path = duplicate, same name/different content ≠ duplicate
- [x] T057 [P] Tạo unit tests cho retry logic — test: auto retry max 3, manual retry reset counter, no retry cho `source_changed`/`telegram_file_too_large`
- [x] T058 [P] Tạo unit tests cho recovery mapping — test: downloading→pending+recovery_interrupted, uploading→pending+recovery_interrupted, completed→completed, không tăng attempt_count
- [x] T059 [P] Tạo integration tests với fake Microsoft adapter — test: recursive scan with pagination, empty source, snapshot totals, source changed detection
- [x] T060 [P] Tạo integration tests với fake Telegram upload adapter — test: upload success transaction (atomic commit), cooldown gate, file too large no retry
- [x] T061 Chạy `tsc --noEmit` để verify TypeScript types trong `app/`
- [x] T062 Chạy `npm run build` để verify production build trong `app/`
- [x] T063 Verify 7 quickstart scenarios từ `specs/001-onedrive-migration/quickstart.md` — manual testing theo từng scenario A-G
- [x] T064 Code cleanup — review error handling, logging, edge cases across all migration modules

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)**: Không phụ thuộc — bắt đầu ngay
- **Phase 2 (Foundation — OneDrive Connect & Tree)**: Phụ thuộc Phase 1 — BLOCKS tất cả phases sau
- **Phase 3 (US1 — Scan)**: Phụ thuộc Phase 2
- **Phase 4 (US2 — Migration)**: Phụ thuộc Phase 3
- **Phase 5 (US3 — Resume/Retry)**: Phụ thuộc Phase 4
- **Phase 6 (Polish)**: Phụ thuộc Phase 5

### Execution Order

```text
Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 5 → Phase 6
  Setup    Connect   Scan     Migrate   Resume    Tests
           & Tree             Worker    & Retry   & Polish
```

### Parallel Opportunities trong mỗi Phase

- **Phase 1**: T003 ∥ T004 (models ∥ db)
- **Phase 2**: T014 ∥ T015 (disconnect ∥ status), T024 ∥ T025 (vi ∥ en i18n)
- **Phase 6**: T055 ∥ T056 ∥ T057 ∥ T058 ∥ T059 ∥ T060 (tất cả tests song song)

---

## Implementation Strategy

### MVP First (Phase 1 + Phase 2)

1. Hoàn tất Phase 1: Setup module structure
2. Hoàn tất Phase 2: OneDrive Connect + Folder Tree
3. **STOP và VALIDATE**: Kết nối Microsoft, duyệt thư mục thành công
4. Đây là mục tiêu Phase 1 của người dùng ✅

### Incremental Delivery

1. Phase 1 + 2 → Kết nối + duyệt thư mục (MVP đầu tiên)
2. Phase 3 → Scan + thống kê
3. Phase 4 → Migration pipeline hoạt động
4. Phase 5 → Resume/retry/duplicate
5. Phase 6 → Tests + hardening → Ready cho converge

---

## Notes

- [P] = khác file, không phụ thuộc → có thể chạy song song
- [US1/US2/US3] = map tới user story tương ứng trong spec.md
- Commit sau mỗi task hoặc nhóm logic
- Dừng tại bất kỳ checkpoint nào để validate độc lập
- Microsoft token KHÔNG persist — sau restart cần reconnect
- Không hardcode Telegram file size limit
