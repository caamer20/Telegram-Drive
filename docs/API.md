# Telegram Drive REST API Documentation

Base URL:

```text
http://localhost:8550/api/v1
```

## Authentication

All endpoints except `/health` require an API key.

Include the following header in every request:

```http
X-API-Key: YOUR_API_KEY
```

Example:

```bash
curl -H "X-API-Key: YOUR_API_KEY" \
http://localhost:8550/api/v1/files
```

---

# Health Check

Check API availability and version information.

### Request

```http
GET /health
```

### Example

```bash
curl http://localhost:8550/api/v1/health
```

### Response

```json
{
  "status": "ok",
  "version": "1.8.4"
}
```

---

# List Files

Retrieve files stored in Telegram Drive.

### Request

```http
GET /files
```

### Query Parameters

| Parameter      | Type    | Description             |
| -------------- | ------- | ----------------------- |
| page           | integer | Page number             |
| limit          | integer | Items per page          |
| folder_id      | integer | Filter by folder        |
| search         | string  | Search by filename      |
| offset_id      | integer | Message offset          |
| sort           | string  | name, size, created_at  |
| order          | string  | asc or desc             |
| mime_type      | string  | Filter by MIME type     |
| created_after  | string  | Filter by creation date |
| created_before | string  | Filter by creation date |
| size_min       | integer | Minimum file size       |
| size_max       | integer | Maximum file size       |
| fields         | string  | Comma-separated fields  |

### Example

```http
GET /files?page=1&limit=20
```

```bash
curl -H "X-API-Key: YOUR_API_KEY" \
"http://localhost:8550/api/v1/files?page=1&limit=20"
```

### Response

```json
{
  "data": [],
  "files": [],
  "page": 1,
  "limit": 20,
  "total": 0
}
```

---

# Get File Details

Retrieve metadata for a specific file.

### Request

```http
GET /files/{message_id}
```

### Example

```bash
curl -H "X-API-Key: YOUR_API_KEY" \
http://localhost:8550/api/v1/files/123
```

### Response

```json
{
  "id": 123,
  "folder_id": 456,
  "name": "document.pdf",
  "size": 102400,
  "mime_type": "application/pdf",
  "created_at": "2026-06-05T10:00:00Z"
}
```

---

# Download File

Download a file directly from Telegram Drive.

### Request

```http
GET /files/{message_id}/download
```

### Example

```bash
curl \
-H "X-API-Key: YOUR_API_KEY" \
-o file.bin \
http://localhost:8550/api/v1/files/123/download
```

### Notes

* Supports HTTP Range Requests.
* Supports resumable downloads.
* Returns file content directly.

---

# Search Files

Search files by filename.

### Request

```http
GET /files/search
```

### Query Parameters

| Parameter | Type    | Description            |
| --------- | ------- | ---------------------- |
| q         | string  | Search query           |
| folder_id | integer | Optional folder filter |
| recursive | boolean | Recursive search       |

### Example

```bash
curl -H "X-API-Key: YOUR_API_KEY" \
"http://localhost:8550/api/v1/files/search?q=python"
```

### Response

```json
[
  {
    "id": 123,
    "name": "python-guide.pdf",
    "size": 204800
  }
]
```

---

# Bulk Operations

Perform actions on multiple files.

### Request

```http
POST /files/bulk
```

## Delete Files

### Request Body

```json
{
  "action": "delete",
  "file_ids": [123, 124, 125]
}
```

### Example

```bash
curl -X POST \
-H "Content-Type: application/json" \
-H "X-API-Key: YOUR_API_KEY" \
-d "{\"action\":\"delete\",\"file_ids\":[123,124]}" \
http://localhost:8550/api/v1/files/bulk
```

---

## Move Files

### Request Body

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

### Example

```bash
curl -X POST \
-H "Content-Type: application/json" \
-H "X-API-Key: YOUR_API_KEY" \
-d "{\"action\":\"move\",\"file_ids\":[123],\"folder_id\":111,\"payload\":{\"folder_id\":222}}" \
http://localhost:8550/api/v1/files/bulk
```

### Response

```json
{
  "success": true,
  "count": 1
}
```

---

# Error Responses

### Unauthorized

```json
{
  "error": {
    "code": "UNAUTHORIZED",
    "message": "Invalid API key"
  }
}
```

### Missing API Key

```json
{
  "error": {
    "code": "UNAUTHORIZED",
    "message": "Missing X-API-Key header"
  }
}
```

### File Not Found

```json
{
  "error": {
    "code": "NOT_FOUND",
    "message": "File not found"
  }
}
```

### Invalid Request

```json
{
  "error": {
    "code": "INVALID_ACTION",
    "message": "Unsupported bulk action"
  }
}
```
