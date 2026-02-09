# Telegram Drive 

**Telegram Drive** is an open-source, cross-platform desktop application that turns your Telegram account into an unlimited, secure cloud storage drive. built with **Tauri**, **Rust**, and **React**.

![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20MacOS%20%7C%20Linux-blue)


![Login](screenshots/LoginScreen.png)

##  What is Telegram Drive?

Telegram Drive leverages the Telegram API to allow you to upload, organize, and manage files directly on Telegram's servers. It treats your "Saved Messages" and created Channels as folders, giving you a familiar file explorer interface for your Telegram cloud.

### ✨ Key Features

*   **Unlimited Cloud Storage**: Utilizing Telegram's generous cloud infrastructure.
*   **High Performance Grid**: Virtual scrolling handles folders with thousands of files instantly.
*   **Auto-Updates**: Seamless updates for Windows, macOS, and Linux.
*   **Media Streaming**: Stream video and audio files directly without downloading.
*   **Drag & Drop**: Intuitive drag-and-drop upload and file management.
*   **Thumbnail Previews**: Inline thumbnails for images and media files.
*   **Folder Management**: Create "Folders" (private Telegram Channels) to organize content.
*   **Privacy Focused**: API keys and data stay local. No third-party servers.
*   **Cross-Platform**: Native apps for macOS (Intel/ARM), Windows, and Linux.

## 📸 Screenshots

| Dashboard | File Preview |
|-----------|--------------|
| ![Dashboard](screenshots/DashboardWithFiles.png) | ![Preview](screenshots/ImagePreview.png) |

| Dark Mode | Authentication |
|-----------|----------------|
| ![Dark Mode](screenshots/DarkModeGrid.png) | ![Login](screenshots/LoginScreen.png) |

| Audio Playback | Video Playback |
|----------------|----------------|
| ![Audio Playback](screenshots/AudioPlayback.png) | ![Video Playback](screenshots/VideoPlayback.png) |

| Auth Code Screen | Auth Screen |
|------------------|-------------|
| ![Auth Code Screen](screenshots/AuthCodeScreen.png) | ![Auth Screen](screenshots/AuthScreen.png) |

| Folder Creation | Folder List View |
|-----------------|------------------|
| ![Folder Creation](screenshots/FolderCreation.png) | ![Folder List View](screenshots/FolderListView.png) |

| Upload Example |
|----------------------|
|  ![Upload Example](screenshots/UploadExample.png) |

##  Tech Stack

*   **Frontend**: React, TypeScript, TailwindCSS, Framer Motion
*   **Backend**: Rust (Tauri), Grammers (Telegram Client)
*   **Build Tool**: Vite


##  Getting Started

### Prerequisites
*   Node.js (v18+)
*   Rust (latest stable)
*   A Telegram Account
*   API ID and Hash from [my.telegram.org](https://my.telegram.org)

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

##  Open Source & License

This project is **Free and Open Source Software**. You are free to use, modify, and distribute it.

Licensed under the **MIT License**.

---
*Disclaimer: This application is not affiliated with Telegram FZ-LLC. Use responsibly and in accordance with Telegram's Terms of Service.*


<a href="https://www.paypal.me/Caamer20">
  <img src="https://raw.githubusercontent.com/stefan-niedermann/paypal-donate-button/master/paypal-donate-button.png" alt="Donate with PayPal" width="200" />
</a>
