# Data Model: Automated Account Migration

## AutoMigrationProfile

| Field | Type | Rule |
|---|---|---|
| id | integer | Primary key |
| account_id | text | Unique Microsoft account ID/email |
| enabled | boolean | Master Switch |
| active_job_id | integer nullable | Auto snapshot hiện tại |
| default_telegram_dest_id | integer nullable | Null = Saved Messages |
| default_telegram_dest_name | text | Mặc định `Saved Messages` |
| local_temp_dir | text nullable | App temp khi null |
| pause_reason | text nullable | `daily_quota`, `user`, `auth`, `error` |
| last_auto_scan_at | timestamp nullable | Chỉ đổi khi initial/manual scan |
| created_at | timestamp | Required |
| updated_at | timestamp | Required |

## MigrationJob extensions

| Field | Type | Rule |
|---|---|---|
| job_origin | text | `manual` hoặc `auto` |

Auto profile có tối đa một `active_job_id`. Auto engine không được resume job không thuộc profile.

## MigrationItem extensions

| Field | Type | Rule |
|---|---|---|
| queue_position | integer | Thứ tự xử lý ổn định trong snapshot |

Unique `(job_id, source_path)` được giữ nguyên. Worker chọn `pending` theo `(queue_position, id)`.

## MigrationActivity

| Field | Type | Rule |
|---|---|---|
| id | integer | Primary key |
| job_id | integer | Required |
| item_id | integer nullable | Null cho job-level activity |
| item_name | text nullable | Snapshot tên tại thời điểm event |
| phase | text | `scan`, `downloading`, `uploading`, `completed`, `failed`, `quota` |
| status | text | Trạng thái event |
| attempt | integer | Lần thử của item; 0 cho job-level activity |
| revision | integer | Tăng đơn điệu trong một attempt |
| message | text nullable | Không chứa token |
| created_at | timestamp | Required |

Index `(job_id, created_at DESC)`. Activity được append từ backend transition thật.

## DailyMigrationQuota

| Field | Type | Rule |
|---|---|---|
| date_string | text | Ngày local `YYYY-MM-DD` |
| uploaded_bytes | integer | Chỉ file auto upload thành công |
| updated_at | timestamp | Required |

Chỉ job `auto` sử dụng bảng này. Preflight hợp lệ khi `uploaded_bytes + next_file_size <= 250 GiB`; item completed và quota increment phải cùng transaction.

## PersistedMicrosoftSession

| Field | Type | Rule |
|---|---|---|
| client_id | text | Dùng đúng app registration khi refresh |
| access_token | secret text | Không log |
| refresh_token | secret text | Không log |
| expires_at | timestamp | Required |
| tenant | text | Required |
| redirect_uri | text | Required |
| account_info | object | Tên/email |

File nằm trong app-private data, không trong repository; xóa khi Disconnect.

## State Transitions

```text
profile disabled → enabled
enabled + no snapshot → scanning → ready → running
enabled + empty root → completed(empty snapshot)
running → completed
running → paused(user|daily_quota|auth)
paused → running
terminal + manual rescan → scanning(new snapshot)
```

```text
item pending → downloading → uploading → completed
item pending → skipped_duplicate
item downloading|uploading → failed|pending(recovery)
```
