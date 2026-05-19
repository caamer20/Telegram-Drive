# Dashboard Screen - Learnings

## File Created
- `mobile/lib/screens/dashboard_screen.dart` — Main dashboard with:
  - Adaptive layout (phone drawer vs tablet sidebar, breakpoint 600px)
  - Sidebar with Saved Messages, folder list, search, connection status, bottom actions
  - TopBar with breadcrumb, file search, view toggle, theme toggle, settings
  - File area with grid/list views, empty state, error state, shimmer loading
  - FileCard (4:3 aspect ratio, file type icons, quick actions overlay, press animation)
  - FileListItem (icon, name, size, date, popup menu)
  - Upload FAB
  - Pull-to-refresh
  - Create folder dialog

## Key Decisions
- Semantic colors (success green, error red) defined locally since `TelegramColors` in `app_theme.dart` doesn't expose them
- File type icon/color helpers defined locally to avoid modifying `app_theme.dart`
- Used `Icons.storage` instead of unavailable `Icons.hard_drive`
- Used `Matrix4.diagonal3Values` for card press animation (avoiding deprecated `scale`)
- Used `withValues(alpha:)` for opacity (modern Flutter API)

## Providers Used
- FoldersProvider: folders, activeFolderId, setActiveFolder(), loadFolders()
- FilesProvider: files, isLoading, error, search(), displayedFiles, loadFiles(), clearSearch()
- ConnectionProvider: disconnect()

## Navigation
- File tap (media/images) → pushNamed('/media-player', arguments: {'file': file})
- Logout → pushNamedAndRemoveUntil('/', ...)
- Settings → snackbar (placeholder)
