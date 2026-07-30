# Telegram Media Library plugin

This version-controlled Tauri mobile plugin owns the Android Compose UI, Room
metadata, Paging queries, synchronization orchestration, and app-private
thumbnail cache. Telegram access remains in Rust through the existing
authenticated IPv4 loopback server; Android never opens the Telegram session
file or creates another Telegram client.

## Credential and offline policy

The loopback base URL and bearer token live only in process-local session
stores. They are never written to Room, preferences, saved state, intents, or
files. After process death, Room metadata and cached thumbnails remain
available, but synchronization and video playback stay disabled until Tauri
supplies a new trusted session.

## Synchronization and deletion guarantees

Full sync commits one bounded page and cursor at a time and resumes from the
last committed cursor. Incremental refresh reads only messages newer than the
newest indexed media, then reconciles the most recent 200 scanned messages so
recent deletions and media-to-non-media edits can be marked deleted when their
absence is confirmed. It does not claim perfect historical deletion detection.
Historical rows are reconciled only after an explicit full resync reaches the
end of that peer's history.

## Thumbnail cache

Thumbnails are fetched lazily as authenticated binary responses and stored
under `media-thumbnails/{accountId}`. Writes use temporary files and rename,
simultaneous requests are deduplicated, download concurrency is four, and the
default global LRU limit is 256 MiB. Files retained by an active grid or preview
are not evicted; account cleanup defers their deletion until the view releases
them.
