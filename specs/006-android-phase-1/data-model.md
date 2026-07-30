# Data Model: Android Phase 1

## AuthorizationSession

- `state`: Initializing, MissingConfiguration, WaitPhone, WaitCode, WaitPassword, WaitEmailAddress, WaitEmailCode, WaitOtherDevice, Ready, LoggingOut, Closing, Closed, Unsupported, Failed.
- `pendingAction`: optional action identity; chỉ một action tại một thời điểm.
- `prompt`: non-secret metadata như password hint hoặc other-device link đã sanitize.
- `error`: sanitized, không chứa credential.

Transition chỉ đến từ TDLib update hoặc terminal request response; input phone/code/password không trở thành retained state.

## SavedMediaItem

- `messageId`, `fileId`, `chatId`: stable identity.
- `name`, `kind`, optional dimensions/duration/size.
- `download`: NotDownloaded, Downloading(bytes/total), Complete(verified path), Failed, Canceled.

Uniqueness theo `fileId`; thứ tự theo message recency. Local path chỉ hợp lệ khi file tồn tại tại thời điểm route.

## LibraryState

- Loading, Content(items), Empty, Error(sanitized).
- Không giữ history ngoài batch hiện tại; refresh thay thế atomically.

## DownloadOperation

- `fileId`, `generation`, progress bytes, terminal result.
- Start idempotent theo fileId; update cũ không được ghi đè generation mới; cancel là terminal cho generation hiện tại.

## PreviewDestination

- Image(itemId, verifiedPath) hoặc Video(itemId, verifiedPath).
- Không chứa remote URL; invalid/missing path không tạo destination.

## PlayerBoundary

- Idle, Preparing, Ready, Playing, Paused, Error, Released.
- Released là terminal cho player instance; navigation tạo instance mới khi cần.
