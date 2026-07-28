## [2.1.0] - 2026-07-28

### Italian Language Support & Build Fixes

- **Localization**
  - Added complete Italian (`it`) language support across all application surfaces, settings, dialogs, and error messages.
- **Build & Quality**
  - Resolved build warnings and runtime layout/formatting issues.

## [2.0.0] - 2026-07-26

### Major Release — Internationalization & Quiet Utility Redesign

- **Internationalization & Localization Engine**
  - Added multi-language architecture with 13 supported locales (English, Spanish, Russian, Simplified Chinese, French, Arabic, Brazilian Portuguese, German, Hindi, Indonesian, Turkish, Japanese, Korean).
  - Added system language preference resolution, locale formatters for dates/sizes/rates, Bidi text isolation primitives, and Rust share route localization.
  - Implemented automated localization validation scripts and CI workflow.

### Quiet Utility Redesign

- **New application-wide visual language**
  - Implemented the Path B “Quiet Utility” redesign across the desktop shell, mobile shell, authentication, sponsored surfaces, file workspace, settings, transfers, dialogs, menus, viewers, and empty/loading states.
  - Added semantic canvas, surface, border, text, accent, status, radius, elevation, typography, density, and motion tokens instead of relying on scattered component-specific styling.
  - Rebalanced the interface toward neutral graphite and warm-light surfaces, restrained depth, thinner borders, reduced decorative blur, and content-first hierarchy.
  - Reduced desktop typography and control dimensions to a denser modern scale inspired by current productivity applications while retaining 44–48px touch targets on mobile.
  - Replaced excessive card-lift and button-scale animations with faster, quieter state transitions and reduced-motion-safe behavior.
- **Shared UI primitives and development gallery**
  - Added reusable buttons, icon buttons, fields, selects, switches, segmented controls, surfaces, menus, badges, status dots, progress indicators, dividers, and skeleton states.
  - Added centralized Quiet Utility layout contracts for toolbar, navigation, row, sidebar, card, dialog, typography, and motion sizing.
  - Added a development-only design gallery for light, dark, custom theme, compact-density, LTR, RTL, loading, authentication, mobile-sheet, and viewer-state review.
- **Desktop shell and toolbar refinements**
  - Reduced and aligned the sidebar identity bar and primary file toolbar to the same 48px height so their bottom borders remain pixel-aligned.
  - Corrected the collapsed-sidebar layout so the logo and expand/collapse control remain within the application bounds.
  - Consolidated search, sort, grid/list selection, thumbnail sizing, folder creation, settings, and upload actions into a more compact toolbar hierarchy.
  - Added a unified transfer center for upload and download activity while preserving cancellation, retry, clear-finished, progress, speed, and concurrency behavior.
- **Settings, authentication, and viewer polish**
  - Rebuilt Settings as a larger, calmer category-based surface with denser controls, clearer section hierarchy, and updated tab/category switching motion.
  - Restyled API setup, phone login, QR login, Telegram code, 2FA, help, donation, and session-restoration surfaces without changing their underlying authentication commands.
  - Modernized image, PDF, audio/video, adaptive-streaming, and archive viewer chrome, navigation controls, loading states, and error presentation while retaining existing shortcuts and media behavior.
- **Mobile experience**
  - Updated the mobile toolbar, file rows, folder drawer, bottom navigation, action sheets, selection controls, transfer views, settings, and safe-area handling.
  - Replaced overly card-heavy mobile presentation with quieter edge-to-edge rows and compact semantic surfaces.

### Theme System

- Preserved Default, System, Light, Dark, built-in preset, and user-created custom themes through the redesign.
- Added semantic theme adapters so legacy presets and stored custom themes continue to drive the new design tokens.
- Fixed theme switching so users can return to the redesigned Default theme after selecting Light, Dark, System, a preset, or a custom theme.
- Improved custom-theme contrast mappings, theme persistence, scrollbar styling, and light/dark surface behavior.

### File Workspace, Layout & Preview Performance

- **Responsive file cards**
  - Reworked file-card containment so long filenames, thumbnails, metadata, encryption state, media badges, cached-quality badges, selection controls, and hover actions cannot overlap adjacent cards.
  - Added protected minimum card dimensions and virtualizer remeasurement while retaining responsive 4:3 sizing.
  - Restored thumbnail scaling below 100%; users can now resize file cards from 50% through 200% in 25% increments.
  - Replaced oversized file icons and controls with compact, consistently aligned variants.
- **Faster image previews and thumbnails**
  - Added bounded in-memory preview and thumbnail caches with one-hour expiry, least-recently-used refresh behavior, and duplicate in-flight request coalescing.
  - Added backend preview/thumbnail caching, partial-download guards, file integrity checks, progress events, cache quotas, stale-part cleanup, and cache pruning.
  - Added lazy image loading, asynchronous decoding, skeleton placeholders, fade-in presentation, cache invalidation on decode failure, and shared preview reuse between cards and the full viewer.
  - Avoided clearing usable preview caches during ordinary component unmounts, making repeat opens substantially faster.
- **Folder and query stability**
  - Fixed duplicated files on the initially restored folder by deduplicating streamed folder chunks by message ID.
  - Delayed file queries until the persisted startup folder is restored, preventing overlapping Saved Messages and startup-folder requests.
  - Coalesced concurrent initial folder sync requests and cleared image memory caches on account/logout transitions.
- Added skeleton layouts for grid/list loading states and refined empty, error, search, selection, and drag-target presentation.

### Buttons, Actions & Sponsored Content

- Added restrained color-matched gradients to Sync and Log Out actions.
- Updated Upload buttons to use the same compact gradient treatment and Quiet Utility sizing.
- Preserved the desktop 300×250 advertisement’s original in-flow placement so it no longer cuts into or displaces the top application banner/toolbar.
- Retained desktop ad launch timing, recurrence, countdown, hover pause, auto-dismiss, sandboxing, and external click-through behavior.
- Restyled the post-authentication sponsor gateway and Android sponsored banner with clearer disclosure, calmer presentation, safe-area handling, and preserved dismissal/click-through behavior.

### Encryption — Opt-In Alpha

- **TDENC2 encrypted Telegram transfers**
  - Added optional Standard, Vault, File Passphrase, and Vault + File Passphrase upload modes; Standard remains the default.
  - Added bounded-memory streaming encryption before Telegram upload and authenticated streaming decryption before final file publication.
  - Added authenticated protected metadata for original filenames and MIME types.
  - Applied explicit protection intent to manual uploads, drag-and-drop, folder ZIPs, retries, Android cached shares, and remote URL uploads.
- **Vault and recovery**
  - Added a persistent passphrase-protected vault, create/unlock/lock controls, inactivity auto-lock, background/sleep lock, logout lock, and exit lock.
  - Added authenticated recovery-bundle export/import and safe vault replacement only after bundle verification and explicit confirmation.
  - Added vault passphrase changes without rotating the vault key or re-encrypting every stored file.
  - Added short-lived opaque single-use prompt tokens so raw per-file passphrases are not persisted in transfer queues.
  - Added queue pause/retry states for credentials, decryption, and verification rather than silently falling back to plaintext.
- **Format and security hardening**
  - Replaced the quarantined TDENC1 prototype with the versioned TDENC2 envelope using XChaCha20-Poly1305, full 24-byte wrap nonces, keyed header authentication, authenticated metadata/chunks/final records, and exact ciphertext-length accounting.
  - Added strict parser bounds for versions, algorithms, KDF parameters, chunk sizes, slot counts, lengths, truncation, trailing data, integer overflow, and Telegram’s post-encryption size limit.
  - Added owner-only permissions for decrypted partial files and Unix remote-upload temporary files.
  - Added transactional encryption registry migrations, RAII bandwidth reservations, registry reconstruction from strict remote TDENC2 header probes, and reconciliation handling for move/copy/delete operations.
  - Added fail-closed detection for unindexed `.tdenc` and `TDENC2` objects so ciphertext is never treated as a normal preview, thumbnail, stream, REST download, or shared plaintext file.
- **Encryption UI and mobile parity**
  - Added encryption capability diagnostics, accurate loading/ready/blocked/error states, build identifiers, vault status, protected-file badges, and locked-name placeholders.
  - Added the encryption settings surface to both desktop and mobile.
  - Added a localized key-loss disclaimer explaining that forgotten passphrases, keys, and recovery material cannot be recovered by Telegram Drive, with acknowledgement required before vault creation.
  - Preserved verified Android download publication through MediaStore.
- **Intentionally unavailable for encrypted files**
  - Encrypted in-app previews, thumbnails, PDF/archive/media playback, transcoding, HLS/range streaming, plaintext share links, REST plaintext access, remote rename, migration, and rekey workflows remain explicitly disabled until credential-scoped logical-media implementations are complete.
  - Existing plaintext behavior for these features is unchanged.

### Language & Internationalisation Infrastructure

- Added System language preference and locale alias resolution while preserving all 13 supported language choices.
- Added centralized locale metadata for text direction, number/date locale, aliases, and future font selection.
- Added locale-aware number, byte-size, transfer-rate, date/time, relative-time, duration, percent, and list formatting helpers.
- Added bidi-safe primitives for user filenames and technical strings such as URLs, paths, hashes, API keys, IP addresses, and ports.
- Added generated typed translation keys, locale structure/type/interpolation validation, copied-English reporting, Arabic plural awareness, UI-literal scanning, pseudo-locales, and an i18n CI workflow.
- Added translated encryption settings, recovery actions, status copy, passphrase warnings, and key-loss disclaimer across English, Spanish, Russian, Simplified Chinese, French, Arabic, Brazilian Portuguese, German, Hindi, Indonesian, Turkish, Japanese, and Korean.
- Added a comprehensive language-support implementation plan covering remaining English extraction, native review, RTL, text expansion, accessibility, and release gates.

### Reliability, Compatibility & Documentation

- Added encryption-aware REST, local streaming, sharing, preview, and file-operation safety checks while preserving existing plaintext API behavior.
- Added stable encryption registry records, protected metadata state, plaintext/ciphertext sizes, header hashes, protection modes, and reconciliation state to SQLite.
- Preserved advertisements, Telegram authentication, folder/group navigation, sync, uploads/downloads, sharing, proxy/VPN settings, REST API, archives, viewers, updates, light/dark themes, and custom theme editing throughout the redesign.
- Added the Quiet Utility design handoff and implementation plan, comprehensive language plan, encryption architecture/remediation plans, TDENC2 format ADR, and encryption execution report.
- Verified the current work with a production frontend build, Rust compilation, 23 passing Rust library tests, localization validation/scanning, and whitespace validation.

### Known Pre-Release Work

- Complete the broader language plan’s remaining copied-English cleanup, hardcoded UI-string extraction, native linguistic review, RTL/long-string/CJK review, and accessibility testing.
- Complete credential-scoped logical plaintext sources before enabling encrypted previews, media, archives, sharing, REST access, migration, or rekey workflows.
- Run isolated Telegram integration tests, cross-platform recovery drills, fuzz/property testing, dependency/license review, and an independent cryptographic audit before describing encryption as generally available.

---

## [1.9.9] - 2026-07-13

### Bug Fixes & UI Enhancements

- **Video Playback Stability**
  - Resolved MediaSource buffer corruption error ("Invalid data found while parsing box") triggered during directory sorting.
  - Implemented state isolation using unique file IDs as React keys on the media player, preventing React from recycling HTMLMediaElement nodes and mixing video data streams.
  - Added explicit MediaSource stream termination and video element source reset during player unmount.
- **File Card UI Layout Fixes**
  - Corrected overlap issues between file names, metadata badges, and file type icons.
  - Restricted the icon container boundaries to avoid collision with the bottom info overlay on smaller or resized cards.
  - Added flex wrap and shrink constraints to file card metadata rows to handle narrow viewport dimensions.
- **Shift-Click Range Selection**
  - Fixed range selection index calculations to align with the visually sorted file list instead of the raw backend array.

---

## [1.9.8] - 2026-07-10

### Features, Security & Performance

- **Optimized Large Folder Handling**
  - Fixed application freezing and infinite loading loops when browsing folders with large datasets (8,000+ files).
  - Replaced bulk JSON payloads with incremental, event-driven chunk streaming from the Tauri backend.
  - Configured safe boundaries in directory reading commands to stop redundant API polling cycles.
- **Secure RAR Archive Extraction**
  - Replaced the deprecated `rar` dependency with the maintained `unrar` crate.
  - Added path normalization checks via the `path-clean` library to prevent directory traversal and arbitrary file write vulnerabilities during extraction.
  - Switched from disk-bound extraction to direct memory stream processing for listing contents safely.

---

## [1.9.7] - 2026-07-01

- Hotfixes

## [1.9.6] - 2026-06-30

### Features & Ad System Refinement

- **Ad system link updates**
  - Updated the desktop ad banner to load the hosted HTML page directly in the sandboxed iframe.
  - Set the clickable outer wrapper button to route to the actual Adsterra destination URL, ensuring correct navigation flow.
  - Reverted to using inline srcDoc for the desktop ad banner to ensure ads render reliably across all platforms, while preserving the user's custom click destination.
  - Updated the Mobile ad banner to route clicks to the custom ad landing page.

---

## [1.9.5] - 2026-06-24

### Features & Grouping Polish

- **Local-First Folder Grouping & Management**
  - Integrated a local SQLite database to track custom folder groups and user-defined sort order for Telegram channels.
  - Added horizontal scrollable group tabs at the top of the sidebar with dnd-kit drag-and-drop sorting support.
  - Added inline name editing and a custom color picker for folder groups.
  - Added a "Move to Group" submenu within the sidebar item context menu to quickly assign folders to groups.
  - Added a "Hide Groups" setting to toggle the visibility of the group tabs.

- **Theme Engine & Custom Styling**
  - Built a comprehensive React Context theme engine with support for custom themes and theme persistence.
  - Included a preset Cyber Teal theme.
  - Implemented a dedicated "Themes" tab in Settings to manage, configure, and delete custom user themes.

- **Proxy Connection Status Indicator**
  - Added real-time proxy connection checking and status indicator display in the sidebar.

- **Localization & Theme System Cleanups**
  - Added translated i18n keys for group management controls across all 13 supported languages.
  - Cleaned up custom theme contrast, scrollbar visibility, and light mode class toggles.

---

## [1.9.1] - 2026-06-20

### Localization & Internationalisation

- **Comprehensive Multi-Language Support**
  - Extended full translation coverage to all modal titles, tab headers, button labels, and input placeholders across both Desktop and Mobile interfaces.
  - Localized the Settings modal title and all five tab headers (General, Proxy, VPN, Sharing, About) across all thirteen supported languages: English, Spanish, Russian, Simplified Chinese, French, Arabic, Brazilian Portuguese, German, Hindi, Indonesian, Turkish, Japanese, and Korean.
  - Translated modal titles and interactive elements for the Rename Folder, Rename File, Move to Folder, and Remote Upload dialogs on Desktop.
  - Translated the mobile Rename Folder bottom sheet title, description prompt, input placeholder, and action buttons.
  - Translated the mobile bottom navigation bar tab labels (Files, Transfers, Settings) dynamically via the active language setting.
  - All translated strings correctly respond to language selection changes at runtime without requiring an application restart.

---

## [1.9.0] - 2026-06-16

### Features & Bug Fixes

- **Expanded REST API Interface**
  - Upgraded the REST API server to a fully functional programmatic Cloud Drive interface.
  - Implemented secure API key authentication and endpoints for file upload (via multipart forms), renaming, moving, copying, and deletion.
  - Added folder CRUD endpoints to manage Telegram channels programmatically.
  - Added endpoints for storage statistics, duplicate detection, empty folder discovery, and media metadata inspection.
  - Implemented a streaming ZIP archive generation endpoint for downloading bulk files.

- **Built-in Archive Viewer & Extractor**
  - Added support for viewing the contents of ZIP, RAR, and 7Z archives directly inside the application.
  - Implemented archive extraction capabilities to extract files to local storage or re-upload them.

- **Drag and Drop Interface**
  - Integrated drag-and-drop mechanics inside the file explorer to move files between folder views dynamically.

- **Context Menu Alignment**
  - Refined context menu positioning, boundaries, and quick-action menu options.

- **Thumbnail Resolution & Scoping**
  - Fixed duplicate thumbnail displays by scoping cache lookups and storage directories under folder-specific keys.

---

## [1.8.8] - 2026-06-12

### Features & Caching Polish

- **Enterprise-Grade Remote Upload**
  - Integrated dual-phase remote file upload from direct HTTP/HTTPS URLs.
  - Added frontend dialog `RemoteUploadModal` for URL input and destination folder selection.
  - Implemented backend download cache manager (`cmd_upload_from_url`) with disk space pre-flight checks, resumable downloads using HTTP `Range` headers, SOCKS5/MTProto proxy routing config, oneshot cancellation tracking, and bandwidth throttling.
  - Integrated dual-phase progress states and progress bars in `UploadQueue` UI.

---

## [1.8.7] - 2026-06-09

### Bug Fixes & Windows Enhancements

- **Windows Filename Sanitization**
  - Added filename sanitization to strip colons (`:`) and other invalid OS characters from target filenames, preventing Windows OS from refusing to save downloads or writing streams to NTFS Alternate Data Streams (ADS).
- **Cache Download Integrity**
  - Added strict file size verification in backend caching logic to detect interrupted downloads or server connection drop-offs. Prevents truncated files from being cached and causing FFmpeg remux parsing errors (like `Invalid data found while parsing box`).

---

## [1.8.4] - 2026-06-03

### Bug Fixes & UI Enhancements

- **Grid Virtualizer Cache Invalidation**
  - Resolved layout overlapping issues where "Upload File" and "Upload Folder" buttons overlapped with files in the Grid view by forcing virtualizer remeasurement on layout changes.

---

## [1.8.3] - 2026-06-03

### Bug Fixes & UI Enhancements

- **Cross-Platform Video Streaming Resolution**
  - Resolved dynamic CORS blocking behavior across client webviews, rectifying media loading and playback errors on Windows, macOS, and Linux.
- **File Grid UI & Layout Alignment**
  - Corrected card shifting and overlapping behaviors within the file explorer's grid layout.
  - Configured automated scroll reset and virtualizer cache purging during directory navigation.

---

## [1.8.2] - 2026-06-03

### Streaming Hotfix: Cross-Platform Video Playback Correction

- **CORS Configuration Update**
  - Updated CORS configuration on local streaming and API servers to use dynamic origin matching.
  - Resolved video playback failures (`TypeError: Failed to fetch` / `TypeError: Load failed`) on Windows, macOS, and Linux clients by explicitly supporting platform-specific custom schemes and WebKit's `null` origin.

---

## [1.8.1] - 2026-06-03

### Features, Security & Architecture (MimoPro Cleanups Part 2)

- **User Interface Enhancements**
  - Optimized the media player controls by standardizing close and fullscreen overlay buttons.
  - Refined layout geometries for player circle buttons and balanced spacing gaps.
  - Integrated a new periodic interstitial ad flow, featuring timed countdowns and automated clicks/focus dismissal logic.

- **Storage & Backend Stability Fixes**
  - Recreated missing mobile capability rules to support compilation of Tauri configuration context.
  - Corrected progress field mappings (`downloadedBytes`) in UI progress bars and download queues.
  - Resolved dynamic import resolution behavior for local builds.
  - Expanded Content Security Policy (CSP) rules to permit ad network script invocations.

- **Improved Accessibility Support**
  - Auto-enabled performance and reduced-motion modes using platform media queries.
  - Wrapped major component views in granular error boundaries to prevent app-wide crashes.

---

## [1.8.0] - 2026-06-02

### Features, Security & Architecture (MimoPro Analysis Cleanups)

- **Critical Fixes**
  - Synchronized versions across `package.json`, `tauri.conf.json`, and `Cargo.toml`.
  - Replaced hardcoded `localhost` references in `sharing.rs` with `127.0.0.1` to ensure local loopback resolution works reliably.
  - Removed misleading mock rate limit headers (`x-ratelimit-limit`, etc.) from API endpoint responses.

- **Dependency Upgrades**
  - Upgraded deprecated dependencies (`base64` to `0.22`, `rand` to `0.9`).
  - Standardized Tauri plugin version strings (`tauri-plugin-dialog`, `tauri-plugin-updater`, `tauri-plugin-process`) to consistently use version `"2"`.

- **Dead Code Cleanup**
  - Completely removed empty `DropZoneContext.tsx` and removed the `<DropZoneProvider>` wrapper from `App.tsx`.
  - Cleaned up unused state variables (`_internalDragFileId`) and unused hook exports (`isNetworkError`, `forceLogout`, `handleDownload`).
  - Removed dead settings fields (`proxySecret`, `ProxyConfig.secret`).

- **Security Enhancements**
  - Implemented secure `bcrypt` hashing (work factor `12`) for folder share passwords.
  - Swapped direct comparisons in API key verification for constant-time checks using `constant_time_eq` to prevent timing attacks.

- **Architecture & Code Deduplication**
  - Extracted shared byte-range parsing and chunk calculation logic for downloads and media streaming.
  - Consolidated duplicate MP4 container box header navigation and box validations into a single module.
  - Unified duplicate password hashing and verification code.
  - Corrected field naming inconsistency (`uploadedBytes` to `downloadedBytes` inside `DownloadItem`).

---

## [1.7.9] - 2026-06-02

### Features & Fixes

- **Folder Rename Bug Fix** — Resolved an issue where renaming a folder in the sidebar failed because the capture-phase click listener unmounted the context menu before the button's action could execute.
- **Media Player Styling** — Equalized the close and fullscreen circle buttons to standard sizes with a small gap between them.

---

## [1.7.8] - 2026-06-01

### Features & Enhancements

- **Major UI improvements**
- **Complete media player revamp**
- **Multiple bug fixes**

---

## [1.7.0] - 2026-05-29

### Features & Documentation

- **Pre-built Android APK Update** — Updated the sidelined APK build release reference to `v2.1.0-beta` in the documentation.
- **Android App Gallery** — Added an extensive screenshots section highlighting the beautiful mobile interface layout, theme views, and active queue transfers.
- **Desktop Bump to v1.7.0** — Bumped desktop versions to v1.7.0 for the synchronized release.

---

## [1.6.9] - 2026-05-29

### Features & Enhancements

- **Android Compilation Fixes** — Cleaned build configuration files to prevent deepLinkProtocols syntax conflicts.
- **Mobile Shell & macOS Entitlements** — Configured custom shell integration capabilities and set up macOS entitlements plists.

---

## [1.6.8] - 2026-05-25

### Features & Fixes

- **In-App Update Permission Fix** — Granted the `"process:allow-restart"` capability permission in `src-tauri/capabilities/default.json` to allow the frontend updater to safely relaunch the app after installing an update.

---

## [1.6.7] - 2026-05-23

### Features & Fixes

- **Windows Build & Git Checkout Fix** — Untracked and ignored `app/.npm-cache` files from Git to fix "Filename too long" checkouts and build errors on Windows platforms.
- **Tauri signing key security** — Replaced updater signing key with a password-protected keypair and restored the secret password integration in the CI pipeline.

---

## [1.6.6] - 2026-05-22

### Features & Fixes

- **Tauri Updater Integration & Dedicated UI** — Fully integrated and resolved production updater configurations.
  - **Updater Build Artifacts**: Set `createUpdaterArtifacts` to `true` in `tauri.conf.json` to generate signing signatures (`.sig`) and the `latest.json` manifest dynamically during production builds.
  - **In-App Update Interface**: Added a native "Check for Updates" control panel within the General Settings tab, complete with a visual download progress bar, status toasts, and automatic "Update & Restart" integration.
  - **Promise Safety**: Handled fire-and-forget background update-check Promises by appending explicit `.catch` error logging to prevent unhandled rejection behaviors.
  - **Automated Workflow Releases**: Enhanced the GitHub Release CI workflow to automatically parse and extract only the latest release notes from `CHANGELOG.md` dynamically using `awk`.

---

## [1.6.5] - 2026-05-21

### Features & Enhancements

- **REST API Enhancements (Actix-web & Rust)** — Fully implemented the comprehensive REST API extension in Rust/Actix-web with backwards-compatible response structures.
  - **Refined Folder Navigation**: Resolved `folder_id` query handling into three deterministic query states: all files when omitted, root-only when `?folder_id=`, and subfolder files when filtering specifically by a folder ID.
  - **Standardized Pagination Envelope**: Wrapped collections in a clean payload format featuring a `data` array, `pagination` metrics (`page`, `limit`, `total_items`, `total_pages`), and a `filters` echo block.
  - **Advanced Query Parameters**: Introduced server-side sorting (`sort_by`, `sort_order`) and robust filters for MIME type, file size bounds, and creation date ranges.
  - **Sparse Fieldsets**: Added a `?fields=` selector enabling clients to request specific metadata subsets to reduce bandwidth overhead.
  - **Bulk Operations & Global Search**: Added `POST /api/v1/files/bulk` for batch moves and deletes, and `GET /api/v1/files/search` supporting the full pagination envelope.
  - **Rate Limiting Integration**: Injected simulated API rate-limit headers (`X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset`) to standard responses.

---

## [1.6.0] - 2026-05-21

### Features & Fixes

- **"Copy Telegram Link" Feature** — Added a right-click context menu option to copy raw `t.me` message links for files in public channels (`https://t.me/{username}/{message_id}`). If the channel is private, the item displays in a disabled state with a descriptive tooltip.
- **Tauri 2 Tokio Runtime Panic Fix** — Fixed the `there is no reactor running` panic caused by `tokio::task::spawn_blocking` executing outside of a Tokio runtime context within the Bandwidth Manager. Replaced the asynchronous task with a lightweight, synchronous write, resolving the panic completely.

---

## [1.5.0] - 2026-05-19

### Feature

- **VPN Optimizer & Proxy Configuration** — Added robust support for toggling VPN mode to optimize network connection timeouts, retry limits, backoff delays, adaptive polling, flood wait handling, and peer caches. Fully integrated proxy configuration (SOCKS5 and MTProto) to allow custom routing and bypass geo-blocks.

---

## [1.4.2] - 2026-05-18

### Feature

- **Folder Upload with Automatic Zipping** — Support uploading entire folders directly, automatically compressing them into highly-optimized zip archives before transfer.

---

## [1.1.7] - 2026-05-01

### Feature

- Added a donation button and popup modal to the main login screen to support the project via PayPal, Litecoin, and Bitcoin.

---

## [1.1.6] - 2026-04-28

### Fix

- Fixed process not terminating on Ctrl+C (SIGINT) when launched from a terminal.
  The Actix-web streaming server and grammers network runner were running on
  non-daemon threads with no shutdown signal wired to process exit, causing the
  application to hang indefinitely after the main window closed. The app now
  registers a RunEvent::Exit handler that gracefully stops both background
  services before the process exits.

---

## [1.1.5] - 2026-04-27

### Hotfix

- **CI fix: AppImage patch step now runs cleanly** — Replaced the fragile `grep -oP` Perl lookahead (which exited with code 2 under `set -euo pipefail`) with a safe `awk`-based `.desktop` file lookup. Added `APPIMAGE_EXTRACT_AND_RUN=1` so `appimagetool` doesn't require the FUSE kernel module on GitHub Actions runners.

---

## [1.1.4] - 2026-04-27

### Hotfix

- **Deeper AppImage EGL fix for Arch/rolling-release Linux** — Added a CI post-build patching step that strips the Ubuntu-bundled `libEGL`, `libGL`, `libGLdispatch`, `libGLX`, and `libGLESv2` from the AppImage squashfs and replaces the `AppRun` wrapper with one that: normalises the locale to `C.UTF-8`, sets `NO_AT_BRIDGE=1` to silence ATK warnings, auto-detects `EGL_PLATFORM` from `$WAYLAND_DISPLAY`/`$DISPLAY`, points GLVND at the system ICD vendor dirs, preloads the system `libEGL.so.1`, and orders `LD_LIBRARY_PATH` so host GPU drivers are always resolved before bundled stubs.

---

## [1.1.3] - 2026-04-27

### Hotfix

- **Fixed Arch Linux AppImage crash** — Resolved `EGL_BAD_ALLOC` error on Arch Linux (and other rolling-release distros) caused by bundled Mesa/EGL libraries conflicting with the host GPU driver stack. The app now automatically disables WebKitGTK's DMA-BUF renderer on Linux before the WebView initializes, with no impact to Windows or macOS builds.

---

## [1.0.4] - 2026-02-13

### Fixes

- Finally squashed the grid overlap bug for real. Cards were using CSS `aspect-[4/3]` to size themselves, but the virtualizer was computing row heights separately — at certain window widths these disagreed and rows would bleed into each other. Now both use the same explicit pixel height, so no more overlap regardless of how you resize the window.

### Cleanup

- Went through the whole codebase and ripped out every `console.log` / `console.error` we'd left in from debugging (16 of them). The one in `ErrorBoundary` stays since that's the whole point of an error boundary.
- Got rid of all `as any` casts on the frontend — everything's properly typed now.
- Ran Clippy and fixed all 7 warnings, including a couple of `collapsible_match` ones in `fs.rs` that needed manual refactoring.
- Dropped `clsx`, `tailwind-merge`, and `@tauri-apps/plugin-opener` from `package.json` — none of them were actually imported anywhere.
- General comment cleanup throughout.

---

## [1.0.3] - 2026-02-09

### Bug Fixes

- **Grid Spacing Fix** - Fixed cards overlapping in grid view
- **Dynamic Row Height** - Grid now properly calculates row height based on window size
- **Virtualizer Re-measurement** - Grid correctly updates when resizing window

---

## [1.0.2] - 2026-02-07

### Automated Release Pipeline

- **GitHub Actions Workflow** - Automatic builds triggered on version tags
- **Cross-Platform Builds** - Windows, Linux, macOS (Intel + ARM) built in parallel
- **Signed Updates** - All builds signed with Ed25519 for secure auto-updates
- **Automatic Publishing** - Releases published to GitHub automatically

---

## [1.0.1] - 2026-02-07

### Auto-Update System

- **Automatic Update Checks** - App checks for updates 5 seconds after startup
- **Update Banner** - Beautiful animated banner when new version available
- **One-Click Updates** - Download and install updates with progress indicator
- **Cross-Platform** - Windows, Mac, and Linux users get platform-specific updates

### 🔧 Technical

- Added Tauri updater plugin with Ed25519 signing
- Created `useUpdateCheck` hook for update lifecycle management
- Added `UpdateBanner` component with download progress

---

## [1.0.0] - 2026-02-06 🎉

### First Stable Release

Telegram Drive is now production-ready! This release focuses on performance, reliability, and user experience polish.

### ✨ New Features

- **Virtual Scrolling** - Smooth performance with folders containing 1000+ files
- **Inline Thumbnails** - Image files now display thumbnails directly in the file grid
- **Thumbnail Caching** - Thumbnails are cached locally for instant loading on revisit
- **API Setup Help Guide** - Step-by-step modal explaining how to get Telegram API credentials

### 🚀 Performance Improvements

- Grid and list views now only render visible items (virtualized)
- Responsive column layout adapts to window width
- Lazy loading of thumbnails to reduce initial load time

### 🎨 UI/UX Improvements

- Refined grid spacing (6px gaps between cards)
- Gradient overlay on thumbnail cards for text readability
- Improved light mode support across all components

### 🔧 Technical

- Added `@tanstack/react-virtual` for virtualization
- Separate thumbnail cache directory (`app_data_dir/thumbnails/`)
- FileTypeIcon now supports multiple sizes

---

## [0.6.0] - 2026-02-05

### Reliability Update

- Session persistence (window state, UI state, active folder)
- Network resilience with connection status indicator
- Queue persistence for uploads/downloads
- Light mode UI fixes

---

## [0.5.0] - 2026-02-04

### Drag & Drop Update

- Stable hybrid drag-drop system
- External drop blocker
- GitHub Actions workflow fixes

---

## [0.4.0] - 2026-02-01

### Media & Performance

- Audio/Video streaming player
- Global search filter
- Internal drag & drop between folders
