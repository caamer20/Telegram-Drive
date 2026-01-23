# Component Tree

## Layouts
1. **RootLayout**: Handles global theme (Dark Navy) and Toast notifications.
2. **AuthLayout**: Centered card layout for the login wizard.
3. **DashboardLayout**: Split view (Sidebar + Main Area).

## Components

### Authentication
- `AuthWizard`: Manages the state machine for the login flow.
- `PhoneInput`: Input with country code selector.
- `CodeInput`: OTP style input.
- `PasswordInput`: Secure input for 2FA.

### Navigation
- `Sidebar`: Lists "Drives" (Chats) and "Favorites".
- `SidebarItem`: Individual drive/folder link.
- `Breadcrumbs`: Path navigation (e.g., Saved Messages > Work > Project A).

### File Explorer
- `FileGrid`: CSS Grid container for items.
- `FolderCard`: Represents a directory.
    - Props: `name`, `itemCount`.
    - Interaction: Double click to enter.
- `FileCard`: Represents a file.
    - Props: `name`, `size`, `type`, `previewUrl` (if applicable).
    - Design: "The Inverse Telegram" style - Deep Navy card, Amber glow on hover.
- `ContextMenu`: Right-click menu for actions (Download, Delete, Rename).

### Functionality
- `UploadZone`: Full-screen or drop-target overlay for drag-and-drop.
- `ProgressBar`: Thin Amber line at the top of the window for uploads/downloads.
- `StatusIndicator`: Small badge for "Connected", "Syncing", "Offline".

### Common (Design System)
- `Button`: Variants (Primary Amber, Ghost, Danger).
- `Input`: Styled text input.
- `Modal`: For creating folders or confirming deletions.
- `Icon`: Wrapper for `lucide-react` icons.
