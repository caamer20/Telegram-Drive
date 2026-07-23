# IPC Contracts: OneDrive Migration

**Feature**: 001-onedrive-migration | **Phase**: 1 — Design & Contracts | **Ngày**: 2026-07-23

## Tổng quan

Giao tiếp giữa frontend (React/TypeScript) và backend (Rust/Tauri) qua **Tauri IPC** gồm hai cơ chế:
- **Commands** (`invoke`): Frontend gọi backend, trả về kết quả đồng bộ hoặc bất đồng bộ
- **Events** (`emit`/`listen`): Backend push thông báo cho frontend

---

## Tauri Commands (~15)

### Auth — Microsoft OAuth

#### `cmd_migration_ms_connect`
Kết nối tài khoản Microsoft qua OAuth.

```
Invoke: cmd_migration_ms_connect()
Returns: Result<MsAccountInfo, String>

MsAccountInfo {
    account_name: String,   // Tên hiển thị
    account_email: String,  // Email
}
```
**Side effects**: Mở browser, start local callback server, exchange code lấy token, lưu trong process memory.

---

#### `cmd_migration_ms_disconnect`
Ngắt kết nối Microsoft, xóa token.

```
Invoke: cmd_migration_ms_disconnect()
Returns: Result<(), String>
```
**Side effects**: Xóa token khỏi process memory, xóa thông tin thư mục nguồn đã chọn.

---

#### `cmd_migration_ms_status`
Lấy trạng thái kết nối Microsoft hiện tại.

```
Invoke: cmd_migration_ms_status()
Returns: Result<Option<MsAccountInfo>, String>
```
Trả về `null` nếu chưa kết nối.

---

### OneDrive Browsing

#### `cmd_migration_list_onedrive_folders`
Duyệt danh sách children (thư mục và file) của một thư mục OneDrive. Dùng cho folder tree browsing.

```
Invoke: cmd_migration_list_onedrive_folders(parent_id: Option<String>)
Returns: Result<Vec<OneDriveItem>, String>

OneDriveItem {
    id: String,             // OneDrive item ID
    name: String,           // Tên file/thư mục
    item_type: String,      // "folder" | "file"
    size: i64,              // Dung lượng (bytes), 0 cho folder
    path: Option<String>,   // Đường dẫn tương đối
    child_count: Option<i64>, // Số children (chỉ cho folder)
}
```
**Input**: `parent_id = None` → root OneDrive. `parent_id = Some("...")` → children của thư mục đó.
**Guard**: Microsoft phải đã kết nối.
**Pagination**: Tự động xử lý `@odata.nextLink` phía backend — trả về toàn bộ children.

---

### Job Management

#### `cmd_migration_create_job`
Tạo migration job mới.

```
Invoke: cmd_migration_create_job()
Returns: Result<MigrationJob, String>
```
**Side effects**: INSERT vào `migration_jobs` với state `draft`.

---

#### `cmd_migration_get_jobs`
Lấy danh sách tất cả job (history).

```
Invoke: cmd_migration_get_jobs()
Returns: Result<Vec<MigrationJobSummary>, String>

MigrationJobSummary {
    id: i64,
    state: String,
    onedrive_folder_path: Option<String>,
    total_files: i64,
    completed_files: i64,
    created_at: i64,
}
```

---

#### `cmd_migration_get_job`
Lấy chi tiết một job (kèm stats, items).

```
Invoke: cmd_migration_get_job(job_id: i64)
Returns: Result<MigrationJobDetail, String>

MigrationJobDetail {
    job: MigrationJob,
    stats: MigrationStats,
    folders: Vec<FolderSummary>,
    files: Vec<MigrationItem>,     // Có thể phân trang nếu nhiều
}

MigrationJob {
    id: i64,
    state: String,
    onedrive_folder_id: Option<String>,
    onedrive_folder_path: Option<String>,
    telegram_destination_id: Option<i64>,
    telegram_destination_name: Option<String>,
    local_dir: Option<String>,
    cooldown_until: Option<i64>,
    created_at: i64,
    started_at: Option<i64>,
    completed_at: Option<i64>,
    updated_at: i64,
}

MigrationStats {
    total_folders: i64,
    total_files: i64,
    total_bytes: i64,
    completed_files: i64,
    completed_bytes: i64,
    failed_files: i64,
    skipped_duplicates: i64,
    pending_files: i64,
}

FolderSummary {
    source_path: String,    // Đường dẫn tương đối
    name: String,           // Tên thư mục
    file_count: i64,
    total_size: i64,
}

MigrationItem {
    id: i64,
    item_type: String,      // "file" | "folder"
    name: String,
    source_path: String,
    size_bytes: i64,
    state: String,
    error_type: Option<String>,
    error_message: Option<String>,
    retry_count: i64,
    content_sha256: Option<String>,
    telegram_message_id: Option<i64>,
    completed_at: Option<i64>,
}
```

---

#### `cmd_migration_delete_job`
Xóa job và tất cả items liên quan.

```
Invoke: cmd_migration_delete_job(job_id: i64)
Returns: Result<(), String>
```
**Guard**: Không xóa được job đang `running`.

---

### Configuration

#### `cmd_migration_set_onedrive_folder`
Chọn thư mục OneDrive nguồn.

```
Invoke: cmd_migration_set_onedrive_folder(
    job_id: i64,
    folder_id: String,      // OneDrive item ID
    folder_path: String     // Đường dẫn hiển thị
)
Returns: Result<(), String>
```

---

#### `cmd_migration_set_telegram_destination`
Chọn Telegram destination.

```
Invoke: cmd_migration_set_telegram_destination(
    job_id: i64,
    destination_id: Option<i64>,    // None = Saved Messages
    destination_name: String
)
Returns: Result<(), String>
```

---

#### `cmd_migration_set_local_dir`
Chọn thư mục local làm việc.

```
Invoke: cmd_migration_set_local_dir(
    job_id: i64,
    local_dir: String       // Đường dẫn tuyệt đối
)
Returns: Result<(), String>
```
**Side effects**: Kiểm tra thư mục tồn tại và writable.

---

### Scan

#### `cmd_migration_scan`
Quét thư mục OneDrive nguồn, tạo snapshot.

```
Invoke: cmd_migration_scan(job_id: i64)
Returns: Result<MigrationStats, String>
```
**Guard**: Job phải ở state `draft` hoặc `ready`.
**Side effects**: Xóa snapshot cũ, tạo items mới, cập nhật stats.
**Events**: Emit `migration:job-state`, `migration:stats` khi hoàn tất.

---

### Controls

#### `cmd_migration_start`
Bắt đầu migration.

```
Invoke: cmd_migration_start(job_id: i64)
Returns: Result<(), String>
```
**Guard**: Job phải ở state `ready`. Chỉ 1 job running.
**Side effects**: Spawn Tokio task cho worker loop.
**Events**: `migration:job-state` → `running`.

---

#### `cmd_migration_pause`
Tạm dừng migration (sau file hiện tại).

```
Invoke: cmd_migration_pause(job_id: i64)
Returns: Result<(), String>
```
**Events**: `migration:job-state` → `paused` (sau khi file hiện tại hoàn tất).

---

#### `cmd_migration_resume`
Tiếp tục migration đã pause.

```
Invoke: cmd_migration_resume(job_id: i64)
Returns: Result<(), String>
```
**Guard**: Job phải ở state `paused`.
**Events**: `migration:job-state` → `running`.

---

#### `cmd_migration_cancel`
Hủy migration.

```
Invoke: cmd_migration_cancel(job_id: i64)
Returns: Result<(), String>
```
**Side effects**: Set cancel flag, worker không bắt đầu file mới.
**Events**: `migration:job-state` → `cancelled` (sau khi file hiện tại hoàn tất hoặc ngay lập tức nếu không có file đang xử lý).

---

#### `cmd_migration_retry_item`
Retry một file lỗi.

```
Invoke: cmd_migration_retry_item(job_id: i64, item_id: i64)
Returns: Result<(), String>
```
**Side effects**: Reset `retry_count` về 0, `state` → `pending`.

---

#### `cmd_migration_retry_all_failed`
Retry tất cả file failed trong job.

```
Invoke: cmd_migration_retry_all_failed(job_id: i64)
Returns: Result<i64, String>  // Số file được retry
```

---

## Events (5 events)

### `migration:job-state`
Job chuyển trạng thái.

```typescript
interface JobStatePayload {
    job_id: number;
    state: string;          // "draft" | "ready" | "running" | "paused" | "completed" | "cancelled" | "failed"
    previous_state: string;
}
```

### `migration:item-progress`
Tiến độ file hiện tại.

```typescript
interface ItemProgressPayload {
    job_id: number;
    item_id: number;
    item_name: string;
    phase: string;          // "downloading" | "uploading"
    percent: number;        // 0-100
    bytes_done: number;
    bytes_total: number;
    speed_bytes_per_sec: number;
}
```

### `migration:item-complete`
File hoàn thành xử lý.

```typescript
interface ItemCompletePayload {
    job_id: number;
    item_id: number;
    item_name: string;
    status: string;         // "completed" | "skipped_duplicate" | "failed"
    error_type?: string;
    error_message?: string;
}
```

### `migration:stats`
Thống kê job thay đổi.

```typescript
interface StatsPayload {
    job_id: number;
    stats: MigrationStats;
}
```

### `migration:cooldown`
Trạng thái cooldown thay đổi.

```typescript
interface CooldownPayload {
    job_id: number;
    cooldown_until: number | null;  // Unix timestamp, null = hết cooldown
    seconds_remaining: number;      // 0 nếu hết cooldown
}
```

---

## Frontend Usage Pattern

```typescript
// useMigration.ts — pattern tham khảo

import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { useState, useEffect, useCallback } from 'react';
import { toast } from 'sonner';

export function useMigration() {
    const [currentJob, setCurrentJob] = useState<MigrationJobDetail | null>(null);
    const [jobs, setJobs] = useState<MigrationJobSummary[]>([]);
    const [itemProgress, setItemProgress] = useState<ItemProgressPayload | null>(null);
    const [cooldown, setCooldown] = useState<CooldownPayload | null>(null);

    // Listen events
    useEffect(() => {
        const unlisteners: UnlistenFn[] = [];
        
        listen<JobStatePayload>('migration:job-state', (e) => {
            setCurrentJob(prev => prev ? { ...prev, job: { ...prev.job, state: e.payload.state } } : null);
            toast.info(`Job ${e.payload.state}`);
        }).then(fn => unlisteners.push(fn));

        listen<ItemProgressPayload>('migration:item-progress', (e) => {
            setItemProgress(e.payload);
        }).then(fn => unlisteners.push(fn));

        listen<ItemCompletePayload>('migration:item-complete', (e) => {
            if (e.payload.status === 'failed') {
                toast.error(`${e.payload.item_name}: ${e.payload.error_message}`);
            }
            refreshJob();
        }).then(fn => unlisteners.push(fn));

        listen<StatsPayload>('migration:stats', (e) => {
            setCurrentJob(prev => prev ? { ...prev, stats: e.payload.stats } : null);
        }).then(fn => unlisteners.push(fn));

        listen<CooldownPayload>('migration:cooldown', (e) => {
            setCooldown(e.payload);
        }).then(fn => unlisteners.push(fn));

        return () => { unlisteners.forEach(fn => fn()); };
    }, []);

    // ... command wrappers
}
```

---

## Error Handling Convention

Tất cả commands trả về `Result<T, String>`. Frontend parse error string để hiển thị:

```typescript
try {
    await invoke('cmd_migration_start', { jobId: 1 });
} catch (e) {
    toast.error(`Không thể bắt đầu migration: ${e}`);
}
```

Backend structured errors (trong migration module) được map về String ở Tauri command boundary:

```rust
// Trong migration/commands.rs
#[tauri::command]
async fn cmd_migration_start(
    job_id: i64,
    state: State<'_, MigrationState>,
) -> Result<(), String> {
    state.orchestrator.start_job(job_id).await.map_err(|e| e.to_string())
}
```

---

## Security

- Không command nào trả về `access_token` hoặc `refresh_token`
- `cmd_migration_ms_status` chỉ trả về `account_name` và `account_email`
- Tất cả token operations được xử lý trong backend
- Log filtering: strip `Authorization` header, token values trước khi log
