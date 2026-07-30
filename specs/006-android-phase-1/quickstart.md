# Quickstart Validation: Android Phase 1

## Prerequisites

- P0 report pass; Android SDK/NDK và emulator/device trong `adb devices -l`.
- Với real flow, tạo local ignored configuration theo README và chuẩn bị nhập phone/OTP/2FA thủ công. Không capture secret.

## Automated gates

```bash
cd android-app
./gradlew clean testDebugUnitTest lintDebug assembleDebug -PtelegramDataSource=real
./gradlew testDebugUnitTest lintDebug assembleDebug -PtelegramDataSource=fake
```

Expected: mọi task pass, P0 security/lifecycle regressions pass, hai APK mode build được.

## Fake vertical slice

1. Build/install fake APK và launch bằng adb.
2. Đi qua fake auth đến Ready; mở Saved Messages.
3. Chạy image success/error/cancel và preview/back.
4. Chạy video download complete, play/pause/seek/back.
5. Recreate Activity; xác minh không duplicate client/download/player.
6. Capture Android CLI layout/screenshot không chứa secret.

## Real vertical slice

1. Build/install real APK, clear logcat và launch.
2. Nhập auth data thủ công theo state; không ghi vào artifact.
3. Force-stop/reopen; xác minh Ready không OTP lại.
4. Mở Saved Messages; tải/preview một ảnh và tải/phát một video local.
5. Thử back, recreation và mất mạng giữa download; xác minh recovery/error an toàn.
6. Scan logcat, screenshot và git diff cho credential/session/media cá nhân.

## Evidence rule

Ghi riêng observed fact, inference và unverified item. Chỉ claim ABI/device/authorization state đã chạy thực tế; manual OTP là blocker hợp lệ nếu môi trường không cung cấp.
