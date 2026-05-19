# Media Player - Learnings

## Design System
- `TelegramColors` is defined in `lib/theme/app_theme.dart` with comprehensive dark/light tokens.
- Primary amber: `TelegramColors.primaryDark` (0xFFFFAE00)
- Dark surface: `TelegramColors.bgDark` (0xFF0E1621)
- Text: `TelegramColors.textDark` (white), `TelegramColors.subtextDark` (0xFF8E9FB3)

## Code Patterns
- `RepeatMode` enum conflicts with Flutter's own `RepeatMode` from `material.dart` → must import with `as provider` prefix.
- Keyboard event types (`KeyDownEvent`, `KeyRepeatEvent`, `LogicalKeyboardKey`) live in `package:flutter/services.dart`.
- `video_player` package already in pubspec.yaml — `VideoPlayerController.networkUrl()` supports `httpHeaders`.
- Stream URL format: `{baseUrl}/api/v1/files/{id}/download?folder_id={folderId}&api_key={apiKey}`

## Issues Encountered
- `replaceAll` for `RepeatMode` → `provider.RepeatMode` also matched substring inside `_cycleRepeatMode`, mangling it. Fixed with targeted edits.
