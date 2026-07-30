# Nhật ký Giai đoạn 0

## Checkpoint hiện tại

Checkpoint 8 — hoàn tất validation local.

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

## Còn lại

Không còn mục Giai đoạn 0. Các tính năng sản phẩm thuộc Giai đoạn 1+.

## Blocker

Không có.
