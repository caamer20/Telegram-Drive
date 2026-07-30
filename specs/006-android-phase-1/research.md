# Research Decisions: Android Phase 1

## Authorization

**Decision**: TDLib authorization update là nguồn sự thật; UI action được gate theo state và request pending. Bao phủ mọi state trong spec, state mới giữ dạng Unsupported an toàn.

**Rationale**: Hướng dẫn TDLib chính thức yêu cầu ứng dụng phản ứng với `updateAuthorizationState`; sequence có thể thay đổi theo account/server.

**Alternatives considered**: Hardcode phone → code → password bị loại vì bỏ email/other-device/registration và retry cùng state.

## API credential và session

**Decision**: API ID/hash được inject từ local Gradle property file ignored; generated BuildConfig chỉ chứa local debug value, template không có secret. TDLib database/files đặt trong app-private directories với encryption key rỗng cho POC theo TDLib contract, được bảo vệ bởi backup policy P0.

**Rationale**: Không commit credential; TDLib cần credential trong `setTdlibParameters` và tự quản session database.

**Alternatives considered**: Reuse credential Tauri, hardcode, nhập và persist trong UI đều bị loại.

## JSON concurrency

**Decision**: Một receive loop application-owned phân phối update và response bằng `@extra`; request guard gắn state/action/file identity.

**Rationale**: TDLib JSON receive là global asynchronous stream; nhiều receiver hoặc response không correlation tạo race và duplicate.

**Alternatives considered**: Mỗi repository tự receive hoặc polling request-specific bị loại.

## Saved Messages

**Decision**: Gọi `getMe`, dùng self user id làm private chat id, rồi `getChatHistory` với `from_message_id=0`, offset 0, limit 50, only_local=false; map một batch và deduplicate theo file id.

**Rationale**: TDLib official getting-started mô tả `getChatHistory` reverse chronological và paging; P1 chỉ cần batch gần nhất.

**Alternatives considered**: Full history scan, global search/index, Room cache bị loại khỏi scope.

## Download

**Decision**: `downloadFile` foreground với priority cố định, updateFile làm progress/complete, `cancelDownloadFile` cho cancel; chỉ preview `local.path` tồn tại.

**Rationale**: TDLib đã quản file identity/progress và local cache; không cần WorkManager/background service.

**Alternatives considered**: HTTP URL/local server/custom downloader bị loại.

## Image và video

**Decision**: Image decode dùng Compose-supported local URI loader nhỏ không thêm production dependency nếu platform decode đủ; video dùng Media3 1.10.1 stable, ExoPlayer local URI, một player mỗi preview và release khi dispose.

**Rationale**: Media3 stable hiện tại cung cấp play/pause/seek; official docs yêu cầu cùng application thread và explicit lifecycle ownership.

**Alternatives considered**: Alpha/RC Media3, legacy MediaPlayer, streaming hoặc local HTTP server bị loại.
