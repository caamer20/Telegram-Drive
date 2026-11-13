# Desktop 4.1.5 release verification

## Scope and base

This release is based on published desktop v4.1.4, commit
`6f804fef266f70a812ca26df886220d07f503661`. It includes issue #247's upload preference,
desktop regression journeys, translated labels, reviewed bundle ceilings,
4.1.5 version/download/changelog metadata, and the narrow dependency security
remediation required by the release gate. The original mixed
desktop/Android development checkout is retained separately and is not committed
or merged into this release.

Preferences → Encryption offers **Ask every time**, **Always Standard**, and
**Always Encrypted**. Ask remains the upgrade and reset default. The preference
is local to the installation; encrypted settings sync keeps its existing field
contract. File selection, drops, zipped folders, and URL uploads use the same
captured encryption mode and metadata preference. Required capability checks,
vault creation/unlock, recovery guidance, and the published masked passphrase
dialogs remain in place. A canceled or failed encrypted preparation cannot fall
back to Standard. Account changes cancel pending choices, overlapping requests
supersede their predecessors, and retries retain the queued protection intent.

The port preserves v4.1.4's packed locale pipeline and published question/description
translations. It adds eight keys in each of the 24 canonical language catalogs.
Encryption formats, credential identifiers, supporter keys, entitlement rights,
and release workflows retain their established contracts. The $5.00 USD one-time lifetime supporter
contract and recovery/device allowance remain unchanged.

## Release dependency security remediation

The release's current RustSec scan rejected the previously locked Hickory 0.25.2
resolver and rustix 1.1.4. The precise rustix update is 1.1.6. For Hickory, the
maintained upstream Grammers release is 0.12 and changes protocol/session crates;
there is no 0.8 maintenance release carrying the fix. This patch therefore vendors
only the already-pinned `grammers-mtsender` 0.8.1 and adapts its proxy resolver to
Hickory 0.26.3. All sibling Grammers source revisions are retained.

The [backport provenance](../app/src-tauri/vendor/grammers-mtsender/PROVENANCE.md)
records source commits, both projects' licenses, original and final file hashes,
the exact patch, omitted upstream unit suites, resolver-default comparisons, and
the platform audit. The adapter preserves IPv4-first selection, negative-response
failover, Windows EDNS defaults, Unix configured EDNS, and macOS resolver-file
handling. Cache units changed to responses; its conservative bound is documented,
as is the new Linux parser's tolerance of malformed configuration lines.

The two new native child-process journeys use actual Grammers connections and a
loopback SOCKS peer, covering names/literal IPv4/IPv6, authentication, restart,
bounded failures, credential-free errors, and recovery. Localhost coverage tests
system/hosts resolution and does not claim adversarial DNS-wire coverage.

The final dependency scan passes advisories, bans, licenses, and sources without
new exceptions. Resolved exceptions `RUSTSEC-2026-0118` and `RUSTSEC-2026-0119`
were removed; remaining exceptions and their expiry were retained. Compatible
Windows dependency selections were preserved from the prior lockfile, and
`cargo metadata --locked --all-features` validates the final graph.

The source SBOM identifies the retained mtsender base version and patched Hickory
versions. Its top-level commit and signed release provenance bind the committed
backport/hash manifest; the bare mtsender Cargo purl alone does not identify the
downstream patch. No SBOM or release gate was relaxed.

## Performance review

The architect approved these exact measured ceiling increases on 2026-11-13:

| Budget | Previous ceiling | New measured ceiling | Increase |
| --- | ---: | ---: | ---: |
| Total canonical locale data | 1,970,576 | 1,986,968 | 16,392 bytes (0.832%) |
| Largest locale catalog | 140,073 | 141,254 | 1,181 bytes (0.843%) |
| Desktop dashboard route | 310,269 | 310,439 | 170 bytes (0.055%) |

Initial JavaScript is 504,542 bytes, below the unchanged 513,000-byte ceiling.
Total JavaScript is 2,466,719 bytes; CSS is 138,491 bytes. The packed locale key
dictionary and lazy protection dialog remain in place. All other JavaScript,
CSS, and feature ceilings are unchanged. Measurements are emitted by
`npm run build:verify --prefix app` in `app/dist/bundle-report.json`.

## Verification boundaries

The desktop upload regression failed on this release base before the runtime
port: Always Standard still opened the choice dialog and queued no upload.
The ported browser journeys use the shipped App/provider tree and native IPC
fixtures, including the published masked passphrase UI. They verify queued
intent and application behavior, not live Telegram encryption or network uploads.

The read-only release preflight verified matching 4.1.5 versions, changelog,
download links, required release inputs, and production supporter health with
price `5.00`, currency `USD`, device allowance `3`, and a public key exactly equal
to the unchanged GitHub release variable. No production mutation was performed.

Android source, projects, instrumentation, and local mobile-only changes are
excluded. The existing publication guard reports unchanged legacy pending paths;
that warning is not publication approval for those paths. Outgoing commits must
be reviewed separately, including shared-file hunks.

The existing tag workflow retains dependency assurance, browser/native/service
E2E, formatting/lint, desktop package checks, platform builds, packaged startup,
signing, checksums, SBOMs, and provenance gates before publishing its draft.
Manual installed Windows 10 and live Telegram acceptance are separate and are
not claimed by local fixtures. Android device acceptance is outside this desktop
release.

## Local results on the release source

- Full browser E2E: **201 passed**, including 27 new upload-default journeys.
  The new cases cover saved/default/invalid/reset preferences, settings-read and
  save failures, encrypted readiness and vault setup/unlock, masked per-file
  passphrases, requests pending during hydration, overlapping requests, account
  switches, retry protection, and file/drop/folder/URL entry points. Existing
  visual baselines were retained.
- Final native process/HTTP/storage E2E: **228 passed, 0 failed, 0 ignored** after
  dependency remediation, including both new Grammers proxy journeys. The run used
  Rust 1.92.0 and `RUST_TEST_THREADS=4` as in CI. An older shared SQLite build
  artifact first failed the intentional FTS5 probe; rebuilding only that cached
  dependency with the existing release configuration resolved the cache mismatch
  without source changes. Rust formatting and Clippy with warnings denied passed
  against the final dependency graph.
- Desktop package E2E: **1 passed**, confirming native E2E executables are excluded.
  Worker-issued supporter-token integration: **1 passed**, using generated test
  signing keys and local D1, with no production services or credentials.
- Release assurance CLI E2E: **14 passed**. Final Cargo dependency checks passed
  advisories, bans, licenses, and sources after removal of the two resolved
  exceptions described above.
- Supporter-service typecheck, **27 HTTP/D1 E2E journeys**, and Wrangler
  deployment dry run passed. Only the controlled local service ran; no Worker
  was deployed and no real payment was made.
- Final TypeScript/production build and all reviewed bundle ceilings passed.
  Browser localization tests build their own production assets, so the final
  bundle measurement ran after that suite finished.
- Localization validation/literal scan, workflow-pin protections, Rust formatting,
  E2E-only policy, security configuration, version/tag policy, and scoped
  whitespace checks passed. The exact upstream Hickory Apache license and unified
  patch context retain their original blank-line bytes and are excluded from the
  whitespace-only check; their hashes are verified by the vendor manifest.
  Existing localization allowances were retained.
- Supporter-service npm advisory/license checks passed against the reviewed
  policy; its temporary baseline remains valid through 2026-12-30.
- The read-only production supporter health and unchanged release public-key
  comparison passed. Packaged-platform and final GitHub artifact/provenance
  verification remain distinct gates.

## CI follow-up after the immutable v4.1.5 tag

The first GitHub release attempt on tagged commit
`225c884f46d50e42b441bbb198d333a873535ce3` passed 200 browser journeys and reported
one flaky existing system-motion journey. Its initial attempt missed the short
General-pane animation; the test retry passed. The unchanged `failOnFlakyTests`
gate correctly stopped release creation. The failed release job was rerun once
on the same immutable tag with all checks retained.

A separate main-branch follow-up stabilizes that E2E precondition by capturing
and slowing the real General-pane browser animation when it is created. The
original browser method is restored in `finally`; the active-animation,
reduced-motion, saved-preference, and stale-pane assertions remain in place.
Both system/manual cases passed ten repeats each (**20 passed**) and the full
motion file passed (**7 passed**), with retries disabled. This test-only follow-up
is not part of the v4.1.5 tagged source and does not change the shipped runtime.
