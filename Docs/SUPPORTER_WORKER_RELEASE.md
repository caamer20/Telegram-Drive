# Supporter Worker release record

## Source revision reviewed on 2026-11-23

| Field | Recorded value |
| --- | --- |
| Worker | `telegram-drive-supporter` |
| Authoritative source for the next service release | `supporter-service/` in the service maintainer’s working tree, based on `3c503d70855886777047898bec1713ee2197f2f8`, including the reviewed changes outside that commit |
| Service source revision (SHA-256) | `09b0e5dc8aa976427c6092cd3b0896f7de5877dc4243c6d821145b47e840a013` |
| Required schema | `0001_initial.sql` and `0002_webhook_recovery.sql`, in order |
| Active Cloudflare version and traffic allocation | The service maintainer verified the active version and its 100% traffic allocation in the authenticated dashboard on 2026-11-23. |
| Running JavaScript | SHA-256 `577ca6f8f7b67cd26f2eb17f8219c974052d89e434e86f68197d1db9a8b06eb1`, exactly reproduced from `6e927309a46e550fd62cf611680e545a3e57198d`. |
| Production schema | The service maintainer verified the applied migration ledger and schema independently. Consult the operational release record before any Worker rollout. |

The version of the service included with desktop baseline `913c3cd9ff7a26faf54c0e28900de59b99acb6d8` has only migration 0001. Its source fingerprint is `55c33f8a307923aa6be9b262ce0c30c9260cf81fcea83e47027bc718beccd858`. Use the authoritative service revision above when preparing the webhook recovery rollout.

### Reproduce the source revision

From the root of the candidate checkout, run in Bash with `shasum` available. The digest covers every source and migration file, the package manifest and lockfile, TypeScript configuration and Wrangler configuration. Paths are relative to `supporter-service/`. Each manifest entry is the SHA-256 of the file bytes, two spaces, its path and a newline, in bytewise path order.

```bash
set -euo pipefail
cd supporter-service
{
  find src migrations -type f
  printf '%s\n' package.json package-lock.json tsconfig.json wrangler.jsonc
} | LC_ALL=C sort | while IFS= read -r file; do
  shasum -a 256 "$file"
done | shasum -a 256
```

### Verify the active deployment

In Cloudflare, open **Workers & Pages → telegram-drive-supporter → Deployments → Active Deployment** and record each version ID and traffic percentage. When the repository workflow produced the version, match its ID to the **Deploy Worker** step of the **Supporter Service** run, then reproduce the source fingerprint from that run’s checkout commit. A timestamp alone cannot establish the source. [Cloudflare deployment history](https://developers.cloudflare.com/workers/versions-and-deployments/)

For the next authorized service deployment, the service operator records the source commit and fingerprint, the deployment receipt, UTC timestamp, active Worker version ID(s) and traffic shares, and the applied D1 migration names. Verify the active versions in Cloudflare deployment history and the applied migrations in D1. The public health check verifies client compatibility and does not identify the running revision. Keep credentials and private provider data out of the record.
