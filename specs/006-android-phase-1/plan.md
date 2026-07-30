# Implementation Plan: Android Phase 1 Vertical Slice

**Branch**: `agent/android-phase-0` | **Date**: 2026-07-30 | **Spec**: [spec.md](spec.md)

## Summary

Mở rộng Android application-owned TDLib client thành một vertical slice gồm state-driven authorization, session restore, Saved Messages batch, foreground download, image preview và video local playback. Giữ repository/gateway boundary Phase 0, không thêm database/index/background transfer.

## Technical Context

**Language/Version**: Kotlin 2.3.20, Java 17

**Primary Dependencies**: Compose BOM 2026.03.01, coroutines 1.10.2, TDLib JSON JNI pinned Phase 0, AndroidX Media3 1.10.1 stable

**Storage**: TDLib-owned encrypted database/session và app-private downloaded files; không Room

**Testing**: JUnit 4, kotlinx-coroutines-test, Compose/AndroidX instrumented runtime checks

**Target Platform**: Android minSdk 26, compile/target 36; package ABI arm64-v8a + x86_64

**Project Type**: Standalone Android mobile app trong `android-app/`

**Performance Goals**: Không block main thread; một batch tối đa 50 message; một operation cho mỗi auth submit/file/player identity

**Constraints**: Credential local ignored; không background transfer/streaming/global index; backup/device transfer disabled; real OTP thủ công

**Scale/Scope**: Một account, Saved Messages, image/video/animation/basic document, foreground proof of concept

## Constitution Check

- PASS — Spec tồn tại trước implementation và artifacts đồng bộ bằng SpecKit.
- PASS — Tiếng Việt được dùng cho spec/plan/tasks/docs.
- PASS — Telegram vẫn là data source; Android feature độc lập không tạo Rust/Actix/Tauri surface nên các principle desktop không áp dụng.
- PASS — Không thêm SQLite/Room hoặc background worker; không mâu thuẫn persistence/background constitution.
- PASS — UI text mới phải có Việt/Anh ngay khi thêm.
- PASS — Application-owned client duy nhất giữ nguyên principle shared Telegram state.

## Milestones

1. Foundation: local credential policy, domain contracts, JSON request correlation/redaction và fake state shape.
2. Authorization/session: state-driven actions, login UI, Ready/restore/logout/reset và runtime auth evidence.
3. Saved Messages: self identity, batch history, mapping/filter/dedup và list states.
4. Download/image: foreground operation state, progress/cancel/dedup/path validation và image preview.
5. Video: Media3 local-only playback, controls và lifecycle release.
6. Hardening: full tests, P0 regression, fake/real runtime matrix, independent review, reports/evidence.

## Dependency và risk

- Real Saved Messages/download phụ thuộc authorization Ready và credential/OTP thủ công.
- TDLib JSON response/update multiplexing là concurrency risk chính; mọi request cần correlation và single receiver owner.
- Session/data path phải app-private và tiếp tục nằm ngoài backup/transfer.
- Download update có thể đến sau cancel; operation generation/file identity là nguồn sự thật.
- Player chỉ nhận verified local URI; lifecycle owner release trên exit/dispose.

## Project Structure

```text
specs/006-android-phase-1/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/android-p1-contracts.md
└── tasks.md

android-app/app/src/main/java/com/nmtuong/telegramdrive/
├── bootstrap/
├── domain/
├── telegram/
├── data/
├── feature/auth/
├── feature/library/
├── feature/preview/
└── navigation/

android-app/app/src/test/java/com/nmtuong/telegramdrive/
└── unit tests theo boundary/state
```

**Structure Decision**: Giữ một Android module và inward dependency `UI/feature → repository → gateway`; domain/UI không import TDLib.

## Validation

- Mỗi milestone chạy targeted unit tests; cuối cùng chạy toàn bộ real/fake unit/lint/build.
- Runtime fake phải đi hết vertical slice không native; runtime real chạy login/session/Saved Messages/image/video khi manual credential/OTP có sẵn.
- APK/native/merged manifest/security diff scan phải được chạy lại sau implementation.

## Post-design Constitution Check

PASS — data model/contracts không thêm technology boundary trái constitution, không có NEEDS CLARIFICATION hoặc violation cần Complexity Tracking.
