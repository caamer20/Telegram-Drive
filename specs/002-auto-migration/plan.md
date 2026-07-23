# Implementation Plan: Automated Account Migration (Smart Auto-Sync)

**Feature Branch**: `002-auto-migration` | **Spec**: [spec.md](./spec.md)

---

## Summary
Nâng cấp tính năng Migration từ dạng thủ công (tạo job, chọn folder, chọn destination, bấm scan, bấm start) thành hệ thống **Tự Động 100% (Smart Auto-Sync)**:
- **Backend**: Thêm bảng `auto_migration_profiles` trong `migration.db`. Khi bật Auto-Migration, daemon background service tự động kiểm tra tài khoản Microsoft, tự chọn Temp Directory & Telegram Target mặc định, tự động kích hoạt worker ngầm.
- **Frontend**: Thay thế trang thiết lập phức tạp bằng **Smart Auto-Migration Center** với công tắc Master Switch duy nhất, Thẻ tài khoản Microsoft, Nhật ký tự động ngầm kết hợp **Thanh tiến trình trực quan (Live Activity Stream & Real-time Progress Bar)** theo dõi sát sao tốc độ tải file, và Drawer tùy chọn nâng cao.

---

## Proposed Technical Changes

### 1. Database Schema (`app/src-tauri/src/migration/db.rs`)
Thêm bảng `auto_migration_profiles`:
```sql
CREATE TABLE IF NOT EXISTS auto_migration_profiles (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id              TEXT UNIQUE NOT NULL,
    enabled                 INTEGER NOT NULL DEFAULT 1,
    default_telegram_dest_id INTEGER,
    default_telegram_dest_name TEXT,
    local_temp_dir          TEXT,
    last_auto_scan_at       INTEGER,
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS daily_migration_quota (
    date_string             TEXT PRIMARY KEY,  -- Định dạng YYYY-MM-DD
    uploaded_bytes          INTEGER NOT NULL DEFAULT 0,
    updated_at              INTEGER NOT NULL
);
```

### 2. Auto-Migration Engine (`app/src-tauri/src/migration/auto_engine.rs`)
- Background Tokio task tự động kiểm tra profile khi app khởi động hoặc khi bật Master Switch.
- Tự động dò tìm thư mục làm việc tạm (mặc định `<app_data>/temp_migration`).
- Kiểm tra `daily_migration_quota` trước mỗi file. Nếu > 250GB, tạm dừng worker và chờ đến ngày hôm sau.
- Tự động gọi `scan_folder_recursive` và tự kích hoạt `run_migration_worker`.

### 3. Tauri IPC Commands (`app/src-tauri/src/migration/commands.rs`)
- `cmd_migration_toggle_auto`: Bật/Tắt chế độ tự động migrate cho tài khoản.
- `cmd_migration_get_auto_status`: Trả về trạng thái Auto-Migration Profile hiện tại.
- `cmd_migration_update_auto_settings`: Cập nhật cấu hình mặc định (Telegram Dest, Temp Dir) nếu người dùng chỉnh sửa trong Advanced Drawer.
- `cmd_migration_get_daily_quota`: Lấy thông tin dung lượng đã upload trong ngày để hiển thị UI.

### 4. Frontend UI Components (`app/src/components/migration/`)
- `AutoMigrationCenter.tsx`: Giao diện chính chứa Master Switch, Status Card, Nhật ký ngầm, Thanh tiến trình, và **Daily Quota Tracker** cảnh báo giới hạn 250GB.
- `AdvancedSettingsDrawer.tsx`: Drawer trượt tùy chỉnh kênh Telegram / thư mục tạm nếu muốn.
- Update `OneDriveMigrationPage.tsx` để hiển thị chế độ Tự Động làm mặc định.

---

## Verification Plan

### Automated Verification
- `cargo check` trong `app/src-tauri`
- `npx tsc --noEmit` và `npm run build` trong `app`

### Manual Verification
1. Đăng nhập Microsoft -> Bật Master Switch "Tự Động Migrate".
2. Kiểm tra dữ liệu tự động quét và tải lên Telegram ngầm mà không cần bấm Scan hay Start.
3. Tắt ứng dụng, mở lại -> Xác nhận hệ thống tự động nhận diện và duy trì trạng thái ngầm.
