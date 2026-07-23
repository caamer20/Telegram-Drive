# IPC Contracts: Automated Account Migration

## Commands

### `cmd_migration_toggle_auto`

Input: `{ enabled: boolean }`  
Output: `AutoMigrationProfile`

Enabling restores/resumes `active_job_id`; creates initial root snapshot only when none exists.
Disabling pauses an auto job đang chạy với `pause_reason = "user"` tại boundary an toàn.

### `cmd_migration_rescan_auto`

Input: none  
Output: `MigrationJobDetail`

Errors: `microsoft_not_connected`, `migration_running`, `source_unavailable`.

Creates a new ordered root snapshot and atomically updates `active_job_id`.

### `cmd_migration_get_auto_status`

Output:

```ts
{
  profile: AutoMigrationProfile | null;
  account: MsAccountInfo | null;
  active_job: MigrationJobDetail | null;
}
```

### `cmd_migration_get_activity`

Input: `{ jobId: number, limit?: number }`  
Output: `MigrationActivity[]`, newest first.

### `cmd_migration_get_daily_quota`

Output:

```ts
{
  date_string: string;
  uploaded_bytes: number;
  limit_bytes: number;
  remaining_bytes: number;
  resets_at: number;
}
```

## Events

### `migration:item-progress`

```ts
{
  job_id: number;
  item_id: number;
  item_name: string;
  phase: "downloading" | "uploading";
  event_id: string;
  attempt: number;
  revision: number;
  percent: number;
  bytes_done: number;
  bytes_total: number;
  timestamp: number;
}
```

### `migration:activity`

Payload là một `MigrationActivity` đã persist. UI dedupe theo `id`.

### `migration:quota`

Payload gồm quota hiện tại và `paused_item_id` nếu projected-size gate chặn file.

## UI Invariants

- `account === null` ⇒ không mount snapshot/transfer/activity views.
- Download List chỉ nhận phase `downloading`.
- Upload List chỉ nhận phase `uploading`.
- Cùng `(job_id,item_id)` không tồn tại trong cả hai list.
- Event trùng/cũ theo `(job_id,item_id,attempt,revision)` bị bỏ qua.
- Activity chỉ đến từ command/event backend, không tạo placeholder.
