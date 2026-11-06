# Desktop 4.1.3 release verification

## Scope

This release starts from public main `cdf799a` (desktop 4.1.2 plus its published download-page change). Desktop interface/motion and failed-upload batch retry changes are selectively ported from the local development checkout. Existing upload thumbnails, MP4 streaming metadata, filename captions, localization, account isolation, encrypted-file recovery and lifetime supporter compatibility remain on their published implementation. Android project/source/configuration and Android-only viewer controls are excluded. The original mixed checkout is preserved.

## Frontend performance review

Both the published baseline and release candidate were built on the same host using the locked frontend dependencies for each release. The candidate also aligns Tauri API/updater packages with the patched native versions. No locale loading or catalog-packing changes, production test-fixture inclusion, new UI dependency, or additional startup request is introduced.

| Measure | 4.1.2 baseline | 4.1.3 candidate | Change |
| --- | ---: | ---: | ---: |
| Initial JavaScript | 504,619 | 503,071 | -1,548 |
| Desktop route JavaScript | 301,789 | 310,077 | +8,288 |
| Desktop route gzip / Brotli | 86,903 / 73,780 | 90,010 / 76,458 | +3,107 / +2,678 |
| All JavaScript | 2,452,344 | 2,464,062 | +11,718 |
| All JavaScript gzip / Brotli | 731,722 / 622,332 | 736,200 / 626,330 | +4,478 / +3,998 |
| CSS | 137,386 | 138,491 | +1,105 |
| Canonical locale assets | 1,961,436 | 1,969,806 | +8,370 |
| Locale gzip / Brotli | 488,572 / 427,835 | 490,795 / 429,772 | +2,223 / +1,937 |
| Largest locale asset | 139,401 | 139,976 | +575 |

The desktop route grows 2.75% for the revised Files/Transfers controls, truthful progress, bulk-retry guards and reporting. All JavaScript grows 0.48%; startup JavaScript shrinks. Settings remains lazy and grows from 120,591 to 125,001 bytes inside its existing 159,000-byte cap. Media/PDF feature sizes are unchanged. Extracting only the retry loop would retain its caller state/account guards and add an asynchronous loading/error state to a small action; the reviewed increase keeps that operation directly available. Existing startup, total JavaScript, largest-chunk, CSS and feature budgets remain unchanged. Only desktop route and locale caps are set to the exact measured candidate values, with no extra allowance. Nine new labels across all 24 catalogs cover category search, progress/retry outcomes and two previously missing color labels; system-theme wording is corrected in four catalogs. These measurements establish bundle-size impact, not runtime responsiveness.

## Dependency security

The current advisory gate required patched Rust TLS, Tokio, Zstd and ZIP dependencies. ZIP 9.4.1 and Tauri updater 2.14.1 replace vulnerable extraction dependencies; the updater brings Tauri 2.12.5, compatible with the pinned Rust 1.92 toolchain. JavaScript API 2.12.4, updater 2.14.1 and CLI 2.12.4 are aligned with the native package family. ZIP filename decoding now propagates invalid-name errors, and the corruption fixture explicitly requires its known data offset. Extraction bounds, sanitization, CRC checks, cleanup and account validation remain intact.

The existing local supporter-tooling Undici override changes from 7.29.1 to patched 7.30.1. Worker payment logic, schema, signing material, secure-storage identifiers, price, device allowance, refresh and recovery behavior are unchanged. No new advisory exemption is introduced. Production Worker deployment is outside this release.

## Evidence and limitations

Completed local verification:

- Full browser suite: 169 passed, with no skipped or retried tests. All 39 targeted integration/port regressions also passed. Seven rendered desktop screenshots at 1000×700 and 1440×1000 were reviewed without overflow or unreachable controls; visual snapshot baselines are unchanged.
- Full native process/storage/HTTP suite: 218 passed, zero failed or ignored. The three new strict-retry journeys first failed before implementation and passed afterward; their final rerun passed after fixture formatting. Existing ZIP CRC/limits/extraction, video metadata, account-isolation and recovery journeys remain covered.
- Native Clippy with warnings denied, rustfmt, native line-format policy, TypeScript, locale validation/literal checks, E2E-only policy, app security and workflow protections passed.
- Production frontend build and every reviewed bundle limit passed.
- Supporter service: 27 local HTTP/D1 journeys, TypeScript and Wrangler deployment dry run passed with the patched tooling. One service-issued token journey passed against the native application. Production health matched the release public key, $5.00 USD price and three-device allowance.
- Rust dependency advisories/licenses/bans/sources and the required service npm advisory/license checks passed without new exemptions. The repository's existing disabled app npm assurance policy is unchanged.
- Release-assurance CLI: 14 passed. Packaged-startup runner: four Unix journeys passed; four Windows-only journeys were skipped on this macOS host and remain CI requirements. The macOS package-content journey passed and excluded native test executables.
- A normal Tauri debug build and cross-platform GitHub publication gates are verified separately; local fixture results do not certify a published release.

 Browser journeys use the application with controlled Tauri/Telegram boundaries. Native journeys use child processes, real local storage and HTTP; external Telegram transport is controlled. Service journeys use local HTTP/workerd/D1 with a simulated PayPal transport. These results do not claim live Telegram/PayPal or manual installed-device acceptance. Cross-platform packaging, installed startup checks, checksums and provenance remain publication gates.
