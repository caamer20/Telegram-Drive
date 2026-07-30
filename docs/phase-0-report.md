# Báo cáo Phase 0 hardening

Trạng thái: **HOÀN THÀNH** ngày 2026-07-30 sau validation độc lập trên source hiện tại.

## Quyết định

- Giữ TDLib official commit `022d60202e446ad1287b9fb68e687c8a0760788b` vì rebuild thực tế chứng minh tương thích.
- Crypto: OpenSSL 3.5.7 LTS, source release chính thức, SHA-256 `a8c0d28a529ca480f9f36cf5792e2cd21984552a3c8e4aa11a24aa31aeac98e8`; upstream support đến 2030-04-08.
- Backup: `allowBackup=false`, cộng exclude toàn miền cho legacy full-backup, Android 12+ cloud backup và device transfer.
- Lifecycle: `NEW → STARTING → RUNNING → CLOSING → CLOSED`, `FAILED → CLOSING → CLOSED`; start/close idempotent, close thắng initialization race, counter chỉ gắn với client đã tạo. Không dùng `Application.onTerminate()`; logout/reset/test teardown là explicit owners, process kill là abrupt.

## Provenance và ABI

- Script chạy từ repository root: `scripts/build-tdlib-android.sh`.
- Pin: TDLib commit ở trên; OpenSSL version/checksum; NDK `27.2.12479018`; CMake/Ninja `3.22.1`; API 26.
- Build host đã quan sát: Darwin arm64; toolchain NDK được phát hiện động (`darwin-x86_64` là tên package NDK trên host này, không hardcode).
- `arm64-v8a`: build/package/runtime; SHA-256 `2bf94a0c10605b370a23a908d25bb6e1c263115b498060eef00ebf00f0ed390b`.
- `x86_64`: build/package, chưa runtime-test; SHA-256 `a8c1a2da4ce873e7e3b09c3d1fea22255e6419beeca1aa88df597ba2173cbb5d`.
- `android-app/tdlib-build-metadata.txt` là record truy xuất. ELF inspection cho thấy cả hai binary chứa `OpenSSL 3.5.7 9 Jun 2026` và chỉ dynamic-link Android system libraries (`libdl`, `libz`, `liblog`, `libm`, `libc`).

## Validation

- Baseline và sau hardening: unit test, lint, real/fake debug build pass.
- Lifecycle regression: double start/close, close trong init, load/create failure, immediate start-close, counter floor pass.
- Security regression: merged manifest `allowBackup=false` + hai rule resource; source pin/checksum; metadata và binary hash/version pass.
- APK chứa `lib/arm64-v8a/libtdjsonjava.so` và `lib/x86_64/libtdjsonjava.so`.
- Fake runtime ARM64 API 36: native `not loaded`, client `not created`.
- Real runtime ARM64 API 36: native `loaded`, client `created`, `WaitingForTdlibParameters`, active clients `1`.
- Recreation: PID giữ `21424`, active clients giữ `1`. Force-stop/reopen: PID `21424 → 21662`, load/client/auth thành công lại, active clients `1`.
- Evidence mới: `docs/runtime/phase-0-hardening-*.json` và hai screenshot PNG.

## Giới hạn bằng chứng

- Runtime chỉ xác minh `arm64-v8a`; x86_64 mới build/package/inspect.
- GitNexus host không parse Kotlin do optional native parser thiếu; blast-radius chính trả UNKNOWN. Codebase Kotlin graph và diff review cho thấy thay đổi lifecycle giới hạn ở Application/container/repository/gateway/diagnostics.
- Phase 0 không xác minh credential/login/session/Saved Messages/download/preview; các mục này thuộc Phase 1.
