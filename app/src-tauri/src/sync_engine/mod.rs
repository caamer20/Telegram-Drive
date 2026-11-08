pub mod config;
pub mod executor;
pub mod planner;
pub mod policy;
pub mod preview;
pub mod readiness;
pub mod watcher;

#[cfg(feature = "native-e2e")]
pub(crate) mod native_fixture;

use crate::{
    commands::{
        utils::{flood_wait_seconds, media_size, resolve_peer},
        TelegramState,
    },
    db::DbConnection,
};
use config::{load_pairs, load_settings, log_sync, SyncPair};
use planner::{FileTree, SyncOperation, SyncedEntry, SyncedTree, TreeEntry};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlite::State;
use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{Emitter, Listener, Manager};
use tokio::{
    sync::{Mutex as AsyncMutex, RwLock},
    task::JoinHandle,
};

/// How long a shutdown request waits for the engine to stop on its own before
/// the current operation is abandoned. Quitting, updating and reconfiguring a
/// mapping must not hang on a large transfer.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(10);
/// Interval between reconciliations that no local change triggered.
const FULL_SCANNER_POLL: Duration = Duration::from_secs(30);
/// With the incremental scanner an idle mapping re-reads its Telegram folder
/// this often; local changes still reconcile immediately through the watcher.
const INCREMENTAL_REMOTE_POLL: Duration = Duration::from_secs(5 * 60);
/// The incremental scanner re-hashes every file this often, to catch an edit
/// that preserved both size and modification time.
const FULL_VERIFICATION_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// A file modified this recently is always hashed: its whole-second timestamp
/// cannot yet tell a later edit within the same second apart.
const RACY_MTIME_WINDOW_SECS: i64 = 2;

/// The transfer the executor is waiting on, so shutdown can cancel it.
#[derive(Debug, Clone)]
pub(crate) struct ActiveSyncTransfer {
    pub transfer_id: String,
    /// Reserved staging file of an in-flight download.
    pub temporary: Option<PathBuf>,
}

/// Clears the engine's active transfer when the operation ends for any reason.
pub(crate) struct ActiveTransferGuard(Arc<Mutex<Option<ActiveSyncTransfer>>>);

impl Drop for ActiveTransferGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.0.lock() {
            *active = None;
        }
    }
}

pub struct SyncEngine {
    pub running: Arc<AtomicBool>,
    pub db: DbConnection,
    pub app_handle: tauri::AppHandle,
    pub status: Arc<RwLock<SyncStatus>>,
    shutdown_tx: Mutex<tokio::sync::watch::Sender<bool>>,
    task: Mutex<Option<JoinHandle<()>>>,
    pub(crate) operation_lock: AsyncMutex<()>,
    pub(crate) reconfigure_lock: AsyncMutex<()>,
    pub(crate) preview_receipts: Mutex<HashMap<String, preview::PreviewReceipt>>,
    active_transfer: Arc<Mutex<Option<ActiveSyncTransfer>>>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub enabled: bool,
    pub running: bool,
    pub active_pairs: usize,
    pub pending_ops: usize,
    pub conflicts: usize,
    pub last_error: Option<String>,
    pub pairs: Vec<SyncPairStatus>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPairStatus {
    pub pair_id: i64,
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_reason: Option<&'static str>,
    pub pending_ops: usize,
    pub conflicts: usize,
    pub last_error: Option<String>,
    pub last_checked_at: Option<i64>,
}

impl SyncEngine {
    pub fn new(db: DbConnection, app_handle: tauri::AppHandle) -> Self {
        let (shutdown_tx, _) = tokio::sync::watch::channel(false);
        Self {
            running: Arc::new(AtomicBool::new(false)),
            db,
            app_handle,
            status: Arc::new(RwLock::new(SyncStatus::default())),
            shutdown_tx: Mutex::new(shutdown_tx),
            task: Mutex::new(None),
            operation_lock: AsyncMutex::new(()),
            reconfigure_lock: AsyncMutex::new(()),
            preview_receipts: Mutex::new(HashMap::new()),
            active_transfer: Arc::new(Mutex::new(None)),
        }
    }

    /// Registers the transfer the executor is about to wait on.
    pub(crate) fn track_transfer(
        &self,
        transfer_id: String,
        temporary: Option<PathBuf>,
    ) -> ActiveTransferGuard {
        if let Ok(mut active) = self.active_transfer.lock() {
            *active = Some(ActiveSyncTransfer {
                transfer_id,
                temporary,
            });
        }
        ActiveTransferGuard(self.active_transfer.clone())
    }

    fn current_transfer(&self) -> Option<ActiveSyncTransfer> {
        self.active_transfer
            .lock()
            .ok()
            .and_then(|active| active.clone())
    }

    /// Asks the in-flight upload or download to stop at its next chunk. The
    /// transfer commands remove their own staging files when cancelled.
    async fn cancel_active_transfer(&self) -> Option<String> {
        let active = self.current_transfer()?;
        let telegram = self.app_handle.try_state::<TelegramState>()?;
        telegram
            .cancelled_transfers
            .write()
            .await
            .insert(active.transfer_id.clone());
        Some(active.transfer_id)
    }

    pub async fn start(&self) -> Result<(), String> {
        let settings = load_settings(self.db.clone()).await?;
        let pairs = load_pairs(self.db.clone(), false).await?;
        if !settings.enabled {
            let status = self.status.clone();
            let app = self.app_handle.clone();
            {
                let snapshot = SyncStatus {
                    enabled: false,
                    active_pairs: pairs.iter().filter(|pair| pair.is_active).count(),
                    pairs: pairs
                        .iter()
                        .map(|pair| SyncPairStatus {
                            pair_id: pair.id,
                            phase: "paused".into(),
                            last_error: Some("Automatic sync is paused".into()),
                            ..SyncPairStatus::default()
                        })
                        .collect(),
                    ..SyncStatus::default()
                };
                *status.write().await = snapshot.clone();
                let _ = app.emit("sync-status-changed", snapshot);
            }
            return Ok(());
        }
        if self.running.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        *self
            .shutdown_tx
            .lock()
            .map_err(|_| "Sync shutdown lock poisoned")? = shutdown_tx;
        let app = self.app_handle.clone();
        let db = self.db.clone();
        let status = self.status.clone();
        let running = self.running.clone();
        let task = tokio::spawn(async move {
            engine_loop(app, db, status, running, settings, pairs, shutdown_rx).await;
        });
        *self.task.lock().map_err(|_| "Sync task lock poisoned")? = Some(task);
        Ok(())
    }

    pub fn shutdown(&self) {
        if let Ok(sender) = self.shutdown_tx.lock() {
            let _ = sender.send(true);
        }
    }

    pub(crate) fn subscribe_shutdown(&self) -> Result<tokio::sync::watch::Receiver<bool>, String> {
        self.shutdown_tx
            .lock()
            .map(|sender| sender.subscribe())
            .map_err(|_| "Sync shutdown lock poisoned".to_string())
    }

    /// Stops the engine. An in-flight transfer is cancelled first; if the
    /// engine still has not stopped after [`SHUTDOWN_GRACE`], its task is
    /// abandoned so the caller is never blocked indefinitely. Uploads that
    /// finished are already journaled and are completed on the next start.
    pub async fn shutdown_and_wait(&self) -> Result<(), String> {
        self.shutdown();
        let cancelled_transfer = self.cancel_active_transfer().await;
        let task = self
            .task
            .lock()
            .map_err(|_| "Sync task lock poisoned")?
            .take();
        let mut outcome = Ok(());
        if let Some(mut task) = task {
            match tokio::time::timeout(SHUTDOWN_GRACE, &mut task).await {
                Ok(result) => {
                    outcome = result
                        .map_err(|error| format!("Sync engine stopped unexpectedly: {error}"));
                }
                Err(_) => {
                    log::warn!(
                        "Folder sync did not stop within {}s; abandoning the current operation",
                        SHUTDOWN_GRACE.as_secs()
                    );
                    task.abort();
                    let _ = task.await;
                    self.discard_abandoned_transfer().await;
                }
            }
        }
        if let Some(transfer_id) = cancelled_transfer {
            // The request is single-use; do not leave it behind for an
            // unrelated transfer if the operation had already finished.
            if let Some(telegram) = self.app_handle.try_state::<TelegramState>() {
                telegram
                    .cancelled_transfers
                    .write()
                    .await
                    .remove(&transfer_id);
            }
        }
        self.running.store(false, Ordering::SeqCst);
        outcome
    }

    /// Clean up after an operation that had to be abandoned mid-flight.
    async fn discard_abandoned_transfer(&self) {
        let abandoned = self
            .active_transfer
            .lock()
            .ok()
            .and_then(|mut active| active.take());
        if let Some(temporary) = abandoned.and_then(|transfer| transfer.temporary) {
            // Only the engine's own reserved staging name is ever removed.
            if temporary
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".td-sync-tmp"))
            {
                let _ = tokio::fs::remove_file(&temporary).await;
            }
        }
        {
            let mut current = self.status.write().await;
            current.running = false;
        }
        emit_status(&self.app_handle, &self.status).await;
    }

    pub async fn restart(&self) -> Result<(), String> {
        let _reconfigure = self.reconfigure_lock.lock().await;
        self.shutdown_and_wait().await?;
        self.start().await
    }
}

pub async fn restart_sync_engine(app: &tauri::AppHandle) -> Result<(), String> {
    app.state::<SyncEngine>().restart().await
}

async fn emit_status(app: &tauri::AppHandle, status: &Arc<RwLock<SyncStatus>>) {
    let snapshot = status.read().await.clone();
    let _ = app.emit("sync-status-changed", snapshot);
}

pub(crate) fn pair_account(
    app: &tauri::AppHandle,
    pair: &SyncPair,
) -> Result<crate::workspace::AccountGuard, String> {
    let expected = pair
        .account_owner
        .as_deref()
        .ok_or("Review this mapping before activating it for your current Telegram account")?;
    let root = sync_data_dir(app)?;
    crate::workspace::AccountGuard::open(&root, Some(expected))
}

fn sync_data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    #[cfg(feature = "native-e2e")]
    if let Some(fixture) = app.try_state::<Arc<native_fixture::NativeSyncFixture>>() {
        return Ok(fixture.root.clone());
    }
    app.path().app_data_dir().map_err(|error| error.to_string())
}

async fn set_pair_status(
    app: &tauri::AppHandle,
    status: &Arc<RwLock<SyncStatus>>,
    pair_status: SyncPairStatus,
) {
    let mut current = status.write().await;
    if let Some(existing) = current
        .pairs
        .iter_mut()
        .find(|entry| entry.pair_id == pair_status.pair_id)
    {
        *existing = pair_status;
    } else {
        current.pairs.push(pair_status);
    }
    drop(current);
    emit_status(app, status).await;
}

async fn set_waiting_status(
    app: &tauri::AppHandle,
    status: &Arc<RwLock<SyncStatus>>,
    pairs: &[SyncPair],
    error: Option<&str>,
) {
    let mut current = status.write().await;
    current.running = false;
    current.last_error = error.map(str::to_owned);
    for pair in pairs {
        if let Some(entry) = current
            .pairs
            .iter_mut()
            .find(|entry| entry.pair_id == pair.id)
        {
            entry.phase = if pair.is_active && error.is_none() {
                "waiting"
            } else {
                "paused"
            }
            .into();
            entry.wait_reason = (pair.is_active && error.is_none()).then_some("telegram");
            entry.last_error = if pair.is_active {
                error.map(str::to_owned)
            } else {
                Some("Paused by you".into())
            };
        }
    }
    drop(current);
    emit_status(app, status).await;
}

/// Releases what the engine loop owns even when its task is abandoned.
struct EngineLoopGuard {
    app: tauri::AppHandle,
    listener: tauri::EventId,
    watcher: JoinHandle<()>,
    running: Arc<AtomicBool>,
}

impl Drop for EngineLoopGuard {
    fn drop(&mut self) {
        self.app.unlisten(self.listener);
        self.watcher.abort();
        self.running.store(false, Ordering::SeqCst);
    }
}

async fn engine_loop(
    app: tauri::AppHandle,
    db: DbConnection,
    status: Arc<RwLock<SyncStatus>>,
    running: Arc<AtomicBool>,
    settings: config::SyncSettings,
    pairs: Vec<SyncPair>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    {
        let mut current = status.write().await;
        current.enabled = true;
        current.active_pairs = pairs.iter().filter(|pair| pair.is_active).count();
        current.last_error = None;
        current.pairs = pairs
            .iter()
            .map(|pair| SyncPairStatus {
                pair_id: pair.id,
                phase: if pair.is_active { "waiting" } else { "paused" }.into(),
                ..SyncPairStatus::default()
            })
            .collect();
    }
    emit_status(&app, &status).await;

    // A trigger means "the trees may have changed", not "perform exactly one
    // reconciliation". A capacity of one coalesces filesystem bursts and
    // prevents a large copy from queueing hundreds of full remote scans.
    let (trigger_tx, mut trigger_rx) = tokio::sync::mpsc::channel(1);
    let watcher = watcher::LocalWatcher::spawn(
        pairs
            .iter()
            .filter(|pair| pair.is_active)
            .map(|pair| (PathBuf::from(&pair.local_path), pair.preferences.clone()))
            .collect(),
        Duration::from_millis(settings.debounce_ms),
        shutdown.clone(),
        trigger_tx.clone(),
    );
    let vault_trigger = trigger_tx.clone();
    let listener_id = app.listen("vault-unlocked", move |_| {
        let _ = vault_trigger.try_send(());
    });
    let loop_guard = EngineLoopGuard {
        app: app.clone(),
        listener: listener_id,
        watcher,
        running: running.clone(),
    };
    let _ = trigger_tx.try_send(());
    // The initial trigger handles startup. Avoid a second immediate timer pass.
    let poll = FULL_SCANNER_POLL;
    let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + poll, poll);
    let readiness = app.state::<TelegramState>().sync_readiness.clone();
    // A retained watch snapshot covers both ready-before-start and readiness
    // changing during a scan. No frontend event or engine restart is required.
    let mut readiness_rx = readiness.subscribe();
    let mut waiting = true;
    let mut retry_delay = readiness::RETRY_MIN;
    let mut retry_at = tokio::time::Instant::now() + retry_delay;
    let mut readiness_error = None;
    let incremental = settings.uses_incremental_scanner();
    let mut last_reconcile: Option<Instant> = None;
    let mut last_full_verification: Option<Instant> = None;

    loop {
        let (triggered, retry, readiness_changed) = tokio::select! {
            changed = shutdown.changed() => { if changed.is_err() || *shutdown.borrow() { break }; continue },
            _ = interval.tick() => (false, false, false),
            event = trigger_rx.recv() => { if event.is_none() { break }; (true, false, false) },
            changed = readiness_rx.changed() => { if changed.is_err() { break }; (true, false, true) },
            _ = tokio::time::sleep_until(retry_at), if waiting => (true, true, false),
        };
        if *shutdown.borrow() {
            break;
        }
        // Readiness changes and its targeted retry are forced triggers. Only
        // ordinary idle ticks retain the incremental scanner's five-minute gate.
        if incremental
            && !triggered
            && !waiting
            && last_reconcile.is_some_and(|at| at.elapsed() < INCREMENTAL_REMOTE_POLL)
        {
            continue;
        }
        while trigger_rx.try_recv().is_ok() {}
        let active = pairs.iter().any(|pair| pair.is_active);
        if readiness_changed {
            readiness_error = None;
        }
        // While waiting, only the backoff timer probes Telegram. Filesystem
        // bursts cannot cause repeated hashing, remote scans or login attempts.
        // Ordinary passes also verify a retained ready connection before work.
        let probe_now =
            active && (retry || (!readiness_changed && readiness.snapshot().owner.is_some()));
        if probe_now {
            let probe = tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { break; }
                    continue;
                },
                probe = readiness::probe(&app) => probe,
            };
            readiness_error = probe.error;
            retry_at = tokio::time::Instant::now() + probe.retry_after.unwrap_or(retry_delay);
            retry_delay = (retry_delay * 2).min(readiness::RETRY_MAX);
        }
        let ready = readiness_rx.borrow_and_update().owner.is_some();
        if !ready || readiness_error.is_some() || !active {
            if !waiting {
                retry_delay = readiness::RETRY_MIN;
                if !probe_now {
                    retry_at = tokio::time::Instant::now() + retry_delay;
                }
            }
            waiting = active;
            set_waiting_status(
                &app,
                &status,
                &pairs,
                if active {
                    readiness_error.as_deref()
                } else {
                    None
                },
            )
            .await;
            continue;
        }
        waiting = false;
        retry_delay = readiness::RETRY_MIN;
        let verify_every_file = !incremental
            || last_full_verification.is_none_or(|at| at.elapsed() >= FULL_VERIFICATION_INTERVAL);
        let scan_mode = if verify_every_file {
            LocalScanMode::HashEveryFile
        } else {
            LocalScanMode::ReuseRecordedHashes
        };
        {
            let mut current = status.write().await;
            current.running = true;
            current.last_error = None;
        }
        emit_status(&app, &status).await;

        let mut pending = 0usize;
        let mut conflicts = 0usize;
        let mut last_error = None;
        for pair in &pairs {
            if *shutdown.borrow() {
                break;
            }
            if !pair.is_active {
                set_pair_status(
                    &app,
                    &status,
                    SyncPairStatus {
                        pair_id: pair.id,
                        phase: "paused".into(),
                        last_error: Some("Paused by you".into()),
                        ..SyncPairStatus::default()
                    },
                )
                .await;
                continue;
            }
            set_pair_status(
                &app,
                &status,
                SyncPairStatus {
                    pair_id: pair.id,
                    phase: "scanning".into(),
                    ..SyncPairStatus::default()
                },
            )
            .await;
            let engine = app.state::<SyncEngine>();
            let _operation = engine.operation_lock.lock().await;
            // Recheck after the operation lock, before cleanup or local hashing.
            if readiness.snapshot().owner.is_none() {
                waiting = true;
                retry_at = tokio::time::Instant::now() + readiness::RETRY_MIN;
                set_waiting_status(&app, &status, &pairs, None).await;
                break;
            }
            match reconcile_pair(&app, &db, pair, &settings, shutdown.clone(), scan_mode).await {
                Ok((pair_pending, pair_conflicts, pair_error)) => {
                    set_pair_status(
                        &app,
                        &status,
                        SyncPairStatus {
                            pair_id: pair.id,
                            phase: if pair_error.is_some() || pair_conflicts > 0 {
                                "paused"
                            } else {
                                "ready"
                            }
                            .into(),
                            pending_ops: pair_pending,
                            conflicts: pair_conflicts,
                            last_error: pair_error.clone(),
                            last_checked_at: Some(chrono::Utc::now().timestamp()),
                            wait_reason: None,
                        },
                    )
                    .await;
                    pending += pair_pending;
                    conflicts += pair_conflicts;
                    if pair_error.is_some() {
                        last_error = pair_error;
                    }
                }
                Err(error) => {
                    if readiness.snapshot().owner.is_none() {
                        waiting = true;
                        retry_at = tokio::time::Instant::now() + readiness::RETRY_MIN;
                        set_waiting_status(&app, &status, &pairs, None).await;
                        break;
                    }
                    set_pair_status(
                        &app,
                        &status,
                        SyncPairStatus {
                            pair_id: pair.id,
                            phase: "paused".into(),
                            last_error: Some(error.clone()),
                            last_checked_at: Some(chrono::Utc::now().timestamp()),
                            ..SyncPairStatus::default()
                        },
                    )
                    .await;
                    log::error!("Folder sync pair {} paused: {error}", pair.id);
                    log_sync(
                        db.clone(),
                        Some(pair.id),
                        "error".to_string(),
                        None,
                        Some(error.clone()),
                    )
                    .await;
                    last_error = Some(error);
                }
            }
        }
        conflicts = conflicts.max(count_conflicts(&db).await.unwrap_or(conflicts));
        {
            let mut current = status.write().await;
            current.running = false;
            current.pending_ops = pending;
            current.conflicts = conflicts;
            current.last_error = last_error;
        }
        emit_status(&app, &status).await;
        // An unavailable client did not reconcile or verify any files. Do not
        // postpone the next incremental scan/full verification in that state.
        if !*shutdown.borrow() && !waiting {
            last_reconcile = Some(Instant::now());
            if verify_every_file {
                last_full_verification = last_reconcile;
            }
        }
    }

    drop(loop_guard);
    {
        let mut current = status.write().await;
        current.running = false;
    }
    emit_status(&app, &status).await;
}

async fn count_conflicts(db: &DbConnection) -> Result<usize, String> {
    crate::db::with_connection(db.clone(), |connection| {
        let mut statement = connection
            .prepare("SELECT COUNT(*) FROM sync_state WHERE sync_status = 'conflict'")
            .map_err(|error| error.to_string())?;
        if statement.next().map_err(|error| error.to_string())? == State::Row {
            return Ok(statement.read::<i64, _>(0).unwrap_or(0).max(0) as usize);
        }
        Ok(0)
    })
    .await
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// Whether a local scan may trust hashes recorded by an earlier cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalScanMode {
    /// Read and hash every file.
    HashEveryFile,
    /// Reuse the recorded hash of a file whose size and modification time
    /// still match what was recorded when it was last in sync.
    ReuseRecordedHashes,
}

/// Result of walking a mapped local folder.
#[derive(Debug, Default)]
pub(crate) struct LocalScan {
    pub tree: FileTree,
    /// Files whose content was read and hashed in this scan.
    pub hashed: usize,
    /// Files whose recorded hash was reused.
    pub reused: usize,
    /// Unchanged files whose recorded size or modification time is out of date
    /// (for example after a download), so later scans can reuse their hash.
    pub refresh_metadata: Vec<TreeEntry>,
}

#[derive(Clone)]
struct RecordedLocalFile {
    file_size: u64,
    modified_at: Option<i64>,
    hash: String,
}

fn recorded_local_files(synced: &SyncedTree) -> HashMap<String, RecordedLocalFile> {
    synced
        .values()
        .filter(|entry| entry.sync_status == "synced")
        .filter_map(|entry| {
            let hash = entry.local_hash.clone().filter(|hash| !hash.is_empty())?;
            Some((
                entry.relative_path.clone(),
                RecordedLocalFile {
                    file_size: entry.file_size,
                    modified_at: entry.local_mtime,
                    hash,
                },
            ))
        })
        .collect()
}

/// A timestamp is trustworthy only once it is old enough that another write
/// could not share the same second.
fn settled_mtime(modified_at: Option<i64>, now: i64) -> Option<i64> {
    modified_at.filter(|modified_at| now - *modified_at >= RACY_MTIME_WINDOW_SECS)
}

pub(crate) async fn scan_local(
    root: &str,
    preferences: &policy::SyncPreferences,
    mode: LocalScanMode,
    synced: Option<&SyncedTree>,
) -> Result<LocalScan, String> {
    let root = PathBuf::from(root);
    let preferences = preferences.clone();
    // Recorded hashes are needed in both modes: to reuse them, or to notice
    // that an unchanged file's recorded metadata has gone stale.
    let recorded = synced.map(recorded_local_files).unwrap_or_default();
    tokio::task::spawn_blocking(move || {
        let canonical_root = root.canonicalize().map_err(|error| error.to_string())?;
        let now = chrono::Utc::now().timestamp();
        let mut scan = LocalScan::default();
        let entries = walkdir::WalkDir::new(&canonical_root).follow_links(false).into_iter().filter_entry(|entry| {
            if entry.depth() == 0 { return true; }
            let relative = entry.path().strip_prefix(&canonical_root).unwrap_or(entry.path()).components().map(|component| component.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            !(preferences.ignores(&relative) || (entry.file_type().is_dir() && preferences.ignores(&format!("{relative}/"))))
        });
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            if !entry.file_type().is_file() {
                continue;
            }
            if entry
                .file_name()
                .to_string_lossy()
                .ends_with(".td-sync-tmp")
            {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(&canonical_root)
                .map_err(|error| error.to_string())?;
            let relative_path = relative
                .components()
                .map(|part| {
                    part.as_os_str().to_str().ok_or_else(|| {
                        format!(
                            "Folder contains a non-UTF-8 filename that cannot be represented safely: {}",
                            entry.path().display()
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
                .join("/");
            let metadata = entry.metadata().map_err(|error| error.to_string())?;
            let modified_at = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_secs() as i64);
            let previous = recorded.get(&relative_path);
            let metadata_matches = previous.is_some_and(|previous| {
                previous.file_size == metadata.len()
                    && previous.modified_at.is_some()
                    && previous.modified_at == settled_mtime(modified_at, now)
            });
            let hash = match previous {
                Some(previous)
                    if mode == LocalScanMode::ReuseRecordedHashes && metadata_matches =>
                {
                    scan.reused += 1;
                    previous.hash.clone()
                }
                _ => {
                    scan.hashed += 1;
                    hash_file(entry.path())?
                }
            };
            let tree_entry = TreeEntry {
                relative_path: relative_path.clone(),
                hash,
                file_size: metadata.len(),
                modified_at,
                message_id: None,
            };
            if previous.is_some_and(|previous| previous.hash == tree_entry.hash)
                && !metadata_matches
                && settled_mtime(modified_at, now).is_some()
            {
                scan.refresh_metadata.push(tree_entry.clone());
            }
            scan.tree.insert(relative_path, tree_entry);
        }
        Ok(scan)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Record the current size and modification time of files whose content is
/// unchanged, so the incremental scanner can skip hashing them next time.
async fn refresh_local_metadata(
    db: &DbConnection,
    pair_id: i64,
    entries: Vec<TreeEntry>,
) -> Result<(), String> {
    if entries.is_empty() {
        return Ok(());
    }
    crate::db::with_connection(db.clone(), move |connection| {
        let mut statement = connection
            .prepare(
                "UPDATE sync_state SET file_size = ?, local_mtime = ? WHERE pair_id = ? AND relative_path = ? AND local_hash = ? AND sync_status = 'synced'",
            )
            .map_err(|error| error.to_string())?;
        for entry in &entries {
            statement.reset().map_err(|error| error.to_string())?;
            statement
                .bind((1, entry.file_size as i64))
                .map_err(|error| error.to_string())?;
            statement
                .bind::<(usize, Option<i64>)>((2, entry.modified_at))
                .map_err(|error| error.to_string())?;
            statement
                .bind((3, pair_id))
                .map_err(|error| error.to_string())?;
            statement
                .bind((4, entry.relative_path.as_str()))
                .map_err(|error| error.to_string())?;
            statement
                .bind((5, entry.hash.as_str()))
                .map_err(|error| error.to_string())?;
            statement.next().map_err(|error| error.to_string())?;
        }
        Ok(())
    })
    .await
}

fn is_safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains(['\\', '\0'])
        && path
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

async fn scan_remote(
    app: &tauri::AppHandle,
    pair: &SyncPair,
    synced: &SyncedTree,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    account: &crate::workspace::AccountGuard,
) -> Result<FileTree, String> {
    let mut attempt = 0u32;
    loop {
        account.validate()?;
        match scan_remote_once(app, pair, synced, account, &shutdown).await {
            Ok(tree) => return Ok(tree),
            Err(error) => {
                let Some(server_wait) = flood_wait_seconds(&error) else {
                    return Err(error);
                };
                if attempt >= 5 {
                    return Err(error);
                }
                let wait = server_wait.max(1u64 << attempt.min(8));
                log::warn!("Remote sync scan hit FLOOD_WAIT; retrying in {wait}s");
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(wait)) => {}
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            return Err("Folder sync shutdown requested".to_string());
                        }
                    }
                }
                attempt += 1;
            }
        }
    }
}

/// What one Telegram file message contributes to a mapped folder's remote tree.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteFile {
    pub message_id: i32,
    #[serde(default)]
    pub caption: String,
    /// Document filename. `None` for a photo, which Telegram stores unnamed.
    #[serde(default)]
    pub document_name: Option<String>,
    pub media_id: i64,
    pub file_size: u64,
    pub created_at: i64,
    #[serde(default)]
    pub edited_at: Option<i64>,
    /// For a protected file with no recorded path: the path stored inside its
    /// encrypted metadata by the sync upload that created it.
    #[serde(default)]
    pub sync_path: Option<String>,
    /// A protected file's real size. Telegram reports the envelope's size.
    #[serde(default)]
    pub plaintext_size: Option<u64>,
}

pub(crate) const MAX_REMOTE_FILES: usize = 50_000;

/// Telegram photos have no filename. Naming each after its message keeps
/// several photos in one folder from all claiming the same sync path.
fn photo_file_name(message_id: i32) -> String {
    format!("Photo-{message_id}.jpg")
}

fn remote_file_from_message(message: &grammers_client::types::Message) -> Option<RemoteFile> {
    let media = message.media()?;
    let (document_name, media_id) = match &media {
        grammers_client::types::Media::Document(document) => {
            (Some(document.name().to_string()), document.id())
        }
        grammers_client::types::Media::Photo(photo) => (None, photo.id()),
        _ => return None,
    };
    Some(RemoteFile {
        message_id: message.id(),
        caption: message.text().to_string(),
        document_name,
        media_id,
        file_size: media_size(&media),
        created_at: message.date().timestamp(),
        edited_at: message.edit_date().map(|date| date.timestamp()),
        sync_path: None,
        plaintext_size: None,
    })
}

fn is_protected(file: &RemoteFile) -> bool {
    crate::workspace::envelope_cache::suspected_envelope(
        file.document_name.as_deref().unwrap_or_default(),
        &file.caption,
    )
}

/// The path a remote file occupies: the path recorded for its message, else a
/// safe caption (sync writes the relative path there), else its filename.
fn remote_relative_path(file: &RemoteFile, known_paths: &HashMap<i32, String>) -> Option<String> {
    if let Some(known) = known_paths.get(&file.message_id) {
        return Some(known.clone());
    }
    // A protected file carries the envelope marker as its caption and an
    // opaque remote name; neither is its path. Its path is the one its sync
    // upload stored inside the encrypted metadata. A protected file without
    // one was not uploaded by sync and is left alone rather than synced under
    // a meaningless or shared name.
    if is_protected(file) {
        return file.sync_path.clone().filter(|path| is_safe_relative(path));
    }
    if is_safe_relative(&file.caption) {
        return Some(file.caption.clone());
    }
    let name = file
        .document_name
        .clone()
        .unwrap_or_else(|| photo_file_name(file.message_id));
    is_safe_relative(&name).then_some(name)
}

fn remote_tree_entry(file: &RemoteFile, relative_path: String) -> TreeEntry {
    TreeEntry {
        relative_path,
        hash: remote_fingerprint(
            file.file_size,
            file.created_at,
            file.edited_at,
            file.message_id,
            file.media_id,
        ),
        // The fingerprint keeps Telegram's own size; comparisons with local
        // files use the size of the content.
        file_size: file.plaintext_size.unwrap_or(file.file_size),
        modified_at: Some(file.edited_at.unwrap_or(file.created_at)),
        message_id: Some(file.message_id),
    }
}

/// Add one remote file to the tree. Two messages claiming one path stop the
/// mapping, because deleting or replacing either would be a guess.
fn insert_remote_file(
    tree: &mut FileTree,
    file: &RemoteFile,
    known_paths: &HashMap<i32, String>,
    preferences: &policy::SyncPreferences,
) -> Result<(), String> {
    let Some(relative_path) = remote_relative_path(file, known_paths) else {
        return Ok(());
    };
    if let Some(existing) = tree.get(&relative_path) {
        if preferences.ignores(&relative_path) {
            return Ok(());
        }
        return Err(format!(
            "Multiple Telegram messages map to the same sync path '{relative_path}' (message {} and {}); rename or remove the duplicate before syncing",
            existing.message_id.unwrap_or_default(),
            file.message_id
        ));
    }
    tree.insert(
        relative_path.clone(),
        remote_tree_entry(file, relative_path),
    );
    Ok(())
}

/// Build a mapping's remote tree from a list of file messages. Production
/// streams the same per-file logic from Telegram; the native E2E driver uses
/// this to supply the Telegram side as a controlled fixture.
#[cfg(feature = "native-e2e")]
pub(crate) fn build_remote_tree(
    files: &[RemoteFile],
    synced: &SyncedTree,
    preferences: &policy::SyncPreferences,
) -> Result<FileTree, String> {
    if files.len() > MAX_REMOTE_FILES {
        return Err(format!(
            "Telegram channel contains more than {MAX_REMOTE_FILES} file messages; sync paused rather than building an unsafe remote tree"
        ));
    }
    let known_paths = known_message_paths(synced);
    let mut tree = FileTree::new();
    for file in files {
        insert_remote_file(&mut tree, file, &known_paths, preferences)?;
    }
    Ok(tree)
}

fn known_message_paths(synced: &SyncedTree) -> HashMap<i32, String> {
    synced
        .values()
        .filter_map(|entry| entry.message_id.map(|id| (id, entry.relative_path.clone())))
        .collect()
}

/// Longest time one scan spends fetching envelope headers from Telegram, and
/// the longest it waits for one of them. What is not reached is fetched by a
/// later scan; fetched headers are kept.
const PROTECTED_HEADER_SCAN_BUDGET: Duration = Duration::from_secs(60);
const PROTECTED_HEADER_TIMEOUT: Duration = Duration::from_secs(5);
/// Record of a protected file that was looked at and is not a sync upload.
const UNPLACED_RECORD_KIND: &str = "sync-unplaced-v1";

#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Eq, Hash)]
struct UnplacedEnvelope {
    folder: i64,
    message: i32,
    /// A message can be edited to hold a different document.
    document: i64,
}

enum ProtectedPlacement {
    /// The envelope was read: it either has a sync path or is not ours.
    Known(crate::commands::fs::ProtectedSyncMetadata),
    /// The vault is locked, so nothing can be said about it.
    Locked,
    /// Its header could not be fetched in this scan.
    Deferred,
}

/// Finds where protected files without a recorded path belong, by reading the
/// path their sync upload stored inside the envelope. Needed after the
/// mapping's records are lost: a reinstall, a new device, or a mapping that
/// was removed and added again.
struct ProtectedPathResolver<'a> {
    account: &'a crate::workspace::AccountGuard,
    client: &'a grammers_client::Client,
    folder: i64,
    vault_key: Option<crate::crypto::secret::SecretKey>,
    unplaced: std::collections::HashSet<UnplacedEnvelope>,
    budget: Duration,
}

impl<'a> ProtectedPathResolver<'a> {
    async fn new(
        app: &tauri::AppHandle,
        account: &'a crate::workspace::AccountGuard,
        client: &'a grammers_client::Client,
        folder: i64,
    ) -> Result<Self, String> {
        let store_account = account.clone();
        let unplaced = tokio::task::spawn_blocking(move || {
            store_account.validate()?;
            let store =
                crate::workspace::store::Store::open(&store_account.root, store_account.owner)?;
            Ok::<_, String>(
                store
                    .records::<serde_json::Value>(UNPLACED_RECORD_KIND)?
                    .into_iter()
                    .filter_map(|value| serde_json::from_value::<UnplacedEnvelope>(value).ok())
                    .collect(),
            )
        })
        .await
        .map_err(|error| error.to_string())??;
        Ok(Self {
            account,
            client,
            folder,
            vault_key: app
                .state::<crate::crypto::state::CryptoState>()
                .get_current_wrapping_key()
                .ok(),
            unplaced,
            budget: PROTECTED_HEADER_SCAN_BUDGET,
        })
    }

    async fn place(
        &mut self,
        message: &grammers_client::types::Message,
        file: &RemoteFile,
    ) -> Result<ProtectedPlacement, String> {
        let marker = UnplacedEnvelope {
            folder: self.folder,
            message: file.message_id,
            document: file.media_id,
        };
        if self.unplaced.contains(&marker) {
            return Ok(ProtectedPlacement::Known(
                crate::commands::fs::ProtectedSyncMetadata {
                    sync_path: None,
                    plaintext_size: file.file_size,
                },
            ));
        }
        let Some(vault_key) = self.vault_key.as_ref() else {
            return Ok(ProtectedPlacement::Locked);
        };
        let Some(media) = message.media() else {
            return Ok(ProtectedPlacement::Deferred);
        };
        let identity = crate::workspace::envelope_cache::RemoteEnvelopeIdentity {
            owner: self.account.owner,
            folder: Some(self.folder),
            message: file.message_id,
            document: file.media_id,
            ciphertext_size: file.file_size,
        };
        // A header fetched before is read from this account's store; the
        // network is used, within the scan's budget, only for a new one.
        let budget = self.budget;
        let started = Instant::now();
        let mut fetched = false;
        let account = self.account;
        let client = self.client;
        let header = crate::workspace::envelope_cache::resolve(self.account, &identity, async {
            fetched = true;
            if budget.is_zero() {
                return Err("deferred".to_string());
            }
            tokio::time::timeout(
                PROTECTED_HEADER_TIMEOUT.min(budget),
                crate::commands::fs::probe_tdenc2_header(account, client, &media),
            )
            .await
            .map_err(|_| "deferred".to_string())?
        })
        .await;
        if fetched {
            self.budget = self.budget.saturating_sub(started.elapsed());
        }
        self.account.validate()?;
        let Ok(header) = header else {
            return Ok(ProtectedPlacement::Deferred);
        };
        let metadata = crate::commands::fs::protected_sync_metadata(&header, vault_key)?;
        if metadata.sync_path.is_none() {
            // Remembered, so later scans need neither the vault nor the header.
            let store_account = self.account.clone();
            let record = marker.clone();
            tokio::task::spawn_blocking(move || {
                store_account.validate()?;
                crate::workspace::store::Store::open(&store_account.root, store_account.owner)?
                    .put_record(
                        UNPLACED_RECORD_KIND,
                        &crate::workspace::store::file_key(
                            Some(record.folder),
                            record.message.into(),
                        ),
                        &record,
                    )
            })
            .await
            .map_err(|error| error.to_string())??;
            self.unplaced.insert(marker);
        }
        Ok(ProtectedPlacement::Known(metadata))
    }
}

async fn scan_remote_once(
    app: &tauri::AppHandle,
    pair: &SyncPair,
    synced: &SyncedTree,
    account: &crate::workspace::AccountGuard,
    shutdown: &tokio::sync::watch::Receiver<bool>,
) -> Result<FileTree, String> {
    #[cfg(feature = "native-e2e")]
    if let Some(fixture) = app.try_state::<Arc<native_fixture::NativeSyncFixture>>() {
        if *shutdown.borrow() {
            return Err("Folder sync shutdown requested".into());
        }
        return fixture.scan(pair, account).await;
    }
    let telegram = app.state::<TelegramState>();
    let client = telegram.client.lock().await.clone().ok_or(
        "Telegram is offline; remote tree unavailable, so no reconciliation was attempted",
    )?;
    account.validate_client(&client).await?;
    let peer = resolve_peer(&client, Some(pair.channel_id), &telegram.peer_cache).await?;
    let known_paths = known_message_paths(synced);
    let mut protected = ProtectedPathResolver::new(app, account, &client, pair.channel_id).await?;
    let mut unidentified = 0usize;
    let mut messages = client.iter_messages(&peer);
    let mut tree = FileTree::new();
    let mut scanned_files = 0usize;
    while let Some(message) = messages.next().await.map_err(|error| error.to_string())? {
        account.validate()?;
        // A long history must not hold up quitting or reconfiguring a mapping.
        // An incomplete tree is never returned, so nothing is planned from it.
        if *shutdown.borrow() {
            return Err("Folder sync shutdown requested".to_string());
        }
        let Some(mut file) = remote_file_from_message(&message) else {
            continue;
        };
        scanned_files += 1;
        if scanned_files > MAX_REMOTE_FILES {
            return Err(format!(
                "Telegram channel contains more than {MAX_REMOTE_FILES} file messages; sync paused rather than building an unsafe remote tree"
            ));
        }
        if is_protected(&file) && !known_paths.contains_key(&file.message_id) {
            match protected.place(&message, &file).await? {
                ProtectedPlacement::Known(metadata) => {
                    file.sync_path = metadata.sync_path;
                    file.plaintext_size = Some(metadata.plaintext_size);
                }
                // With the vault locked nothing protected can be uploaded or
                // opened by this mapping either; the file stays out of the
                // tree, as it always has.
                ProtectedPlacement::Locked => {}
                ProtectedPlacement::Deferred => unidentified += 1,
            }
        }
        insert_remote_file(&mut tree, &file, &known_paths, &pair.preferences)?;
    }
    account.validate()?;
    // Planning without these files could upload a second copy of each. The
    // headers fetched so far are kept, so every scan gets further.
    if unidentified > 0 {
        return Err(format!(
            "{unidentified} protected file(s) in this Telegram folder are still being identified; sync continues automatically"
        ));
    }
    Ok(tree)
}

/// Read back the messages this cycle uploaded, so their fingerprints can be
/// recorded without re-reading the whole Telegram folder.
async fn fetch_uploaded_entries(
    app: &tauri::AppHandle,
    pair: &SyncPair,
    account: &crate::workspace::AccountGuard,
    uploads: &[(String, i32)],
) -> Result<FileTree, String> {
    let mut tree = FileTree::new();
    if uploads.is_empty() {
        return Ok(tree);
    }
    let telegram = app.state::<TelegramState>();
    let client = telegram
        .client
        .lock()
        .await
        .clone()
        .ok_or("Telegram is offline; uploaded files will be verified on the next sync")?;
    account.validate_client(&client).await?;
    let peer = resolve_peer(&client, Some(pair.channel_id), &telegram.peer_cache).await?;
    for chunk in uploads.chunks(100) {
        let ids: Vec<i32> = chunk.iter().map(|(_, message_id)| *message_id).collect();
        let messages = client
            .get_messages_by_id(&peer, &ids)
            .await
            .map_err(|error| error.to_string())?;
        let files: Vec<RemoteFile> = messages
            .iter()
            .flatten()
            .filter_map(remote_file_from_message)
            .collect();
        tree.extend(uploaded_tree(chunk, &files));
        account.validate()?;
    }
    Ok(tree)
}

/// Remote entries for this cycle's uploads, at the paths they were planned
/// for rather than whatever name Telegram reports for the message.
pub(crate) fn uploaded_tree(uploads: &[(String, i32)], files: &[RemoteFile]) -> FileTree {
    let mut tree = FileTree::new();
    for file in files {
        if let Some((relative_path, _)) = uploads
            .iter()
            .find(|(_, message_id)| *message_id == file.message_id)
        {
            tree.insert(
                relative_path.clone(),
                remote_tree_entry(file, relative_path.clone()),
            );
        }
    }
    tree
}

fn remote_fingerprint(
    file_size: u64,
    created_at: i64,
    edited_at: Option<i64>,
    message_id: i32,
    media_id: i64,
) -> String {
    format!(
        "v2:{:x}",
        Sha256::digest(
            format!("{file_size}:{created_at}:{edited_at:?}:{message_id}:{media_id}").as_bytes()
        )
    )
}

fn message_fingerprint(message: &grammers_client::types::Message) -> Result<String, String> {
    let media = message
        .media()
        .ok_or("The remote file has no downloadable media")?;
    let media_id = match &media {
        grammers_client::types::Media::Document(document) => document.id(),
        grammers_client::types::Media::Photo(photo) => photo.id(),
        _ => return Err("The remote media type cannot be compared safely".into()),
    };
    Ok(remote_fingerprint(
        media_size(&media),
        message.date().timestamp(),
        message.edit_date().map(|date| date.timestamp()),
        message.id(),
        media_id,
    ))
}

fn retain_tree_paths(
    local: &mut FileTree,
    remote: &mut FileTree,
    synced: &mut SyncedTree,
    preferences: &policy::SyncPreferences,
) -> usize {
    let ignored: std::collections::BTreeSet<_> = local
        .keys()
        .chain(remote.keys())
        .chain(synced.keys())
        .filter(|path| preferences.ignores(path))
        .cloned()
        .collect();
    for path in &ignored {
        local.remove(path);
        remote.remove(path);
        synced.remove(path);
    }
    ignored.len()
}

#[derive(Debug, PartialEq, Eq)]
enum CleanupAction {
    DeleteOld,
    AlreadyGone,
    PreserveOld,
}

fn cleanup_action(old_exists: bool, new_exists: bool, old_unchanged: bool) -> CleanupAction {
    match (old_exists, new_exists, old_unchanged) {
        (false, _, _) => CleanupAction::AlreadyGone,
        (true, true, true) => CleanupAction::DeleteOld,
        (true, _, _) => CleanupAction::PreserveOld,
    }
}

/// Retry publication cleanup before scanning a channel, so a temporary error
/// cannot turn the application's own superseded versions into a permanent
/// duplicate-path failure. The journal is persisted before deleting anything.
async fn retry_pending_cleanup(
    app: &tauri::AppHandle,
    db: &DbConnection,
    pair: &SyncPair,
    account: &crate::workspace::AccountGuard,
) -> Result<(), String> {
    replay_pending_cleanup(db, pair.id, |item| async move {
        account.validate()?;
        let telegram = app.state::<TelegramState>();
        let client = telegram
            .client
            .lock()
            .await
            .clone()
            .ok_or("Telegram is offline; replacement cleanup will retry when connected")?;
        let peer = resolve_peer(&client, Some(pair.channel_id), &telegram.peer_cache).await?;
        let messages = client
            .get_messages_by_id(&peer, &[item.old_message_id, item.new_message_id])
            .await
            .map_err(|error| {
                format!("Uploaded replacement is preserved; cleanup will retry: {error}")
            })?;
        let old_exists = messages
            .iter()
            .flatten()
            .any(|message| message.id() == item.old_message_id);
        let new_exists = messages
            .iter()
            .flatten()
            .any(|message| message.id() == item.new_message_id && message.media().is_some());
        let old_unchanged = messages.iter().flatten().find(|message| message.id() == item.old_message_id)
            .and_then(|message| message_fingerprint(message).ok()).zip(item.old_remote_hash.as_ref())
            .is_some_and(|(actual, expected)| actual == *expected);
        let action = cleanup_action(old_exists, new_exists, old_unchanged);
        let preserve_old = action == CleanupAction::PreserveOld;
        match action {
            CleanupAction::DeleteOld => {
                account.validate()?;
                executor::delete_remote(app, pair.channel_id, item.old_message_id, account)
                    .await
                    .map_err(|error| {
                        format!("Uploaded replacement is preserved; cleanup will retry: {error}")
                    })?;
            }
            CleanupAction::PreserveOld => {
                set_state_status(db, pair.id, &item.relative_path, "conflict").await?;
                log_sync(db.clone(), Some(pair.id), "conflict".into(), Some(item.relative_path.clone()), Some("The previous remote file changed or its replacement disappeared before cleanup; the previous copy was preserved".into())).await;
            }
            CleanupAction::AlreadyGone => {}
        }
        account.validate()?;
        Ok(preserve_old)
    }).await
}

async fn replay_pending_cleanup<F, Fut>(
    db: &DbConnection,
    pair_id: i64,
    mut perform: F,
) -> Result<(), String>
where
    F: FnMut(config::PendingCleanup) -> Fut,
    Fut: std::future::Future<Output = Result<bool, String>>,
{
    let mut pending = config::load_pending_cleanup(db.clone(), pair_id).await?;
    while let Some(item) = pending.first().cloned() {
        let preserve_old = perform(item).await?;
        pending.remove(0);
        config::save_pending_cleanup(db.clone(), pair_id, &pending).await?;
        if preserve_old {
            return Err("A previous remote file changed or its replacement disappeared before cleanup. Both available copies were preserved; review the conflict".into());
        }
    }
    Ok(())
}

pub(crate) async fn load_synced_tree(
    db: &DbConnection,
    pair_id: i64,
) -> Result<SyncedTree, String> {
    crate::db::with_connection(db.clone(), move |connection| {
    let mut statement = connection.prepare(
        "SELECT relative_path, local_hash, remote_hash, file_size, local_mtime, remote_date, message_id, sync_status FROM sync_state WHERE pair_id = ?",
    ).map_err(|error| error.to_string())?;
    statement
        .bind((1, pair_id))
        .map_err(|error| error.to_string())?;
    let mut tree = SyncedTree::new();
    while statement.next().map_err(|error| error.to_string())? == State::Row {
        let relative_path: String = statement.read(0).map_err(|error| error.to_string())?;
        tree.insert(
            relative_path.clone(),
            SyncedEntry {
                relative_path,
                local_hash: statement.read::<Option<String>, _>(1).ok().flatten(),
                remote_hash: statement.read::<Option<String>, _>(2).ok().flatten(),
                file_size: statement.read::<i64, _>(3).unwrap_or(0).max(0) as u64,
                local_mtime: statement.read::<Option<i64>, _>(4).ok().flatten(),
                remote_date: statement.read::<Option<i64>, _>(5).ok().flatten(),
                message_id: statement
                    .read::<Option<i64>, _>(6)
                    .ok()
                    .flatten()
                    .and_then(|id| i32::try_from(id).ok()),
                sync_status: statement.read(7).unwrap_or_else(|_| "synced".to_string()),
            },
        );
    }
    Ok(tree)
    }).await
}

async fn upsert_state(
    db: &DbConnection,
    pair_id: i64,
    path: &str,
    local: Option<&TreeEntry>,
    remote: Option<&TreeEntry>,
    status: &str,
) -> Result<(), String> {
    let path = path.to_string();
    let local = local.cloned();
    let remote = remote.cloned();
    let status = status.to_string();
    crate::db::with_connection(db.clone(), move |connection| {
    let mut statement = connection.prepare(
        "INSERT INTO sync_state (pair_id, relative_path, local_hash, remote_hash, file_size, local_mtime, remote_date, message_id, sync_status)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(pair_id, relative_path) DO UPDATE SET local_hash=excluded.local_hash, remote_hash=excluded.remote_hash, file_size=excluded.file_size, local_mtime=excluded.local_mtime, remote_date=excluded.remote_date, message_id=excluded.message_id, sync_status=excluded.sync_status",
    ).map_err(|error| error.to_string())?;
    statement
        .bind((1, pair_id))
        .map_err(|error| error.to_string())?;
    statement
        .bind((2, path.as_str()))
        .map_err(|error| error.to_string())?;
    statement
        .bind::<(usize, Option<&str>)>((3, local.as_ref().map(|entry| entry.hash.as_str())))
        .map_err(|error| error.to_string())?;
    statement
        .bind::<(usize, Option<&str>)>((4, remote.as_ref().map(|entry| entry.hash.as_str())))
        .map_err(|error| error.to_string())?;
    statement
        .bind((
            5,
            local
                .as_ref()
                .or(remote.as_ref())
                .map(|entry| entry.file_size as i64)
                .unwrap_or(0),
        ))
        .map_err(|error| error.to_string())?;
    // A timestamp from the last couple of seconds is not recorded: a later
    // edit within the same second would otherwise be mistaken for no change.
    let settled_local_mtime = settled_mtime(
        local.as_ref().and_then(|entry| entry.modified_at),
        chrono::Utc::now().timestamp(),
    );
    statement
        .bind::<(usize, Option<i64>)>((6, settled_local_mtime))
        .map_err(|error| error.to_string())?;
    statement
        .bind::<(usize, Option<i64>)>((7, remote.as_ref().and_then(|entry| entry.modified_at)))
        .map_err(|error| error.to_string())?;
    statement
        .bind::<(usize, Option<i64>)>((8, remote.as_ref().and_then(|entry| entry.message_id).map(i64::from)))
        .map_err(|error| error.to_string())?;
    statement
        .bind((9, status.as_str()))
        .map_err(|error| error.to_string())?;
    statement.next().map_err(|error| error.to_string())?;
    Ok(())
    }).await
}

async fn delete_state(db: &DbConnection, pair_id: i64, path: &str) -> Result<(), String> {
    let path = path.to_string();
    crate::db::with_connection(db.clone(), move |connection| {
        let mut statement = connection
            .prepare("DELETE FROM sync_state WHERE pair_id = ? AND relative_path = ?")
            .map_err(|error| error.to_string())?;
        statement
            .bind((1, pair_id))
            .map_err(|error| error.to_string())?;
        statement
            .bind((2, path.as_str()))
            .map_err(|error| error.to_string())?;
        statement.next().map_err(|error| error.to_string())?;
        Ok(())
    })
    .await
}

async fn set_state_status(
    db: &DbConnection,
    pair_id: i64,
    path: &str,
    status: &str,
) -> Result<(), String> {
    let path = path.to_string();
    let status = status.to_string();
    crate::db::with_connection(db.clone(), move |connection| {
        let mut statement = connection
            .prepare(
                "UPDATE sync_state SET sync_status = ? WHERE pair_id = ? AND relative_path = ?",
            )
            .map_err(|error| error.to_string())?;
        statement
            .bind((1, status.as_str()))
            .map_err(|error| error.to_string())?;
        statement
            .bind((2, pair_id))
            .map_err(|error| error.to_string())?;
        statement
            .bind((3, path.as_str()))
            .map_err(|error| error.to_string())?;
        statement.next().map_err(|error| error.to_string())?;
        Ok(())
    })
    .await
}

/// What one cycle decided before it touches Telegram or the local folder.
pub(crate) struct PreparedReconciliation {
    pub local: FileTree,
    pub remote: FileTree,
    pub synced: SyncedTree,
    pub operations: Vec<SyncOperation>,
    pub conflicts: usize,
    /// Local files read and hashed by this cycle's scan.
    pub hashed: usize,
    /// Local files whose recorded hash was reused.
    pub reused: usize,
    /// Paths recorded as already in sync without a transfer.
    pub baselined: Vec<String>,
}

/// Scan the local folder, reconcile it with the remote tree and the recorded
/// state, and plan the cycle. Persists only bookkeeping that needs no
/// transfer: vanished entries, completed or adopted baselines, and conflicts.
pub(crate) async fn prepare_reconciliation(
    db: &DbConnection,
    pair: &SyncPair,
    scan_mode: LocalScanMode,
    mut synced: SyncedTree,
    mut remote: FileTree,
) -> Result<PreparedReconciliation, String> {
    let LocalScan {
        tree: mut local,
        hashed,
        reused,
        refresh_metadata,
    } = scan_local(
        &pair.local_path,
        &pair.preferences,
        scan_mode,
        Some(&synced),
    )
    .await?;
    retain_tree_paths(&mut local, &mut remote, &mut synced, &pair.preferences);
    for vanished_path in synced
        .keys()
        .filter(|path| !local.contains_key(*path) && !remote.contains_key(*path))
    {
        delete_state(db, pair.id, vanished_path).await?;
    }
    // Finish uploads that were journaled but not verified before the engine
    // last stopped, then record files an adopting mapping already shares.
    let mut baselined = planner::resume_interrupted_uploads(&local, &remote, &mut synced);
    if pair.preferences.adopt_matching_files {
        baselined.extend(planner::adopt_matching_files(&local, &remote, &mut synced));
    }
    for path in &baselined {
        upsert_state(
            db,
            pair.id,
            path,
            local.get(path),
            remote.get(path),
            "synced",
        )
        .await?;
    }
    refresh_local_metadata(
        db,
        pair.id,
        refresh_metadata
            .into_iter()
            .filter(|entry| local.contains_key(&entry.relative_path))
            .collect(),
    )
    .await?;
    let operations = planner::plan_for_policy(
        &local,
        &remote,
        &synced,
        &pair.sync_direction,
        pair.preferences.propagate_deletions,
    )
    .map_err(|error| error.to_string())?;
    let conflicts = operations
        .iter()
        .filter(|operation| matches!(operation, SyncOperation::Conflict { .. }))
        .count();
    for operation in operations
        .iter()
        .filter(|operation| matches!(operation, SyncOperation::Conflict { .. }))
    {
        if synced.contains_key(operation.path()) {
            set_state_status(db, pair.id, operation.path(), "conflict").await?;
        } else {
            upsert_state(
                db,
                pair.id,
                operation.path(),
                local.get(operation.path()),
                remote.get(operation.path()),
                "conflict",
            )
            .await?;
        }
    }
    Ok(PreparedReconciliation {
        local,
        remote,
        synced,
        operations,
        conflicts,
        hashed,
        reused,
        baselined,
    })
}

/// Operations that still need a transfer or a deletion.
fn actionable_operations(operations: &[SyncOperation]) -> usize {
    operations
        .iter()
        .filter(|operation| {
            !matches!(
                operation,
                SyncOperation::Skip { .. } | SyncOperation::Conflict { .. }
            )
        })
        .count()
}

/// Existing unchanged entries need no transfer or database write. Keeping
/// them out of the executor avoids one log row per file every poll cycle.
pub(crate) fn drop_settled_skips(prepared: &mut PreparedReconciliation) {
    let PreparedReconciliation {
        local,
        remote,
        synced,
        operations,
        ..
    } = prepared;
    operations.retain(|operation| {
        !matches!(operation, SyncOperation::Skip { .. })
            || !synced.contains_key(operation.path())
            || (local
                .get(operation.path())
                .zip(remote.get(operation.path()))
                .zip(synced.get(operation.path()))
                .is_some_and(|((local, remote), previous)| {
                    previous.local_hash.as_deref() != Some(local.hash.as_str())
                        || previous.remote_hash.as_deref() != Some(remote.hash.as_str())
                }))
    });
}

/// Journal uploaded message ids before anything else. They map encrypted
/// Telegram filenames back to their relative paths, and they let the next
/// cycle finish the bookkeeping if this one is interrupted from here on.
pub(crate) async fn journal_uploads(
    db: &DbConnection,
    pair_id: i64,
    local: &FileTree,
    results: &[executor::ExecutionResult],
) -> Result<Vec<(String, i32)>, String> {
    let mut uploads = Vec::new();
    for result in results
        .iter()
        .filter(|result| result.success && result.action == "upload")
    {
        let (Some(message_id), Some(local_entry)) =
            (result.message_id, local.get(&result.relative_path))
        else {
            continue;
        };
        journal_upload(db, pair_id, &result.relative_path, local_entry, message_id).await?;
        uploads.push((result.relative_path.clone(), message_id));
    }
    Ok(uploads)
}

/// Record that `message_id` holds the upload of `local_entry`, before its
/// remote fingerprint is known.
pub(crate) async fn journal_upload(
    db: &DbConnection,
    pair_id: i64,
    relative_path: &str,
    local_entry: &TreeEntry,
    message_id: i32,
) -> Result<(), String> {
    let mut remote_stub = local_entry.clone();
    remote_stub.message_id = Some(message_id);
    remote_stub.hash.clear();
    upsert_state(
        db,
        pair_id,
        relative_path,
        Some(local_entry),
        Some(&remote_stub),
        "syncing",
    )
    .await
}

/// Persist the outcome of every executed operation from the trees the cycle
/// planned against. `uploaded` holds the read-back entries of this cycle's
/// uploads; an upload missing from it stays journaled as `syncing`.
pub(crate) async fn record_results(
    db: &DbConnection,
    pair_id: i64,
    prepared: &PreparedReconciliation,
    results: Vec<executor::ExecutionResult>,
    uploaded: &FileTree,
) -> Result<(), String> {
    let PreparedReconciliation {
        local,
        remote,
        synced,
        ..
    } = prepared;
    for result in results {
        let path = result.relative_path.as_str();
        if !result.success {
            let status = if result.action == "conflict" {
                "conflict"
            } else if result
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("VAULT_LOCKED"))
            {
                "paused_vault"
            } else if result.action == "upload"
                && result.detail.as_deref().is_some_and(|detail| {
                    detail.contains("Telegram sync limit") || detail.contains("Telegram size limit")
                })
            {
                "skipped"
            } else {
                "error"
            };
            if synced.contains_key(path) {
                set_state_status(db, pair_id, path, status).await?;
            } else {
                upsert_state(db, pair_id, path, local.get(path), remote.get(path), status).await?;
            }
            continue;
        }
        match result.action.as_str() {
            // Baseline the exact hash that was planned and verified. If the
            // source changes again afterwards, retaining that earlier hash
            // guarantees the next cycle sees another local change.
            "upload" => {
                if let Some(remote_entry) = uploaded.get(path) {
                    upsert_state(
                        db,
                        pair_id,
                        path,
                        local.get(path),
                        Some(remote_entry),
                        "synced",
                    )
                    .await?;
                }
            }
            "download" => {
                let verified_download = result.local_hash.as_ref().and_then(|hash| {
                    remote.get(path).cloned().map(|mut entry| {
                        entry.hash = hash.clone();
                        entry.message_id = None;
                        // The local copy was written just now; a later scan
                        // records its timestamp once it has settled.
                        entry.modified_at = None;
                        entry
                    })
                });
                upsert_state(
                    db,
                    pair_id,
                    path,
                    verified_download.as_ref(),
                    remote.get(path),
                    "synced",
                )
                .await?;
            }
            "delete_local" | "delete_remote" => delete_state(db, pair_id, path).await?,
            // `keep_both` and `skip` leave both copies as they were planned.
            _ => match (local.get(path), remote.get(path)) {
                (None, None) => delete_state(db, pair_id, path).await?,
                (local_entry, remote_entry) => {
                    upsert_state(db, pair_id, path, local_entry, remote_entry, "synced").await?
                }
            },
        }
    }
    Ok(())
}

async fn reconcile_pair(
    app: &tauri::AppHandle,
    db: &DbConnection,
    pair: &SyncPair,
    settings: &config::SyncSettings,
    shutdown: tokio::sync::watch::Receiver<bool>,
    scan_mode: LocalScanMode,
) -> Result<(usize, usize, Option<String>), String> {
    let account = pair_account(app, pair)?;
    retry_pending_cleanup(app, db, pair, &account).await?;
    let synced = load_synced_tree(db, pair.id).await?;
    let remote = scan_remote(app, pair, &synced, shutdown.clone(), &account).await?;
    account.validate()?;
    let mut prepared = prepare_reconciliation(db, pair, scan_mode, synced, remote)
        .await
        .inspect_err(|message| {
            if message.starts_with("mass deletion protection") {
                let _ = app.emit("sync-mass-deletion-blocked", message);
            }
        })?;
    if scan_mode == LocalScanMode::ReuseRecordedHashes {
        log::debug!(
            "Folder sync pair {} hashed {} local file(s) and reused {} recorded hash(es)",
            pair.id,
            prepared.hashed,
            prepared.reused
        );
    }
    if !prepared.baselined.is_empty() {
        log::info!(
            "Folder sync pair {} recorded {} file(s) as already in sync without a transfer",
            pair.id,
            prepared.baselined.len()
        );
    }
    let conflicts = prepared.conflicts;
    if pair.preferences.pause_on_conflicts && conflicts > 0 {
        return Ok((
            actionable_operations(&prepared.operations),
            conflicts,
            Some("Paused because this mapping has conflicts; review them before continuing".into()),
        ));
    }
    if prepared
        .operations
        .iter()
        .any(|operation| matches!(operation, SyncOperation::Upload { .. }))
    {
        if let Err(reason) = executor::upload_protection_mode(app, settings) {
            return Ok((
                actionable_operations(&prepared.operations),
                conflicts,
                Some(reason),
            ));
        }
    }
    drop_settled_skips(&mut prepared);
    let operation_count = actionable_operations(&prepared.operations);
    if operation_count > 0 {
        let engine = app.state::<SyncEngine>();
        set_pair_status(
            app,
            &engine.status,
            SyncPairStatus {
                pair_id: pair.id,
                phase: "syncing".into(),
                pending_ops: operation_count,
                conflicts,
                ..SyncPairStatus::default()
            },
        )
        .await;
    }
    let operations = std::mem::take(&mut prepared.operations);
    let results = executor::execute(app, db, pair, settings, operations, &account).await;
    account.validate()?;
    let pending = results
        .iter()
        .filter(|result| !result.success && result.action != "conflict")
        .count();
    let execution_error = results
        .iter()
        .find(|result| !result.success && result.action != "conflict")
        .and_then(|result| result.detail.clone());

    // Our uploads publish replacements as new messages. Journal superseded
    // messages before cleanup so a transient error or restart can retry them.
    for result in results
        .iter()
        .filter(|result| result.success && result.action == "upload")
    {
        let old_message_id = prepared
            .remote
            .get(&result.relative_path)
            .and_then(|entry| entry.message_id);
        if let (Some(old_message_id), Some(new_message_id)) = (old_message_id, result.message_id) {
            if old_message_id != new_message_id {
                let mut pending = config::load_pending_cleanup(db.clone(), pair.id).await?;
                let cleanup = config::PendingCleanup {
                    relative_path: result.relative_path.clone(),
                    old_message_id,
                    new_message_id,
                    old_remote_hash: prepared
                        .remote
                        .get(&result.relative_path)
                        .map(|entry| entry.hash.clone()),
                };
                if !pending.contains(&cleanup) {
                    pending.push(cleanup);
                }
                config::save_pending_cleanup(db.clone(), pair.id, &pending).await?;
            }
        }
    }

    let uploads = journal_uploads(db, pair.id, &prepared.local, &results).await?;

    // Complete durable cleanup even on an idle cycle.
    account.validate()?;
    retry_pending_cleanup(app, db, pair, &account).await?;
    account.validate()?;

    // Only the messages uploaded in this cycle need to be read back. If that
    // is not possible now (offline, or the engine is stopping), they stay
    // journaled as `syncing` and are completed by the next cycle.
    let uploaded = if *shutdown.borrow() {
        FileTree::new()
    } else {
        match fetch_uploaded_entries(app, pair, &account, &uploads).await {
            Ok(tree) => tree,
            Err(error) => {
                log::warn!(
                    "Folder sync pair {} will verify its uploads on the next cycle: {error}",
                    pair.id
                );
                FileTree::new()
            }
        }
    };
    record_results(db, pair.id, &prepared, results, &uploaded).await?;
    Ok((pending, conflicts, execution_error))
}
