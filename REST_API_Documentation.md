# Telegram Drive REST API Reference

Telegram Drive includes an opt-in local REST API for automating file, folder, search, storage, thumbnail, media, and bulk operations. The API is available in the desktop app on Windows, macOS, and Linux. It is disabled by default, binds only to the loopback interface, and requires the desktop app to remain open and signed in.

## Enable the API

1. Open **Settings → Advanced** in the desktop app.
2. Generate an API key and copy it immediately. The app stores only its hash, so the plaintext key cannot be shown again.
3. Choose an unused port. The default is `8550`.
4. Enable the REST API and confirm that its status is running.

Regenerating the API key immediately invalidates the previous key. Keep the key out of source control, shell history, screenshots, and public issue reports.

## Base URL

```text
http://127.0.0.1:8550/api/v1
```

The port is configurable. Replace `8550` in examples if a different port is selected in Settings. Do not expose, proxy, or forward this loopback service to a LAN or the internet.

---

## Authentication

All endpoints except `/health` and `/openapi.json` require an API key passed in the request headers. The key is checked before the request body, query, or path is read.

| Header | Type | Description |
| :--- | :--- | :--- |
| `X-API-Key` | String | Your Telegram Drive API access key |

### Example Request

```bash
curl -H "X-API-Key: YOUR_API_KEY" \
  http://127.0.0.1:8550/api/v1/files
```

---

## Machine-readable contract

An OpenAPI 3.1 description of every route, parameter, and response is served at `/api/v1/openapi.json` (no key required) and kept in the repository at `app/src-tauri/api/openapi-v1.json`. A repository check fails when a route and the contract disagree.

```bash
curl http://127.0.0.1:8550/api/v1/openapi.json
```

## Conventions

- **File identifiers** are Telegram message identifiers: positive 32-bit integers, unique within their folder. Pass `folder_id` together with a file identifier unless the file is in Saved Messages. An identifier outside that range is refused with `400 INVALID_FILE_ID`.
- **Folder identifiers** are 64-bit integers. `folder_id` is `null` for Saved Messages.
- **Timestamps** are RFC 3339 in UTC, for example `2026-06-05T10:00:00Z`.
- **Listing scope.** A cold folder read walks history once and retains the newest 50,000 files. Larger folders return `"complete": false`; totals, pages and reports describe the retained subset. Incomplete desktop and WebDAV snapshots do not authorize removal of older cached files.
- **Freshness.** A recent snapshot can be reused for 30 seconds. A scheduled tick performs catch-up every 30 seconds for folders used in the last ten minutes. Verified uploads from desktop, REST, WebDAV and Folder Sync dirty only their destination; the next listing catches up without a full audit. Verified renames and deletions check their affected identifiers. `refresh=true` requests catch-up even inside the reuse window.
- **External edits and deletion.** A rotating audit shares at most 25 lookup batches per rolling minute across monitored folders. Retention is bounded by 100,000 rows and 1,000 cross-folder audit batches, with up to 1,024 metadata entries. Healthy folders finish a shared sweep before starting another; failed folders retain stale status and retry after four minutes without blocking later healthy rounds. A sweep waits at most four minutes before starting and has a nominal 40-minute request allowance plus scheduling and request time. Offline periods, failed requests and Telegram FLOOD waits suspend that detection bound. Idle or evicted folders perform one cold walk on their next read. A mutation during a cold walk may require a later audit to repair; cold reads no longer immediately verify the entire result a second time.
- **Cost and memory.** Unchanged identifier cursors are not rewritten. Message snapshots share immutable references across listing readers. The retained-row limit is a cache bound; at most two reconciliations fetch or clone maps concurrently, and consumer snapshots can temporarily retain additional rows. Active search generations pin included folders. When a new folder cannot fit, global search returns an explicitly partial answer from retained folders; alternating global and folder search scopes can rebuild the cached index. Folder discovery is rechecked independently about every four minutes for active accounts; its dialog requests are additional to the file-history and lookup counters. Whole-library reports still copy their response data and may cost one cold walk per folder outside the retained set.
- **First request.** Reading every folder of a large library takes time and is subject to Telegram's rate limits. Pass `folder_id` when one folder is enough.

---

## Endpoints

### 1. Health Check
Check API availability, status, and running version.

* **URL:** `/health`
* **Method:** `GET`
* **Auth Required:** No

#### Response (200 OK)
```json
{
  "status": "ok",
  "version": "x.y.z"
}
```

---

### 2. List Files
Retrieve metadata for files stored in Telegram Drive.

* **URL:** `/files`
* **Method:** `GET`
* **Auth Required:** Yes

#### Query Parameters

| Parameter | Type | Description |
| :--- | :--- | :--- |
| `page` | Integer | Page number (default: `1`) |
| `limit` | Integer | Items per page, `1`–`100` (default: `20`) |
| `folder_id` | Integer | Read one folder. Omit to read every folder. An empty value, `null`, `none`, or `home` reads Saved Messages |
| `search` | String | Case-insensitive match on the file name |
| `offset_id` | Integer | Include only files whose identifier is lower than this one |
| `sort` | String | Field to sort by: `name`, `size`, or `created_at` (default) |
| `order` | String | Sort order: `asc` (default) or `desc` |
| `mime_type` | String | Case-insensitive match on the MIME type |
| `created_after` | String | Include files created at or after this time. RFC 3339; a plain date (`2026-06-05`) and Unix seconds are also accepted |
| `created_before` | String | Include files created at or before this time |
| `size_min` | Integer | Minimum file size in bytes |
| `size_max` | Integer | Maximum file size in bytes |
| `fields` | String | Comma-separated response fields when a reduced representation is needed |
| `refresh` | Boolean | `true` reads Telegram again instead of reusing a recent reading |

A value that cannot be read as a timestamp returns `400 INVALID_TIMESTAMP`; a non-numeric `folder_id` returns `400 INVALID_FOLDER_ID`.

#### Response (200 OK)
```json
{
  "data": [
    {
      "id": 123,
      "folder_id": 456,
      "name": "document.pdf",
      "size": 102400,
      "mime_type": "application/pdf",
      "created_at": "2026-06-05T10:00:00Z",
      "encrypted": false
    }
  ],
  "files": [
    {
      "id": 123,
      "folder_id": 456,
      "name": "document.pdf",
      "size": 102400,
      "mime_type": "application/pdf",
      "created_at": "2026-06-05T10:00:00Z",
      "encrypted": false
    }
  ],
  "page": 1,
  "limit": 20,
  "total": 1,
  "complete": true,
  "pagination": {
    "page": 1,
    "limit": 20,
    "total": 1,
    "total_pages": 1,
    "has_next": false,
    "has_prev": false
  }
}
```

`files` repeats `data` for earlier clients. `total` counts every file that matches the filters, not only the returned page.

---

### 3. Get File Details
Retrieve detailed metadata for a specific file.

* **URL:** `/files/{message_id}`
* **Method:** `GET`
* **Auth Required:** Yes

#### Response (200 OK)
```json
{
  "id": 123,
  "folder_id": 456,
  "name": "document.pdf",
  "size": 102400,
  "mime_type": "application/pdf",
  "created_at": "2026-06-05T10:00:00Z",
  "encrypted": false
}
```

`encrypted` is `true` for a file stored as an encrypted envelope. Its name, type, and size then describe the envelope, not the original file.

---

### 4. Download File
Stream or download a file directly from Telegram Drive.

* **URL:** `/files/{message_id}/download`
* **Method:** `GET`
* **Auth Required:** Yes

---

### 5. Search Files
Search files by filename with optional filtering.

* **URL:** `/files/search`
* **Method:** `GET`
* **Auth Required:** Yes

#### Query Parameters

| Parameter | Type | Description |
| :--- | :--- | :--- |
| `q` | String | **Required.** Case-insensitive text to find in file names |
| `folder_id` | Integer | Search one folder. Omit to search every folder |
| `refresh` | Boolean | `true` reads Telegram again instead of reusing a recent reading |

The response is an array of matching files in the same shape as **Get File Details**. The response header `X-Inventory-Complete` is `false` when results cover only the retained inventory.

---

### 6. Upload File
Upload a file to Telegram Drive.

* **URL:** `/files`
* **Method:** `POST`
* **Auth Required:** Yes
* **Content-Type:** `multipart/form-data`

#### Form Fields
* `file`: Exactly one binary file field, up to Telegram's 2,000,000,000-byte upload limit
* `folder_id` (Optional): One integer target folder/channel ID; omit it for Saved Messages

Unknown fields, duplicate fields, malformed multipart bodies, and path-like filenames are rejected or normalized before upload. Bandwidth is reserved atomically and released automatically if staging, Telegram upload, or message creation fails.

#### Example

```bash
curl -X POST \
  -H "X-API-Key: YOUR_API_KEY" \
  -F "file=@./report.pdf" \
  -F "folder_id=456" \
  http://127.0.0.1:8550/api/v1/files
```

The endpoint returns `400` for invalid multipart fields, `413` for an oversized file, and `500` if local staging or Telegram delivery fails.

#### Response (200 OK)
```json
{
  "id": 123,
  "folder_id": 456,
  "name": "uploaded_file.txt",
  "size": 1024,
  "mime_type": "text/plain",
  "created_at": "2026-06-16T01:00:00Z",
  "encrypted": false
}
```

---

### 7. Delete File
Delete a specific file.

* **URL:** `/files/{message_id}`
* **Method:** `DELETE`
* **Auth Required:** Yes

#### Query Parameters
* `folder_id` (Optional): ID of folder containing the file

Encrypted files can be deleted. Nothing is decrypted, and the app's record of the envelope is removed with the file.

---

### 8. Copy File
Forward a file/message to another folder.

* **URL:** `/files/{message_id}/copy`
* **Method:** `POST`
* **Auth Required:** Yes

#### Request Body
```json
{
  "folder_id": 789,
  "source_folder_id": 456
}
```

An encrypted file is copied as stored, still encrypted, and the copy is registered so the app can open it.

---

### 9. Update File (Rename / Move)
Rename (edit description) or move a file.

* **URL:** `/files/{message_id}`
* **Method:** `PATCH`
* **Auth Required:** Yes

#### Request Body (All fields optional)
```json
{
  "name": "new_name.txt",
  "folder_id": 789,
  "source_folder_id": 456
}
```

---

### 10. Folder Management

#### List Folders
* **URL:** `/folders`
* **Method:** `GET`

#### Create Folder
* **URL:** `/folders`
* **Method:** `POST`
* Request Body: `{"name": "New Folder"}`

#### Rename Folder
* **URL:** `/folders/{folder_id}`
* **Method:** `PATCH`
* Request Body: `{"name": "New Folder Name"}`

#### Delete Folder
* **URL:** `/folders/{folder_id}`
* **Method:** `DELETE`

---

### 11. Storage Stats & Analytics

#### Storage Stats
Retrieve total storage consumed, file counts, and breakdown by folders and MIME types.
* **URL:** `/storage/stats`
* **Method:** `GET`

#### Response (200 OK)
```json
{
  "total_storage_used_bytes": 10485760,
  "total_file_count": 12,
  "complete": true,
  "folders": [
    { "id": 456, "name": "Documents", "file_count": 5, "size_bytes": 5242880 }
  ],
  "mime_types": [
    { "mime_type": "application/pdf", "file_count": 5, "size_bytes": 5242880 }
  ]
}
```

#### Duplicate Files Finder
List groups of files with identical uploaded filenames and sizes. A file renamed in Telegram Drive is still matched by the name it was uploaded with. The response header `X-Inventory-Complete` reports whether all folders were fully inspected.
* **URL:** `/storage/duplicates`
* **Method:** `GET`

#### Empty Folders
List folders whose complete inventory proves that they contain no files. The response omits Saved Messages and any folder whose scan was incomplete. The response header `X-Inventory-Complete` reports whether every folder was fully inspected.
* **URL:** `/folders/empty`
* **Method:** `GET`

---

### 12. File Media & Thumbnails

#### Get File Thumbnail
Return the raw binary image data for a file's thumbnail.
* **URL:** `/files/{message_id}/thumbnail`
* **Method:** `GET`
* Query Param: `folder_id` (Optional)

#### Get Extended Media Info
Return video duration, resolution, audio title, or audio performer metadata.
* **URL:** `/files/{message_id}/media-info`
* **Method:** `GET`
* Query Param: `folder_id` (Optional)

---

### 13. Bulk Operations
Perform action operations (such as move, delete, or archive) across multiple files.

* **URL:** `/files/bulk`
* **Method:** `POST`
* **Auth Required:** Yes

#### Bulk Archive (Zip Download)
Download selected files as a zip archive. The selection is checked first (missing, unsupported, and encrypted files, and the 256 MiB archive limit), then each file is written to the archive as it downloads, so memory use does not grow with the selection. Files that share a name are stored as `name (2).ext`, `name (3).ext`, and so on.
```json
{
  "action": "archive",
  "file_ids": [123, 124, 125],
  "folder_id": 456
}
```

#### Bulk Delete
```json
{
  "action": "delete",
  "file_ids": [123, 124, 125],
  "folder_id": 456
}
```

#### Bulk Move
```json
{
  "action": "move",
  "file_ids": [123],
  "folder_id": 111,
  "payload": {
    "folder_id": 222
  }
}
```

---

## Security and limitations

- The API listens only on `127.0.0.1` and is intended for trusted software running on the same computer.
- Treat the API key as a password. Any local process or user with the key can use the enabled operations.
- Telegram Drive stores only a hash of the key and does not write the plaintext key to application logs.
- Encrypted files fail closed wherever plaintext would be exposed or the envelope's record could be lost: download, thumbnail, media-info, rename, move, bulk move, and bulk archive return `409`, or `503` when the encryption state cannot be determined. Delete, bulk delete, and copy are permitted because they neither decrypt the file nor separate it from its record.
- Every request acts for the account that was signed in when it started. If the account changes before the response is ready, the request returns `409 ACCOUNT_CHANGED` and no data from either account.
- Telegram service limits, channel permissions, proxy/VPN settings, bandwidth limits, and flood waits still apply.
- Folder Sync, WebDAV, and the REST API operate on the same Telegram channels. Allow one operation to finish before changing the same file through another integration.
- Disabling the server or regenerating its key revokes access. The server is unavailable when the desktop app is closed or signed out.

See the [Folder Sync guide](SYNC_GUIDE.md), [WebDAV guide](WEBDAV_GUIDE.md), and [Privacy Policy](PRIVACY.md) for related behavior.

---

## Error Responses

The API returns standardized JSON error formats on failure:

```json
{
  "error": {
    "code": "UNAUTHORIZED",
    "message": "Invalid API key"
  }
}
```

| Status | Codes | Meaning |
| :--- | :--- | :--- |
| `400` | `INVALID_QUERY`, `INVALID_BODY`, `INVALID_TIMESTAMP`, `INVALID_FOLDER_ID`, `INVALID_FILE_ID`, `INVALID_FILE_IDS`, `INVALID_ACTION`, `PEER_ERROR` | The request could not be understood |
| `401` | `UNAUTHORIZED`, `NO_KEY_CONFIGURED` | Missing or wrong API key |
| `404` | `NOT_FOUND`, `MESSAGE_NOT_FOUND`, `ARCHIVE_FILE_MISSING` | No such file or route |
| `409` | `ACCOUNT_CHANGED`, `ENCRYPTED_ROUTE_UNAVAILABLE`, `ENCRYPTED_UPDATE_UNAVAILABLE`, `ENCRYPTED_BULK_ACTION_UNAVAILABLE` | The account changed, or the operation is not available for an encrypted file |
| `413` | `FILE_TOO_LARGE`, `ARCHIVE_TOO_LARGE` | Over a size limit |
| `502` | `FETCH_ERROR`, `ARCHIVE_DOWNLOAD_FAILED`, `ARCHIVE_DOWNLOAD_INCOMPLETE`, `MOVE_COPY_INCOMPLETE` | Telegram did not return a complete result; nothing partial is reported as complete |
| `503` | `NOT_CONNECTED`, `ACCOUNT_UNAVAILABLE`, `ENCRYPTION_STATE_UNKNOWN` | The app is not connected or not signed in |

---

## Changes in 4.0

- `created_at` is RFC 3339 (`2026-06-05T10:00:00Z`). Earlier releases returned `2026-06-05 10:00:00 UTC`; both forms are accepted by `created_after` and `created_before`.
- List, search, statistics, duplicates, and empty folders cover each folder's whole history. Earlier releases read only the newest 100–200 messages per folder, so totals and reports were understated.
- `/files/search` now answers. Earlier releases returned `404` because the path was read as a file identifier.
- File objects carry `encrypted`; list and statistics responses carry `complete`.
- An unreadable Telegram listing returns `502` instead of a shorter list.
- A file identifier outside the 32-bit range returns `400` instead of addressing a different file.
- Every endpoint is bound to the signed-in account, and the API key is checked before any input is parsed.
