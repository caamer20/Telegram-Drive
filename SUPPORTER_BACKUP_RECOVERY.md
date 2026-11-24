# Supporter D1 Backup and Recovery

The `Supporter D1 Backup` workflow exports the complete production D1 schema and data every day, compresses the SQL export, creates a SHA-256 sidecar, uploads both objects to a private Cloudflare R2 bucket, and verifies the uploaded archive size. Backup objects use this layout:

```text
d1/telegram-drive-supporter/YYYY/MM/DD/telegram-drive-supporter-YYYYMMDDTHHMMSSZ.sql.gz
d1/telegram-drive-supporter/YYYY/MM/DD/telegram-drive-supporter-YYYYMMDDTHHMMSSZ.sql.gz.sha256
```

Cloudflare D1 Time Travel remains the fastest recovery option for a recent accidental write. The R2 SQL exports provide independent, longer-term recovery from a bad migration, dropped table, or incident outside the Time Travel retention window.

## Runtime separation and initial setup status

This independent backup workflow was introduced with v3.5.0. Earlier releases did not run it, which is why missing backup credentials did not previously appear as a GitHub Actions failure.

The scheduled job intentionally validates all required GitHub secrets before it contacts production D1. If configuration is missing, it exits with an error before exporting, uploading, or modifying supporter data. The failure means that a new independent R2 recovery copy was not created; it does **not** mean that checkout, license activation, entitlement refresh, recovery codes, ad removal, the live Worker, or production D1 stopped working.

Backup health must never become a runtime dependency of the $5 lifetime supporter flow. Do not disable or mark this job successful merely to hide missing disaster-recovery configuration. Either complete the one-time private R2 setup below or make an explicit repository-owner decision to remove this backup strategy and accept the reduced recovery protection.

## One-time setup

Create a private R2 bucket dedicated to backups and give its API token object read/write access only to that bucket. Configure these GitHub Actions repository secrets:

- `CLOUDFLARE_API_TOKEN` — D1 read access to `telegram-drive-supporter`
- `CLOUDFLARE_ACCOUNT_ID`
- `R2_ACCESS_KEY_ID`
- `R2_SECRET_ACCESS_KEY`
- `R2_BACKUP_BUCKET` — the private bucket name

Run the workflow manually once and confirm that the archive and checksum appear in R2. Configure an R2 lifecycle rule appropriate for the project's recovery policy; 90 days is the minimum recommended starting point. Protect or replicate the bucket separately if the threat model includes account-wide Cloudflare loss.

## Local recovery rehearsal

From `supporter-service/`, run `node --test tests/e2e/restore-drill.mjs` after installing the locked dependencies. The drill creates synthetic purchases, exports local D1 SQL, checks a gzip archive against its SHA-256 sidecar, stops the source Worker, and imports into a fresh local D1. It compares schema and row contents, then exercises receipt delivery, refresh, device recovery, the three-device limit, pending checkout and refund redelivery. Key material and the database archive occupy separate, private temporary directories that are deleted after the drill.

For an operational recovery, authorized custodians must be able to retrieve the original entitlement signing JWK, recovery lookup key and recovery encryption key from the approved secret store. Record who can restore them and where the access procedure is kept. Verify the signing public key against the one in the candidate, and exercise the restored keys on a dedicated test account. Keep secrets and recovery codes out of evidence and logs. Record the result of the real backup and signed-upgrade exercises separately.

### Rehearsal on 2026-11-19

The local drill restored the SQL archive into a fresh D1 and verified receipt delivery, refresh, device limits, pending checkout, and refusal of a refunded purchase using separately recovered synthetic keys. On an Android 36 emulator with WebView 133, the prior and candidate applications verified the same device-bound entitlement before and after a signed in-place upgrade and relaunch with networking disabled. Those builds shared a disposable signing certificate. A separate offline upgrade of the original distribution-signed Android 4.1.0 and 4.1.3 APKs on Android 7 preserved package identity, installation time, application data and permission.

PayPal was simulated. Production archive availability, custodian access to production keys, physical devices, and desktop signed-upgrade acceptance require their own evidence.

## Quarterly restore drill

Perform restore drills against a new D1 database. Never use the production database as a test target.

1. Select an archive and its matching `.sha256` object from R2.
2. Download both with an S3-compatible client using the R2 endpoint `https://<ACCOUNT_ID>.r2.cloudflarestorage.com`.
3. Run `sha256sum -c <archive>.sha256` from the directory containing both files.
4. Decompress the archive with `gzip -dc <archive>.sql.gz > supporter-restore.sql`.
5. Create a temporary D1 database, then import the SQL with `npx wrangler d1 execute <temporary-database> --remote --file=supporter-restore.sql`.
6. Query the restored database and compare counts for `entitlements`, `entitlement_devices`, `checkout_claims`, and `webhook_events` with the source or backup-date operational record.
7. Confirm active and revoked entitlements, device bindings, PayPal order/capture IDs, and recovery lookup hashes are present. Do not attempt to decrypt recovery-code ciphertext during a drill.
8. Delete the temporary drill database only after recording the date, selected backup key, checksum result, row counts, and operator.

## Production recovery

Recovery changes live payment state and must use a reviewed incident plan.

1. Stop supporter-service deployment and migration jobs, record the incident time, and take a fresh export if the database is still readable.
2. Prefer D1 Time Travel for a recent, precisely bounded bad write. Record the pre-restore bookmark so the operation can be undone.
3. For R2 recovery, verify the checksum and import into a newly created D1 database as described above. Do not import over the damaged production database.
4. Validate schema, row counts, representative active/revoked records, and foreign-key relationships in the new database.
5. Update the Worker's D1 binding to the new database ID, deploy, and run `/health`, checkout-status, activation, refresh, and revocation smoke tests before reopening payment traffic.
6. Retain the damaged database and incident artifacts until the recovery has been reviewed and the retention requirement has passed.

The exports contain payment identifiers, entitlement records, cryptographic device identifiers, and encrypted recovery-code material. Keep the R2 bucket private, scope credentials narrowly, and never publish backup artifacts as GitHub Actions artifacts or commit them to the repository.
