# Nhật ký Giai đoạn 0

## Baseline hardening — 2026-07-30

- Nhánh xác minh: `agent/android-phase-0`; HEAD trước hardening: `3f0d24e10041410ae8677598e5074e214dcce07f`; worktree sạch.
- So với `main`, nhánh có 68 file mới/thay đổi, 2.049 dòng thêm; đây là baseline Phase 0 hiện hữu, chưa phải bằng chứng hardening.
- Validation chạy lại trước thay đổi: `./gradlew testDebugUnitTest lintDebug assembleDebug` pass (53 task; 1 executed, 52 up-to-date). Chưa dùng kết quả này để xác nhận runtime hoặc reproducibility.
- Quan sát trực tiếp: manifest vẫn bật backup; hai XML backup/data-extraction còn là template; build script vẫn pin `OpenSSL_1_1_1w` và toolchain `darwin-x86_64`; gateway dùng cờ `started` nhưng có race giữa initialize/close và giảm counter khi client có thể chưa được tạo; Application dựa vào `onTerminate()` để close.
- Quyết định: trạng thái P0 được mở lại để hardening. Các tuyên bố hoàn tất trong report cũ chưa còn hiệu lực cho đến khi A2–A8 có bằng chứng mới.
- Chưa xác minh tại baseline này: rebuild source/binary, merged manifest, APK/native inspection, install/launch, recreation, force-stop/reopen và authorization state runtime.

## Checkpoint hiện tại

Checkpoint A8 — P0 hardening hoàn tất; tiếp tục Phase 1 không chờ xác nhận.

## Đã hoàn thành

- Khảo sát môi trường; tạo Compose project độc lập bằng Android CLI.
- Thiết lập Application/domain/gateway/repository/feature/navigation composition/UI boundary.
- Build TDLib official commit `022d602…` cho `arm64-v8a` và `x86_64`.
- Diagnostics real runtime nhận `WaitingForTdlibParameters`; fake catalog/source chọn được bằng Gradle property.
- Hoàn thiện test, tài liệu, AGENTS, screenshot và layout evidence.

## Validation đã chạy

- `./gradlew clean testDebugUnitTest lintDebug assembleDebug` — pass; 4 tests, 0 failure/error; lint 0 issue.
- Android CLI install/launch — pass trên emulator ARM64 API 36.
- Native load/client/authorization — pass; active client = 1.
- Activity recreation cùng PID và force-stop/reopen PID mới — pass, không crash, active client vẫn 1.
- APK chứa TDLib cho cả hai ABI; fake build/runtime hiển thị source `fake`.

## Hardening đã hoàn thành

- Rebuild hai ABI bằng TDLib commit pin + OpenSSL 3.5.7 LTS; source checksum, host/toolchain checks, metadata và binary hash đã xác minh.
- Tắt backup/device transfer, giữ exclude toàn miền; merged manifest và regression test pass.
- Thay cờ lifecycle bằng state machine; bỏ `Application.onTerminate()`; concurrency/failure tests pass.
- Real/fake runtime, recreation và force-stop/reopen đã chạy lại trên emulator ARM64 API 36; evidence mới nằm trong `docs/runtime/phase-0-hardening-*`.

## Còn lại

Không còn finding P0 mức High/Medium đã biết. Runtime x86_64 chưa có và được ghi rõ là giới hạn coverage, không phải runtime claim.

## Blocker

Không có.
