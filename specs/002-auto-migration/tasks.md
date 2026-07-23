# Tasks: Automated Account Migration (Smart Auto-Sync)

## Phase 1: Database & Engine (Backend)

- [ ] T001 Thêm bảng `auto_migration_profiles` vào `app/src-tauri/src/migration/db.rs`
- [ ] T002 Implement `get_auto_profile()` và `upsert_auto_profile()` trong `app/src-tauri/src/migration/db.rs`
- [ ] T002B Thêm logic tracking `daily_migration_quota` vào db.rs và worker.rs để tạm dừng nếu upload > 250GB/ngày
- [ ] T003 Tạo `app/src-tauri/src/migration/auto_engine.rs` — daemon task tự động khởi chạy khi ứng dụng mở, tự động chọn Temp Directory và Telegram Target mặc định
- [ ] T004 Implement IPC commands `cmd_migration_toggle_auto`, `cmd_migration_get_auto_status`, `cmd_migration_update_auto_settings` trong `app/src-tauri/src/migration/commands.rs`

---

## Phase 2: High-Tech Minimalist UI (Frontend)

- [ ] T005 Tạo `app/src/components/migration/AutoMigrationCenter.tsx` với Master Switch Bật/Tắt, Thẻ tài khoản Microsoft, và Live Activity Stream kết hợp **Real-time Progress Bar (Thanh tiến trình trực quan)**
- [ ] T006 Tạo `app/src/components/migration/AdvancedSettingsDrawer.tsx` cho phép đổi Telegram destination hoặc thư mục tạm nếu muốn
- [ ] T007 Cập nhật `app/src/components/migration/OneDriveMigrationPage.tsx` chuyển sang giao diện Tự động mặc định
- [ ] T008 Cập nhật i18n keys trong `app/src/i18n/locales/{vi,en}.json`

---

## Phase 3: Verification & Hardening

- [ ] T009 Chạy `npx tsc --noEmit` và `npm run build`
- [ ] T010 Chạy `cargo check`
