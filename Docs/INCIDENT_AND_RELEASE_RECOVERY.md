# Release and incident recovery

The repository maintainer, Cameron Amer, owns incident coordination, release decisions and follow-up. The private intake route is in the [security policy](../SECURITY.md). A report can come from a user, a contributor or a failed release job. The maintainer records an incident lead and an operator for the affected system in the private incident record. For a small incident the same person can hold both roles. A delegate may carry out only actions for which they already have access and explicit authorization. If the authorized operator is unavailable, hold publication or recovery at the last reversible step and escalate privately to the maintainer.

| Surface | Accountable owner | Operator and handoff |
| --- | --- | --- |
| Desktop release, updater, OS packages | Repository maintainer | Release operator with signing and release access. Hand off candidate commit, version, failed job and affected asset type. |
| Account boundary, sync, transfers, vault and local services | Repository maintainer | Client maintainer reproduces with disposable accounts or directories. Hand off OS, app version, operation, stage and a sanitized error category. |
| Supporter checkout, refund, recovery and ad visibility | Repository maintainer | Supporter-service operator with approved provider access checks Worker and provider records privately. Client maintainer verifies persisted entitlement behavior. |
| D1/R2 backup and restore | Repository maintainer | Authorized backup and secret custodians perform separate archive and key recovery. Confirm availability of both before cutover. Record custody details in the private operational record. |
| Privacy or exposure | Repository maintainer | Private incident lead limits access to evidence, determines exposure and coordinates user communication. |

## Maintainer’s release checklist

- [ ] Before each release, the repository maintainer checks the [Rust exception baseline](../dependency-policy/rust-advisory-baseline.json). The current exceptions expire **2026-12-30**. Complete the review before that deadline. Run `node scripts/check-rust-advisory-baseline.cjs` and the locked candidate’s [Rust dependency assurance job](../.github/workflows/dependency-assurance.yml). Review each advisory and yanked crate against its dependency path and available compatible fixes. Record the reviewer, date, disposition and supporting job in the release record. Remove resolved exceptions from the baseline and `deny.toml`. Any retained exception needs a rationale and a reviewed expiry.
- [ ] For a supporter-service release, match the candidate to the [Worker release record](SUPPORTER_WORKER_RELEASE.md), then attach the deployment and schema evidence there after authorized rollout.

## First response

1. Record UTC onset, last known good version, affected versions/platforms, observed impact, reproduction with synthetic data, owner and next checkpoint. Record the difference between a user report and a confirmed effect.
2. Hold the affected release immediately if another account can receive data, authenticated files cannot be recovered, a verified payment can lead to a second checkout, or an updater fails an identity/signature check. Preserve prior signed assets, original user data and service evidence. Coordinate any intervention on a user's installation privately with that user's approval.
3. Collect the smallest useful evidence. Record status, counts and build or job identifiers. Keep private filenames, local paths, Telegram identifiers, sessions, capability URLs, payment references, recovery material, tokens and keys out of incident summaries and public issues.

## Playbooks

### Release or updater failure

The release operator identifies the exact tag, commit, candidate platform, draft status and failed job in the [release workflow](../.github/workflows/release.yml). Check version alignment, dependency and supporter gates, the platform build and install smoke checks, then the final draft asset checksums and provenance. Compare each expected platform's installer and updater payload with the manifest for that same candidate. A green build on one platform cannot authorize publication of a partial draft.

Hold a failed draft. Rehearse installation and update on disposable profiles using the previous signed release, existing files and an authorized activated test profile when available. Check vault access, resumed queues, account identity and ad-free startup after an offline relaunch. Only a compatible binary/data pairing can roll back. If an updated schema or persisted state cannot be safely read by the older binary, prepare a higher-version forward fix, preserve the installed profile and keep the rollout on hold. For an already-published bad release, the maintainer authorizes containment of further delivery and a replacement through the usual signed channel. Preserve the original tag and evidence. Verify the actual update path from both affected and last-good versions. [Release acceptance](../TESTING.md)

Record platform-specific signoff in the [desktop release acceptance matrix](RELEASE_ACCEPTANCE.md). Use the [encryption recovery drill](ENCRYPTION_RECOVERY_DRILL.md) for independent verification of encrypted data.

### Account isolation, shared links or local services

On evidence of a foreign listing, download, queue item or share response after an account switch, stop the affected operation and pause its local server or mapping. For a disclosed local capability, the owner disables the server and regenerates that capability through its settings before re-enabling approved clients. Use private intake for any possible disclosure. Capture only whether a response crossed the switch and whether bytes were delivered.

Reproduce with synthetic accounts A and B that share a folder/message number. Start the request on A, switch to B before completion, and verify that B cannot list, receive or retry A's work, including after restart. Verify normal access when returning to A. A local rollback must support the installed ownership schema. Restore a local endpoint on loopback with the intended read/write setting and test with a fresh capability. [WebDAV operations](../WEBDAV_GUIDE.md) · [REST operations](../REST_API_Documentation.md)

### Interrupted transfers, sync conflicts or unexpected deletions

Record direction, phase, paused state, available space, whether Telegram accepted the transfer, and whether a completed destination exists. Preserve the queue, upload source, journal, original destination and any partial file. Pause the affected mapping when its owner, target channel or deletion plan is in question. Allow server-requested backoff to finish. For a locked vault, use the normal unlock flow before resuming.

For a lost upload reply, reconcile the durable receipt and remote identity under the original account before retrying. Retry one failed item from that account once connectivity, credentials and source are available. For competing local and remote edits, let the user choose the authoritative copy or **Keep Both**. On a mass-deletion warning, verify the mount, remote tree and last successful mapping with the owner. Preserve both copies until that review is complete. Use a disposable mapping to verify pause/restart, one known edit, one completed destination and a second idle pass with zero pending work before resuming a larger set. [Folder Sync guide](../SYNC_GUIDE.md)

### Authenticated file or vault recovery

Stop replacement of originals after a failed integrity check. Keep the ciphertext, vault and recovery bundle under the user's control. Record the format/version and error class. Distinguish a locked vault, a missing key and an authenticated-read failure using a copy and the app's verification flow. Ask users to enter secrets only in the app.

The client maintainer first exercises the native legacy-envelope and recovery-drill journeys in a private fixture. On an owner-approved copy, verify the intended bundle and key profile, decrypt one known file into a new destination and compare the expected contents. Where a user explicitly chooses to import recovery material, retain the prior vault and reopen after restart before accepting success. A failed drill leaves the incident open and the originals intact. [Native and installed-app verification](../TESTING.md)

### Checkout, recovery or advertisements on an activated device

Identify the stage privately: checkout claim, external approval, verified capture, local activation, receipt acknowledgement, refresh or recovery. A purchaser who has already paid must retain the original claim or recovery path. Preserve the installation and secure storage. Hold any build that shows a sponsor during entitlement loading or to an active or offline-grace supporter. Verify both ad suppression and ordinary sponsor display for a non-supporter in a disposable profile.

The service operator compares the approved service's health contract with the candidate inputs, then checks webhook verification, deduplication and outstanding claims in the authorized environment. For an interrupted capture or delivery, resume the original claim and verify one entitlement. Follow the documented verified-redelivery procedure for an interrupted refund. In a local or authorized sandbox, verify the exact one-time USD 5.00 purchase, Unicode payer return, receipt retrieval, offline relaunch, recovery on an additional device, fourth-device refusal and the matching refund. Permit re-release after affected installed clients pass their update path. Changes to signing or secure-storage identities require an owner-approved migration. [Supporter compatibility](../SUPPORTER_LICENSE_INVARIANTS.md) · [Service recovery](../SUPPORTER_SERVICE.md)

### Missing backup or suspected supporter database loss

For a failed backup alone, keep checkout and cached supporter operation available. Record the failed run, last verified export/checksum and available restore point. The backup operator checks the independent workflow and access to the private bucket. For suspected DB damage, preserve the source, record the last safe time and assess purchases and reversals after the proposed restore point. The custodian confirms key recovery privately. Run the documented restore on a fresh database, verify the checksum, schema, reconciliation triggers and entitlement/device outcomes, and reconcile the post-backup interval. Only the owner authorizes a production binding switch or rollback. Keep the previous binding and data recoverable through verification. [D1 backup, drills and cutover](../SUPPORTER_BACKUP_RECOVERY.md)

## Closing and rehearsing an incident

The lead records the last good and first affected versions, affected population or its known limits, diagnosis, containment, tests with results, residual risk, decision owner and next action. Distinguish fixture rehearsal, signed device acceptance and provider evidence. The maintainer closes the incident after the affected workflow succeeds on the specified platform and the original failure condition is checked. Record any postponed acceptance as open.

Rehearse with a withheld draft asset, a switched synthetic account, an interrupted encrypted transfer, a copied corrupt envelope, a simulated interrupted receipt and a restored local D1. Each operator should identify the owner, containment step, preserved data, recovery test and evidence required to resume. Scheduled production backups and access to private signing or recovery material need separate custodian confirmation.
