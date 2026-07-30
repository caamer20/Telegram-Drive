# Kế hoạch triển khai

## Stack

AGP 9.0.1, Gradle 9.1, Kotlin 2.3.20, Compose BOM 2026.03.01, JDK 17, compile/target SDK 36, minSdk 26, coroutines 1.10.2.

## Cấu trúc

- `bootstrap`: Application-owned dependency container.
- `domain`: model thuần Kotlin.
- `telegram`: JNI/TDLib gateway và JSON mapping.
- `data`: real/fake repository boundary.
- `feature/diagnostics`: ViewModel và Compose diagnostics.
- `ui/theme`: theme; `MainActivity` là composition root UI.

## TDLib

Dùng JSONJava JNI chính thức từ `tdlib/td` commit `022d60202e446ad1287b9fb68e687c8a0760788b`; build OpenSSL 3.5.7 LTS đã xác minh SHA-256 và `libtdjsonjava.so` bằng NDK 27.2/CMake 3.22.1/API 26 cho `arm64-v8a`, `x86_64`. Ghi metadata và hash binary; không dùng binary bên thứ ba.

Backup/device transfer bị tắt và exclude toàn miền. Gateway dùng state machine terminal, application-owned, explicit close cho logout/reset/test teardown; Android process kill không dựa vào callback cleanup.

## Validation

Gradle là nguồn sự thật cho unit test/lint/APK. Android CLI và adb dùng cho install/run/layout/screenshot/log/lifecycle runtime.
