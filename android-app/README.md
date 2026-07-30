# Telegram Drive Android

Ứng dụng Android độc lập Kotlin/Jetpack Compose. Giai đoạn 0 chỉ chứng minh nền tảng, TDLib và fake source; không có login hoặc tính năng duyệt/preview.

## Dependency rule

`feature/UI → data.TelegramRepository → telegram.TdLibGateway`. UI và domain không được import API generated/JNI của TDLib. `TelegramDriveApplication` sở hữu một `AppContainer`; Activity recreation không tạo client mới.

## Build và kiểm tra

```bash
cd android-app
./gradlew testDebugUnitTest lintDebug assembleDebug
```

Mặc định debug dùng real TDLib. Chọn fake source ổn định:

```bash
./gradlew -PtelegramDataSource=fake assembleDebug
```

Build TDLib từ source chính thức đã pin:

```bash
scripts/build-tdlib-android.sh
```

## Android runtime

```bash
adb devices -l
adb install -r android-app/app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n com.nmtuong.telegramdrive/.MainActivity
android layout --help
android screen --help
```

Emulator: `emulator -list-avds`, rồi khởi động AVD bằng Android CLI hoặc emulator. Thiết bị thật cần USB debugging và phải hiện trạng thái `device` trong `adb devices -l`; ABI kiểm tra bằng `adb shell getprop ro.product.cpu.abilist`.

Không commit API ID/hash, phone, OTP, password, session, local.properties, keystore, database TDLib hoặc dữ liệu tài khoản. Cố ý chưa triển khai login, browsing, download, preview, Room, background work, release và CI/CD.
