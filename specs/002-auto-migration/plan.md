# Implementation Plan: Automated Account Migration

**Branch**: `002-auto-migration` | **Ngày**: 2026-07-23
**Spec**: [spec.md](./spec.md) | **Constitution**: v1.2.0

## Summary

Hoàn thiện Auto Migration như một lớp điều phối trên worker OneDrive Migration hiện có. Khi người dùng kết nối Microsoft lần đầu, hệ thống bật Auto Migration mặc định, dùng toàn bộ OneDrive root, tạo một snapshot có thứ tự ổn định, rồi download và upload tuần tự từng file vào Saved Messages. Snapshot không tự thay đổi; người dùng dùng nút **Quét lại** khi muốn tạo danh sách mới.

Giao diện chỉ hiển thị dữ liệu migration sau khi Microsoft đã kết nối, hiển thị loading trong lúc tạo snapshot đầu tiên và cung cấp hành động đổi tài khoản ngay trên thẻ OneDrive. File ở phase `downloading` và `uploading` được trình bày trong hai danh sách độc lập. Live Activity Stream chỉ dùng event và migration item thực tế.

## Technical Context

**Backend**: Rust, Tauri v2, Tokio, reqwest, Grammers/MTProto, sqlite
**Frontend**: React 19, TypeScript, Tailwind CSS v4, Tauri IPC/events, i18next
**Persistence**: `migration.db` cho profile/job/item/activity/quota; app-private file cho Microsoft session
**Transfer model**: Một job active, một file active, download hoàn tất trước upload
**Source**: OneDrive root
**Destination mặc định**: Saved Messages (`telegram_destination_id = NULL`)
**Quota**: 250 GiB/ngày local, preflight trước download, chỉ cộng sau upload thành công
**Platforms**: Windows, macOS, Linux; không áp dụng Android/iOS
**Validation**: Rust unit/integration tests, TypeScript type-check, production build và quickstart

## Constitution Check

| Nguyên tắc | Trạng thái | Thiết kế đáp ứng |
|---|---|---|
| I. Actix Web | PASS | Feature không thêm HTTP route; orchestration nội bộ dùng Tokio theo ngoại lệ constitution. |
| II. Tauri IPC + React | PASS | Commands/events là boundary; state feature được gom vào `MigrationContext` thay vì state rời rạc trong page. |
| III. Telegram MTProto | PASS | Tái sử dụng `TelegramState` và `upload_core`; không tạo Telegram client thứ hai. |
| IV. SQLite | PASS | Raw SQL trong `migration.db`, WAL và synchronous=FULL; migration schema idempotent. |
| V. Spec-Driven Development | PASS | Spec đã clarify, checklist 16/16 trước khi plan/tasks được sinh lại. |
| VI. Background Processing | PASS | Worker nằm trong Tauri process; feature không cam kết chạy sau khi process bị terminate. |
| i18n | PASS | Mọi UI text mới có key tiếng Việt và tiếng Anh trong cùng phase. |
| Security | PASS | Microsoft session ở app-private data, không nằm trong repository/log và bị xóa khi Disconnect. |

**Gate**: PASS. Không có constitution violation được chấp nhận.

## Current-State Gap Analysis

- Auto engine đã scan root và reuse worker nhưng có thể chọn nhầm manual job vì truy vấn mọi job `running/ready/paused`.
- Engine tự scan mỗi khi không tìm thấy job phù hợp; chưa có liên kết profile → active snapshot rõ ràng.
- Daily quota dùng ngày UTC và chỉ kiểm tra tổng đã dùng, chưa preflight `uploaded + next_file_size`.
- Microsoft session chỉ nằm trong memory; restart không thể tự resume.
- UI luôn render file table dù chưa kết nối Microsoft.
- UI hiện ghép hai progress bar vào một card thay vì hai transfer list loại trừ lẫn nhau.
- Chưa có activity store bền vững; danh sách lớn hiện là snapshot file table, không phải Live Activity Stream.
- Nút refresh hiện chỉ refresh status/quota, chưa tạo manual snapshot mới.

## Project Structure

```text
app/src-tauri/src/migration/
├── auto_engine.rs          # profile lifecycle, startup resume, manual rescan
├── commands.rs             # IPC commands và event payloads
├── db.rs                   # schema/repository/transactions
├── microsoft.rs            # OAuth, session persistence helpers, Graph scan
├── models.rs               # profile, activity, quota, transfer DTO
├── session_store.rs        # app-private Microsoft session persistence
├── worker.rs               # ordered sequential pipeline + quota preflight
└── tests.rs                # feature-level integration tests

app/src/
├── components/migration/
│   ├── AutoMigrationCenter.tsx
│   ├── ConnectionGate.tsx
│   ├── TransferList.tsx
│   ├── ActivityStream.tsx
│   └── AdvancedSettingsDrawer.tsx
├── context/MigrationContext.tsx
├── hooks/useMigration.ts
└── types.ts
```

## Design Decisions

### Microsoft Session Persistence

- Serialize `MicrosoftSession` cùng `client_id` vào app-private data directory, ngoài repository.
- Ghi atomic qua file tạm rồi rename.
- Trên Unix đặt permission `0600`; các platform khác dựa vào app-private directory.
- Không log token hoặc nội dung file.
- Connect/exchange/refresh ghi lại session mới; Disconnect xóa file.
- App startup load session trước khi khởi động auto engine.

### Snapshot Ownership

- `auto_migration_profiles.active_job_id` liên kết profile với snapshot hiện tại.
- `migration_jobs.job_origin` phân biệt `manual` và `auto`.
- Auto engine chỉ resume job được profile tham chiếu; không chọn manual job.
- Nếu profile chưa có `active_job_id`, engine scan root một lần, sort file theo `source_path` rồi `source_item_id`, persist `queue_position` và tạo job.
- Kết quả scan rỗng vẫn là một snapshot hợp lệ và được persist, tránh tự scan lại ở lần khởi động sau.
- Khi snapshot tồn tại, startup chỉ resume; không scan.
- Manual rescan chỉ chạy khi không có job `running`; tạo job auto mới và cập nhật `active_job_id` trong một transaction.
- Tài khoản mới chưa có profile được tạo profile `enabled = true`; frontend chờ lệnh tạo snapshot đầu tiên để duy trì loading chính xác và hydrate danh sách ngay khi hoàn tất.

### Sequential Ordering

- `migration_items.queue_position` là thứ tự canonical.
- Worker chọn item `pending` theo `queue_position ASC, id ASC`.
- Chỉ một worker được phép chạy qua `worker_running` và DB one-active-job guard.
- File không bao giờ ở `downloading` và `uploading` cùng lúc.

### Daily Quota

- Quota key dùng ngày local `YYYY-MM-DD`.
- Trước download: nếu `uploaded_bytes + item.size_bytes > 250 GiB`, giữ item `pending`, đặt job `paused` với reason `daily_quota`, emit quota state và dừng worker.
- Chỉ job `job_origin = auto` chịu quota; job manual không đọc hoặc cộng bộ đếm Auto Migration.
- Với job auto, item completed và quota increment được commit trong cùng transaction sau upload thành công.
- Auto engine đặt timer đến local midnight; nếu profile còn enabled và pause reason là quota, resume snapshot.

### UI Data Integrity

- Khi `msAccount === null`, page chỉ render Connection Gate; settings, file table, quota, activity và transfer lists không được mount.
- `downloadTransfers` chỉ chứa migration item thực tế ở phase `downloading`.
- `uploadTransfers` chỉ chứa migration item thực tế ở phase `uploading`.
- Progress event có `event_id`, `attempt`, `revision` và `timestamp`; reducer bỏ event trùng hoặc cũ theo `job_id + item_id + attempt`.
- Progress event cập nhật item theo `job_id + item_id`; item không thể tồn tại trong cả hai collection.
- Activity lấy từ `migration_activity`, keyed theo activity ID; không sinh placeholder.
- Snapshot file table chỉ hiển thị sau khi kết nối và khác với Live Activity Stream.
- Tắt Master Switch chuyển auto job đang chạy sang `paused(user)` và worker dừng tại boundary an toàn; bật lại resume đúng snapshot.

## Implementation Phases

### Phase 1 — Persistence và Schema

- Thêm `session_store.rs`.
- Mở rộng `MicrosoftSession` với `client_id`.
- Thêm `active_job_id`, `pause_reason`, `job_origin`, `queue_position`.
- Thêm bảng `migration_activity`.
- Thêm migration schema idempotent và repository functions.

**Exit**: Session round-trip được test; schema upgrade không làm mất job hiện có.

### Phase 2 — Auto Engine và Worker

- Khôi phục session trước auto engine.
- Tạo snapshot root ban đầu có thứ tự.
- Resume đúng `active_job_id`, không scan lại.
- Thêm manual rescan command.
- Áp dụng Saved Messages mặc định.
- Sửa quota local-day và projected-size gate.
- Persist activity tại các phase/state transition.

**Exit**: Startup resume không gọi Graph scan; manual rescan tạo snapshot mới; worker tuần tự và quota gate đúng.

### Phase 3 — IPC và React State

- Bổ sung commands: restore status, rescan, activity list và quota.
- Chuẩn hóa event payload có `job_id`, `item_id`, `phase`, bytes và timestamp.
- Tạo `MigrationContext` sở hữu state/listener lifecycle.
- `useMigration` trở thành consumer API của context.

**Exit**: Listener chỉ đăng ký một lần; reconnect/reload trả cùng authoritative state.

### Phase 4 — Connection Gate và Transfer UI

- Chỉ render nội dung migration sau khi kết nối.
- Tách `TransferList` cho OneDrive download và Telegram upload.
- Thêm `ActivityStream` từ dữ liệu thực.
- Đổi refresh thành nút **Quét lại**, disabled khi running.
- Giữ snapshot file table như lịch trình đầy đủ, không dùng nó thay activity.
- Bổ sung i18n vi/en và empty/error states.

**Exit**: Không có list trước connect; file chuyển giữa hai list trong ≤2 giây và không trùng.

### Phase 5 — Verification và Hardening

- Unit tests: ordering, projected quota, local date, session persistence, activity dedupe.
- Integration tests: initial snapshot, restart resume without scan, manual rescan, Saved Messages default, one-active-job.
- UI tests cho connection gate và phase-exclusive lists.
- Chạy type-check, production build, Rust tests và quickstart.

**Exit**: Tất cả automated checks và quickstart pass; đủ điều kiện `/speckit-converge`.

## Complexity Tracking

| Quyết định | Lý do | Phương án đơn giản hơn bị loại |
|---|---|---|
| Persist activity riêng | Activity phải chính xác và tồn tại sau restart | Suy ra từ UI state sẽ mất lịch sử và dễ tạo dữ liệu giả |
| Profile tham chiếu active job | Tránh resume nhầm manual job | Query “job mới nhất” không đảm bảo ownership |
| `queue_position` explicit | Chứng minh thứ tự ổn định | Dựa vào row ID hoặc Graph traversal không phải contract ổn định |
| Session file riêng | Người dùng ưu tiên tự resume sau restart | Memory-only buộc reconnect, trái SC-002 |

## Post-Design Constitution Check

Thiết kế vẫn PASS toàn bộ constitution gates. Feature chạy trong Tauri process, dùng chung Telegram client, dùng Tauri IPC, SQLite raw SQL và không mở rộng cam kết unattended operation sau khi process đã terminate.
