# Telegram Folder Sync

Folder Sync maps a directory on your computer to one Telegram channel. It is disabled by default and remains disabled until you turn on **Settings → Folder Sync → Enable Sync**. That choice is stored in the local application database and is restored when Telegram Drive restarts.

## How synchronization protects your files

Telegram Drive compares three snapshots for every mapping:

1. **Local Tree** — the files currently present in the mapped directory.
2. **Remote Tree** — the documents currently present in the mapped Telegram channel.
3. **Synced Tree** — the last successfully reconciled local and remote state.

The Synced Tree is the safety anchor. A missing remote tree caused by an offline session or network failure is never treated as a set of deletions: reconciliation stops until Telegram can provide a valid remote snapshot. When both the local and remote versions changed since the Synced Tree, Telegram Drive records a conflict instead of overwriting either version.

As an additional guard, a plan that would delete more than half of the Synced Tree is aborted. The files remain untouched, the sync status shows a warning, and the event is written to the sync log.

## Transfer safety and limits

- Telegram folder uploads are capped at exactly **2,000,000,000 bytes**. Larger files stay on your computer and are recorded as skipped; they are never truncated.
- Encrypted files need room for the authenticated TDENC2 envelope, so a plaintext file just below the 2 GB boundary can also be skipped when its encrypted result would exceed Telegram's cap.
- Downloads are written beside the destination as `<filename>.td-sync-tmp`. Telegram Drive verifies and closes the download before atomically renaming it to the requested filename.
- The `.td-sync-tmp` suffix is reserved. If that path already exists, sync preserves it and pauses that download; inspect and remove or rename the stale temp file before retrying.
- Telegram `FLOOD_WAIT` responses pause the queue for the server-requested interval and retry with exponential backoff.
- When a mapping uses vault encryption, uploads pause while the vault is locked and resume after the vault is unlocked.
- Sync transfers run in the same queue as your own transfers. They appear in Transfers with their progress and can be paused or cancelled there; a paused sync transfer holds Folder Sync until it is resumed. An upload larger than 10 MB continues where it stopped after an interruption.
- A file that sync uploads with vault encryption keeps its folder path inside its encrypted metadata. If the mapping's records are lost (a reinstall, a new device, or a mapping that was removed and added again), sync reads that path with the vault unlocked and recognises the file instead of uploading it again. While those files are being identified the mapping shows that it is waiting and continues by itself. Protected files uploaded before version 4.0 carry no path and are not matched this way.
- A Telegram path that is not representable on every supported desktop filesystem (for example `CON.txt`, a trailing dot, or `report?.pdf`) is rejected rather than silently renamed.
- Each Telegram channel can map to only one local folder, and mapped local folders cannot overlap or be nested.
- Duplicate Telegram messages that resolve to the same relative path pause that pair. Rename or remove the duplicate in Telegram before resuming; the engine never guesses which duplicate should win.
- A mapping pauses if its channel contains more than 50,000 file-bearing messages. This safety ceiling prevents an incomplete remote scan from being mistaken for mass deletion; split very large archives across channels before mapping them.

## Options

- **Skip unchanged files when scanning** (Settings → Folder Sync, off by default). A file whose size and modified time are unchanged since it was last in sync is not read again. With this on, an idle mapping checks Telegram every 5 minutes instead of every 30 seconds, local changes are still picked up immediately, and every file is verified in full every 6 hours.
- **Treat same-size files already in both places as synced** (per mapping). When a folder and a Telegram channel already hold the same files before the first sync, each shared path is otherwise reported as a conflict, because local and Telegram copies cannot be compared by content without downloading them. With this on, files at the same path with the same size are recorded as in sync; files that differ in size remain conflicts. Preview the mapping to see the effect before saving.

## Resolve a conflict

The desktop sidebar displays an amber Folder Sync status when conflicts exist. Open the conflict drawer and choose one action for each file:

- **Keep Local** uploads the current computer copy to Telegram.
- **Keep Remote** downloads the Telegram copy and atomically replaces the local file.
- **Keep Both** preserves the local file and downloads the Telegram version with a `.remote-conflict-<id>` suffix.

No conflict is resolved automatically.

## Using WebDAV or the REST API at the same time

WebDAV, the REST API, and Folder Sync all operate against the same Telegram channels. Their changes appear in the Remote Tree during the next reconciliation. Keep these rules in mind:

- Let a WebDAV or REST upload finish before editing the same path locally.
- Simultaneous local and remote edits become an explicit Folder Sync conflict.
- A large remote delete through WebDAV or REST triggers the same mass-deletion protection.
- Encrypted files remain subject to the vault and scoped-credential rules described in the WebDAV and REST API guides.

See [WEBDAV_GUIDE.md](WEBDAV_GUIDE.md) and [REST_API_Documentation.md](REST_API_Documentation.md) for connection and authentication details.

## Maintainer measurement of idle sync

On 2026-11-19 the native debug driver ran the production timer, local filesystem scanner, planner and SQLite against one active synthetic mapping on Intel macOS. Each file contained 64 KiB, had an existing matching remote entry and SQLite mapping, and remained untouched. No process polled the engine or triggered a wake during the passive window. The remote fixture counted complete tree requests and readiness probes. The observer timed native running-to-idle transitions with a monotonic clock. Each trial finished with every original file recorded as synced, zero pending operations and zero conflicts.

| Scanner | Files | Idle window | Remote tree reads / readiness probes | Native idle cycle durations | Startup to ready | Median of 5 additional unchanged-library wakes |
| --- | ---: | ---: | ---: | --- | ---: | ---: |
| Full | 1 | 65 s | 2 / 2 | 14, 21 ms | 166 ms | 24 ms |
| Full | 256 | 65 s | 2 / 2 | 1378, 2276 ms | 1445 ms | 1453 ms |
| Incremental | 256 | 337 s | 1 / 1 | 103 ms | 1444 ms | 83 ms |

The full scanner calls the file hash reader for each eligible file. For the 256-file fixture that accounts for 16 MiB of payload per full pass and 32 MiB over the measured idle window. These are application reads derived from the fixture size. The OS may serve them from memory. One tree request in production can require multiple Telegram history pages. The in-memory remote supplies neither network bytes nor network latency, and its matching algorithm can also affect local timings. Debug builds and concurrent verification work limit throughput comparisons.

To reproduce from the repository root with the locked dependencies available locally:

```sh
cargo build --locked --offline --manifest-path app/src-tauri/Cargo.toml --features native-e2e --bin native-e2e-driver
python3 -B scripts/measure-idle-sync.py app/src-tauri/target/debug/native-e2e-driver --files 256 --seconds 65
```

If Cargo uses another target directory, pass its driver path to the script. To sample the existing incremental setting, append `--scanner incremental` and set `--seconds 337` to span its five-minute remote check. The script creates and removes its own private synthetic profile. Before changing a timer, measure release-build page counts, network bytes, scan duration and remote-edit discovery latency on an approved test account. Record only aggregate counts, compare similar channel sizes and verify watcher wake, offline reconnect and mass-deletion protection.
