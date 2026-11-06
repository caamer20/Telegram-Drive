<div align="center">

<img src="Docs/assets/logo.svg" alt="Telegram Drive logo" width="92">

# Telegram Drive

### A local-first file workspace powered by your own Telegram account

Organize, transfer, preview, stream, sync, and share the files you keep in Saved Messages and Telegram channels—on Windows, macOS, Linux, Android, and Google TV.

[![Latest release](https://img.shields.io/github/v/release/caamer20/Telegram-Drive?display_name=tag&sort=semver)](https://github.com/caamer20/Telegram-Drive/releases/latest)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux%20%7C%20Android-blue)](https://github.com/caamer20/Telegram-Drive/releases)
[![GitHub Downloads (all assets, all releases)](https://img.shields.io/github/downloads/caamer20/Telegram-Drive/total?style=flat)](https://github.com/caamer20/Telegram-Drive/releases)
[![Release](https://github.com/caamer20/Telegram-Drive/actions/workflows/release.yml/badge.svg)](https://github.com/caamer20/Telegram-Drive/actions/workflows/release.yml)
[![oosmetrics](https://api.oosmetrics.com/api/v1/badge/achievement/ae8e5a6b-e815-4799-a408-4a59980cf9c8.svg)](https://oosmetrics.com/repo/caamer20/Telegram-Drive)
[![oosmetrics](https://api.oosmetrics.com/api/v1/badge/achievement/029fb97b-a54a-4566-a1eb-aa1a5039065d.svg)](https://oosmetrics.com/repo/caamer20/Telegram-Drive)
[![oosmetrics](https://api.oosmetrics.com/api/v1/badge/achievement/2aa6f3f9-fd8a-4523-bd73-6625ee6a948a.svg)](https://oosmetrics.com/repo/caamer20/Telegram-Drive)

[Download for desktop](https://github.com/caamer20/Telegram-Drive/releases/latest) · [Download Android 4.1.0](https://github.com/caamer20/Telegram-Drive/releases/tag/Androidv4.1.0) · [Product website](https://telegram-drive.com/) · [Changelog](CHANGELOG.md)

</div>

![Telegram Drive desktop dashboard](Docs/assets/dashboard.webp)

Telegram Drive turns Telegram's familiar file storage into a focused, desktop-style workspace. The app connects directly to Telegram with credentials you provide; it does not run a separate file-storage or relay service. Every application feature is available without payment.

> [!IMPORTANT]
> Telegram Drive is an independent project and is not affiliated with Telegram FZ-LLC. Files remain subject to Telegram's account, service, and per-file limits. Telegram Drive should not be treated as literally unlimited storage or as the only backup of important data.

## Contents

- [Download and install](#download-and-install)
- [First-time setup](#first-time-setup)
- [Features](#features)
- [Screenshots](#screenshots)
- [Folder Sync](#folder-sync-desktop)
- [Encryption](#optional-client-side-encryption-beta-format-frozen)
- [Privacy and security](#privacy-and-security)
- [Android and Google TV](#android-and-google-tv-signed-apk)
- [Documentation](#documentation)
- [Build from source](#build-from-source)

## At a glance

| | |
| --- | --- |
| **Platforms** | Windows, macOS Intel, macOS Apple Silicon, Linux, Android, and Google TV |
| **Storage model** | Saved Messages is the home location; Telegram channels can be presented as folders |
| **Transfers** | Files, folders, drag and drop, remote URLs, durable queues, background operation, and retry controls |
| **Media** | Image preview, PDF viewing, audio playback, video streaming, thumbnails, and archive tools |
| **Desktop integrations** | Folder Sync, local REST API, WebDAV, local share links, tray controls, and native notifications |
| **Privacy** | Local configuration and caches, opt-in crash reporting, loopback-only local servers, and no separate file relay |
| **Languages** | 24 selectable locales plus automatic system-language selection |
| **Price** | Every feature is free; an optional one-time $5 supporter license removes sponsor placements for life on up to three supported devices total |

## Download and install

Desktop **4.1.3** and Android / Google TV **4.1.0** use separate version lines. The desktop release does not update or include Android or Google TV. Downloads are listed below.

| Platform | Download | Notes |
| --- | --- | --- |
| **Windows x64** | [Installer](https://github.com/caamer20/Telegram-Drive/releases/download/v4.1.3/Telegram.Drive_4.1.3_x64-setup.exe) | Includes the required Microsoft Visual C++ runtime setup |
| **macOS — Apple Silicon** | [DMG](https://github.com/caamer20/Telegram-Drive/releases/download/v4.1.3/Telegram.Drive_4.1.3_aarch64.dmg) | For M-series Macs |
| **macOS — Intel** | [DMG](https://github.com/caamer20/Telegram-Drive/releases/download/v4.1.3/Telegram.Drive_4.1.3_x64.dmg) | For Intel-based Macs |
| **Linux x64** | [AppImage](https://github.com/caamer20/Telegram-Drive/releases/download/v4.1.3/Telegram.Drive_4.1.3_amd64.AppImage) · [Debian](https://github.com/caamer20/Telegram-Drive/releases/download/v4.1.3/Telegram.Drive_4.1.3_amd64.deb) · [RPM](https://github.com/caamer20/Telegram-Drive/releases/download/v4.1.3/Telegram.Drive-4.1.3-1.x86_64.rpm) · [Arch](https://github.com/caamer20/Telegram-Drive/releases/download/v4.1.3/telegram-drive-bin-4.1.3-1-x86_64.pkg.tar.zst) | Choose your distribution; Arch updates remain managed by pacman |
| **Android phones and tablets** | [Signed universal APK](https://github.com/caamer20/Telegram-Drive/releases/download/Androidv4.1.0/Telegram-Drive-v4.1.0-android-universal.apk) | Android 7.0 or newer; supports ARM64, ARMv7, x86, and x86_64 |
| **Android TV / Google TV** | [Signed TV-compatible APK](https://github.com/caamer20/Telegram-Drive/releases/download/Androidv4.1.0/Telegram-Drive-v4.1.0-android-universal.apk) | The same universal package includes the TV launcher and remote navigation |

The new desktop workspace adds collections, saved searches, a media timeline, Continue Watching, offline trip packs, transfer activity, storage controls, cleanup, and sync previews. Android 4.1.0 includes the latest native media and offline improvements and fixes the folder sidebar reported in [issue #212](https://github.com/caamer20/Telegram-Drive/issues/212). Install the signed APK over your existing app to preserve its data and activation.

Desktop release assets also include updater/signature sidecars where applicable. Download installers only from this repository's GitHub Releases page. See the [Linux packaging and rendering guide](Docs/LINUX_PACKAGING.md) for Arch installation, updater ownership, diagnostics, and AppImage safe mode.

## First-time setup

Telegram requires third-party clients to use an application API ID and API hash. Telegram Drive does not provide shared credentials.

1. Sign in at [my.telegram.org](https://my.telegram.org) and open **API development tools**.
2. Create a Telegram application and copy its `api_id` and `api_hash`.
3. Install and open Telegram Drive, then enter those credentials locally.
4. Authenticate with your phone number and login code, or use QR login. If Telegram cloud-password protection is enabled, complete that step as well.
5. Open **Saved Messages** or select/create Telegram channels to use as folders.

> [!WARNING]
> Treat the API hash, Telegram login codes, session data, encryption credentials, REST keys, and WebDAV links as secrets. Do not paste them into issues, logs, screenshots, or support messages.

## How it works

Telegram Drive uses Telegram's MTProto API for authentication and file operations:

- **Saved Messages** appears as the main personal storage location.
- Telegram **channels** created or selected by the app can appear as folders.
- Transfers move directly between the application and Telegram.
- Desktop metadata, settings, queues, sync state, thumbnails, and preview caches stay on the local device.
- Standard uploads are stored in Telegram in their normal form. Optional TDENC2 uploads are encrypted locally before transfer.

The application caps a Telegram object at exactly **2,000,000,000 bytes**. Encrypted files need additional envelope space, so their maximum original plaintext size is slightly lower.

## Features

### File management and organization

- Grid and list views with virtualized rendering for large folders.
- Adjustable file-card sizing from **50% to 200%**.
- Filename search, persistent sorting, multi-select, range selection, and bulk actions.
- Upload, download, rename, copy, move, and delete workflows where Telegram permits them.
- Drag-and-drop uploads and internal drag-and-drop organization.
- Folder creation, rename, deletion, visibility controls, and custom local folder groups.
- Whole-folder uploads with optional ZIP creation before transfer.
- Remote URL uploads with progress, cancellation, retries, redirects, bounded size checks, and resumable ranges when supported by the source server.
- Duplicate-file, empty-folder, media-metadata, and storage-insight tools through the local REST API.

### Reliable transfers and desktop operation

- Durable desktop upload and download queues that recover across window and application restarts.
- Independent upload/download concurrency controls, bandwidth controls, pause, resume, retry, cancel, cooldown, network-waiting, and vault-unlock states.
- Background operation from the system tray while transfers are active.
- Native transfer notifications for completion, failure, pause, and attention-required states, with per-category controls.
- Suspend, resume, network-recovery, and Telegram flood-wait handling.
- Verified downloads written through private temporary files and atomically published to their destination.
- A versioned local desktop file inventory so previously scanned folders appear while Telegram reconciliation continues.

### Viewing, playback, and archives

- Lazy-loaded thumbnails and bounded memory/disk preview caches.
- Desktop HEIC/HEIF still images use bounded JPEG display renditions in previews, gallery/slideshow and offline viewing. macOS tries `/usr/bin/sips` first; other desktops need user-installed FFmpeg 8.1 or newer. External opening and downloads keep the original. See [HEIC preview limits](Docs/HEIC_PREVIEW.md).
- A desktop image viewer with zoom, pan, fit-to-window, actual-size, wheel, double-click, and keyboard navigation controls.
- Built-in PDF viewer and audio player.
- Video playback with seeking, HLS/fMP4 streaming, remuxing/transcoding fallbacks, and cache recovery for supported plaintext media.
- Desktop MP4/M4V/MOV uploads in Media mode attach a locally generated JPEG preview and video dimensions/duration. Windows uses its native media decoder first; macOS/Linux need FFmpeg. If decoding is unavailable, choose File mode. Original video bytes are preserved; immediate streaming requires early MP4 metadata.
- Native Android playback with system media controls, resume, subtitle/audio-track selection, playback speed, and Picture-in-Picture where supported.
- ZIP, RAR, and 7z archive browsing and extraction, including extraction back into Telegram Drive where the platform supports it.
- Offline preview-cache controls for recently viewed standard files.
- Recorded plain previews, thumbnails and ready offline copies from 3.9.x retain external opening after an update. The first open verifies and registers each private cache file; arbitrary files and edited copies remain refused.

Some media depends on operating-system codec support or FFmpeg. Supported protected audio, video, and PDF content can use authenticated local byte-range streaming while the vault is unlocked, without publishing a persistent plaintext copy. See [encrypted-file limitations](#current-encrypted-file-limitations).

### Sharing and local integrations

- Password-protected local download links with optional expiration, listing, and revocation.
- Native Telegram message links for files in public channels.
- An opt-in desktop REST API for programmatic file, folder, bulk, storage, thumbnail, and media operations.
- Opt-in desktop WebDAV access for mounting Telegram Drive in compatible file managers.
- Optional, passphrase-encrypted settings backup through the user's own Telegram Saved Messages. Credentials, API/WebDAV keys, supporter activation, proxy details, and file data are excluded.

> [!NOTE]
> REST, WebDAV, and Telegram Drive share links are served by the running desktop application. They are not hosted public internet links. Keep the app open, do not forward its local ports, and protect every generated password, key, and capability URL.

### Personalization, language, and network controls

- System, light, dark, and default appearance preferences.
- Built-in theme presets and a custom-theme editor.
- Reduced-motion and interface-performance preferences.
- 24 selectable production locales with locale-aware dates, numbers, file sizes, and transfer rates.
- Right-to-left document direction and bidirectional-text protections for Arabic, Persian, and Urdu.
- SOCKS5 and HTTP/HTTPS proxy routing with proxy credentials stored in the operating system's secure credential manager.
- Configurable VPN-oriented timeouts, retries, keep-alive behavior, polling, bandwidth, transfer chunk size, and archive limits.
- Signed desktop updates on supported self-updating packages; pacman-managed Arch installs open the verified release for package-manager updates.
- Android share-sheet intake, foreground transfers, and native downloaded-file publication.

## Screenshots

Screenshots show representative workflows; small details may change between releases.

### Desktop

| Folder and list management | Video playback |
| --- | --- |
| ![Telegram Drive desktop folder list](Docs/assets/folders.webp) | ![Telegram Drive desktop video player](Docs/assets/video.webp) |

### Android

| Files | Transfer queue |
| --- | --- |
| ![Telegram Drive Android file list](Docs/assets/android-folders.webp) | ![Telegram Drive Android transfer queue](Docs/assets/android-transfers.webp) |

Additional desktop and mobile captures are available in the [`screenshots`](screenshots/) directory.

## Folder Sync (desktop)

Folder Sync is an opt-in, bidirectional mapping between a local directory and one Telegram channel on Windows, macOS, or Linux. It compares three states—the current local tree, the current remote tree, and the last successfully synced tree—before changing either side.

Safety behavior includes:

- Explicit conflicts instead of silent overwrites when local and remote versions both change.
- **Keep Local**, **Keep Remote**, and **Keep Both** conflict choices.
- A greater-than-50% mass-deletion guard.
- Atomic temporary-file downloads and exact transfer-size checks.
- Protection against nested mappings, duplicate remote paths, reserved temporary names, incomplete remote scans, and platform-incompatible filenames.
- Vault-aware queue pauses for encrypted mappings.
- A 50,000 file-bearing-message safety ceiling per mapped channel.

Folder Sync is disabled until enabled in **Settings → Folder Sync**. Read the [Folder Sync guide](SYNC_GUIDE.md) before mapping an important directory, and keep a separate tested backup.

## Optional client-side encryption (beta, format frozen)

See the [frozen format specification](Docs/TDENC2_FORMAT.md) and [recovery drill](Docs/ENCRYPTION_RECOVERY_DRILL.md).

Telegram Drive includes opt-in encrypted transfers using the versioned **TDENC2** envelope. Standard uploads remain the default. Encryption can be selected as a default for future uploads or chosen for an individual upload.

| Mode | Protection | Required to unlock |
| --- | --- | --- |
| **Standard** | Existing plaintext Telegram upload | Nothing |
| **Vault** | Encrypts with a key held in the local encrypted vault | Unlocked vault |
| **File passphrase** | Adds a passphrase slot to the individual file | File passphrase |
| **Vault + file passphrase** | Adds both vault and per-file passphrase access | Any supported valid slot |

The implementation provides streaming XChaCha20-Poly1305 encryption before upload, authenticated streaming downloads, optional protection of the original filename and MIME type, vault auto-lock controls, recovery-bundle export/import, per-file state badges, and encryption-aware transfer queues.

> [!CAUTION]
> **Telegram Drive cannot recover, reset, or reconstruct a lost vault passphrase, file passphrase, encryption key, or unusable recovery bundle.** Protect and test your recovery material and keep an independent backup of important files. Lost credentials or damaged recovery material can make encrypted data permanently unrecoverable.

The encryption design and implementation have **not received an independent security audit**. Use isolated test data first. This frozen-format beta is not a substitute for a tested backup strategy.

### Current encrypted-file limitations

The following operations fail closed for encrypted objects while retaining their normal behavior for standard files:

- In-app encrypted image and archive previews.
- Encrypted thumbnails, HLS/fMP4 remuxing, and transcoding.
- Local plaintext share links and REST/local-server plaintext access.
- WebDAV read, overwrite, rename, move, copy, and delete operations.
- Remote rename of encrypted Telegram media.
- Plaintext-to-encrypted migration, decrypt-in-place migration, rekey/slot-management UI, and full format migration.

TDENC2-protected audio, video, and PDF content can stream in supported in-app viewers while the vault is unlocked. The session-scoped credential is revoked when the vault locks, and the app does not publish a persistent plaintext copy. Download and authenticate other encrypted files before opening them in another application.

## Privacy and security

- The Telegram API ID and application state are stored locally. The API hash is kept separately in the desktop operating-system credential manager or Android Keystore; legacy plaintext values are removed only after secure migration succeeds.
- Telegram session data, settings, transfer state, sync metadata, encryption registry data, and vault material are stored locally in the application's data directories.
- Standard uploads send ordinary file content to Telegram. TDENC2 uploads send ciphertext, although Telegram can still observe transport/account metadata such as ciphertext size, time, account, and destination channel.
- The project does not operate a separate file-relay service.
- Crash reporting is disabled by default and requires explicit consent. Reports exclude file names, paths, contents, messages, Telegram identifiers, credentials, phone numbers, and user-entered values.
- The REST API and WebDAV server are disabled by default and bind only to `127.0.0.1`.
- The REST API stores only a hash of its generated key; the plaintext key is shown once.
- WebDAV capability links are shown once and can be revoked by regeneration.
- Proxy credentials use the operating system's secure credential manager.
- Optional supporter verification receives no Telegram credentials or file activity and does not create a purchaser email profile.

Read the complete [Privacy Policy](PRIVACY.md), [WebDAV guide](WEBDAV_GUIDE.md), and [REST API reference](REST_API_Documentation.md).

## Languages

Telegram Drive supports a **System** language preference and these 24 selectable locales:

| | | | |
| --- | --- | --- | --- |
| English | Spanish | Russian | Ukrainian |
| Polish | Persian | Urdu | Malay |
| Simplified Chinese | Traditional Chinese | French | Italian |
| Arabic | Brazilian Portuguese | German | Hindi |
| Bengali (Bangladesh) | Indonesian | Filipino (Philippines) | Turkish |
| Thai (Thailand) | Japanese | Korean | Vietnamese |

Locale selection and formatting are production-supported. Authentication, encryption, desktop supporter copy and visible data arrays use canonical translations, and the eleven large locales have zero copied-English findings. Mobile follow-up changes remain local under the owner’s pre-commit hook. Noncritical English copy and full native-language, legal-copy, RTL, long-string, CJK and device review remain outstanding; see [the measured locale review](Docs/LOCALIZATION_BUDGET_REVIEW.md).

## Optional $5 lifetime ad-free supporter license

Every Telegram Drive feature is available to non-paying users. Supported desktop and Android builds offer an optional **$5.00 USD Lifetime Ad-Free Supporter License** that removes sponsor placements on up to three supported Windows, macOS, Linux, or Android devices in total after one verified PayPal payment.

- It is a one-time purchase, **not a subscription**.
- Existing purchasers are not required to pay again after normal application updates.
- Active and offline-grace entitlements keep sponsor placements hidden.
- Reinstallation or another device can be restored with the recovery code, subject to the three-device allowance.
- Checkout is available only inside **Settings → Privacy → Supporter**.
- Payment does not create a Telegram Drive account or purchaser email profile.

> [!WARNING]
> Save the recovery code shown after activation. Payment alone does not bypass verification. Refunds are not automatic or guaranteed except where required by law. A refund, payment reversal, chargeback, or upheld dispute revokes the associated ad-free entitlement. Read the [Supporter Terms](SUPPORTER_TERMS.md) before paying.

## Android and Google TV (signed APK)

The Android build is currently distributed separately from the desktop release as a signed sideload release. It is not available through Google Play.

1. Open the [Android / Android TV 4.1.0 release](https://github.com/caamer20/Telegram-Drive/releases/tag/Androidv4.1.0).
2. Download the signed universal Android APK listed in that release.
3. On Android, allow **Install unknown apps** for the browser or file manager you used.
4. Open the APK and choose **Install**.
5. Enter your own Telegram API ID and API hash on first launch.

Android requirements and notes:

- Android 7.0 / API 24 or newer.
- One signed APK supports phones, tablets, Android TV, and Google TV.
- Google TV users can transfer the APK to the television or install it with ADB:

  ```bash
  adb install -r Telegram-Drive-v4.1.0-android-universal.apk
  ```

- Android validates the package signature during installation. Future compatible updates must use the same signing identity.
- Release maintainers should follow the [Android and Google TV release runbook](Docs/ANDROID_SIDELOAD_RELEASE.md).

## Local REST API and WebDAV

Both integrations are desktop-only, disabled by default, and loopback-only.

| Integration | Default address | Authentication | Access |
| --- | --- | --- | --- |
| **REST API** | `http://127.0.0.1:8550/api/v1` | `X-API-Key` header | File/folder operations, bulk actions, storage data, thumbnails, and media metadata |
| **WebDAV** | `http://127.0.0.1:8551/dav/<private-token>/` | One-time capability URL | Read-only by default; writes require explicit opt-in |

Do not expose these ports to a LAN or the internet. See the [REST API endpoint reference](REST_API_Documentation.md) and [WebDAV guide](WEBDAV_GUIDE.md) for setup, security behavior, examples, and limitations.

## Documentation

### User documentation

| Document | Purpose |
| --- | --- |
| [Changelog](CHANGELOG.md) | Release notes, compatibility information, and upgrade history |
| [Privacy Policy](PRIVACY.md) | Local data, Telegram data flow, crash reporting, sponsors, sharing, and supporter privacy |
| [Security Policy](SECURITY.md) | Private vulnerability reporting, redaction guidance, and responsible testing boundaries |
| [Folder Sync guide](SYNC_GUIDE.md) | Sync safety model, limits, conflict resolution, and integration behavior |
| [WebDAV guide](WEBDAV_GUIDE.md) | Setup instructions for macOS, Windows, and Linux clients |
| [REST API reference](REST_API_Documentation.md) | Authentication, endpoints, request examples, and error responses |
| [Linux packaging guide](Docs/LINUX_PACKAGING.md) | Linux formats, Arch installation, rendering safe mode, diagnostics, and release safeguards |
| [Linux/Arch implementation plan](Docs/LINUX_ARCH_IMPLEMENTATION_PLAN.md) | Design decisions, compatibility rules, release gates, acceptance matrix, and rollback |
| [Supporter Terms](SUPPORTER_TERMS.md) | Activation, recovery, device allowance, refunds, availability, and privacy |

### Maintainer and release documentation

| Document | Purpose |
| --- | --- |
| [Supporter license invariants](SUPPORTER_LICENSE_INVARIANTS.md) | Protected $5 lifetime-license compatibility contract and required verification |
| [Supporter service operations](SUPPORTER_SERVICE.md) | Worker, PayPal, D1, secure configuration, and release checks |
| [Supporter backup and recovery](SUPPORTER_BACKUP_RECOVERY.md) | D1 backup provisioning, restore drills, and production recovery |
| [Android and Google TV release runbook](Docs/ANDROID_SIDELOAD_RELEASE.md) | Signing, packaging, acceptance, and sideload-release checks |
| [Desktop code signing](Docs/CODE_SIGNING.md) | macOS notarization and Windows Authenticode settings, and first-signed-release acceptance |

## Build from source

### Prerequisites

- **Node.js:** `20.19.0+` within Node 20, or `22.12.0+`.
- **Rust:** the latest stable toolchain installed with [rustup](https://rustup.rs/).
- **Telegram API credentials:** an `api_id` and `api_hash` created at [my.telegram.org](https://my.telegram.org).
- **Tauri system dependencies:** follow the official [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform.

Common requirements:

- **macOS:** Xcode Command Line Tools (`xcode-select --install`).
- **Windows:** Visual Studio Build Tools with **Desktop development with C++** and the Microsoft Edge WebView2 Runtime.
- **Ubuntu/Debian:**

  ```bash
  sudo apt-get update
  sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file \
    libssl-dev libdbus-1-dev libgtk-3-dev libayatana-appindicator3-dev \
    librsvg2-dev libfuse2
  ```

### Install and run

```bash
git clone https://github.com/caamer20/Telegram-Drive.git
cd Telegram-Drive/app
npm ci
npm run tauri dev
```

The first run downloads and compiles the Rust dependency graph and may take several minutes. Later builds reuse Cargo's compilation cache.

Create release bundles with:

```bash
cd app
npm run tauri build
```

On Windows, the build scripts download and validate Microsoft's signed Visual C++ Redistributable before creating the NSIS installer.

### Validation

Behavioral tests are end-to-end journeys; there are no unit suites. Run the relevant checks from `app/` before submitting changes:

```bash
npm run build:verify
npm test
npm run i18n:check
cd src-tauri
cargo fmt --all -- --check
cargo clippy --locked --features native-e2e --lib --all-targets -- -D warnings
cargo test --locked --features native-e2e --test native_e2e
```

`node scripts/run-e2e.cjs` from the repository root runs every host suite. See [TESTING.md](TESTING.md) for what each suite covers and what still needs installed-app or device acceptance.

The application is built with Tauri 2, Rust, React 19, TypeScript, Tailwind CSS 4, TanStack Query/Virtual, SQLite, Tokio, Actix Web, Grammers, PDF.js, HLS.js, and MP4Box.

## Support and community

- Report reproducible, non-sensitive problems through [GitHub Issues](https://github.com/caamer20/Telegram-Drive/issues).
- Use [GitHub Discussions](https://github.com/caamer20/Telegram-Drive/discussions) for questions, ideas, and general feedback.
- Never include Telegram credentials, session data, payment identifiers, recovery codes, encryption keys, private filenames, or file contents in a public report.
- For a sensitive security report, follow the private-disclosure guidance in the [Security Policy](SECURITY.md).

Direct cryptocurrency tips help support development but **do not** activate the lifetime ad-free entitlement:

<div align="center">

<a href="litecoin:ltc1q6wkr5ac4u0pxx4hx7xgwn0gsaku25ws0df73rp">
  <img src="https://img.shields.io/badge/Donate-LTC-345D9D?style=for-the-badge&logo=litecoin&logoColor=white" alt="Donate Litecoin">
</a>

`ltc1q6wkr5ac4u0pxx4hx7xgwn0gsaku25ws0df73rp`

<a href="bitcoin:bc1q5pt7m2fk6w0dzsnf6vvd5k6nw5k44785286ujy">
  <img src="https://img.shields.io/badge/Donate-BTC-F7931A?style=for-the-badge&logo=bitcoin&logoColor=white" alt="Donate Bitcoin">
</a>

`bc1q5pt7m2fk6w0dzsnf6vvd5k6nw5k44785286ujy`

</div>

---

<div align="center">

Telegram Drive is not affiliated with Telegram FZ-LLC. Use the application responsibly and in accordance with Telegram's terms and applicable law.

</div>

### TV field navigation

With TV directional navigation, arrows edit a focused native field. After finishing an edit (or closing a native select picker), press Escape to return that field to directional navigation; the next arrow moves focus to another control. Enter resumes editing. A second Escape while already in navigation mode keeps the normal dialog-close behavior. Browser Escape is verified; physical Google TV Back event delivery and remote acceptance remain unrun.
