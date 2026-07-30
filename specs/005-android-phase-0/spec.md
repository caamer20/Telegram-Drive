# Đặc tả: Android độc lập — Giai đoạn 0

## Mục tiêu

Thiết lập ứng dụng Kotlin/Jetpack Compose độc lập trong `android-app/`, chứng minh TDLib native chạy trên Android mà không cần credential, và cung cấp fake source ổn định.

## Tiêu chí bắt buộc

- Application ID `com.nmtuong.telegramdrive`, minSdk 26, Kotlin DSL, một module `app`.
- UI → ViewModel → repository → TDLib gateway; domain không import `org.drinkless.tdlib`.
- TDLib từ source Telegram chính thức được pin, nạp JNI, tạo client và trả authorization state đầu tiên.
- Gateway singleton theo Application, start idempotent, close rõ ràng, không block main thread.
- Gateway có transition kiểm chứng được cho start/close concurrent và lỗi native; backup/device transfer không được chứa account, session, key, cache hoặc media.
- TDLib/crypto source và hai ABI binary có provenance, integrity check và metadata/hash tái kiểm tra được.
- Diagnostics hiển thị native/client/authorization/source/error an toàn.
- Fake catalog có account, Saved Messages, nguồn file, 5 loại media và ba trạng thái download.
- Gradle test/lint/build pass; Android CLI/adb cài, chạy, đọc layout, screenshot và kiểm tra reopen/recreation.

## Ngoài phạm vi

Login thật, session restore, browsing thật, paging, download manager, preview/playback, Room, background transfer, release, CI/CD, MCP và Lightbuild.
