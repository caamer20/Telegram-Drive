# Telegram Drive — Mobile (Flutter)

**Telegram Drive Mobile** is a companion iOS and Android app for [Telegram Drive](https://github.com/caamer20/Telegram-Drive), the open-source desktop application that turns your Telegram account into unlimited cloud storage.

Built with **Flutter 3** and **Dart**, the mobile app communicates with the desktop Tauri app's local REST API, giving you on-the-go access to your Telegram Drive files.

---

## Features

### Browsing & Navigation
- **File Explorer UI** — browse folders and files stored in your Telegram Drive, matching the desktop app's visual design
- **Breadcrumb Navigation** — tap any ancestor folder in the breadcrumb bar to jump back instantly
- **Pull-to-Refresh** — swipe down on any folder to refresh its contents from the REST API
- **Adaptive Layout** — responsive design works across phones and tablets
- **Dark & Light Themes** — matches the Telegram Drive brand palette (amber/gold accent, dark navy background)

### File Operations
- **Download Files** — download any file to your device's local storage with a progress indicator
- **Share Files** — share files directly via the OS share sheet (iOS/Android)
- **Streaming Playback** — play audio and video files inline using platform-native media controls
- **Image Preview** — view images with a full-screen, zoomable preview dialog
- **File Metadata** — see file size, upload date, and MIME type for every file

### Authentication
- **REST API Key Login** — enter the API key and port configured in the desktop app's Settings → REST API
- **Connection Validation** — the app validates the API key and host before loading your drive
- **Persistent Sessions** — credentials are cached securely via `shared_preferences` so you don't need to log in every time

### API Settings & Remote Configuration
- **REST API Settings Screen** — configure host, port, and API key directly from the app
- **Proxy Toggle** — enable/disable the desktop app's proxy settings remotely
- **Stream Proxy URL** — configure the stream proxy URL for media playback
- **One-time Upload Token Rotation** — trigger a new upload token from your phone

---

## Screenshots

| Login Screen | File Browser | Drawer Menu |
|:---:|:---:|:---:|
| ![Login](screenshots/login.png) | ![Browser](screenshots/browser.png) | ![Drawer](screenshots/drawer.png) |

*Screenshots to be added. See the `screenshots/` directory.*

---

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Framework | **Flutter 3** with Dart 3 |
| HTTP Client | `package:http` |
| State Management | `StatefulWidget` + `FutureBuilder` (lightweight) |
| Navigation | `go_router` (declarative routing) |
| Persistence | `shared_preferences` |
| Media Playback | `audioplayers`, `video_player` |
| File Handling | `path_provider`, `open_file`, `share_plus` |
| Image Viewing | InteractiveViewer (built-in) |

---

## Getting Started

### Prerequisites

- **Flutter SDK** (3.x): [Install Flutter](https://docs.flutter.dev/get-started/install)
- **Telegram Drive Desktop** running with REST API enabled:
  1. Open Telegram Drive desktop app
  2. Go to **Settings → REST API**
  3. Toggle **"Enable REST API"** to ON
  4. Note the **Port** and **API Key**

### Running the App

```bash
# Navigate to the mobile directory
cd mobile

# Get dependencies
flutter pub get

# Run on connected device or simulator
flutter run
```

> **Note for iOS:** You may need to add `NSAppTransportSecurity` exceptions in `ios/Runner/Info.plist` if connecting to a non-HTTPS local server. See the [Apple documentation](https://developer.apple.com/documentation/bundleresources/information_property_list/nsapptransportsecurity) for details.

### Building for Release

```bash
# iOS (requires Xcode)
flutter build ios

# Android
flutter build apk
# or
flutter build appbundle
```

---

## Architecture

### Project Structure

```
mobile/
├── lib/
│   ├── api/
│   │   └── telegram_drive_api.dart      # REST API client (all endpoints)
│   ├── models/
│   │   ├── drive_item.dart              # File/folder data model
│   │   ├── folder_item.dart             # Folder-specific model
│   │   └── api_settings.dart            # API connection settings model
│   ├── screens/
│   │   ├── login_screen.dart            # API key login screen
│   │   ├── home_screen.dart             # Main file browser screen
│   │   ├── api_settings_screen.dart     # REST API configuration screen
│   │   └── media_preview_screen.dart    # Video/audio playback screen
│   ├── widgets/
│   │   ├── file_list_item.dart          # Individual file row widget
│   │   ├── file_preview_dialog.dart     # Image/file preview dialog
│   │   ├── breadcrumb_bar.dart          # Navigation breadcrumb bar
│   │   ├── settings_section.dart        # Reusable settings UI section
│   │   └── settings_tile.dart           # Settings list tile widget
│   ├── theme/
│   │   └── app_theme.dart               # Telegram Drive design system
│   └── main.dart                        # App entry point & routing
├── test/                                # Unit & widget tests
├── ios/                                 # iOS platform project
├── android/                             # Android platform project
├── pubspec.yaml                         # Dart dependencies
└── README.md                            # This file
```

### REST API Client (`telegram_drive_api.dart`)

The API client communicates with the desktop app's local HTTP server. All endpoints are documented in the desktop app's OpenAPI spec (`app/openapi.json`).

**Key endpoints used:**
- `GET /api/folders` — list root folders
- `GET /api/folders/{id}` — list folder contents (files and subfolders)
- `GET /api/files/{id}/download` — download a file
- `GET /api/files/{id}/stream` — stream media
- `PUT /api/settings` — update remote settings (proxy, tokens)
- `GET /api/settings` — read current settings

### Navigation (`go_router`)

```dart
/                    → Login screen
/home                → File browser (root)
/home/:folderId      → File browser (inside folder)
/api-settings        → REST API configuration
/media-preview/:id   → Video/audio playback
```

### Design System

The theme matches the desktop React app's design language:

| Token | Dark Theme | Light Theme |
|-------|-----------|-------------|
| Background | `#0E1621` | `#F0F2F5` |
| Surface | `#17212B` | `#FFFFFF` |
| Primary | `#FFAE00` (amber) | `#E69500` |
| Secondary | `#2481CC` (blue) | `#2481CC` |
| Text | `#FFFFFF` | `#1A1A1A` |
| Subtext | `#8E9FB3` | `#65676B` |

---

## Project Status

The mobile app is in active development. Current features are functional but improvements are ongoing:

- [x] File browsing & navigation
- [x] Download & share files
- [x] Image preview
- [x] Audio/video streaming
- [x] API settings management
- [x] Dark/light theme
- [ ] Batch file operations
- [ ] Upload from mobile
- [ ] Push notifications
- [ ] Biometric auth for API key storage

---

## License

MIT License — same as the desktop application.
