## [4.1.5] - 2026-11-13

### Default upload mode

- Add Ask every time, Always Standard, and Always Encrypted under Preferences → Encryption (#247). The selected preference persists on this installation; existing installations keep Ask every time.
- Apply the default to file selection, drag-and-drop, zipped folders, and URL uploads. Protected uploads retain their encryption checks, vault setup/unlock, recovery guidance, and masked per-file passphrase prompts.
- Keep the selected encryption method and metadata protection consistent through preparation, cancel pending choices when accounts change, and preserve the original protection on queued uploads and retries.
- Translate the new preference and preparation messages in all supported languages.

### Dependency security

- Update the proxy DNS resolver and filesystem dependencies to address newly published security advisories.
- Backport resolver compatibility within the existing Telegram transport package, preserving the current protocol/session versions, IPv4-first proxy selection, system DNS settings, and SOCKS authentication behavior.

### Compatibility and verification

- Preserve existing Telegram sessions, vaults, settings, transfer behavior, and lifetime supporter activations. The optional $5.00 USD one-time lifetime ad-free license and recovery rights remain unchanged.
- Keep desktop platform builds, packaged startup, dependency, E2E, signing, checksum, SBOM, and provenance gates. Automated Telegram boundaries use controlled fixtures; live-service and manual installed-platform acceptance are separate.
- This release updates desktop only. Android remains on its separate release line.

## [4.1.4] - 2026-11-07

### Folder Sync recovery

- Fix active folder mappings incorrectly reporting Telegram as offline after restarting the desktop app (#241).
- Show Waiting for Telegram during connection startup and resume automatically when the authenticated session is ready, including with the incremental scanner. Retained readiness and bounded retries recover without toggling a mapping.
- Reject stale connection replies and preserve manually paused mappings, account isolation, encryption restrictions and remote cleanup safeguards.

### Files header sizing

- Fit the folder heading and filter controls to their visible contents, removing the unused space reserved for selection actions.
- Expand the header only when bulk actions are visible, while preserving filter shortcuts, layout switching and keyboard controls.

### Dependency security

- Update the HTTP server and file-response dependencies to address newly published request parsing, decompression and Windows file-disclosure advisories.
- Preserve authenticated byte-range video streaming and account-revocation checks while adapting to the patched file-response API.

### Compatibility and verification

- Preserve existing Telegram sessions, vaults, settings, transfer behavior and lifetime supporter activations. The optional $5.00 USD one-time lifetime ad-free license and recovery rights remain unchanged.
- Keep the existing desktop platform builds, packaged startup, dependency, E2E, signing, checksum, SBOM and provenance gates. Automated Telegram boundaries use controlled fixtures; live-service and manual installed-platform acceptance are separate.
- This release updates desktop only. Android remains on its separate release line.

## [4.1.3] - 2026-11-06

### Files, transfers and settings

- Refine the desktop Files view with a clearer folder heading, filter shortcuts, compact list columns and a stable selection toolbar.
- Show measured transfer progress, distinguish failed, skipped and completed work, and display an honest waiting state when progress cannot yet be measured.
- Add searchable settings categories, consistent theme controls and motion that respects the system preference and the saved Performance Mode choice. Closing dialogs releases controls immediately and restores focus.

### Retry failed uploads

- Retry the current account's failed uploads with one action while preserving each file's destination, protection, video settings and queue position.
- Prevent duplicate retry batches, stop remaining retries when the queue is paused or cancelled, and report queued, skipped, failed and unattempted counts. Queue acceptance does not mean upload completion.
- Protected uploads retain their per-file credential confirmations. Account changes cancel the old batch; newly failing uploads wait for the next explicit action.

### Dependency security

- Update TLS, Tokio, Zstd and ZIP dependencies to patched versions. Align the native and JavaScript Tauri updater packages while preserving the existing signing configuration and application identity.
- Update the local supporter-service HTTP tooling dependency without changing payment or entitlement behavior.

### Compatibility and verification

- Preserve the upload and streaming fixes shipped in 4.1.2, existing sessions, vaults, settings and lifetime supporter activations. The optional $5.00 USD one-time lifetime ad-free license and recovery rights are unchanged.
- Browser, native process/storage, service, dependency, security, localization, packaged startup and provenance checks remain release gates. Automated Telegram and PayPal boundaries use controlled fixtures; live-service and manual installed-platform acceptance are separate.
- Android remains a separate binary release. Android source and project changes are excluded.

## [4.1.2] - 2026-10-14

### Desktop video streaming metadata

- Correct the overly restrictive MP4 streaming-eligibility check introduced in 4.1.1. Valid trailing-metadata layouts accepted by Telegram Desktop now receive the matching streaming flag for local and URL Media uploads.
- Preserve the original video bytes, JPEG previews, dimensions, duration and filename captions. File mode, protected uploads, resumable transfers, cancellation, account isolation and quota checks retain their existing paths.
- Existing sessions, vaults, settings and lifetime supporter activations remain compatible. The optional $5.00 USD one-time lifetime ad-free license and recovery rights are unchanged.

### Desktop media controls and dropdowns

- Give labeled playback actions room to fit and group skip, speed, volume and library actions with consistent spacing. Controls and keyboard hints wrap cleanly in smaller windows.
- Keep video and audio viewers inside the available viewport, with scrollable bookmarks/queue details and accessible Close/Fullscreen controls. Quality badges remain anchored to the video.
- Match speed, sleep timer, slideshow interval, settings, sync and workspace dropdown styling while preserving native selection, keyboard behavior and touch targets.

### Dependency security

- Update Rustls to [0.23.46](https://github.com/rustls/rustls/releases/tag/v/0.23.46), addressing the upstream TLS advisories detected by the release dependency gate.

### Verification

- The existing large-container upload journey failed at the streaming-flag assertion before the correction. All 215 native journeys and all 133 browser journeys passed locally, including captions, sync settings, slideshow, visual, accessibility and TV keyboard navigation checks; full release suites and platform package checks remain publication gates.
- Automated Telegram boundaries use controlled fixtures. Streaming and seeking with the reporter's actual video over 200 MB on a clean Windows Telegram Desktop receiver remain pending owner acceptance after this release.

## [4.1.1] - 2026-10-13

### Desktop video media uploads

- Attach a locally generated JPEG preview and explicit dimensions/duration to unencrypted MP4 Media uploads, including large files and uploads from a URL. This supplies the thumbnail and video attributes Telegram Desktop needs to display its player.
- Set streaming metadata from the MP4 container layout while preserving the original video bytes. Videos with metadata at the end of the file may still need downloading before playback.
- Use the native Windows video decoder first, with FFmpeg as a fallback. macOS and Linux require FFmpeg for video thumbnails; an unavailable decoder offers File mode instead of silently uploading incomplete Media metadata.
- Preserve filename captions, account isolation, cancellation, retries and transfer quotas. Protected videos retain their existing private upload path.

### Compatibility and verification

- Existing vaults, sessions, queues, settings and lifetime supporter activations remain compatible. Every feature remains available without payment; the optional $5.00 USD one-time lifetime ad-free license, recovery codes and three-device allowance are unchanged.
- Desktop native and browser regressions passed, including synthetic H.264 MP4 containers over 200 MiB and Windows native decoding. Automated Telegram boundaries use controlled fixtures; the reporter's exact file/codec, live Telegram Desktop playback and rotated-video acceptance remain unrun.
- Android 4.1.0 remains a separate APK release. The product page deploys from the protected main branch after stable Desktop publication.

## [4.1.0] - 2026-10-13

### Video uploads and settings

- Add original filenames as captions for unencrypted video media uploads by default, with a saved setting to disable them. Queued uploads retain their caption choice across interruption and restart. Protected uploads keep filenames private.
- Align desktop transfer preferences and vault settings with consistent grouped controls, readable labels, larger targets, and verified light/dark contrast.
- Improve protected-upload setup, cancellation, loading failures and retry. Passphrase-only protection remains available without creating or unlocking a vault.

### Compatibility and verification

- Preserve the account isolation, transfer recovery, vault recovery drills, protected media handling, search, HEIC previews and release protections shipped in 4.0.0.
- TDENC2 remains a frozen-format beta with independent cryptographic review pending. Existing vaults, settings, queues, sessions and lifetime supporter activations remain compatible.
- Every feature remains available without payment. The optional $5.00 USD one-time lifetime ad-free license, recovery codes and three-device allowance are unchanged.
- Android is compiled locally and published independently as a separate binary release. Android project and source changes are excluded from this desktop release.
- Automated browser, native, Worker, localization, security, dependency, packaging and provenance checks remain release gates. Live Telegram, physical-device and manual installed-platform acceptance remain separate from controlled-fixture results.

## [4.0.0] - 2026-10-05

### Highlights

- Search your Telegram library with account-scoped full-text filters, collections, saved searches and a media timeline.
- Browse HEIC/HEIF photos and offline copies while preserving the original files. macOS previews are built in; Windows and Linux require FFmpeg 8.1 or newer.
- Reopen unchanged app-produced files instantly on eligible filesystems, including after a restart. Files with changed or unsupported identities are verified in full.
- Keep transfers, folder listings, local services and sign-in responsive, with clearer recovery and localized navigation across 24 languages.

### Compatibility and visible changes

- **REST API changes for scripts:** timestamps use RFC 3339, identifiers support 64-bit values, listings include complete results and explicit partial coverage, and the API publishes an OpenAPI document. Update clients that depend on the older shapes.
- **Open externally** permits only files the app produced or recorded plain caches from earlier releases. Account and protection checks remain active. Arbitrary paths, symlinks and modified contents are refused; recorded 3.9.x previews, thumbnails and ready offline copies remain available after upgrade.
- **Sponsor placements return every 15 minutes.** Active and offline-grace supporter entitlements continue to suppress every sponsor advertisement.
- **TDENC2 encryption is a frozen-format beta.** Passphrases use masked dialogs, and recovery drills verify bundles without replacing the active vault. Independent cryptographic review remains pending.
- Every Telegram Drive feature is available to non-paying users. Supported desktop and Android builds offer an optional **$5.00 USD Lifetime Ad-Free Supporter License** that removes sponsor placements on up to three supported Windows, macOS, Linux, or Android devices in total after one verified PayPal payment. It is a one-time purchase, **not a subscription**. Normal app updates preserve activation; existing purchasers do not need to buy again, and recovery codes remain supported.
- **Android and Google TV are not part of this release.** Desktop 4.0.0 and the Android/Google TV 4.1.0 preview use independent version lines.

### Workspace, reliability and localization

- Folder, REST and WebDAV views share account-scoped inventory, retain older cached entries in partial listings and refresh after local changes. Search applies type, size, date, folder, tag, collection, favorite and protection filters before paging.
- Offline packs, gallery/slideshow navigation, archive previews, preview-cache controls and transfer activity preserve source identity and account isolation. Protected files do not produce plaintext HEIC renditions.
- Sign-out revokes the Telegram session before removing it locally and explains remote revocation failures. Wrong two-step verification passwords can be corrected without restarting sign-in.
- Local servers recover from occupied ports. Redacted diagnostics identify startup failures, and staged files use private directories with cleanup at startup.
- Storage checks correctly detect available space on Windows.
- **Clear finished** removes completed, failed and cancelled uploads and downloads from the transfer queue while preserving ongoing work.
- Tray menus, transfer notifications, share-password pages, settings, authentication, encryption, Help and viewer controls use localized copy. Language changes preserve dialog and application state; accessibility improvements cover focus, labels and contrast.

### Verify downloads

Download the installer and `SHA256SUMS.txt` from this release. On Linux use `sha256sum -c SHA256SUMS.txt`; on macOS use `shasum -a 256` for the selected file; on Windows use `Get-FileHash -Algorithm SHA256` and compare the matching manifest entry.

Verify provenance with the expected repository and inspect the reported workflow and tag:

```sh
gh attestation verify ./DOWNLOADED_ARTIFACT --repo caamer20/Telegram-Drive
```

Do not install if verification fails or identifies an unexpected producer. Source and Arch runtime SBOMs and Sigstore bundles accompany the release. See [download verification](Docs/RELEASE_NOTES_TEMPLATE.md).

## [3.9.8] - 2026-09-29

### Desktop reliability

- Share update checks and installation progress between Settings and the update banner, with recovery from unavailable local release-note storage.
- Correct proxy/VPN settings requests and preserve saved settings when startup reads fail.
- Improve search-filter presentation, supporter license explanations, and translated desktop messaging.
- Correct local streaming/share route coexistence and remove unused internal code while preserving account, encryption, transfer, and lifetime supporter compatibility.

### Release verification

- Replace retired unit runners with browser, native process/HTTP/storage, Worker HTTP/local-D1, and release-tool E2E journeys.
- Retain dependency assurance, security/configuration checks, formatting, localization, bundle budgets, and platform build gates.
- External Telegram/PayPal boundaries use controlled fixtures in automated suites; these do not replace live-service or installed-device acceptance.

## [3.9.5] - 2026-09-22

### Desktop reliability

- Fixed session database contention that could interrupt streaming or leave file loading unavailable after upgrading.
- Restored workspace access after signing out and signing back in without restarting, including QR, phone-code, and two-step authentication.
- Added bounded retries for temporary account lookup failures and made manual sync retry account verification.
- Prevented stalled encrypted metadata requests from blocking an entire folder. Deferred files remain protected until their metadata can be retrieved.
- Allowed folder refresh to recover from malformed optional cached metadata while preserving account isolation and stored records.
- Kept the loading indicator visible while an uncached folder is being scanned instead of prematurely displaying an empty folder.
- Improved sign-out progress, failure reporting, duplicate-attempt handling, and legacy local credential cleanup. Optional haptic feedback can no longer block confirmation.

### Themes

- Added AMOLED, Ocean, Forest, Plum, Paper, Rose, Mint, and Sky presets while preserving existing themes.

### Compatibility

- Preserved Telegram sessions, existing data, account isolation, and the $5.00 USD one-time lifetime ad-free license, recovery codes, and three-device allowance.
- This desktop release remains compatible with the database format introduced in 3.9.0. Older 3.8.x applications cannot open that newer format; downgrading the executable does not downgrade saved data.
- Added regression coverage for concurrent session activity, fresh sign-in, stale authentication replies, folder recovery, sign-out confirmation, and theme contrast.

## [3.9.0] - 2026-09-12

### Workspace and media

- Added a local library with collections, tags, saved searches, a photo timeline, and slideshows.
- Added Continue Watching, playback queues, and offline trip packs with storage controls.
- Added transfer activity, storage insights, and scheduled cleanup with a restore window.
- Added sync plan previews and clearer conflict and retry controls.
- Aligned desktop headers, dividers, and content spacing across the sidebar and main workspace.

### Reliability

- Isolated local metadata, transfers, shares, and sync operations by the signed-in account.
- Improved download collision choices, file refresh, offline previews, media navigation, and recovery from interrupted operations.
- Preserved existing sessions, settings, and the one-time $5.00 USD lifetime ad-free supporter license.

## [3.8.5] - 2026-08-30

### Reliability and Safety

- Improved desktop startup and isolated WebView loading reliability.
- Hardened external navigation so only validated, user-initiated HTTP or HTTPS destinations can open outside the application.
- Improved network fallback behavior while keeping application data and local credentials isolated.
- Expanded privacy disclosure and regression coverage for startup gating, network fallbacks, and cross-frame messaging.
- Preserved existing Telegram sessions, settings, transfer queues, secure credentials, and the protected one-time $5.00 USD lifetime supporter entitlement.

### Verification

- Frontend, native, supporter-service, localization, release-version, and production bundle checks pass.
- Cloudflare Worker packaging was verified with a deployment dry run; no production service or payment state was changed.

## [3.8.0] - 2026-08-30

### Desktop Security and Data Safety

- Hardened local share pages with bounded password handling, throttling, private-cache controls, content-security headers, and capability-safe diagnostics.
- Added strict loopback CORS origin parsing, bounded fail-closed multipart uploads, sanitized upload names, and securely staged temporary files.
- Added archive expansion limits to protect previews from decompression bombs and preserved recoverable Telegram session copies when corruption is detected.
- Moved the Telegram API hash into operating-system credential storage with a verified legacy migration path, and bound crash submission to the reviewed compile-time endpoint.
- Made bandwidth accounting overflow-safe and atomically persisted so malformed or interrupted writes cannot silently corrupt quota state.

### Reliability, Accessibility, and Performance

- Added serialized settings persistence with visible failure state and retry behavior while preserving every existing settings key and storage location.
- Lazy-loaded optional media, archive, settings, help, and language resources, with release bundle ceilings that prevent startup-size regressions.
- Normalized user-facing failures into localized, privacy-safe messages while retaining technical detail in sanitized diagnostics.
- Expanded keyboard, focus, dialog, mobile-navigation, and Axe coverage across representative authentication, dashboard, settings, dialog, and mobile screens.
- Reduced untranslated UI literal debt across all 24 shipped locale bundles and strengthened generated-key and no-regression localization checks.

### Linux and Arch Compatibility

- Replaced broad AppImage library rewriting with a testable WebKit shared-memory rendering fallback that is limited to AppImage launches and respects explicit user environment choices.
- Added a checksummed `telegram-drive-bin` Arch package, standard desktop integration, pacman-owned updater behavior, install verification, upgrade-preservation checks, and a runtime SBOM.
- Prevented the desktop updater from replacing pacman-owned files while retaining update availability notices and the existing self-managed updater on Windows, macOS, AppImage, Debian, and RPM installations.
- Removed post-signing AppImage mutation so published Linux artifacts remain byte-for-byte consistent with their updater signatures and release provenance.

### Release Assurance and Compatibility

- Added pinned dependency, license, advisory, bundle, accessibility, packaging, checksum, SBOM, and provenance gates before a draft desktop release can be published.
- Enforced exact parity between the release tag, npm package, Cargo package, Tauri configuration, Android version contract, and first changelog entry.
- Standardized the optional supporter wording across application, service, privacy, terms, README, and website surfaces without changing the protected one-time $5.00 USD lifetime ad-free entitlement or three-device allowance.
- Existing Telegram sessions, secure credentials, supporter tokens and recovery codes, settings, encrypted-file records, transfer queues, folder mappings, shares, and synchronized preferences remain compatible with this update.

### Verification

- Frontend unit and coverage tests, production compilation, bundle ceilings, locale validation, and visual accessibility checks pass.
- Rust library tests, formatting, Clippy with warnings denied, supporter-service tests, Worker type checking, and the deployment dry-run pass.
- Release contract tests verify signed-artifact integrity, package-manager updater ownership, Arch packaging, SBOM generation, and protected supporter invariants.

## [3.7.0] - 2026-08-27

### Protected Media Streaming

- Added authenticated byte-range streaming for TDENC2-protected video, audio, and PDF content, allowing supported files to play or preview while the vault is unlocked without publishing a persistent plaintext copy.
- Added session-scoped media credentials that are revoked when the vault locks, plus independently authenticated chunk decryption that fails closed on truncated or modified ciphertext.
- Carried the protected-stream credential through the desktop media player, PDF viewer, and Android native player while preserving local-only stream URLs and bounded range responses.

### Video Uploads and Transfer Efficiency

- Added a saved File/Media upload preference for MP4-family videos. File mode remains the compatibility-first default, while Media mode records duration, orientation-aware dimensions, MIME type, and Telegram streaming metadata for inline playback.
- Preserved the video upload choice through durable desktop queues, remote uploads, Android shared-file intake, restart recovery, and encrypted settings synchronization.
- Reduced Android shared-file staging work by using an atomic move when cache and app storage share a filesystem, with a safe copy fallback for devices that separate those locations.

### Android Playback and Large-Library Performance

- Extended native Android streaming to audio as well as video, improved MIME-based media detection, added download fallback for both media types, and refreshed playback history immediately after the native player closes.
- Virtualized long mobile file lists while retaining selection, long-press actions, keyboard operation, dynamic row measurement, and a non-virtualized Android TV path.
- Replaced frequent Android polling with native environment and playback events plus slower recovery watchdogs, and throttled foreground-transfer notification updates.
- Added Baseline and Startup Profiles, localized native-player messages, release verification for profile packaging, and signed ABI-specific APK packaging alongside the universal artifacts.

### Release Engineering and Compatibility

- Expanded Android CI with protected behavior tests, Android 7.0 minimum-version instrumentation, current Android phone coverage, Google TV coverage, R8/JNI checks, 16 KB page-size verification, and ABI validation.
- Existing Telegram sessions, supporter entitlements, recovery codes, settings, encrypted-file records, transfer queues, folder mappings, and synchronized preferences remain compatible with this update.
- The v3.7.0 tag publishes signed Windows, Linux, macOS Intel, and macOS Apple Silicon desktop artifacts through the existing verified release pipeline.

### Verification

- All 125 frontend tests passed across 35 test files, along with the production frontend build and localization no-regression checks for all 24 language bundles.
- All 142 Rust library tests passed; Rust formatting and Clippy with warnings denied also passed.
- All 36 supporter-service tests, TypeScript validation, and the production Worker bundle dry-run passed.

## [3.6.0] - 2026-08-25

### Desktop File Listing Reliability

- Added a versioned local SQLite file inventory so previously scanned folders render immediately while Telegram reconciliation continues in the background.
- Correlated file-list chunks with their originating request and cooperatively cancelled stale scans, preventing older refreshes from clearing or overwriting newer results.
- Reduced initial chunk latency and preserved visible cached rows when Telegram is slow or temporarily unavailable.
- Kept the local inventory synchronized after file uploads, renames, moves, deletions, and activity-flag changes.

### Linux Startup and Refresh Performance

- Removed repeated focus-driven account rescans and added a six-hour folder-discovery freshness window with an explicit manual refresh path.
- Reused verified folder metadata and peer discovery results instead of repeating expensive full-channel requests on every startup.
- Added timing diagnostics for file scans, folder discovery, and transcode-cache reconciliation to make platform-specific performance measurable.

### Preferences and Transcode Cache

- Persisted file sort field and direction across restarts and synchronized them with the existing settings profile.
- Replaced synchronous transcode-cache traversal with an immediate snapshot backed by single-flight background reconciliation.
- Removed the misleading 12-second cache failure path, retained the last healthy snapshot during refresh, and made background cleanup safe around newly started transcodes.

### Compatibility and Verification

- Existing Telegram sessions, supporter entitlements, encrypted-file records, folder mappings, and synchronization settings remain compatible.
- All 114 frontend tests and all 139 Rust library tests passed.
- Production frontend compilation, Rust formatting, and release diff validation passed.

## [3.5.0] - 2026-08-24

### Release Overview

Telegram Drive 3.5 is a major desktop reliability, security, and media release. It moves critical transfer and application-lifecycle work into durable native services, strengthens file and network boundaries, and adds professional image-viewing and media-recovery behavior across Windows, macOS, and Linux.

### Desktop-Native Experience

- Added configurable tray and background operation so active transfers can continue after the main window is closed.
- Added native transfer notifications for completed, failed, paused, and attention-required work, with privacy-conscious message content, duplicate suppression, and per-category controls.
- Added tray transfer summaries and direct navigation back to the main workspace, transfer center, and settings.
- Added reliable explicit-quit handling, single-instance behavior, window restoration, and first-use guidance for background mode.
- Added suspend, resume, and network-recovery coordination so interrupted work is reevaluated when the device wakes or connectivity returns.
- Added native Linux Wayland detection while preserving explicit environment overrides and X11 compatibility.

### Durable Transfers and File Integrity

- Added a backend-owned SQLite transfer scheduler with durable queue ordering, independent upload and download concurrency limits, revision-safe state updates, and restart recovery.
- Preserved pause, resume, retry, cancel, cooldown, network-waiting, and vault-unlock states across desktop window and process lifecycle changes.
- Reworked downloads to use private temporary files, scoped bandwidth reservations, explicit verification, filesystem synchronization, and atomic destination replacement.
- Ensured failed or cancelled downloads release reserved quota and cannot leave a partially published destination file.
- Hardened URL uploads against server-side request forgery by validating schemes, credentials, DNS results, redirect destinations, private and reserved address ranges, and streamed response-size limits.
- Reworked temporary archive creation around an application-owned directory, unpredictable identifiers, an ownership registry, explicit symlink rejection, bounded file and byte limits, and fail-closed handling of unreadable entries.

### Security and Privacy Hardening

- Moved proxy passwords out of serialized settings and into the operating system credential manager, with safe migration of legacy values and redaction of proxy URLs and authentication logs.
- Tightened the production Content Security Policy by removing inline-script execution and keeping executable content restricted to approved application resources.
- Moved runtime SQLite work behind a centralized blocking boundary to prevent synchronous database access from stalling asynchronous services.
- Replaced periodic vault polling with an event-driven auto-lock deadline supervisor that responds to visible user activity without weakening lock behavior.
- Hardened REST, WebDAV, streaming, and local-service restart ownership with graceful shutdown, generation checks, and bounded port-conflict retries.
- Strengthened entitlement verification with fixed token headers, Ed25519 signature validation, device binding, ordered validity claims, and safe offline-grace handling.
- Hardened payment webhooks with strict header, certificate-host, timestamp, algorithm, signature, and raw-payload validation before any database mutation.
- Added independent, checksummed D1 backup exports to private object storage with a documented restore and disaster-recovery procedure.

### Media Playback and Image Viewing

- Fixed MP4 parsing failures caused by false box detection inside media payloads, including support for extended-size boxes and bounded validation of movie metadata.
- Improved adaptive playback recovery with native-video fallback, bounded startup handling, safer file changes, and clearer remux state transitions.
- Prevented the streaming-conversion overlay from remaining visible after playback has already started.
- Added desktop image-viewer controls for zoom in, zoom out, fit to window, actual size, zoom percentage, wheel zoom, panning, double-click zoom, and keyboard navigation.
- Preserved progressive thumbnail loading and full-resolution image replacement while zooming or navigating between files.

### Synchronization, Networking, and Platform Reliability

- Preserved folder-sync deletion guards, conflict handling, portable naming, atomic replacement, retry classification, and vault-aware behavior while moving database access onto safe asynchronous boundaries.
- Changed network keep-alive probes to resolve Telegram hostnames dynamically instead of depending on a fixed datacenter address.
- Isolated desktop capabilities so platform-specific permissions cannot leak into production desktop builds.
- Centralized application provider ownership and Query Client lifetime to prevent cross-mount state leakage and initialization-order ambiguity.
- Improved startup and shutdown ownership for streaming, synchronization, notification, and transfer services.

### Release Engineering and Documentation

- Expanded pull-request checks across Windows, macOS, and Linux with frontend tests, production compilation, Rust tests, formatting, linting, and packaging smoke coverage.
- Added tag-gated release preflight checks before any draft release or installer build can begin.
- Added operational runbooks for desktop signing-key backup, upgrade verification, service backup, and restore drills.
- Updated the product website and project documentation for the Windows, macOS, and Linux release model.

### Compatibility and Upgrade Notes

- Existing Telegram sessions, folders, settings, encrypted files, share links, synchronization mappings, and recovery material remain compatible.
- Legacy proxy credentials are migrated to secure operating system storage after the native backend confirms successful persistence.
- Some video formats still depend on platform codec availability or FFmpeg. The player now provides bounded fallback and recovery behavior when direct streaming is unavailable.

### Verification

- All 108 frontend tests passed across 32 test files.
- All 137 Rust library tests passed.
- All 36 supporter-service tests, TypeScript validation, and the production Worker bundle dry-run passed.
- All 6 Playwright visual-regression scenarios passed.
- Production TypeScript and frontend builds passed.
- Rust formatting, Clippy with warnings denied, and release-route validation passed.
- Locale structure, interpolation, generated-key, and no-regression literal-budget checks passed for all shipped language bundles.

## [3.0.0] - 2026-08-21

### Desktop Folder Sync

- Added opt-in bidirectional folder mappings between local directories and Telegram channels on Windows, macOS, and Linux.
- Added three-tree reconciliation using local, remote, and last-synced snapshots so incomplete remote scans cannot be mistaken for deletions.
- Added explicit conflict resolution, a greater-than-50-percent mass-deletion guard, duplicate-path protection, cross-platform path validation, and mapping overlap checks.
- Added atomic temporary-file downloads, an exact 2,000,000,000-byte upload cap, vault-aware queue pausing, and Telegram `FLOOD_WAIT` retry handling.

### Transfer and Supporter Reliability

- Hardened desktop transfer, preview, network-state, and encrypted upload handling around the new synchronization pipeline.
- Improved durable transfer-queue recovery across network interruptions and application restarts.
- Clarified supporter entitlement recovery and device-limit terms without changing feature access or pricing.

### Language Support

- Added the Folder Sync interface and status messages to all 24 shipped locale bundles, including right-to-left languages and regional variants.

### Product Website

- Added a responsive Folder Sync feature overview with the three-tree safety model, desktop platform scope, limits, and conflict behavior.
- Published the complete list of all 24 supported interface languages and updated navigation, metadata, and FAQ content.

### Verification

- Production frontend build and all 66 frontend unit tests passed.
- All 88 Rust library tests passed, including synchronization persistence, deletion protection, portable paths, transfer limits, and atomic replacement coverage.
- All 24 locale bundles passed structural and type validation; the supporter service passed TypeScript validation and all 16 tests.
- The product website passed responsive desktop/mobile, light/dark, internal-link, accessibility-attribute, and browser-console checks.

## [2.5.5] - 2026-08-20

### Sponsor Display

- Restored the live 300 × 250 sponsor creative through an isolated loopback frame with a validated, cached loader relay for desktop environments where the provider script cannot load directly.
- Starts the 10-second countdown only after the creative loads or the bounded fallback appears, preventing the sponsor placement from disappearing while it is still loading.
- Removed manual dismissal from the desktop placement while retaining automatic dismissal, the 45-minute return interval, secure external-link handling, and ad-free supporter suppression.

### Language Support

- Added complete selectable locale bundles for Ukrainian, Polish, Persian, Urdu, and Malay, increasing the application to 24 supported language choices.
- Added regional alias detection, locale-aware number and date formatting, plural validation, and localized core actions for all five languages.
- Added right-to-left document handling and UI regression coverage for Persian and Urdu.

### Product Website

- Added a persistent light theme and refined the mobile navigation controls.
- Balanced footer spacing and improved responsive presentation across the product page.

### Verification

- All localization structure, interpolation, plural, and type checks passed.
- All 51 frontend unit tests, all 78 Rust tests, and the production frontend build passed.

## [2.5.4] - 2026-08-18

### Sponsor Experience

- Replaced the unreliable embedded desktop creative with a consistent in-app sponsor card that retains the 10-second countdown and 45-minute return interval.
- Standardized desktop sponsor actions on the configured EffectiveCPMNetwork destination and kept browser fallbacks isolated with `noopener,noreferrer`.
- Removed the obsolete local advertisement relay and its external creative-loading dependencies.

### Language Support

- Added Bengali (Bangladesh), Thai (Thailand), Filipino (Philippines), and Traditional Chinese translations.
- Added regional locale detection, language selection, localized public-share password pages, and layout regression coverage for the new languages.

### Product Website

- Launched the redesigned GitHub Pages product site and corrected responsive screenshot sizing so every image preserves its original aspect ratio.

## [2.5.3] - 2026-08-16

- Applied maintenance hotfixes and stability improvements.
- Refined the user interface for a clearer, more polished experience.
- Streamlined the donation and lifetime ad-free supporter activation path.

## [2.5.2] - 2026-08-15

### Windows Video Playback

- Added clear Windows guidance when FFmpeg is unavailable, including a link to the official download page and instructions to add `ffmpeg.exe` to `PATH` before restarting Telegram Drive.
- Fixed the handoff from completed FFmpeg jobs to the built-in player by carrying local authentication through HLS playlists and segment requests.
- Improved WebView2 playback startup by attaching the media source before loading the HLS manifest and adding bounded network and media recovery.

### Transcode Cache Reliability

- Validates cached playlists and segments before marking a quality variant ready, preventing incomplete output from appearing with a misleading checkmark.
- Removes invalid cached variants and regenerates them when playback is retried.
- Allows stale failed, cancelled, or missing-output jobs to restart and preserves actionable transcode errors in the player.

### Verification

- Production frontend build and all 37 frontend tests passed.
- All 69 desktop Rust tests passed; the live advertisement network test remains intentionally ignored.
- Added route-level coverage for authenticated HLS playlist and segment delivery.

## [2.5.1] - 2026-08-14

### Sponsor Banner Reliability

- Restored sponsor artwork in the desktop banner when an embedded WebView does not reliably paint the provider’s nested frame.
- Confirms the 300 × 250 creative image has loaded before displaying it, while retaining the isolated sandbox, popup blocking, and `unsafe-eval` protection.
- Keeps the existing 10-second countdown, manual dismissal, and 45-minute return interval unchanged.

### Encrypted Settings Sync

- Added opt-in, manual cross-device settings sync through the user’s own Telegram Saved Messages with no Telegram Drive sync server.
- Protects synced preferences with Argon2id and authenticated XChaCha20-Poly1305 encryption using a user-provided passphrase that is never stored or uploaded.
- Restricts sync to an explicit allowlist of portable preferences. Credentials, API and WebDAV keys, proxy details, supporter activation, crash consent, and file data are never included.
- Added confirmation before replacing a backup from another device or applying downloaded preferences.

### Offline Files and Windows Cache Reliability

- Made recently viewed plaintext preview files use true least-recently-used retention, so actively reopened files are not evicted ahead of stale files.
- Added actionable offline-cache inspection and cleanup errors instead of leaving Storage settings in an indefinite loading state.
- Fixed Windows Transcode Cache commands to use the application’s managed shared cache state consistently.

### Japanese Language Support

- Completed the Japanese application translation and added Japanese locale registration, resolution, fallback, and CJK visual regression coverage.
- Strengthened localization validation with locale-specific invariant handling and zero copied-English baseline debt for Japanese, Italian, and Vietnamese.

### Supporter Experience

- Ensures verified ad-free users never begin loading sponsor content while entitlement status is being checked.
- Licensed users skip supporter promotion pages. Unlicensed desktop users receive a clear optional $5 lifetime ad-free offer at most once on app launch every 24 hours.
- Clarified throughout the application and supporter terms that every feature remains free and the supporter license only removes sponsor placements.
- Preserves the existing activation, recovery-code, device-limit, update, and refund/reversal safeguards.

### Verification

- Production frontend build and complete frontend regression suite passed.
- Complete desktop Rust test suite passed with only the existing live advertisement network test ignored.
- Localization structure, variables, invariant, and type validation passed.

## [2.4.0] - 2026-08-11

### Sponsor Display

- Restored the desktop sponsor creative and its visible 10-second countdown.
- Pauses the countdown while the pointer is over the sponsor, supports manual dismissal, and brings the placement back after 45 minutes.
- Keeps sponsor content inside an opaque-origin local sandbox, blocks popups and access to other localhost routes, and falls back to a clear sponsor link when the network or provider blocks the image.

### Verified Ad-Free Supporter Access

- Replaced the self-declared ad-free switch with a verified $5 USD one-time PayPal checkout for up to three desktop devices.
- Added payment-status polling, recovery-code restoration, signed device-bound entitlements, 30-day verification tokens, and a seven-day offline grace period.
- Stores the device key and recovery code in macOS Keychain, Windows Credential Manager, or Linux Secret Service using a stable credential name, so normal application updates do not require reactivation.
- Added automatic verification refresh on startup. Confirmed PayPal refunds and reversals revoke the matching ad-free entitlement when the app reconnects.
- Added a first-run introduction to the optional supporter feature and a complete activation/recovery surface in Settings → Privacy.

### Payment Privacy and Terms

- Added a minimal Cloudflare Worker and D1 verification service that stores PayPal transaction IDs, entitlement state, terms acceptance, and cryptographic device identifiers without requesting or storing purchaser email addresses.
- Added versioned Supporter Terms with clear activation, device-limit, recovery, availability, refund, reversal, chargeback, and dispute disclosures.
- Requires the purchaser to accept the current terms before checkout or recovery activation. Refunds are not described as automatic or guaranteed except where applicable law requires otherwise.
- Clarified that direct tips and cryptocurrency donations do not activate ad-free access.

### Verification

- Cloudflare Worker TypeScript checks, unit tests, and the Wrangler production-bundle dry run passed.
- PayPal sandbox checkout, immediate return verification, signed activation, second-device recovery, entitlement refresh, refund webhook, and post-refund denial passed end to end.
- Production frontend build, frontend regression tests, the live ad-loader check, and the complete desktop Rust test suite passed.

## [2.3.0] - 2026-08-10

### Desktop Workspace

- Added Recents, Favorites, Pinned, Offline, Large Files, Old Files, and Duplicate Files views with safer cross-folder behavior.
- Added advanced search filters, deterministic fuzzy ranking, folder-sync progress, keyboard shortcut help, an onboarding tour, and an in-app help center.
- Added pause, resume, retry, cancellation, and persistent recovery behavior for upload and download queues.
- Added clearer startup progress, post-update release highlights, Telegram cooldown status, and session-recovery feedback.

### Authentication and Settings

- Hardened phone-number authentication with normalized E.164 input, resend and cancellation support, delivery-method guidance, stale-attempt protection, and QR fallback while preserving QR login.
- Reorganized Settings into focused categories with dedicated Privacy and Advanced surfaces, reusable accessible controls, and persisted settings/theme adapters.
- Added opt-in, crash-only diagnostics with a privacy-safe payload, explicit consent, and no file names, paths, credentials, messages, or contents.

### Storage and Cache Reliability

- Fixed Transcode Cache remaining indefinitely on “Loading” after a Windows filesystem or native-command failure.
- Added explicit loading, empty, error, timeout, retry, and recovery states while keeping Clear All available when inspection fails.
- Moved transcode cache scanning and deletion off the asynchronous runtime, made deletion failures actionable, and blocked cache path traversal.
- Added bounded image-cache policy, offline-cache visibility, and consistent search/cache behavior.

### Database Safety

- Added an application-wide, checksummed SQLite migration ledger without rewriting existing user records.
- Added recognition for released database layouts, downgrade protection, integrity checks, transactional baselining, and safe rejection of unknown or partial schemas.
- Added verified pre-migration recovery backups with restrictive permissions before upgrading an existing database.

### Architecture and Verification

- Split shared utilities, domain types, authentication views, settings controls, sidebar actions, toolbar coordination, file-action policy, and persistence policy into focused modules while preserving existing APIs.
- Added frontend regression coverage for authentication, settings persistence, sidebar actions, transfer queues, cache policies, file actions, and Transcode Cache failures.
- Production frontend build, TypeScript validation, frontend tests, Rust tests, and desktop integration tests passed.

## [2.2.7] - 2026-08-04

### Windows Build Experience

- Fixed `npm run tauri dev` failing when `resources/vc_redist.x64.exe` had not been downloaded.
- Separated development-safe Windows settings from release-only NSIS resources and installer hooks.
- Made the standard `npm run tauri build` command automatically prepare and validate Microsoft's signed Visual C++ Redistributable on Windows.
- Kept the Visual C++ runtime and embedded WebView2 bootstrapper in the finished Windows installer for reliable startup on clean Windows systems.

### Verification

- Production frontend build passed.
- Desktop Rust tests passed.
- The tagged release workflow prepares and verifies the Windows runtime on a native Windows runner before publishing the NSIS installer.

## [2.2.6] - 2026-08-04

### Windows Startup Reliability

- Added the signed Microsoft Visual C++ 2015–2022 x64 Redistributable to the Windows NSIS installer so fresh Windows installations have `MSVCP140.dll` and the matching runtime libraries before Telegram Drive starts.
- Added silent prerequisite detection and installation with explicit failure handling and reboot-required support.
- Embedded the WebView2 bootstrapper for Windows systems where the Evergreen WebView2 Runtime is unavailable.
- Added release-time checks for the redistributable's Microsoft Authenticode signature, its presence inside the finished NSIS installer, and the built executable's PE dependencies.
- Restricted Windows release artifacts to the verified NSIS installer so an installer without the prerequisite cannot be published accidentally.

### Desktop Advertising

- Fixed blank desktop advertisement panels by relaying and validating the configured advertisement loader through the local desktop server.
- Added resilient DNS recovery, short-term loader caching, and a clear sponsored-content fallback when the service is blocked or unavailable.

### Windows Folder Sidebar

- Fixed folder option and right-click menus opening without allowing actions to be selected in Windows WebView2.
- Isolated sidebar menu pointer events from drag-and-drop handling and rendered the menu outside the draggable folder element.

### Verification

- Production frontend build passed.
- Desktop Rust tests passed.
- The tagged release workflow verifies and compiles the Windows installer on a native Windows runner before publishing.

## [Unreleased] - 2026-08-02

### Vietnamese Language Support

- Added complete Vietnamese (`vi`) translations across the application, settings, dialogs, notifications, authentication, encryption, sharing, and WebDAV surfaces.
- Added Vietnamese to desktop and mobile language selectors with automatic `vi` and `vi-VN` system-language detection and locale-aware number and date formatting.
- Added Vietnamese localization for password-protected public share pages.

### File Size Metadata

- Fixed Telegram photos being reported as `0 KB` in folder listings, REST responses, storage statistics, streaming headers, and Windows clients by consistently using the largest available photo representation size.
- Aligned file-size metadata across the application, downloads, previews, public sharing, and WebDAV while safely handling invalid negative values from remote metadata.

### Security

- Hardened ad-link fallback windows with `noopener,noreferrer` to prevent opened ad pages from accessing the application window through `window.opener`.

## [2.2.5] - 2026-08-03

### Highlights

- Added complete Vietnamese language support across desktop, mobile, settings, sharing, authentication, encryption, and WebDAV.
- Added automatic Vietnamese system-locale detection and localized number/date formatting.
- Added Vietnamese localization for password-protected share pages.

### Fixes

- Fixed photos being displayed as `0 KB` in folder listings, REST responses, storage statistics, streaming headers, downloads, previews, and WebDAV clients.
- Standardized file-size metadata using Telegram’s largest available photo representation.
- Improved handling of invalid remote size metadata.

### Security

- Added `noopener,noreferrer` to ad-link `window.open` fallbacks to prevent tabnabbing through `window.opener`.

### Verification

- Production frontend build passed.
- Full Rust test suite passed.
- Localization validation passed.

## [2.2.0] - 2026-07-29

### WebDAV Access

- Added a local WebDAV server for browsing Telegram Drive in macOS Finder, Windows File Explorer, and Linux WebDAV clients.
- Added a WebDAV tab in Settings with server status, custom port selection, read-only-by-default access, and optional write access.
- Added private connection links with secure token regeneration.
- Supports Saved Messages and Telegram Drive folders, directory browsing, range downloads, uploads, rename, move, copy, and delete for unencrypted files when write access is enabled.
- Added port conflict checks, staged upload cleanup, client-lock support, and macOS Finder compatibility fixes.
- Added WebDAV setup and troubleshooting documentation.

### Desktop File Handling & Interface

- Added external file drag-and-drop uploads from the desktop or file manager into the currently open folder.
- Moved WebDAV in Settings to sit between VPN and Encryption.
- Improved spacing and hit areas in folder action menus and the Grid/List layout picker.

### Italian Language Support & Build Fixes

- Added complete Italian (`it`) language support across all application surfaces, settings, dialogs, and error messages.
- Resolved build warnings and runtime layout/formatting issues.

### Verification

- Verified production frontend build, localization validation, Rust compilation, and WebDAV regression tests.

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
