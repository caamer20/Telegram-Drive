# Quickstart: Automated Account Migration

## Prerequisites

- Telegram Drive desktop đã đăng nhập Telegram.
- Microsoft account test có file trong OneDrive root.
- Saved Messages khả dụng.

## A. Connection Gate

1. Xóa Microsoft session hoặc Disconnect.
2. Mở Auto Migration.
3. Xác nhận chỉ có trạng thái kết nối; không có snapshot table, Download List, Upload List hoặc Activity Stream.

## B. Initial Snapshot và Sequential Transfer

1. Connect Microsoft và bật Master Switch.
2. Xác nhận root được scan đúng một lần và snapshot có toàn bộ file.
3. Xác nhận chỉ một file active.
4. Khi download: file chỉ ở Download List.
5. Khi upload: cùng file rời Download List và chỉ ở Upload List.
6. Khi completed: file rời transfer list và xuất hiện trong Activity Stream.

## C. No Automatic Rescan

1. Khi snapshot đang chạy, thêm một file test mới vào OneDrive.
2. Xác nhận file không tự xuất hiện trong snapshot.
3. Đợi job không còn `running`, nhấn **Quét lại**.
4. Xác nhận snapshot mới chứa file test và thứ tự ổn định.

## D. Restart Resume

1. Đóng app khi snapshot còn pending.
2. Mở lại app.
3. Xác nhận Microsoft session tự restore.
4. Xác nhận snapshot cũ được resume, không có Graph scan mới và file completed không chạy lại.

## E. Saved Messages Default

1. Xóa destination tùy chỉnh trong Advanced Settings.
2. Tạo snapshot mới.
3. Xác nhận file được upload vào Saved Messages.

## F. Daily Quota

1. Dùng test fixture đặt quota gần 250 GiB.
2. Đặt file tiếp theo lớn hơn remaining quota.
3. Xác nhận file vẫn pending, worker pause trước download và activity ghi `daily_quota`.
4. Chuyển test clock qua local midnight.
5. Xác nhận quota reset và worker tiếp tục đúng file.

## G. Credential Hygiene

1. Connect và restart app, xác nhận session tự restore.
2. Tìm trong repository và log, xác nhận không có access/refresh token.
3. Disconnect và restart, xác nhận session không tự restore.

## Verification Commands

```bash
cd app/src-tauri
cargo test --lib
cargo check

cd ../../app
npx tsc --noEmit
npm run build
```

## Kết quả xác minh 2026-07-23

| Hạng mục | Bằng chứng tự động | Kết quả |
|---|---|---|
| A. Connection Gate | `ConnectionGate.test.tsx` xác nhận không mount ba vùng dữ liệu trước connect | PASS |
| B. Sequential Transfer | Rust single-worker guard và selector test xác nhận phase-exclusive lists | PASS |
| C. No Automatic Rescan | Profile persist `active_job_id`; engine chỉ scan khi chưa có snapshot hoặc gọi Rescan | PASS |
| D. Restart Resume | Session round-trip/delete test và active snapshot ownership test | PASS |
| E. Saved Messages | Auto engine dùng `Saved Messages` khi destination null | PASS |
| F. Daily Quota | Projected-size/local-day/manual-exclusion/atomic accounting tests | PASS |
| G. Credential Hygiene | Session nằm app-data, permission Unix `0600`, atomic replace và ignore `microsoft-session*.json` | PASS |

Các lệnh `cargo check`, `cargo test --lib`, `npm test`, `npx tsc --noEmit` và `npm run build` đều pass. Smoke test OAuth/Graph/MTProto với tài khoản thật vẫn phụ thuộc phiên Microsoft và Telegram của người dùng trên máy chạy ứng dụng.
