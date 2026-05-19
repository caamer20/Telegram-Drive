# Learnings

## Architecture
- Mobile app uses Provider for state management
- API client bypasses proxy via `findProxy = 'DIRECT'`
- Native video player (VideoPlayerController) does NOT bypass proxy
- All screens in separate files under screens/

## Issues discovered
1. Back nav: _TopBar breadcrumb is static text only
2. Media: Stream server unreachable due to system proxy blocking native player
3. Search: folderId not passed to search, errors silently swallowed

## Fix: Media Player Proxy Bypass
- Added `downloadFileBytes()` to `api_service.dart` using the proxy-bypassing `_client` (uses `findProxy = 'DIRECT'`)
- Modified `_initPlayer()` to download media to temp dir first via the API client, then use `VideoPlayerController.file()` instead of `VideoPlayerController.networkUrl()`
- Added fallback: if download fails, falls back to the streaming URL approach
- Temp files tracked in `Set<String> _tempFiles` and cleaned up in `dispose()` via `File(path).delete().ignore()`
- File names sanitized with `RegExp(r'[^\w\.\-]')` to avoid path issues
- Temp files prefixed with `td_` to avoid collisions and easy identification
- `dart:typed_data` re-exported by `package:flutter/foundation.dart` — no explicit import needed
- `dart:io` import needed for `File` and `Directory` in media_player_screen

## Fix: Back Navigation
- Added `showBackButton` (bool) and `onBackToRoot` (VoidCallback) params to `_TopBar`
- When `showBackButton == true`: hamburger icon → `Icons.arrow_back`
- "Drive" text in breadcrumb: tappable via `TapGestureRecognizer` → calls `onBackToRoot`
- `_buildMainContent` passes `showBackButton: folders.activeFolderId != null`
- Import added: `package:flutter/gestures.dart`

## Fix: Search
- `FilesProvider.search()`: added optional `int? folderId` parameter → passed to API
- Added `_searchError` field + `searchError` getter (separate from file load errors)
- `_onSearchChanged`: reads `FoldersProvider.activeFolderId`, passes to `files.search()`
- After search: checks `files.searchError != null`, shows SnackBar on error
- Errors are now surfaced to the user instead of silently swallowed
