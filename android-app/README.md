# Telegram Drive Android

Ứng dụng Android độc lập Kotlin/Jetpack Compose. Phase 0 cung cấp nền tảng TDLib đã harden; Phase 1 bổ sung vertical slice đăng nhập, Saved Messages, download và preview.

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

Real Phase 1 cần local credential. Copy `telegram-api.properties.example` thành `telegram-api.properties`, điền `apiId`/`apiHash` từ `my.telegram.org`; file thật đã bị ignore. Không đưa file này, phone, OTP, password hoặc nội dung BuildConfig/APK cá nhân vào commit/evidence.

Build TDLib từ repository root, dùng source chính thức đã pin và OpenSSL 3.5.7 LTS:

```bash
cd ..
scripts/build-tdlib-android.sh
```

Script yêu cầu Android SDK với NDK `27.2.12479018`, CMake/Ninja `3.22.1`, API 26 toolchain, tự phát hiện host toolchain, xác minh checksum source và build `arm64-v8a` + `x86_64`. Kết quả truy xuất nằm ở `android-app/tdlib-build-metadata.txt`; hash trong file phải khớp `shasum -a 256 android-app/app/src/main/jniLibs/*/libtdjsonjava.so`.

Backup và device transfer bị tắt mặc định. XML backup rules vẫn exclude toàn bộ root/files/database/shared preferences/external storage để chống regression trên các Android generation khác nhau.

## Android runtime

```bash
adb devices -l
adb install -r android-app/app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n com.nmtuong.telegramdrive/.MainActivity
android layout --help
android screen --help
```

Emulator: `emulator -list-avds`, rồi khởi động AVD bằng Android CLI hoặc emulator. Thiết bị thật cần USB debugging và phải hiện trạng thái `device` trong `adb devices -l`; ABI kiểm tra bằng `adb shell getprop ro.product.cpu.abilist`.

Không commit API ID/hash, phone, OTP, password, session, local.properties, keystore, database TDLib hoặc dữ liệu tài khoản. Room/global index/background transfer/streaming/release và CI/CD vẫn ngoài scope Phase 1.
