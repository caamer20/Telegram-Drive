# Desktop 4.2.0 release verification

## Dependency review

Codex reviewed the public desktop lockfile on 2026-11-25. Its SHA-256 was `25a6d507d7cd43e35d10fb0d0e79f50f51848c56a39fbb712ed3d7fe74ef05fd`. With RustSec revision `87025f60dbbb1f8820c2bd068cee9140244d4dc9`, cargo-deny 0.18.7 passed advisories, licenses, bans and sources with both default and all features. The configured supporter-service npm advisory and license checks, workflow pins and exception expiry checks passed locally.

The existing exceptions are retained through 2026-12-30. Removing the exceptions in a disposable scan reproduced the five advisory IDs and the two previously accepted yanked versions. [Exception register](../dependency-policy/rust-advisory-baseline.json)

| Locked dependency | Release decision and follow-up |
| --- | --- |
| GTK 0.18.2 → glib 0.18.5, RUSTSEC-2024-0429 | The application does not call `VariantStrIter`. The fix starts at glib 0.20.0, outside GTK's 0.18 requirement. Reassess with the Linux GUI stack. |
| actix-http 3.21.3 → h2 0.3.27, RUSTSEC-2026-0258 | The local services use plain Actix listeners. The fix starts at h2 0.4.16. The next actix-http release still requires h2 0.3.27-compatible versions. |
| dav-server 0.11.0 → lru 0.16.4, RUSTSEC-2026-0253 | The DAV filesystem cache uses path keys. The fix starts at lru 0.18.2. Even dav-server 0.12.0 still selects lru 0.16. |
| plist 1.9.0 and wayland-scanner 0.31.10 → quick-xml 0.39.4, RUSTSEC-2026-0194 and RUSTSEC-2026-0195 | These paths process application metadata and build-time protocol XML. Updating plist to 1.10.4 and wayland-scanner to 0.31.11 together resolved to patched quick-xml 0.41.0 and 0.42.0 in a disposable lockfile. Validate packaging and Linux builds before adopting it. |
| rand 0.10.1 → chacha20 0.10.0 and grammers-crypto 0.8.0 → glass_pumpkin 1.10.0, both yanked | Disposable lockfiles resolved to chacha20 0.10.2 and glass_pumpkin 1.9.1 respectively. Exercise native transfers, Telegram authentication and session restoration before adopting the changes. |

Two additional yank warnings concern async-executor 1.14.0 and clipboard-win 5.4.1. Candidate versions 1.14.1 and 5.4.3 resolved separately. All alternative lockfiles received resolution checks only. The baseline and expiry were unchanged.

The hosted release runs the dependency-assurance workflow again on the tagged source. The preceding public candidate passed its hosted check on 2026-11-24. [Previous hosted result](https://github.com/caamer20/Telegram-Drive/actions/runs/45368418358) · [v4.2.0 release run](https://github.com/caamer20/Telegram-Drive/actions/workflows/release.yml?query=branch%3Av4.2.0)

## Supporter and update compatibility

The release preflight checked the configured production service over HTTPS. The public entitlement key matched the release variable. The service reported USD 5.00, three devices and terms version `2026-08-11`. The application identifier, updater public key and endpoints, and the secure-storage identifiers for the device key, recovery code and checkout claim match v4.1.7.

The public desktop candidate changes the recovery-code receipt shown after activation. The production Worker source, database migrations and service configuration have no changes in this release. The local supporter suite passed all 27 HTTP and D1 journeys using simulated PayPal responses. Worker type checking, the deployment dry run and all 15 release-tool journeys also passed. The native app accepted a token issued by the local Worker for its bound device, enforced the active and offline-grace windows, and accepted a refreshed token for the same purchase. These runs used disposable credentials and simulated PayPal responses.

## Desktop acceptance

During release preparation, all 210 browser journeys passed locally with zero retries or skips. The channel guide and saved-proxy recovery flow also passed against the compiled frontend. All 24 language catalogs passed validation and the production assets met the existing bundle budgets. The full native suite passed all 247 journeys at the four-test concurrency used in CI. A separate run under concurrent build load timed out on the 50,000-entry WebDAV response.

A later upgrade regression reproduced two conflicting saved proxy credentials in both the browser and the native child process. The extended journeys passed after the fix. They cover restart, an unrelated preference edit, preservation of both recoverable values and deliberate replacement or clearing. The hosted desktop and end-to-end workflows record complete results for the pushed revision. [Desktop builds and packaged smoke tests](https://github.com/caamer20/Telegram-Drive/actions/workflows/desktop-sync-ci.yml) · [End-to-end and bundle checks](https://github.com/caamer20/Telegram-Drive/actions/workflows/quality-assurance.yml)

A PayPal sandbox transaction and an installed upgrade of a previously activated purchaser device have no recorded result for this candidate. The service and browser fixtures simulate external payment and device boundaries.
