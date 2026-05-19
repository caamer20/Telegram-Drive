# Connect Screen Implementation - Learnings

## Design System Used
- `TelegramColors` from `lib/theme/app_theme.dart` provides dark/light color tokens
- `ConnectionProvider` from `lib/providers/connection_provider.dart` has `connect()`, `isLoading`, `isConnected`, `error`, `clearError()`, `disconnect()`
- `TelegramDriveApi` from `lib/services/api_service.dart` is the REST client

## AuthWizard Visual Match (React → Flutter)
- **Background**: `RadialGradient(center: topLeft, radius: 1.2, colors: [#3CA5FF, #007AFF])` matches `auth-gradient`
- **Glass card**: `BackdropFilter(blur 24)` + `Container(color: surfaceDark 85%, border: borderDark)` matches `auth-glass`
- **Inputs**: `TextField` with `_kInputBg` fill + border `borderDark` + prefix icons matches `glass-input`
- **Button**: Amber/gold gradient (`#FFAE00` → `#FF9500`) as specified (not blue like React's step button)
- **Error**: `_ErrorCard` with animated fade+slide, red dot, red-400 text matches React error display
- **Flood wait**: `_FloodWaitDisplay` with hourglass, "Too Many Requests" heading, MM:SS countdown matches React flood UI
- **Entrance animation**: `FadeTransition` + `ScaleTransition` with `easeOutCubic` matches framer-motion `{ opacity: 0, scale: 0.95 } → { opacity: 1, scale: 1 }`

## Gotchas
- `ImageFilter.blur()` is NOT a const constructor — `const` must be removed from the invocation
- `BackdropFilter` works correctly when placed inside the gradient's `Stack` child hierarchy
- `Consumer<ConnectionProvider>` properly reacts to `notifyListeners()` calls during connect lifecycle
