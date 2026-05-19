# Telegram Drive

**Telegram Drive** is an open-source, cross-platform desktop and mobile application that turns your Telegram account into an unlimited, secure cloud storage drive. Built with **Tauri (Rust/React)** and **Flutter (Dart)**.

<div align="center">

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20MacOS%20%7C%20Linux%20%7C%20iOS%20%7C%20Android-blue)]()
[![Downloads](https://img.shields.io/github/downloads/caamer20/telegram-drive/total)](https://github.com/caamer20/telegram-drive/releases)
[![oosmetrics](https://api.oosmetrics.com/api/v1/badge/achievement/ae8e5a6b-e815-4799-a408-4a59980cf9c8.svg)](https://oosmetrics.com/repo/caamer20/Telegram-Drive)
[![oosmetrics](https://api.oosmetrics.com/api/v1/badge/achievement/029fb97b-a54a-4566-a1eb-aa1a5039065d.svg)](https://oosmetrics.com/repo/caamer20/Telegram-Drive)
[![oosmetrics](https://api.oosmetrics.com/api/v1/badge/achievement/2aa6f3f9-fd8a-4523-bd73-6625ee6a948a.svg)](https://oosmetrics.com/repo/caamer20/Telegram-Drive)

</div>

![Auth Screen](screenshots/AuthScreen.png)

##  What is Telegram Drive?

Telegram Drive leverages the Telegram API to allow you to upload, organize, and manage files directly on Telegram's servers. It treats your "Saved Messages" and created Channels as folders, giving you a familiar file explorer interface for your Telegram cloud.

###  Key Features

*   **Unlimited Cloud Storage**: Utilizing Telegram's generous cloud infrastructure.
*   **Mobile Companion App (Flutter)**: Native iOS and Android app for browsing, downloading, and streaming files on the go.
*   **High Performance Grid**: Virtual scrolling handles folders with thousands of files instantly.
*   **Auto-Updates**: Seamless updates for Windows, macOS, and Linux.
*   **Media Streaming**: Stream video and audio files directly without downloading.
*   **PDF Viewer:** Built-in PDF support with infinite scrolling for seamless document reading.
*   **Drag & Drop**: Intuitive drag-and-drop upload and file management.
*   **Thumbnail Previews**: Inline thumbnails for images and media files.
*   **Thumbnail Cleanup on Folder Delete**: Thumbnails are automatically cleaned up when a folder is deleted.
*   **Folder Management**: Create "Folders" (private Telegram Channels) to organize content.
*   **Folder Upload with Automatic Zipping**: Upload entire folders as compressed zip files with a single click.
*   **Configurable REST API (OpenAPI)**: Secure local API with configurable port, API key authentication, and OpenAPI spec for seamless LLM and tool integration. Off by default.
*   **Privacy Focused**: API keys and data stay local. No third-party servers.
*   **Cross-Platform**: Native apps for macOS (Intel/ARM), Windows, Linux, iOS, and Android.

##  Screenshots

| Dashboard | File Preview |
|-----------|--------------|
| ![Dashboard](screenshots/DashboardWithFiles.png) | ![Preview](screenshots/ImagePreview.png) |

| Grid View | Authentication |
|-----------|----------------|
| ![Dark Mode](screenshots/DarkModeGrid.png) | ![Login](screenshots/LoginScreen.png) |

| Audio Playback | Video Playback |
|----------------|----------------|
| ![Audio Playback](screenshots/AudioPlayback.png) | ![Video Playback](screenshots/VideoPlayback.png) |

| Auth Code Screen | Upload Example |
|------------------|-------------|
| ![Auth Code Screen](screenshots/AuthCodeScreen.png) | ![Upload Example](screenshots/UploadExample.png) |

| Folder Creation | Folder List View |
|-----------------|------------------|
| ![Folder Creation](screenshots/FolderCreation.png) | ![Folder List View](screenshots/FolderListView.png) |

##  Tech Stack

*   **Frontend**: React, TypeScript, TailwindCSS, Framer Motion
*   **Backend**: Rust (Tauri), Grammers (Telegram Client)
*   **Mobile**: Flutter (Dart), go_router, audioplayers, video_player
*   **Build Tool**: Vite


##  Getting Started

### Prerequisites

*   **Node.js (v18+)**: [Download here](https://nodejs.org/)
*   **Rust (latest stable)**: Required to compile the Tauri backend. Install via [rustup](https://rustup.rs/):
    *   **macOS/Linux:** `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
    *   **Windows:** Download and run `rustup-init.exe` from [rustup.rs](https://rustup.rs/)
    *   *Verify installation:* run `rustc --version` and `cargo --version` in your terminal.
*   **OS-Specific Build Tools for Tauri**: 
    *   **macOS:** Xcode Command Line Tools (`xcode-select --install`).
    *   **Linux (Ubuntu/Debian):** `sudo apt update && sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`
    *   **Windows (CRITICAL):** You **must** install the [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/). During installation, select the **"Desktop development with C++"** workload. Without this, you will get a `linker 'link.exe' not found` error.
    *   **Windows (WebView2):** Windows 10/11 users usually have this pre-installed. If not, download the [WebView2 Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/#download-section).
    *   *Reference:* See the official [Tauri v2 Prerequisites Guide](https://v2.tauri.app/start/prerequisites/) for detailed instructions.
*   **Telegram API Credentials**: You need your own API ID and API Hash to communicate with Telegram's servers.
    1. Log into [my.telegram.org](https://my.telegram.org).
    2. Go to "API development tools" and create a new application to get your `api_id` and `api_hash`.

> [!NOTE]  
> **First-run Compile Time:** The initial build (`npm run tauri dev` or `npm run tauri build`) will download and compile over 300 Rust crates. This process can take **5 to 15 minutes** depending on your hardware. Subsequent builds will be much faster.

> [!TIP]
> **NPM Vulnerabilities:** You may see vulnerability warnings during `npm install`. These are usually related to build tools and dev dependencies. You can optionally run `npm audit fix`, but it is not strictly required to run the app.

### Installation

1.  **Clone the repository**
    ```bash
    git clone https://github.com/caamer20/Telegram-Drive.git
    cd Telegram-Drive
    ```

2.  **Install Dependencies**
    ```bash
    cd app
    npm install
    ```

3.  **Run in Development Mode**
    ```bash
    npm run tauri dev
    ```

4.  **Build/Compile**
    ```bash
    npm run tauri build
    ```

### Mobile App (Flutter)

A companion iOS/Android app connects to the desktop app's local REST API.

```bash
cd mobile
flutter pub get
flutter run
```

See [mobile/README.md](mobile/README.md) for full details.

##  Open Source & License

This project is **Free and Open Source Software**. You are free to use, modify, and distribute it.

Licensed under the **MIT License**.

---
*Disclaimer: This application is not affiliated with Telegram FZ-LLC. Use responsibly and in accordance with Telegram's Terms of Service.*

If you're looking for a version of this app that's optimized for VPNs check out this repo:
https://github.com/caamer20/Telegram-Drive-ForVPNs

## Changelog

### v1.4.2
- Folder Upload with Automatic Zipping

### v1.4.1
- Thumbnail Cleanup Fixes

### v1.4.0
- Settings Modal and Secure REST API

<div align="center">
  <!-- PayPal -->
  <div style="margin: 15px 0;">
    <a href="https://www.paypal.me/Caamer20">
      <img src="https://raw.githubusercontent.com/stefan-niedermann/paypal-donate-button/master/paypal-donate-button.png" alt="Donate with PayPal" width="200">
    </a>
    <div style="font-size: 14px; margin-top: 8px;">paypal.me/Caamer20</div>
  </div>

  <!-- Litecoin -->
  <div style="margin: 15px 0;">
    <a href="litecoin:ltc1q6wkr5ac4u0pxx4hx7xgwn0gsaku25ws0df73rp">
      <img src="https://img.shields.io/badge/Donate-LTC-345D9D?style=for-the-badge&logo=litecoin&logoColor=white" alt="Donate LTC">
    </a>
    <div style="font-family: monospace; font-size: 13px; margin-top: 8px; word-break: break-all;">
      ltc1q6wkr5ac4u0pxx4hx7xgwn0gsaku25ws0df73rp
    </div>
  </div>

  <!-- Bitcoin -->
  <div style="margin: 15px 0;">
    <a href="bitcoin:bc1q5pt7m2fk6w0dzsnf6vvd5k6nw5k44785286ujy">
      <img src="https://img.shields.io/badge/Donate-BTC-F7931A?style=for-the-badge&logo=bitcoin&logoColor=white" alt="Donate BTC">
    </a>
    <div style="font-family: monospace; font-size: 13px; margin-top: 8px; word-break: break-all;">
      bc1q5pt7m2fk6w0dzsnf6vvd5k6nw5k44785286ujy
    </div>
  </div>
</div>
