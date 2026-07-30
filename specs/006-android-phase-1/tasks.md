# Tasks: Android Phase 1 Vertical Slice

## Phase 1: Setup

- [x] T001 Cập nhật local credential template/ignore và hướng dẫn không chứa secret trong android-app/README.md, android-app/.gitignore và android-app/telegram-api.properties.example
- [x] T002 Thêm stable Media3 1.10.1 dependency catalog và app dependency trong android-app/gradle/libs.versions.toml và android-app/app/build.gradle.kts
- [x] T003 [P] Tạo progress baseline và validation matrix trong docs/phase-1-progress.md

## Phase 2: Foundational

- [x] T004 Mở rộng domain auth/library/download/preview models trong android-app/app/src/main/java/com/nmtuong/telegramdrive/domain/Models.kt
- [x] T005 Tạo redaction và credential configuration boundary kèm tests trong android-app/app/src/main/java/com/nmtuong/telegramdrive/security/ và android-app/app/src/test/java/com/nmtuong/telegramdrive/security/
- [x] T006 Tạo TDLib request correlation/single-receiver contract và fake native tests trong android-app/app/src/main/java/com/nmtuong/telegramdrive/telegram/ và android-app/app/src/test/java/com/nmtuong/telegramdrive/telegram/
- [x] T007 Mở rộng TelegramRepository contract và real/fake implementations cho auth/library/download/logout trong android-app/app/src/main/java/com/nmtuong/telegramdrive/data/
- [x] T008 Cập nhật AppContainer composition chỉ tạo một gateway/repository và cung cấp explicit logout/reset/test shutdown trong android-app/app/src/main/java/com/nmtuong/telegramdrive/bootstrap/AppContainer.kt

## Phase 3: User Story 1 — Authorization và session (P1)

**Independent Test**: Fake state machine đi qua mọi auth shape; real runtime login thủ công và force-stop/reopen giữ session.

- [x] T009 [P] [US1] Viết tests authorization mapping/transition/invalid action/duplicate guard/session startup/logout trong android-app/app/src/test/java/com/nmtuong/telegramdrive/auth/
- [x] T010 [US1] Implement state-driven TDLib parameters và auth actions trong android-app/app/src/main/java/com/nmtuong/telegramdrive/telegram/TdLibJsonGateway.kt
- [x] T011 [US1] Implement authorization/session orchestration trong android-app/app/src/main/java/com/nmtuong/telegramdrive/data/Repositories.kt
- [x] T012 [US1] Tạo AuthorizationViewModel và bilingual authorization UI trong android-app/app/src/main/java/com/nmtuong/telegramdrive/feature/auth/
- [x] T013 [US1] Route initializing/login/ready an toàn trong android-app/app/src/main/java/com/nmtuong/telegramdrive/navigation/AppNavigation.kt
- [ ] T014 [US1] Xác minh fake auth và real login/session restore; ghi evidence không chứa secret trong docs/runtime/ và docs/phase-1-progress.md

## Phase 4: User Story 2 — Saved Messages (P2)

**Independent Test**: Fake batch chứng minh self chat, filter, stable id, deduplicate và loading/empty/error.

- [x] T015 [P] [US2] Viết tests Saved Messages mapping/filter/dedup/error trong android-app/app/src/test/java/com/nmtuong/telegramdrive/library/
- [x] T016 [US2] Implement getMe + bounded getChatHistory và media mapping trong android-app/app/src/main/java/com/nmtuong/telegramdrive/telegram/
- [x] T017 [US2] Implement library state repository và deterministic fake catalog trong android-app/app/src/main/java/com/nmtuong/telegramdrive/data/
- [x] T018 [US2] Tạo LibraryViewModel và bilingual loading/content/empty/error UI trong android-app/app/src/main/java/com/nmtuong/telegramdrive/feature/library/
- [ ] T019 [US2] Route Ready đến Saved Messages và validate fake/real batch trong android-app/app/src/main/java/com/nmtuong/telegramdrive/navigation/AppNavigation.kt và docs/phase-1-progress.md

## Phase 5: User Story 3 — Download và image preview (P3)

**Independent Test**: Fake download success/failure/cancel/dedup và image preview/back chạy không login thật.

- [x] T020 [P] [US3] Viết tests download state/dedup/cancel/stale path/preview routing trong android-app/app/src/test/java/com/nmtuong/telegramdrive/download/
- [x] T021 [US3] Implement TDLib download/updateFile/cancel routing trong android-app/app/src/main/java/com/nmtuong/telegramdrive/telegram/
- [x] T022 [US3] Implement foreground download operation state và path verification trong android-app/app/src/main/java/com/nmtuong/telegramdrive/data/
- [x] T023 [US3] Tạo bilingual download controls và image preview screen trong android-app/app/src/main/java/com/nmtuong/telegramdrive/feature/preview/
- [ ] T024 [US3] Validate fake/real image flow, network error, recreation và back; ghi evidence trong docs/runtime/ và docs/phase-1-progress.md

## Phase 6: User Story 4 — Video local playback (P4)

**Independent Test**: Fake local fixture tạo đúng một player, play/pause/seek/back và release terminal.

- [x] T025 [P] [US4] Viết tests player lifecycle boundary và video preview routing trong android-app/app/src/test/java/com/nmtuong/telegramdrive/preview/
- [x] T026 [US4] Tạo Media3 player release guard idempotent trong android-app/app/src/main/java/com/nmtuong/telegramdrive/feature/preview/VideoPlayerReleaseGuard.kt
- [x] T027 [US4] Tạo bilingual local-only VideoPreviewScreen controls/error/back trong android-app/app/src/main/java/com/nmtuong/telegramdrive/feature/preview/VideoPreviewScreen.kt
- [x] T028 [US4] Tích hợp video complete → verified path → player route trong android-app/app/src/main/java/com/nmtuong/telegramdrive/navigation/AppNavigation.kt
- [ ] T029 [US4] Validate fake/real video download/playback/recreation/back và ghi evidence trong docs/runtime/ và docs/phase-1-progress.md

## Phase 7: Hardening và handoff

- [x] T030 [P] Chạy P0 security/lifecycle/OpenSSL regression và credential scan trên toàn diff trong android-app/app/src/test/ và repository
- [x] T031 Chạy clean real/fake unit test, lint, build, APK/native/merged-manifest inspection theo specs/006-android-phase-1/quickstart.md
- [x] T032 Chạy independent review authorization/concurrency/session/download/player/test-gap, sửa findings và rerun validation bị ảnh hưởng
- [x] T033 Cập nhật README, AGENTS, docs/phase-1-progress.md và docs/phase-1-report.md với observed/unverified/P2 scope
- [x] T034 Chạy GitNexus detect changes, review git diff/status và xác nhận không có credential/session/media cá nhân

## Dependencies & Execution Order

- Setup → Foundational → US1 → US2 → US3 → US4 → Hardening.
- US2 phụ thuộc Ready từ US1; US3 phụ thuộc media identity từ US2; US4 dùng download/path contract từ US3.
- Tests [P] có thể chuẩn bị độc lập theo file nhưng implementation thực hiện tuần tự vì cùng gateway/repository/navigation boundary.

## Parallel Opportunities

- T003 độc lập với dependency/config T001–T002.
- T009/T015/T020/T025 là test-file riêng nhưng chỉ chạy khi foundation contract tương ứng ổn định.
- Documentation/evidence review không chạy đồng thời với mutation cùng file.

## Implementation Strategy

Hoàn tất từng vertical increment và validation ngay tại checkpoint; không dừng xin xác nhận giữa P0/P1 hoặc giữa user stories. Real credential/OTP chỉ chặn runtime claim, không chặn fake flow, unit tests, UI, data mapping, download/player implementation hay security review.
