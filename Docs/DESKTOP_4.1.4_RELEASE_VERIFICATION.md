# Desktop 4.1.4 release verification

## Scope

The candidate starts from published main `19e11aa00e57f9191f46b713ed6a619936a4da30` (desktop 4.1.3). Only the Folder Sync readiness recovery and content-sized folder header from this task are selectively ported from the mixed development checkout. Published incremental scanning, cancellation, protected remote paths, authentication/logout protections, upload/streaming behavior, and existing E2E coverage are preserved. The original mixed checkout and the separate 4.1.3 worktree remain untouched.

Android source, project, test, configuration and generated files are excluded from the outgoing changes. No Android release, Worker deployment, payment operation or credential/signing-key change is part of this release. Shared authentication additions are limited to the desktop readiness lifecycle and do not port the mixed checkout's private Android changes.

## Frontend performance review

The locked frontend dependencies and packed locale loading remain unchanged. One translated waiting label is added to all 24 catalogs. The badge distinguishes waiting from synced, and the header no longer reserves an invisible selection row.

| Measure | Published 4.1.3 | 4.1.4 candidate | Change |
| --- | ---: | ---: | ---: |
| Initial JavaScript | 503,071 | 503,071 | 0 |
| Desktop route JavaScript | 310,077 | 310,269 | +192 |
| Total JavaScript | 2,464,062 | 2,464,336 | +274 |
| CSS | 138,491 | 138,491 | 0 |
| Canonical locale data | 1,969,806 | 1,970,576 | +770 |
| Largest locale catalog | 139,976 | 140,073 | +97 |

Only the desktop-route and two locale ceilings move to these exact reviewed measurements. No additional headroom is added; startup, total JavaScript, CSS and every feature ceiling remain unchanged. These are size measurements, not a runtime performance benchmark.

## Native dependency security

The required advisory gate identified RUSTSEC-2026-0443, RUSTSEC-2026-0446, RUSTSEC-2026-0447, RUSTSEC-2026-0448 and RUSTSEC-2026-0455 in the published Actix dependencies. The candidate upgrades actix-web to 4.18.2, actix-files to 0.7.4 and actix-http to 3.20.3 without advisory exemptions. The pinned Rust 1.92 toolchain is retained. The file library removed `NamedFile::open_async`; the fMP4 route now opens with Tokio and passes the handle to `NamedFile::from_file`, retaining path checks, account revalidation, MIME type and byte-range streaming. The native HTTP journey verifies the streamed range bytes and revocation after account replacement. The patched response type also crosses Clippy's large-error threshold; five private REST helper errors are boxed and unboxed at response boundaries without changing status codes, headers, bodies or authentication order. No lint or advisory exemption is added.

## Completed local verification

- Browser: 174 passed, zero failed or skipped. Header screenshots at 1920px and 1000px were inspected without excess space or clipped controls; existing visual baselines were retained.
- Native process/storage/HTTP: 226 passed, zero failed or ignored. All eight new lifecycle journeys cover full and incremental scanners; the released incremental startup journey first failed with the reported paused/offline state before the fix. Ten matching sync/protected-transfer journeys and the authenticated fMP4 range journey also passed in the focused run.
- After the mechanical REST error-boxing compatibility adjustment, all four matching REST/catalog/thumbnail journeys passed again. Final strict Clippy, formatting and the normal desktop build passed on Rust 1.92.0.
- Production frontend TypeScript/Vite build, every reviewed bundle ceiling, localization structure/literal checks, native line-format policy, E2E-only policy, security configuration and pinned workflow checks passed.
- Supporter service: 27 local HTTP/D1 journeys, TypeScript and Wrangler deployment dry run passed. One Worker-issued entitlement journey passed against the native application. Production read-only health matched the existing public build key, $5.00 USD price and three-device allowance. No Worker deployment, production payment or D1 mutation was performed.
- Rust dependency advisories, licenses, bans and sources passed after the Actix patches. Required service npm advisory/license checks passed. The published app npm assurance policy is unchanged.
- Release-assurance CLI: 14 passed. Packaged-startup tooling: four Unix journeys passed; four Windows-only journeys were skipped on this macOS host and remain CI requirements. A normal Tauri debug build and the macOS package-content journey passed; native test executables were excluded from the application package.
- Release tag/version policy, source formatting and workflow YAML validation passed. The outgoing commit audit must be rerun against the final commit before pushing; no private Android source or project changes are included.

Browser tests control Tauri/Telegram boundaries. Native tests use child processes, real SQLite/filesystem/HTTP, and controlled external Telegram readiness/tree/transport boundaries. The new readiness journeys cover an observed unavailable connection and successful recovery; they do not classify every possible mid-scan transport failure. Service tests use local workerd/D1 and a controlled PayPal boundary. These results do not claim live Telegram/PayPal or manual installed-device acceptance.

Cross-platform release packages, installed startup checks, existing signing keys, checksums, SBOMs and provenance remain mandatory GitHub publication gates. The release workflow creates a draft and publishes it only after those gates succeed.
