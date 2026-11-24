# Desktop 4.1.7 release verification

## Scope

This desktop release includes the video-upload decoder fallback and Windows
process-monitor fixes for issue #249, plus WebDAV root-cache retention across
incomplete listings, folder identity changes and delayed publication. Original
video bytes, encryption, cancellation, quota accounting and account isolation
remain protected by the existing application-boundary journeys.

The release security gate also required Actix HTTP 3.21.3 for
RUSTSEC-2026-0552/0553/0554 and replacement of direct Chrono use after
RUSTSEC-2026-0543 marked it unmaintained. Direct application date operations now
use Jiff 0.2.28; the existing Telegram and DAV dependencies retain transitive
Chrono under the unchanged workspace-only policy. No advisory exception was
added. Unrelated Windows dependency selections were retained and the complete
lockfile was validated with `cargo metadata --locked`.

The time migration preserves persisted epoch units and timestamp formats,
documented API date filters, local Monday quota boundaries, and local scheduling.
Windows uses its native per-year OS rules, including custom or disabled DST;
Unix retains process/system timezone selection. Compatibility is tested at HTTP
and persistent-storage boundaries, not asserted for every historical parser
spelling outside Jiff's supported calendar.

## Local verification

- Browser E2E: **202 passed**, with existing visual baselines retained.
- Final native process/HTTP/storage E2E: **244 passed, 0 failed, 0 ignored** on
  macOS using Rust 1.92.0 and four test threads. This run includes the final time
  compatibility fix and observed HTTP-pacer readiness checks. Windows-only
  execution remains a separate CI requirement.
- TypeScript, production frontend and all existing bundle ceilings: **passed**.
- Supporter service: typecheck, **27 local HTTP/workerd/D1 E2E journeys**, npm
  advisory/license policy and Wrangler deployment dry run: **passed**. PayPal is
  a controlled transport; no production Worker or database was modified.
- Production supporter health and exact release-public-key comparison: **passed**,
  retaining the one-time **$5.00 USD** lifetime promise and three-device allowance.
- Release assurance CLI: **14 passed**. Packaged-startup CLI fixtures: **4 passed**
  on macOS, with **4 Windows-only cases unrun locally**.
- Desktop package contents: **1 passed**, excluding native test executables.
  Worker-issued lifetime entitlement integration: **1 passed**, using local D1
  and generated test keys, with wrong-device and validity-window checks intact.
- Rust advisory, license, ban and source policies: **passed**, with the existing
  expiring baseline unchanged. The remaining upstream warnings remain visible.
- Rust formatting and Clippy with warnings denied: **passed**, both for the
  native-E2E/all-targets configuration and the normal production library.
- Localization, workflow pins, E2E-only policy, app security configuration,
  version/tag policy and whitespace checks: **passed**.

Both malformed HTTP chunk cases failed against Actix 3.20.3 before the patch;
all six API journeys passed after it. A further legacy Unicode-whitespace date
case failed through actual HTTP after the initial time migration and was fixed
with narrow legacy-only input normalization. Signed redundant year zeros remain
accepted, and whitespace immediately after a sign remains rejected. The strict
RFC 3339 parser is separate from that normalization.

Unix DST journeys exercise the spring gap and repeated autumn hour against an
independent OS oracle and real paused/resumed HTTP bodies. The Windows journey
asserts the child received its process `TZ` override, compares actual OS rules
with an independent PowerShell oracle, and injects only the native test rule
lookup to exercise disabled/custom DST, overnight pacing and persisted weekly
quota rollover. All new pause assertions first observe an active HTTP waiter.
The machine timezone is never changed.

## Platform CI correction

The first candidate, `40bd2a0`, passed the 202-test GitHub browser suite. Intel
macOS reported **243 native passes and one failure** in the existing aborted
legacy-preview hashing/clear journey. The retained worker tuple released its
category lock before unregistering the legacy active path; clear could then
observe the file as active, skip it and report completion. Both the historical
preview and equivalent HEIC retention tuples now release their legacy/source
markers before the category lock. The existing regression and assertions remain
intact; no timeout or cleanup requirement was relaxed. Local pre-fix repetition
passed 105 times, so those repetitions are not treated as evidence against the
observed CI race.

After the ordering correction, the complete local native suite passed again:
**244 passed, 0 failed, 0 ignored**. Serial focused repetitions also passed:
**10** legacy-hash abort/clear journeys (each covers nested and flat caches),
**3** HEIC abort/clear/account-change journeys, and **3** HEIC reuse/pin/unpin/clear
journeys. The original Windows candidate passed **239 native journeys**, including
the OS-rule/DST case; its remaining jobs were cancelled as superseded, so those
lint/package checks are not counted as passes.

## Publication and acceptance boundaries

The complete outgoing history and shared-file changes were reviewed for Android
publication. No Android project, source, signing material, pending-policy path or
Android-specific configuration change is included. The guard's existing pending
owner-review warnings are not treated as approval; those paths are unchanged.

The optional app npm audit still reports
[GHSA-fc5q-39xc-vjpf](https://github.com/advisories/GHSA-fc5q-39xc-vjpf) for
Picomatch 4.0.7, a Vite build dependency absent from the application runtime.
No patched npm release was available at preparation. The owner's existing app
npm assurance setting and all other release gates are unchanged.

GitHub desktop CI and the tag workflow retain platform-native tests, packaged
startup, release signing configuration, checksums, SBOMs and provenance before
publication. Updater signing remains configured; the existing absence of Apple
and Azure platform-signing credentials is unchanged.

The reporter's exact Windows 10 video, manual installed-platform upgrades,
live Telegram and PayPal sandbox acceptance are not claimed by these controlled
tests. Android device acceptance is outside this desktop release.

## Service source follow-up on 2026-11-23

The [Worker release record](SUPPORTER_WORKER_RELEASE.md) identifies the authoritative service candidate and the production version verified by the maintainer against the compiled JavaScript. The health and public-key result above was recorded for the desktop release.
