# Danh sách Nhiệm vụ Triển khai (Tasks) - Chuyển dữ liệu OneDrive

Tài liệu này phân rã kế hoạch thành các nhiệm vụ chi tiết có mã ID chuẩn từ `T001` trở đi, chỉ rõ mối quan hệ phụ thuộc, file thay đổi, các yêu cầu được bao phủ và tiêu chí hoàn thành chi tiết.

---

## Phase 1: Baseline & Migration Safety Setup (Kiểm thử cơ sở và an toàn DB)

### T001: Baseline Characterization Tests
*   **Phase**: 1
*   **Dependency**: Không có.
*   **Parallel**: Không.
*   **File path dự kiến**: `app/src-tauri/src/migration/tests/baseline_tests.rs`
*   **FR/NFR/SC được bao phủ**: NFR-004, NFR-005.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Viết xong các bài kiểm thử cơ sở ghi nhận hành vi hiện tại của database và worker cũ để làm mốc so sánh (characterization tests).
    *   Chạy `cargo test` vượt qua 100%.

### T002: Regression Safety Test cấm xóa OneDrive
*   **Phase**: 1
*   **Dependency**: `T001`.
*   **Parallel**: Không.
*   **File path dự kiến**: `app/src-tauri/src/migration/tests/safety_tests.rs`
*   **FR/NFR/SC được bao phủ**: NFR-001, SC-002.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Viết bài test mock API OneDrive để chứng minh: dù cấu hình Job thế nào, worker cũng không bao giờ gọi hàm API xóa/rename OneDrive (`delete_onedrive_item`).
    *   Loại bỏ hoàn toàn khả năng gọi xóa tệp trong mã nguồn.

### T003: Pipeline-Version DB Migration & WAL Setup
*   **Phase**: 1
*   **Dependency**: `T002`.
*   **Parallel**: Không.
*   **File path dự kiến**: `app/src-tauri/src/migration/db.rs`
*   **FR/NFR/SC được bao phủ**: FR-004, NFR-004, SC-006.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Cài đặt lệnh SQLite ALTER TABLE thêm các trường: `route_kind`, `duplicate_of_item_id`, `artifact_size_bytes`, `local_dest_path`, `telegram_random_id`, `upload_attempt_id`, và cột `pipeline_version` trong `migration_jobs`.
    *   Cấu hình `PRAGMA journal_mode = WAL;` và `PRAGMA synchronous = FULL;`.

### T004: Upgrade Test từ Database v1
*   **Phase**: 1
*   **Dependency**: `T003`.
*   **Parallel**: Không.
*   **File path dự kiến**: `app/src-tauri/src/migration/tests/db_upgrade_tests.rs`
*   **FR/NFR/SC được bao phủ**: FR-004, NFR-004.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Tạo file SQLite mock có cấu trúc schema v1 và dữ liệu Job cũ. Chạy migrate lên schema v2 và xác nhận dữ liệu được bảo toàn, các job cũ giữ nguyên lịch sử, không mass-update route của các item cũ.

---

## Phase 2: Core Pipeline Stage Actors & Routing (Rust Backend)

### T005: Route Classifier đầy đủ
*   **Phase**: 2
*   **Dependency**: `T004`.
*   **Parallel**: Không.
*   **File path dự kiến**: `app/src-tauri/src/migration/pipeline/classifier.rs`
*   **FR/NFR/SC được bao phủ**: FR-001, FR-005, FR-036, FR-037.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Cài đặt module phân loại tệp dựa trên đuôi định dạng, gán route: `video_to_td`, `image_to_td`, hoặc `other_to_local`.
    *   Hỗ trợ phát hiện và định tuyến các thư mục thực sự rỗng thành `other_to_local`.
    *   Tích hợp ma trận quyết định video (`transcode`, `remux_copy`, `passthrough`).
    *   Unit test phân loại đúng 100% các case mở rộng.

### T006: Canonical Claim & Promotion theo Artifact Target
*   **Phase**: 2
*   **Dependency**: `T005`.
*   **Parallel**: Có `[P]`.
*   **File path dự kiến**: `app/src-tauri/src/migration/pipeline/dedupe.rs`
*   **FR/NFR/SC được bao phủ**: FR-006, FR-007, FR-008, FR-009, FR-010, FR-012, SC-004.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Triển khai claim canonical dựa trên `fingerprint_type + fingerprint_value + source_size + artifact_target_key`.
    *   Cài đặt logic tự động chuyển các bản sao sang `skipped_duplicate` hoặc promote bản sao khác khi canonical chính thất bại.
    *   Chạy unit test mô phỏng race condition của dedupe song song thành công.

### T007: Downloader Stage với Path Sanitization
*   **Phase**: 2
*   **Dependency**: `T006`.
*   **Parallel**: Có `[P]`.
*   **File path dự kiến**: `app/src-tauri/src/migration/pipeline/download_stage.rs`
*   **FR/NFR/SC được bao phủ**: FR-019, FR-034, FR-035.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Xây dựng Downloader actor tải tệp tin OneDrive về `.working/download`.
    *   Tích hợp validate an toàn đường dẫn (`../` traversal, UNC, case collisions, symlink escape, absolute path).
    *   Tự động xóa tệp `.part` cũ khi crash và tải lại từ đầu.
    *   Chạy unit test validate path safety vượt qua 100%.

### T008: Video Remux/Transcode Stage (FFmpeg)
*   **Phase**: 2
*   **Dependency**: `T007`.
*   **Parallel**: Có `[P]`.
*   **File path dự kiến**: `app/src-tauri/src/migration/pipeline/transcode_stage.rs`
*   **FR/NFR/SC được bao phủ**: FR-002, FR-036, FR-037, NFR-005.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Xây dựng actor gọi tiến trình FFmpeg đơn lẻ (`concurrency = 1`).
    *   Định nghĩa thread limit `min(2, available_parallelism)`.
    *   Định tuyến tệp tương thích qua nhánh FFmpeg remux đổi container (`-c copy`) hoặc passthrough thay vì chỉ transcode.
    *   Unit test kiểm tra transcode video và remux thành công.

### T010: Image Staging Stage (Ảnh upload nguyên bản)
*   **Phase**: 2
*   **Dependency**: `T008`.
*   **Parallel**: Có `[P]`.
*   **File path dự kiến**: `app/src-tauri/src/migration/pipeline/image_stage.rs`
*   **FR/NFR/SC được bao phủ**: FR-001, SC-001.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Xây dựng actor sao chép ảnh nguyên bản vào vùng đệm tạm (staging area) và sẵn sàng upload trực tiếp lên Telegram Drive (không qua FFmpeg).
    *   Unit test verify ảnh không bị nén hoặc đổi metadata.

### T011: Other-to-Local & Empty Folders Stage
*   **Phase**: 2
*   **Dependency**: `T010`.
*   **Parallel**: Có `[P]`.
*   **File path dự kiến**: `app/src-tauri/src/migration/pipeline/local_commit_stage.rs`
*   **FR/NFR/SC được bao phủ**: FR-005, FR-012, FR-020.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Tải tệp non-media về vùng đệm, sau đó dùng atomic rename di chuyển về `OneDrive_Archive`.
    *   Tái tạo cấu trúc thư mục OneDrive rỗng dưới `OneDrive_Archive`.
    *   Unit test tạo thư mục rỗng và di chuyển tệp an toàn.

---

## Phase 3: Quota, Pacing & Idempotent Upload (Telegram Safety Backend)

### T012: Disk Safety Space Reservation
*   **Phase**: 3
*   **Dependency**: `T011`.
*   **Parallel**: Có `[P]`.
*   **File path dự kiến**: `app/src-tauri/src/migration/pipeline/disk_reserve.rs`
*   **FR/NFR/SC được bao phủ**: FR-003, FR-024, FR-025, FR-026, SC-005.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Cài đặt cơ chế kiểm soát dung lượng ổ đĩa đệm theo công thức hạn mức đĩa `working_budget`.
    *   Thực hiện giữ trước quota đĩa (permit/reservation) trước download/transcode.
    *   Chạy unit test mô phỏng đĩa đầy, pipeline dừng an toàn.

### T013: Quota Reservation & Recovery
*   **Phase**: 3
*   **Dependency**: `T012`.
*   **Parallel**: Có `[P]`.
*   **File path dự kiến**: `app/src-tauri/src/migration/quota_reserve.rs`
*   **FR/NFR/SC được bao phủ**: FR-011, FR-013, FR-014, FR-015.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Triển khai cơ chế atomically reserve/commit/release quota ngày 250GB dựa trên dung lượng artifact thật.
    *   Viết logic quét dọn dẹp (recovery) quota reservation bị kẹt khi khởi động hoặc qua ngày mới.
    *   Unit test quota boundary thành công.

### T014: Telegram Pacing & Cooldown Controller
*   **Phase**: 3
*   **Dependency**: `T013`.
*   **Parallel**: Có `[P]`.
*   **File path dự kiến**: `app/src-tauri/src/migration/pipeline/pacing.rs`
*   **FR/NFR/SC được bao phủ**: FR-016, FR-017.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Cài đặt pacing: cách nhau tối thiểu 3 giây giữa các file, cooldown 120 giây sau mỗi 100 file.
    *   Persist pacing state vào SQLite.
    *   Tự động cooldown khi nhận `FLOOD_WAIT_X` hoặc `FLOOD_PREMIUM_WAIT_X`.
    *   Unit test pacing và cooldown.

### T015: Persisted random_id Idempotency Upload
*   **Phase**: 3
*   **Dependency**: `T014`.
*   **Parallel**: Có `[P]`.
*   **File path dự kiến**: `app/src-tauri/src/migration/upload_adapter.rs`
*   **FR/NFR/SC được bao phủ**: FR-028, FR-029, FR-030, FR-031, FR-032, SC-003.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Xác minh khả năng tiêm `random_id` vào thư viện Grammers bằng cách xây dựng raw request hoặc adapter.
    *   Triển khai upload sử dụng persisted `telegram_random_id`.
    *   Unit test chứng minh zero duplicate message khi retry sau crash.

### T016: Manifest Writer (JSON/CSV)
*   **Phase**: 3
*   **Dependency**: `T015`.
*   **Parallel**: Có `[P]`.
*   **File path dự kiến**: `app/src-tauri/src/migration/pipeline/manifest.rs`
*   **FR/NFR/SC được bao phủ**: FR-033.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Viết module ghi manifest định dạng JSON/CSV an toàn, atomic, chứa đầy đủ các trường yêu cầu.
    *   Unit test ghi manifest chính xác.

---

## Phase 4: Tauri IPC & Destination Preflight Check

### T017: Tauri IPC Command Interface
*   **Phase**: 4
*   **Dependency**: `T016`.
*   **Parallel**: Không.
*   **File path dự kiến**: `app/src-tauri/src/migration/commands.rs`
*   **FR/NFR/SC được bao phủ**: FR-001, FR-004.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Cài đặt 5 tauri commands: `cmd_migration_create_transfer`, `cmd_migration_start_scan`, `cmd_migration_stop_scan`, `cmd_migration_get_plan_summary`, `cmd_migration_start_job` và các nút điều khiển pause/resume/cancel dùng `job_id`.

### T018: Destination Permission Preflight Check
*   **Phase**: 4
*   **Dependency**: `T017`.
*   **Parallel**: Không.
*   **File path dự kiến**: `app/src-tauri/src/migration/preflight.rs`
*   **FR/NFR/SC được bao phủ**: FR-021, FR-022, FR-023.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Triển khai kiểm tra quyền viết của bot/user trên destination channel được chọn trước khi Job bắt đầu chạy (preflight check, không gửi tin nhắn test).
    *   Unit test preflight check thành công.

### T019: IPC Event Ordering & DTO Validation
*   **Phase**: 4
*   **Dependency**: `T018`.
*   **Parallel**: Không.
*   **File path dự kiến**: `app/src-tauri/src/migration/commands.rs`
*   **FR/NFR/SC được bao phủ**: FR-001.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Bắn event tiến độ chi tiết `migration:stage-progress` có chứa `sequence_id` hoặc `revision` và toàn bộ các trường metadata (attempt, timestamps, event_id).
    *   Bảo đảm không dùng `i64` trong TS DTO, thay bằng `number` hoặc `string`.

---

## Phase 5: Giao diện React & Kiểm thử Hệ thống (Frontend & E2E)

### T020: React Context & useMigration Hooks Refactoring
*   **Phase**: 5
*   **Dependency**: `T019`.
*   **Parallel**: Không.
*   **File path dự kiến**: `app/src/hooks/useMigration.ts`
*   **FR/NFR/SC được bao phủ**: NFR-003.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Phân rã hook `useMigration` thành các hook nhỏ hơn làm nhiệm vụ tách biệt nhưng dùng chung `MigrationContext` làm state owner chính.
    *   Bảo đảm type-check TypeScript thành công không có lỗi `noEmit`.

### T021: OneDriveTransferWizard & UI Components (kèm i18n)
*   **Phase**: 5
*   **Dependency**: `T020`.
*   **Parallel**: Không.
*   **File path dự kiến**: `app/src/components/migration/OneDriveTransferWizard.tsx` và các components con.
*   **FR/NFR/SC được bao phủ**: NFR-003.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Cài đặt giao diện 7 trạng thái trực quan, tích hợp `DestinationPicker` và `PlanSummaryView`.
    *   Cài đặt đầy đủ i18n (vi/en) ngay trong task này cho từng component.
    *   Chạy thử frontend React bằng Vitest, giả lập các trạng thái hiển thị.

### T022: Crash Tests & System Validation
*   **Phase**: 5
*   **Dependency**: `T021`.
*   **Parallel**: Không.
*   **File path dự kiến**: `app/src-tauri/tests/crash_recovery_tests.rs`
*   **FR/NFR/SC được bao phủ**: FR-018, FR-019, SC-003.
*   **Tiêu chí hoàn thành (Exit Criteria)**:
    *   Thực hiện chạy kiểm thử tích hợp (integration tests) giả lập crash tiến trình tại mọi stage và kiểm tra khả năng phục hồi dữ liệu, đảm bảo không có file nào bị tải trùng hoặc upload trùng.
    *   Chạy toàn bộ test suite: Vitest, Cargo test, production build đóng gói thành công.
