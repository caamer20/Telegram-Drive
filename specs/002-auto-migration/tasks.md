# Tasks: Automated Account Migration

## Ngôn ngữ

**QUAN TRỌNG**: Toàn bộ nội dung task list này được viết bằng **Tiếng Việt**. Tên công nghệ, thư viện, biến, hàm và lớp giữ nguyên bằng Tiếng Anh.

**Input**: Design documents từ `/specs/002-auto-migration/`

**Prerequisites**: `plan.md`, `spec.md`, `research.md`, `data-model.md`, `contracts/`, `quickstart.md`

**Tests**: Spec yêu cầu tính toàn vẹn state, thứ tự tuần tự, persistence và quota; các task test là bắt buộc.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Có thể thực hiện song song vì khác file và không phụ thuộc task chưa hoàn tất.
- **[Story]**: User story tương ứng.
- Mọi task đều có đường dẫn file cụ thể.

---

## Phase 1: Setup

**Purpose**: Chuẩn bị hạ tầng test và module dùng chung.

- [X] T001 Thêm module Microsoft session store vào `app/src-tauri/src/migration/mod.rs` và khai báo file `app/src-tauri/src/migration/session_store.rs`
- [X] T002 [P] Cấu hình Vitest, Testing Library, jsdom và scripts test trong `app/package.json` và `app/vite.config.ts`
- [X] T003 [P] Tạo test setup cho DOM matcher và mock Tauri IPC/events trong `app/src/test/setup.ts`

---

## Phase 2: Foundational

**Purpose**: Hoàn thiện schema, DTO và primitive state dùng bởi cả hai user story.

**⚠️ CRITICAL**: Không bắt đầu user story trước khi phase này hoàn tất.

- [X] T004 Mở rộng schema idempotent với `active_job_id`, `pause_reason`, `job_origin`, `queue_position`, activity và index trong `app/src-tauri/src/migration/db.rs`
- [X] T005 [P] Mở rộng Rust DTO cho persisted session, auto status, activity, quota và versioned progress event trong `app/src-tauri/src/migration/models.rs`
- [X] T006 [P] Mở rộng TypeScript types tương ứng cho auto profile, activity, quota và progress event trong `app/src/types.ts`
- [X] T007 Cài repository methods cho active snapshot ownership, ordered queue, activity và local-day quota transaction trong `app/src-tauri/src/migration/db.rs`
- [X] T008 [P] Cài session file atomic, permission Unix `0600`, load/save/delete và tests round-trip trong `app/src-tauri/src/migration/session_store.rs`
- [X] T009 Tích hợp `client_id` và callback lưu refresh-token rotation vào Microsoft session lifecycle trong `app/src-tauri/src/migration/microsoft.rs`
- [X] T010 Khôi phục Microsoft session trước khi khởi động auto engine và đăng ký state/commands trong `app/src-tauri/src/lib.rs`

**Checkpoint**: Schema và session persistence sẵn sàng, không log hoặc lưu token trong repository.

---

## Phase 3: User Story 1 — Kích hoạt Tự Động Migrate (Priority: P1) 🎯 MVP

**Goal**: Một lần bật Master Switch tạo snapshot root có thứ tự, migrate tuần tự vào Saved Messages, resume không rescan và tuân thủ quota.

**Independent Test**: Connect Microsoft, bật switch, xác nhận snapshot root được tạo một lần, từng file lần lượt download rồi upload; restart resume cùng snapshot; tắt switch pause; Quét lại chỉ hoạt động khi không running.

### Tests for User Story 1

- [X] T011 [P] [US1] Thêm unit tests cho stable ordering, empty snapshot, ownership và no-rescan trong `app/src-tauri/src/migration/auto_engine.rs`
- [X] T012 [P] [US1] Thêm unit tests cho projected quota, local midnight, manual-job exclusion và atomic completion accounting trong `app/src-tauri/src/migration/db.rs`
- [X] T013 [P] [US1] Thêm worker tests cho sequential single-item execution, pause boundary và Saved Messages mặc định trong `app/src-tauri/src/migration/worker.rs`
- [X] T014 [P] [US1] Thêm command contract tests cho toggle, auto status và rescan errors trong `app/src-tauri/src/migration/commands.rs`

### Implementation for User Story 1

- [X] T015 [US1] Cài initial OneDrive-root snapshot, app-private temp mặc định, persist snapshot rỗng, stable `queue_position`, profile ownership và resume đúng `active_job_id` trong `app/src-tauri/src/migration/auto_engine.rs`
- [X] T016 [US1] Cài Master Switch pause/resume, ngăn auto engine nhận manual job và không tự rescan khi đã có snapshot trong `app/src-tauri/src/migration/auto_engine.rs`
- [X] T017 [US1] Cài manual rescan transaction và guard `migration_running` trong `app/src-tauri/src/migration/auto_engine.rs`
- [X] T018 [US1] Cài ordered worker selection, phase transitions loại trừ nhau và safe pause boundary trong `app/src-tauri/src/migration/worker.rs`
- [X] T019 [US1] Cài quota preflight 250 GiB theo ngày local chỉ cho auto job, atomic item completion/quota increment và midnight resume trong `app/src-tauri/src/migration/worker.rs`
- [X] T020 [US1] Dùng Saved Messages khi destination null và giữ destination tùy chỉnh đã lưu trong `app/src-tauri/src/migration/upload_adapter.rs`
- [X] T021 [US1] Persist activity thật ở scan/download/upload/completed/failed/quota transitions trong `app/src-tauri/src/migration/auto_engine.rs` và `app/src-tauri/src/migration/worker.rs`
- [X] T022 [US1] Expose `cmd_migration_toggle_auto`, `cmd_migration_get_auto_status`, `cmd_migration_rescan_auto`, activity và local quota contracts trong `app/src-tauri/src/migration/commands.rs`
- [X] T023 [US1] Đăng ký commands mới và bảo đảm Disconnect xóa persisted Microsoft session trong `app/src-tauri/src/lib.rs` và `app/src-tauri/src/migration/commands.rs`

**Checkpoint**: Auto Migration backend hoạt động độc lập, tuần tự, bền vững và không tự quét lại.

---

## Phase 4: User Story 2 — Smart Auto-Migration Dashboard (Priority: P2)

**Goal**: Chỉ hiển thị dữ liệu sau connect, có hai transfer list phase-exclusive và activity từ dữ liệu thật.

**Independent Test**: Khi disconnected chỉ thấy Connection Gate; khi connected file downloading chỉ ở OneDrive list, chuyển sang uploading chỉ ở Telegram list trong dưới 2 giây; activity khớp event backend; event trùng/cũ không làm sai UI.

### Tests for User Story 2

- [X] T024 [P] [US2] Viết reducer/selectors tests cho duplicate, out-of-order, attempt mới và phase exclusivity trong `app/src/context/MigrationContext.test.tsx`
- [X] T025 [P] [US2] Viết component tests cho disconnected gate, connected empty states, rescan disabled khi running và chuyển phase trong tối đa 2 giây trong `app/src/components/migration/AutoMigrationCenter.test.tsx`
- [X] T026 [P] [US2] Viết component tests cho Download List, Upload List và Activity Stream ánh xạ dữ liệu thật trong `app/src/components/migration/TransferList.test.tsx`

### Implementation for User Story 2

- [X] T027 [P] [US2] Tạo Connection Gate chỉ render trạng thái tài khoản và hành động connect trong `app/src/components/migration/ConnectionGate.tsx`
- [X] T028 [P] [US2] Tạo Transfer List tái sử dụng cho phase download/upload và empty state độc lập trong `app/src/components/migration/TransferList.tsx`
- [X] T029 [P] [US2] Tạo Activity Stream chỉ nhận persisted activity/event thật trong `app/src/components/migration/ActivityStream.tsx`
- [X] T030 [P] [US2] Tách Advanced Settings và persist destination/temp overrides trong `app/src/components/migration/AdvancedSettingsDrawer.tsx`
- [X] T031 [US2] Tạo `MigrationContext` với listener lifecycle một lần, authoritative hydration và reducer chống event trùng/cũ trong `app/src/context/MigrationContext.tsx`
- [X] T032 [US2] Chuyển `useMigration` thành consumer API của context và loại bỏ global progress suy diễn trong `app/src/hooks/useMigration.ts`
- [X] T033 [US2] Tái cấu trúc dashboard với hard connection gate, hai transfer list, activity thật, quota và nút Quét lại trong `app/src/components/migration/AutoMigrationCenter.tsx`
- [X] T034 [US2] Bọc trang bằng `MigrationContext`, xử lý disconnect tức thời và giữ snapshot table tách biệt activity trong `app/src/components/migration/OneDriveMigrationPage.tsx`
- [X] T035 [US2] Bổ sung toàn bộ text mới và empty/error labels bằng tiếng Việt/Anh trong `app/src/i18n/locales/vi.json` và `app/src/i18n/locales/en.json`

**Checkpoint**: Dashboard không hiển thị file trước connect và phản ánh chính xác hai phase transfer.

---

## Phase 5: Polish & Cross-Cutting Concerns

**Purpose**: Xác minh bảo mật, hồi quy và tài liệu vận hành.

- [X] T036 [P] Thêm integration coverage cho restart restore, refresh rotation, disconnect deletion và no-rescan trong `app/src-tauri/src/migration/tests.rs`
- [X] T037 Thêm integration coverage cho manual rescan, empty root, one-active-auto-job và activity persistence trong `app/src-tauri/src/migration/tests.rs`
- [X] T038 Quét source/log fixtures để bảo đảm token không xuất hiện và cập nhật ignore rule nếu cần trong `/Users/manhtuong/Documents/GitHub/Telegram-Drive/.gitignore`
- [X] T039 Chạy `cargo fmt --check`, `cargo test --lib` và `cargo check` từ `app/src-tauri`
- [X] T040 Chạy frontend tests, `npx tsc --noEmit` và `npm run build` từ `app`
- [X] T041 Thực hiện toàn bộ kịch bản A–G và ghi kết quả xác minh trong `specs/002-auto-migration/quickstart.md`

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: Không phụ thuộc.
- **Foundational (Phase 2)**: Phụ thuộc Phase 1; chặn cả hai user story.
- **US1 (Phase 3)**: Phụ thuộc Foundational; cung cấp backend authoritative state.
- **US2 (Phase 4)**: Phụ thuộc contracts/state của US1, nhưng test reducer và component thuần có thể bắt đầu sau Foundational.
- **Polish (Phase 5)**: Phụ thuộc US1 và US2.

### User Story Dependencies

- **US1 (P1)**: Độc lập sau Foundational và là MVP.
- **US2 (P2)**: Dùng IPC/activity contract từ US1; UI vẫn được test độc lập bằng fixture/mock.

### Within Each User Story

- Viết test trước và xác nhận test thất bại vì hành vi còn thiếu.
- Schema/model trước repository, repository trước engine/worker, engine/worker trước IPC.
- Reducer/context trước dashboard integration.
- Hoàn thành checkpoint trước phase kế tiếp.

### Parallel Opportunities

- T002 và T003 có thể thực hiện song song.
- T005, T006 và T008 có thể thực hiện song song sau T001.
- T011–T014 có thể viết song song sau Foundational.
- T024–T026 có thể viết song song.
- T027–T030 có thể triển khai song song trước khi ghép vào context/dashboard.
- T036 và T037 có thể chia theo nhóm scenario nhưng cùng file nên cần phối hợp merge tuần tự.

## Parallel Example: User Story 2

```text
Task: T027 ConnectionGate.tsx
Task: T028 TransferList.tsx
Task: T029 ActivityStream.tsx
Task: T030 AdvancedSettingsDrawer.tsx
```

---

## Implementation Strategy

### MVP First

1. Hoàn thành Setup và Foundational.
2. Hoàn thành US1 để có auto snapshot, sequential worker, session restore và quota đúng.
3. Chạy backend tests và xác nhận resume không rescan.

### Incremental Delivery

1. Backend authoritative state và persistence.
2. Connection gate cùng hai transfer list.
3. Activity stream, Advanced Settings và UI hardening.
4. Full regression, credential hygiene và quickstart A–G.

## Format Validation

- Tất cả task dùng checkbox `- [ ]`.
- ID liên tục từ T001 đến T041.
- Task user story có `[US1]` hoặc `[US2]`.
- Marker `[P]` chỉ dùng cho công việc khác file hoặc test có thể tách biệt.
- Mọi task chứa đường dẫn file cụ thể.

---

## Phase 6: Convergence

- [X] T042 Hydrate Download List và Upload List trực tiếp từ persisted item phase khi chưa có progress event mới trong `app/src/components/migration/transferState.ts` và `app/src/components/migration/TransferList.tsx` per FR-010, FR-011, SC-006 (partial)
- [X] T043 Bổ sung `event_id`, `attempt`, `revision` vào progress event và reducer ordering trong `app/src-tauri/src/migration/worker.rs`, `app/src/types.ts` và `app/src/components/migration/transferState.ts` per plan: UI Data Integrity (partial)
- [X] T044 Trả composite account/profile/active job từ `cmd_migration_get_auto_status` và hydrate frontend bằng response authoritative trong `app/src-tauri/src/migration/commands.rs` và `app/src/hooks/useMigration.ts` per contracts/ipc-contracts.md (partial)
- [X] T045 Hardening atomic replacement của Microsoft session trên Windows trong `app/src-tauri/src/migration/session_store.rs` per plan: Microsoft Session Persistence (partial)
- [X] T046 [US1] Tạo snapshot OneDrive đầu tiên ngay sau OAuth và bật profile mặc định trong `app/src/hooks/useMigration.ts` và `app/src-tauri/src/migration/auto_engine.rs` per FR-021
- [X] T047 [US2] Hiển thị loading trong bảng file suốt quá trình lấy cây thư mục và cải thiện empty state trong `app/src/components/migration/AutoMigrationCenter.tsx` per FR-022
- [X] T048 [US2] Thêm hành động đổi tài khoản, xóa state tài khoản cũ và mở lại OAuth trong `app/src/components/migration/OneDriveMigrationPage.tsx` và `app/src/hooks/useMigration.ts` per FR-023
- [X] T049 [P] Bổ sung i18n Việt/Anh cho loading, empty state và đổi tài khoản trong `app/src/i18n/locales/vi.json` và `app/src/i18n/locales/en.json`
- [X] T050 [P] Kiểm thử loading snapshot và hành động đổi tài khoản trong `app/src/components/migration/AutoMigrationCenter.test.tsx`
