# Nhật ký Android Phase 1

## Baseline — 2026-07-30

- P0 hardening pass trên branch `agent/android-phase-0`; HEAD baseline ban đầu `3f0d24e10041410ae8677598e5074e214dcce07f`.
- Feature artifacts: `specs/006-android-phase-1/`; requirements checklist pass 16/16.
- Android runtime: Pixel 9 Pro emulator API 36, arm64-v8a. Không claim thiết bị thật/x86_64 runtime.
- Credential policy: local `android-app/telegram-api.properties`, ignored; repository chỉ có template rỗng.
- Runtime real auth chưa xác minh ở baseline vì chưa có local credential/OTP trong workspace.

## Checkpoint B1–B9 — implementation và test

- Đã triển khai auth state-driven cho parameters, phone, code, password, email, email code, other-device, ready/logout/closing/closed/error; action sai state và submit lặp bị chặn.
- Credential chỉ đọc từ `android-app/telegram-api.properties` bị ignore; hiện file này **không tồn tại**, do đó real UI dừng an toàn ở Missing configuration.
- TDLib dùng một receive loop, request có `@extra`, Saved Messages qua `getMe` + `getChatHistory` giới hạn 1–50 và mapping image/video/animation/document.
- Download foreground có progress/complete/failure/cancel/dedup; preview chỉ route khi local path còn tồn tại.
- Media3 1.10.1 phát file video local; release guard idempotent. Fake source có MP4 fixture và không khởi tạo TDLib.
- Unit test fake/real source compile pass, gồm auth mapping/transition/invalid, redaction, lifecycle P0, Saved mapping/filter/dedup, stale path retry, cancel, preview routing, logout và player release.

## Checkpoint B10 — runtime quan sát được

- Runtime: Pixel 9 Pro emulator API 36, arm64-v8a; không claim thiết bị vật lý hoặc x86_64 runtime.
- Fake flow đã đi qua phone → code → fake 2FA → Saved Messages → image download/preview → video download/Media3 preview → back; PID giữ nguyên sau back.
- Finding runtime: image preview ban đầu recycle bitmap đang được Compose vẽ và crash. Đã bỏ manual recycle, rebuild, chạy lại cùng flow; PID `25675` còn sống trong và sau preview.
- Evidence: `docs/runtime/phase-1-fake-library.*`, `phase-1-fake-image-preview.*`, `phase-1-fake-video-preview.png`.
- Unit test + lint + real debug build pass trước finding; fake unit test/build pass sau finding. Final clean matrix sẽ chạy sau independent review.

## Chưa xác minh / blocker môi trường

- Real login, OTP/2FA Telegram, session restore, Saved Messages thật, ảnh/video thật và network-loss runtime chưa thể chạy vì không có local API ID/hash; OTP/manual confirmation cũng chưa được cung cấp.
- Không có dữ liệu account, session, phone, OTP, password hoặc media cá nhân trong evidence.
- Bước tiếp theo: independent review, P0 regression/final validation, credential scan và report; sau đó phần real runtime còn lại phụ thuộc thao tác tối thiểu từ người dùng.

## Final review và validation cục bộ

- Independent review đã sửa: request-error correlation theo `@extra`, retry khi local path stale, bỏ late progress sau cancel, sample/decode ảnh trên IO, bitmap recycle crash, Media3 pause ở `ON_STOP`, error UI và release idempotent.
- Security review logcat phát hiện TDLib verbosity mặc định có thể log request credential. Gateway hiện gửi `setLogVerbosityLevel(0)` trước mọi auth request; unit test kiểm tra thứ tự. Real logcat final chỉ có startup + request hạ verbosity, không có request credential.
- Clean real: `testDebugUnitTest lintDebug assembleDebug` pass.
- Clean fake: `testDebugUnitTest lintDebug assembleDebug` pass.
- 20 unit tests pass; không skip/failure/error.
- APK chứa `arm64-v8a` và `x86_64`; hash native khớp metadata. Merged manifest có `allowBackup=false` và cả hai rule resource.
- Scan operational tree/APK/native không có OpenSSL legacy; credential scan không phát hiện giá trị thật. `git diff --check` pass.
- Final fake runtime chạy lại toàn vertical slice trên PID `26697`; Media3 log xác nhận version 1.10.1 và release. Fake log không có `DLTD`/native client.
- Final real runtime PID `26531`: JNI load thành công, TDLib client 1, commit `022d60202e446ad1287b9fb68e687c8a0760788b`, auth state đầu `WaitTdlibParameters`, sau đó UI Missing configuration đúng policy.
- GitNexus detect-changes báo risk low/0 affected process nhưng index không parse Kotlin; kết quả này chỉ là bằng chứng scope docs, không thay thế test/runtime review.

Các task real-runtime T014/T019/T024/T029 vẫn mở đúng với blocker credential/OTP.
