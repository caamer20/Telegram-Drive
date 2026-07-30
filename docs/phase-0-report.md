# Báo cáo Giai đoạn 0

Trạng thái: **HOÀN THÀNH** — mọi tiêu chí Giai đoạn 0 đã được kiểm chứng local ngày 2026-07-30.

## 1. Kết quả tổng thể

Ứng dụng Android độc lập Kotlin/Jetpack Compose tại `android-app/` build, cài và chạy thành công. TDLib native chính thức được nạp, client được tạo và trả authorization state đầu tiên mà không yêu cầu credential. Real/fake source dùng chung repository boundary; Activity recreation không tạo client thứ hai.

## 2. Checkpoint

1. Môi trường: hoàn tất.
2. Compose project: hoàn tất.
3. Kiến trúc boundary: hoàn tất.
4. TDLib spike: hoàn tất.
5. Diagnostics: hoàn tất.
6. Fake source: hoàn tất.
7. Hướng dẫn: hoàn tất.
8. Validation local: hoàn tất.

## 3. Cấu trúc và dependency

- `bootstrap`: Application-owned `AppContainer`.
- `domain`: model thuần Kotlin, không phụ thuộc TDLib.
- `telegram`: `TdLibGateway`, JSON JNI adapter và mapper.
- `data`: `TelegramRepository`, real/fake implementation.
- `feature/diagnostics`: ViewModel/Compose chỉ dùng repository/domain.
- `ui/theme` và `MainActivity`: theme/composition root.

Luồng dependency: UI → ViewModel → repository → gateway → JSONJava JNI. `TelegramDriveApplication` giữ container qua Activity recreation và cung cấp `close()` rõ ràng.

## 4. Chiến lược TDLib

Dùng JSONJava JNI từ repository Telegram chính thức `tdlib/td`, pin commit `022d60202e446ad1287b9fb68e687c8a0760788b`. Script `scripts/build-tdlib-android.sh` tải source Telegram/OpenSSL chính thức, generate TL source, rồi cross-compile bằng NDK. JSON boundary được chọn để generated TDLib model không lọt vào domain/UI.

## 5. ABI

- `arm64-v8a`: ELF AArch64, SHA-256 `b55c4405985e57dd6381b499c66a71fe41f408dd02a01f97a8083dad52d3ecf8`; package và runtime load đã xác minh.
- `x86_64`: ELF x86-64, SHA-256 `8ff5d16f36ae757885eca9db7f5cb7821b46fb8b35a8a2aea3618998a02241ec`; build và package trong APK đã xác minh.

Runtime hiện có là emulator ARM64 trên Apple Silicon, nên không chạy x86_64 runtime; điều này không làm mất coverage emulator hiện tại. APK chứa đúng cả hai path `lib/arm64-v8a/` và `lib/x86_64/`.

## 6. Real/fake setup

Mặc định `telegramDataSource=real`. `./gradlew -PtelegramDataSource=fake assembleDebug` chọn fake source. Fake catalog gồm account, Saved Messages, hai nguồn khác, image/video/audio/PDF/document và downloading/complete/failed; unit test và fake runtime đều pass.

## 7. Validation

- Gradle 9.1.0 / AGP 9.0.1 / Kotlin 2.3.20 / JDK 17.
- `clean testDebugUnitTest lintDebug assembleDebug`: BUILD SUCCESSFUL.
- Unit: 4 test, 0 failure/error. Lint: 0 issue.
- APK debug 57 MB, minSdk 26, targetSdk 36, debuggable.
- Android CLI install/launch: success.
- Native loader: `libtdjsonjava.so ... ok`.
- TDLib: `AuthorizationStateWaitTdlibParameters` nhận lúc runtime, không credential.
- Layout: `loaded`, `created`, `WaitingForTdlibParameters`, `Active clients = 1`.
- Recreation: PID giữ nguyên `8398`, active clients = 1.
- Reopen: PID `8398 → 8675`, native load/auth lặp lại thành công, không crash.

## 8. Runtime

Pixel 9 Pro emulator, Android API 36, `arm64-v8a`, 1280×2856 @ 480 dpi. Android CLI 1.0.15498356, adb 36.0.2, host macOS 26.5.2 ARM64.

## 9. Evidence

- Screenshot: `docs/runtime/phase-0-diagnostics.png` và bản annotated.
- Layout: `phase-0-layout.json`, `phase-0-layout-after-recreation.json`, `phase-0-layout-after-reopen.json`, `phase-0-layout-fake.json`.
- Gradle reports: `android-app/app/build/reports/` (local generated output).

## 10. Cố ý để lại Giai đoạn 1

TDLib parameters/credential, login/session restore, Saved Messages/channel thật, history paging, download manager và preview/playback.

## 11. Rủi ro còn lại

Native binary debug lớn; chưa hardening production, release signing hoặc runtime x86_64 trên host Intel. Lifecycle process termination do Android quản lý và không có callback graceful bảo đảm; gateway vẫn có explicit close và gửi TDLib `close` khi owner đóng.

## 12. Phạm vi diff

Diff chỉ thêm project Android Phase 0, source-build script, spec/docs/evidence và cập nhật ignore/AGENTS. Không thêm credential/session, MCP, Lightbuild hay CI/CD.
