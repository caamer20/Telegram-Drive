# Tasks: Tối ưu video trước khi lưu trữ

**Input**: Design documents from `/specs/003-video-transcoding/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/media-preparation.md, quickstart.md

## Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup

- [X] T001 Xác nhận module media processor và export module trong `app/src-tauri/src/migration/mod.rs`
- [X] T002 [P] Bổ sung tài liệu dependency runtime FFmpeg/FFprobe trong `app/README.md`

---

## Phase 2: Foundational

- [X] T003 Viết unit tests cho FFprobe JSON parser, rotation và display dimensions trong `app/src-tauri/src/migration/media_processor.rs`
- [X] T004 Viết unit tests cho quyết định passthrough/transcode, scale không upscale và tên output trong `app/src-tauri/src/migration/media_processor.rs`
- [X] T005 Viết unit tests cho validation output, cleanup ownership và error mapping trong `app/src-tauri/src/migration/media_processor.rs`
- [X] T006 Mở rộng runtime discovery để xác định FFprobe cạnh FFmpeg hoặc từ PATH trong `app/src-tauri/src/migration/media_processor.rs`
- [X] T007 Tạo types `MediaProbe`, `TranscodeDecision`, `PreparedUpload`, `MediaProgress`, `MediaProcessError` và parser nền tảng trong `app/src-tauri/src/migration/media_processor.rs`

**Checkpoint**: Parser, decision và dependency discovery có test độc lập.

---

## Phase 3: User Story 1 - Tự động tối ưu video độ phân giải cao (Priority: P1) 🎯 MVP

**Goal**: Video vượt Full HD được tạo thành MP4 H.264/AAC tối đa 1920×1080 trước upload.

**Independent Test**: Chuẩn bị video 4K landscape và portrait; output là H.264, nằm trong bounding box, giữ tỷ lệ và source không đổi.

### Tests for User Story 1

- [X] T008 [US1] Thêm integration test có điều kiện cho FFprobe/FFmpeg với video vượt Full HD trong `app/src-tauri/src/migration/media_processor.rs`
- [X] T009 [P] [US1] Thêm test progress đơn điệu và output zero-byte/codec sai bị từ chối trong `app/src-tauri/src/migration/media_processor.rs`

### Implementation for User Story 1

- [X] T010 [US1] Implement `probe_media`, rotation normalization và `decide_transcode` trong `app/src-tauri/src/migration/media_processor.rs`
- [X] T011 [US1] Implement FFmpeg H.264/AAC command, scale bounding box, progress parser và output validation trong `app/src-tauri/src/migration/media_processor.rs`
- [X] T012 [US1] Tích hợp `prepare_upload` giữa duplicate check và upload, dùng path/name/size đã chuẩn bị trong `app/src-tauri/src/migration/worker.rs`
- [X] T013 [US1] Ghi bandwidth theo upload thực nhưng giữ quota/snapshot completion theo source item trong `app/src-tauri/src/migration/worker.rs`

**Checkpoint**: 4K và video dọc được upload bằng output H.264 Full HD, source không đổi.

---

## Phase 4: User Story 2 - Bỏ qua video không cần xử lý (Priority: P2)

**Goal**: H.264 ≤ Full HD và non-video passthrough; codec khác H.264 vẫn transcode không upscale.

**Independent Test**: H.264 720p giữ nguyên hash; VP9 720p thành H.264 nhưng dimensions không tăng; PDF giữ nguyên.

### Tests for User Story 2

- [X] T014 [US2] Thêm tests passthrough H.264 thấp, transcode codec khác và non-video trong `app/src-tauri/src/migration/media_processor.rs`
- [X] T015 [P] [US2] Thêm integration assertions cho prepared path, output codec/dimensions và upload name `.mp4` trong `app/src-tauri/src/migration/media_processor.rs`

### Implementation for User Story 2

- [X] T016 [US2] Hoàn thiện passthrough source/non-video và không upscale codec khác H.264 trong `app/src-tauri/src/migration/media_processor.rs`
- [X] T017 [US2] Giữ duplicate fingerprint theo source, đồng thời upload đúng name/size prepared qua contract hiện có trong `app/src-tauri/src/migration/worker.rs` và `app/src-tauri/src/migration/upload_adapter.rs`

**Checkpoint**: Không encode thừa và không thay đổi non-video.

---

## Phase 5: User Story 3 - Phục hồi an toàn khi tối ưu thất bại (Priority: P3)

**Goal**: Probe/encode failure, cancel và restart không để process/tệp tạm mồ côi; người dùng thấy phase/lỗi rõ ràng.

**Independent Test**: Video hỏng và cancel encode tạo error code đúng, không upload, không còn output temp; progress UI hiển thị analyzing/processing.

### Tests for User Story 3

- [X] T018 [US3] Thêm tests cancel child process, cleanup mọi nhánh và output path Unicode trong `app/src-tauri/src/migration/media_processor.rs`
- [X] T019 [P] [US3] Mở rộng tests phase ordering `downloading → analyzing → processing → uploading` trong `app/src/components/migration/transferState.test.ts`

### Implementation for User Story 3

- [X] T020 [US3] Implement cancel-safe child lifecycle, bounded stderr errors và cleanup guard trong `app/src-tauri/src/migration/media_processor.rs`
- [X] T021 [US3] Map media errors vào item failure/activity và cleanup source + prepared output ở mọi nhánh trong `app/src-tauri/src/migration/worker.rs`
- [X] T022 [P] [US3] Mở rộng progress phase/types và processing bucket trong `app/src/types.ts` và `app/src/components/migration/transferState.ts`
- [X] T023 [US3] Hiển thị transfer card analyzing/processing với icon, màu và label phù hợp trong `app/src/components/migration/TransferList.tsx` và `app/src/components/migration/AutoMigrationCenter.tsx`
- [X] T024 [P] [US3] Bổ sung processing log category/message song ngữ trong `app/src/hooks/useMigration.ts`, `app/src/components/migration/ProcessingLogPanel.tsx`, `app/src/i18n/locales/en.json` và `app/src/i18n/locales/vi.json`

**Checkpoint**: Failure/cancel an toàn và UI quan sát được toàn bộ bước media.

---

## Phase 6: Polish & Cross-Cutting Concerns

- [X] T025 Format các Rust file đã chạm, chạy targeted tests và toàn bộ Rust library tests trong `app/src-tauri`
- [X] T026 [P] Chạy frontend unit tests, TypeScript build và sửa regression trong `app`
- [X] T027 Xác minh tự động các scenario có thể chạy cục bộ và cập nhật kết quả thực tế trong `specs/003-video-transcoding/quickstart.md`
- [X] T028 Review credential/log hygiene, xác nhận không log path nhạy cảm hoặc stderr không giới hạn trong `app/src-tauri/src/migration/media_processor.rs`

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 là nền tảng bắt buộc.
- US1 phụ thuộc Phase 2 và là MVP.
- US2 phụ thuộc decision/API của US1 nhưng kiểm thử độc lập.
- US3 phụ thuộc pipeline US1/US2 để bao phủ cancel/error/UI.
- Polish chỉ chạy sau ba user stories.

## Parallel Opportunities

- T002 có thể chạy song song với setup module.
- T009 độc lập với integration test T008.
- T015 có thể chạy song song với media processor US2.
- T019, T022 và T024 chạm frontend files khác nhau và có thể chuẩn bị song song sau khi phase contract được chốt.
- T026 có thể chạy song song với Rust validation T025.

## Implementation Strategy

1. Hoàn thành parser/decision bằng tests không cần binary.
2. Hoàn thành US1 để có pipeline 4K → H.264 Full HD.
3. Thêm passthrough/no-upscale US2.
4. Hoàn thiện cancel/cleanup/observability US3.
5. Chạy regression và quickstart.

## Phase 7: Convergence

- [X] T029 Dọn output `mig_<job>_<item>.transcoded.mp4` mồ côi trong local working directories khi startup recovery per FR-009 (partial) trong `app/src-tauri/src/migration/media_processor.rs` và `app/src-tauri/src/migration/db.rs`
