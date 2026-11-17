// ── Transcode Module ────────────────────────────────────────────────────
// Handles: FFmpeg detection, original source caching, HLS transcode jobs,
// and serving HLS playlists/segments via Actix routes.
//
// Cache layout:
//   $APPDATA/streaming/
//     originals/{folder_id}_{message_id}.mp4
//     hls/{folder_id}_{message_id}/
//       360p/index.m3u8 + segment_000.ts ...
//       480p/...
//       720p/...
//       1080p/...

use crate::workspace::AccountGuard;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use actix_web::{web, HttpRequest, HttpResponse, Responder};
use tokio::io::AsyncBufReadExt;
use tokio::sync::{Mutex, Notify, RwLock};

use crate::commands::TelegramState;
use crate::mp4_utils;
use grammers_client::types::Media;
use tauri::Manager;

// ── Constants ───────────────────────────────────────────────────────────

/// Maximum total cache size in bytes (5 GB).
pub const MAX_CACHE_BYTES: u64 = 5 * 1024 * 1024 * 1024;

pub fn persisted_cache_limit_bytes(app_data_dir: &Path) -> u64 {
    let settings_path = app_data_dir.join("settings.json");
    let Some(gigabytes) = std::fs::read_to_string(settings_path)
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
        .and_then(|value| {
            value
                .pointer("/settings/transcodeCacheMaxGb")
                .and_then(serde_json::Value::as_u64)
        })
    else {
        return MAX_CACHE_BYTES;
    };
    gigabytes.clamp(1, 50) * 1024 * 1024 * 1024
}

/// Subdirectory name for the streaming cache inside app_data_dir.
/// Originals subdirectory.
const ORIGINALS_DIR: &str = "originals";

/// HLS output subdirectory.
const HLS_DIR: &str = "hls";

/// HLS segment duration in seconds.
const HLS_SEGMENT_TIME: u32 = 4;

// ── Quality presets ─────────────────────────────────────────────────────

#[derive(Clone)]
pub struct QualityPreset {
    pub label: &'static str,
    pub height: u32,
    pub scale_filter: &'static str,
    pub video_bitrate_k: u32,
    pub audio_bitrate_k: u32,
}

pub const QUALITY_PRESETS: &[QualityPreset] = &[
    QualityPreset {
        label: "360p",
        height: 360,
        scale_filter: "scale=-2:360",
        video_bitrate_k: 800,
        audio_bitrate_k: 96,
    },
    QualityPreset {
        label: "480p",
        height: 480,
        scale_filter: "scale=-2:480",
        video_bitrate_k: 1400,
        audio_bitrate_k: 128,
    },
    QualityPreset {
        label: "720p",
        height: 720,
        scale_filter: "scale=-2:720",
        video_bitrate_k: 2800,
        audio_bitrate_k: 128,
    },
    QualityPreset {
        label: "1080p",
        height: 1080,
        scale_filter: "scale=-2:1080",
        video_bitrate_k: 5000,
        audio_bitrate_k: 160,
    },
];

// ── Types ───────────────────────────────────────────────────────────────

#[derive(serde::Serialize, Clone)]
pub struct TranscodeCapabilities {
    pub available: bool,
    pub variants: Vec<QualityVariant>,
    pub mode: String,
}

#[derive(serde::Serialize, Clone)]
pub struct QualityVariant {
    pub label: String,
    pub height: u32,
    pub available: bool,
}

#[derive(serde::Serialize, Clone)]
pub struct TranscodePrepareResult {
    pub job_id: String,
    pub status: String,
    pub progress: f32,
    pub playlist_url: Option<String>,
    pub error: Option<String>,
}

#[derive(serde::Serialize, Clone)]
pub struct TranscodeStatusResult {
    pub job_id: String,
    pub status: String,
    pub progress: f32,
    pub error: Option<String>,
    pub playlist_url: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TranscodeKey {
    pub owner_id: i64,
    pub folder_id: i64, // 0 = root/me
    pub message_id: i32,
    pub quality: String,
}

impl TranscodeKey {
    pub fn file_key(&self) -> String {
        format!("{}_{}_{}", self.owner_id, self.folder_id, self.message_id)
    }

    pub fn job_id(&self) -> String {
        format!(
            "{}_{}_{}_{}",
            self.owner_id, self.folder_id, self.message_id, self.quality
        )
    }
}

#[derive(Clone, Debug)]
pub enum JobPhase {
    NotStarted,
    CachingOriginal { progress: f32 },
    Transcoding { progress: f32 },
    Ready,
    Error(String),
    Cancelled,
}

pub struct TranscodeJob {
    pub key: TranscodeKey,
    pub phase: JobPhase,
    pub cancel_tx: Option<tokio::sync::oneshot::Sender<()>>,
    pub last_access: Instant,
    pub source_height: Option<u32>,
    worker: Arc<WorkerState>,
}

#[derive(Default)]
struct WorkerState {
    active: AtomicBool,
    cancelled: AtomicBool,
    stopped: Notify,
}
pub(crate) struct WorkerLease(Arc<WorkerState>);
impl Drop for WorkerLease {
    fn drop(&mut self) {
        self.0.active.store(false, Ordering::Release);
        self.0.stopped.notify_waiters();
    }
}
impl TranscodeJob {
    pub fn has_live_writer(&self) -> bool {
        self.worker.active.load(Ordering::Acquire)
    }
    /// Acquire exactly once after get_or_create_job returns is_new=true.
    pub(crate) fn writer_lease(&self) -> WorkerLease {
        WorkerLease(self.worker.clone())
    }
}
async fn wait_for_worker(worker: &WorkerState) -> Result<(), String> {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let changed = worker.stopped.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if !worker.active.load(Ordering::Acquire) {
                return;
            }
            changed.await;
        }
    })
    .await
    .map_err(|_| "CACHE_BUSY: The previous conversion is still stopping".into())
}
fn bounded_source_prefix(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::with_capacity(2 * 1024 * 1024);
    std::fs::File::open(path)?
        .take(2 * 1024 * 1024)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

// ── TranscodeManager ────────────────────────────────────────────────────

#[derive(Clone)]
pub struct TranscodeManager {
    pub network: Arc<crate::vpn_optimizer::NetworkConfig>,
    pub bandwidth: Arc<crate::bandwidth::BandwidthManager>,
    pub cache_root: PathBuf,
    pub ffmpeg_path: Arc<Mutex<Option<PathBuf>>>,
    jobs: Arc<Mutex<HashMap<String, Arc<Mutex<TranscodeJob>>>>>,
    max_cache_bytes: Arc<Mutex<u64>>,
    cache_snapshot: Arc<RwLock<CacheSnapshot>>,
    cache_scan_in_flight: Arc<AtomicBool>,
    cache_scan_pending: Arc<AtomicBool>,
}

#[derive(Clone, Default)]
struct CacheSnapshot {
    entries: Vec<CacheEntry>,
    total_bytes: u64,
    last_scanned_at: Option<i64>,
    last_error: Option<String>,
}

impl TranscodeManager {
    pub fn new(cache_root: PathBuf) -> Self {
        Self::new_with_max_cache_bytes(cache_root, MAX_CACHE_BYTES)
    }

    pub fn new_with_max_cache_bytes(cache_root: PathBuf, max_cache_bytes: u64) -> Self {
        // Ensure subdirectories exist
        let _ = std::fs::create_dir_all(cache_root.join(ORIGINALS_DIR));
        let _ = std::fs::create_dir_all(cache_root.join(HLS_DIR));

        Self {
            network: Arc::new(crate::vpn_optimizer::NetworkConfig::new()),
            bandwidth: Arc::new(crate::bandwidth::BandwidthManager::at(
                cache_root.parent().unwrap_or(&cache_root),
            )),
            cache_root,
            ffmpeg_path: Arc::new(Mutex::new(None)),
            jobs: Arc::new(Mutex::new(HashMap::new())),
            max_cache_bytes: Arc::new(Mutex::new(max_cache_bytes)),
            cache_snapshot: Arc::new(RwLock::new(CacheSnapshot::default())),
            cache_scan_in_flight: Arc::new(AtomicBool::new(false)),
            cache_scan_pending: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_transport(
        mut self,
        network: Arc<crate::vpn_optimizer::NetworkConfig>,
        bandwidth: Arc<crate::bandwidth::BandwidthManager>,
    ) -> Self {
        self.network = network;
        self.bandwidth = bandwidth;
        self
    }

    /// Reconcile the detailed cache inventory once on the blocking pool. The
    /// settings command serves the last snapshot immediately while this work
    /// is running, so large collections of HLS segments cannot block the UI.
    pub fn start_cache_reconciliation(self: &Arc<Self>, clean_partial_outputs: bool) {
        if self
            .cache_scan_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            self.cache_scan_pending.store(true, Ordering::Release);
            return;
        }
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            let scan_started_at = std::time::Instant::now();
            let cache_root = manager.cache_root.clone();
            let result = tokio::task::spawn_blocking(move || {
                if clean_partial_outputs {
                    Self::clean_partial_outputs(&cache_root);
                }
                scan_transcode_cache(&cache_root)
            })
            .await
            .map_err(|error| format!("Transcode cache inspection task failed: {error}"))
            .and_then(|result| result);

            let mut snapshot = manager.cache_snapshot.write().await;
            match result {
                Ok(entries) => {
                    snapshot.total_bytes = entries
                        .iter()
                        .fold(0u64, |total, entry| total.saturating_add(entry.size_bytes));
                    snapshot.entries = entries;
                    snapshot.last_scanned_at = Some(crate::app_time::unix_seconds());
                    snapshot.last_error = None;
                    log::info!(
                        "Transcode cache reconciliation found {} variants ({} bytes) in {:?}",
                        snapshot.entries.len(),
                        snapshot.total_bytes,
                        scan_started_at.elapsed()
                    );
                }
                Err(error) => {
                    log::warn!("Transcode cache reconciliation failed: {error}");
                    snapshot.last_error = Some(error);
                    snapshot.last_scanned_at = Some(crate::app_time::unix_seconds());
                }
            }
            drop(snapshot);
            manager.cache_scan_in_flight.store(false, Ordering::Release);
            if manager.cache_scan_pending.swap(false, Ordering::AcqRel) {
                manager.start_cache_reconciliation(false);
            }
        });
    }

    async fn detailed_cache_snapshot(&self) -> DetailedCacheInfo {
        let snapshot = self.cache_snapshot.read().await.clone();
        DetailedCacheInfo {
            entries: snapshot.entries,
            total_bytes: snapshot.total_bytes,
            max_bytes: self.get_max_cache_bytes().await,
            scan_in_progress: self.cache_scan_in_flight.load(Ordering::Acquire),
            last_scanned_at: snapshot.last_scanned_at,
            last_error: snapshot.last_error,
        }
    }

    /// Clean up incomplete HLS directories from previous runs.
    fn clean_partial_outputs(cache_root: &Path) {
        // Reconciliation is deliberately off the startup critical path. Only
        // remove old partials so a transcode begun immediately after launch
        // cannot race this background cleanup.
        const STALE_PARTIAL_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
        let is_stale = |path: &Path| {
            std::fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
                .is_ok_and(|age| age >= STALE_PARTIAL_AGE)
        };
        let hls_root = cache_root.join(HLS_DIR);
        if let Ok(entries) = std::fs::read_dir(&hls_root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    // Check if index.m3u8 exists in any quality subfolder
                    let mut has_playlist = false;
                    if let Ok(q_dirs) = std::fs::read_dir(&path) {
                        for q_entry in q_dirs.flatten() {
                            if q_entry.path().join("index.m3u8").exists() {
                                has_playlist = true;
                                break;
                            }
                        }
                    }
                    if !has_playlist && is_stale(&path) {
                        log::info!("Transcode: Cleaning up partial output: {:?}", path);
                        let _ = std::fs::remove_dir_all(&path);
                    }
                }
            }
        }
        // Also clean incomplete original downloads (zero-size or .part files)
        let orig_root = cache_root.join(ORIGINALS_DIR);
        if let Ok(entries) = std::fs::read_dir(&orig_root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let should_remove = if path.extension().and_then(|e| e.to_str()) == Some("part")
                    {
                        true // Orphaned partial download
                    } else if let Ok(meta) = std::fs::metadata(&path) {
                        meta.len() == 0 // Zero-size completed file
                    } else {
                        false
                    };
                    if should_remove && is_stale(&path) {
                        log::info!("Transcode: Removing incomplete file: {:?}", path);
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
        }
    }

    /// Register before accessing any source/output. Cache mutation holds the same map lock.
    pub async fn get_or_create_job(&self, key: &TranscodeKey) -> (Arc<Mutex<TranscodeJob>>, bool) {
        let mut jobs = self.jobs.lock().await;
        let job_id = key.job_id();
        if let Some(job) = jobs.get(&job_id) {
            let mut current = job.lock().await;
            if current.has_live_writer() {
                current.last_access = Instant::now();
                return (job.clone(), false);
            }
        }
        let worker = Arc::new(WorkerState::default());
        worker.active.store(true, Ordering::Release);
        let job = Arc::new(Mutex::new(TranscodeJob {
            key: key.clone(),
            phase: JobPhase::NotStarted,
            cancel_tx: None,
            last_access: Instant::now(),
            source_height: None,
            worker,
        }));
        jobs.insert(job_id, job.clone());
        (job, true)
    }

    pub(crate) fn account(&self, expected: Option<&str>) -> Result<AccountGuard, String> {
        AccountGuard::open(
            self.cache_root
                .parent()
                .ok_or("Invalid streaming cache root")?,
            expected,
        )
    }
    pub(crate) fn account_for_key(&self, file_key: &str) -> Result<AccountGuard, String> {
        let parts: Vec<_> = file_key.split('_').collect();
        if parts.len() != 3
            || !parts[0].parse::<i64>().is_ok_and(|id| id > 0)
            || parts[1].parse::<i64>().is_err()
            || !parts[2].parse::<i32>().is_ok_and(|id| id > 0)
        {
            return Err("ACCOUNT_CHANGED: Invalid or unassigned cache identity".into());
        }
        self.account(Some(parts[0]))
    }

    /// Get a clone of the jobs map for status queries.
    pub async fn get_job_snapshot(&self) -> HashMap<String, Arc<Mutex<TranscodeJob>>> {
        self.jobs.lock().await.clone()
    }

    /// Get current max cache bytes.
    pub async fn get_max_cache_bytes(&self) -> u64 {
        *self.max_cache_bytes.lock().await
    }

    /// Set max cache bytes.
    pub async fn set_max_cache_bytes(&self, bytes: u64) {
        *self.max_cache_bytes.lock().await = bytes;
    }

    /// Get total cache size by walking the cache directory.
    pub fn total_cache_size(&self) -> u64 {
        let mut total: u64 = 0;
        let walker = walkdir::WalkDir::new(&self.cache_root).min_depth(1);
        for entry in walker.into_iter().flatten() {
            if entry.file_type().is_file() {
                if let Ok(meta) = entry.metadata() {
                    total += meta.len();
                }
            }
        }
        total
    }

    /// Evict oldest files until cache is under the limit.
    /// Never evict files that belong to active jobs.
    pub async fn evict_lru(&self) {
        let jobs = self.jobs.lock().await;
        let mut protected = Vec::new();
        for job in jobs.values() {
            let job = job.lock().await;
            if job.has_live_writer() {
                let original = self.original_path(&job.key.file_key());
                protected.push(original.with_extension("mp4.part"));
                protected.push(original);
                protected.push(self.hls_output_dir(&job.key.file_key(), &job.key.quality));
                protected.push(self.cache_root.join("fmp4").join(job.key.file_key()));
            }
        }
        let max = *self.max_cache_bytes.lock().await;
        let current = self.total_cache_size();
        if current <= max {
            return;
        }

        // Collect all files with their modification times
        let mut files: Vec<(PathBuf, u64, SystemTime)> = Vec::new();
        let walker = walkdir::WalkDir::new(&self.cache_root).min_depth(1);
        for entry in walker.into_iter().flatten() {
            if entry.file_type().is_file() {
                if let Ok(meta) = entry.metadata() {
                    files.push((
                        entry.path().to_path_buf(),
                        meta.len(),
                        meta.modified().unwrap_or(UNIX_EPOCH),
                    ));
                }
            }
        }

        // Sort by modification time (oldest first)
        files.sort_by_key(|(_, _, mtime)| *mtime);

        let mut freed: u64 = 0;
        let target = current.saturating_sub(max);

        for (path, size, _) in &files {
            if freed >= target {
                break;
            }

            if protected
                .iter()
                .any(|root| path == root || path.starts_with(root))
            {
                continue;
            }
            if let Err(e) = std::fs::remove_file(path) {
                log::warn!("Transcode: Failed to evict {:?}: {}", path, e);
            } else {
                freed += size;
                log::debug!("Transcode: Evicted {:?} ({} bytes)", path, size);
            }
        }

        // Clean up empty directories
        Self::clean_empty_dirs(&self.cache_root, 2);

        log::info!(
            "Transcode: LRU eviction complete. Freed {} bytes, target was {} bytes",
            freed,
            target
        );
    }

    fn clean_empty_dirs(path: &Path, depth: usize) {
        if depth == 0 {
            return;
        }
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    Self::clean_empty_dirs(&p, depth - 1);
                    if std::fs::read_dir(&p)
                        .map(|mut d| d.next().is_none())
                        .unwrap_or(false)
                    {
                        let _ = std::fs::remove_dir(&p);
                    }
                }
            }
        }
    }

    /// Validate that a resolved path stays within the HLS cache directory.
    pub fn validate_hls_path(
        &self,
        file_key: &str,
        quality: &str,
        segment: Option<&str>,
    ) -> Option<PathBuf> {
        self.account_for_key(file_key).ok()?;
        // Sanitize inputs — only allow alphanumeric, underscores, hyphens, dots
        if file_key
            .chars()
            .any(|c| !c.is_alphanumeric() && c != '_' && c != '-')
        {
            return None;
        }
        if quality.chars().any(|c| !c.is_alphanumeric() && c != 'p') {
            return None;
        }

        let hls_root = self.cache_root.join(HLS_DIR);
        let mut resolved = hls_root.join(file_key).join(quality);

        if let Some(seg) = segment {
            // Only allow .ts and .m3u8 files
            if !seg.ends_with(".ts") && !seg.ends_with(".m3u8") {
                return None;
            }
            if seg.contains("..") || seg.contains('/') || seg.contains('\\') {
                return None;
            }
            resolved = resolved.join(seg);
        } else {
            resolved = resolved.join("index.m3u8");
        }

        // Canonicalize and verify it's within the HLS root
        match resolved.canonicalize() {
            Ok(canon) => {
                let hls_canon = hls_root.canonicalize().unwrap_or_else(|_| hls_root.clone());
                if canon.starts_with(&hls_canon) {
                    Some(canon)
                } else {
                    log::error!(
                        "Transcode: Path traversal attempt: {:?} not under {:?}",
                        canon,
                        hls_canon
                    );
                    None
                }
            }
            Err(_) => None, // File doesn't exist yet or path is invalid
        }
    }

    /// Return the path for a cached original file.
    pub fn original_path(&self, file_key: &str) -> PathBuf {
        self.cache_root
            .join(ORIGINALS_DIR)
            .join(format!("{}.mp4", file_key))
    }

    /// Return the HLS output directory for a job.
    pub fn hls_output_dir(&self, file_key: &str, quality: &str) -> PathBuf {
        self.cache_root.join(HLS_DIR).join(file_key).join(quality)
    }
}

// ── FFmpeg Detection ────────────────────────────────────────────────────

/// Detect FFmpeg availability. Tries sidecar first, then PATH.
pub async fn detect_ffmpeg(app_handle: &tauri::AppHandle) -> Option<PathBuf> {
    // 1. Try sidecar binary in the app's resource directory
    if let Ok(resource_dir) = app_handle.path().resource_dir() {
        #[cfg(target_os = "windows")]
        let sidecar_name = "ffmpeg.exe";
        #[cfg(not(target_os = "windows"))]
        let sidecar_name = "ffmpeg";

        let sidecar_path: PathBuf = resource_dir.join(sidecar_name);
        if sidecar_path.exists() {
            match test_ffmpeg(&sidecar_path).await {
                Ok(true) => {
                    log::info!("Transcode: Found FFmpeg sidecar at {:?}", sidecar_path);
                    return Some(sidecar_path);
                }
                Ok(false) => {
                    log::warn!(
                        "Transcode: FFmpeg sidecar at {:?} failed version check",
                        sidecar_path
                    );
                }
                Err(e) => {
                    log::warn!("Transcode: FFmpeg sidecar check error: {}", e);
                }
            }
        }
    }

    // 2. Fallback to PATH
    detect_path_ffmpeg().await
}

pub(crate) async fn detect_path_ffmpeg() -> Option<PathBuf> {
    match test_ffmpeg(Path::new("ffmpeg")).await {
        Ok(true) => {
            log::info!("Transcode: Found FFmpeg on PATH");
            Some(PathBuf::from("ffmpeg"))
        }
        Ok(false) => {
            log::warn!("Transcode: FFmpeg not found on PATH or version check failed");
            None
        }
        Err(e) => {
            log::warn!("Transcode: FFmpeg not available on PATH: {}", e);
            None
        }
    }
}

async fn test_ffmpeg(path: &Path) -> Result<bool, String> {
    let mut command = tokio::process::Command::new(path);
    command
        .arg("-version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::process_util::hide_console(&mut command);
    let output = command
        .output()
        .await
        .map_err(|e| format!("Failed to run ffmpeg: {}", e))?;

    Ok(output.status.success())
}

// ── Source Cache (Phase 2) ──────────────────────────────────────────────

struct SourcePartial(PathBuf);
impl Drop for SourcePartial {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Download the original MP4 file from Telegram to local cache.
/// Returns the total file size on success.
pub async fn cache_original(
    client: &grammers_client::Client,
    media: &Media,
    dest_path: &Path,
    cancel_rx: &mut tokio::sync::oneshot::Receiver<()>,
    progress_callback: impl Fn(f32),
    account: &AccountGuard,
    transport: &TranscodeManager,
) -> Result<u64, String> {
    account.validate()?;
    let total_size = match media {
        Media::Document(d) => d.size() as u64,
        _ => return Err("Not a document".to_string()),
    };

    let mut reservation =
        crate::bandwidth::BandwidthReservation::download(transport.bandwidth.clone(), total_size)?;
    // Ensure parent directory exists
    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create cache dir: {}", e))?;
    }

    let tmp_path = dest_path.with_extension("mp4.part");
    let _temporary = SourcePartial(tmp_path.clone());
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .map_err(|e| format!("Failed to create cache file: {}", e))?;

    use tokio::io::AsyncWriteExt;

    let mut download_iter = client.iter_download(media);
    download_iter = download_iter.chunk_size(65536);
    let mut downloaded: u64 = 0;

    loop {
        tokio::select! {
            _ = async { loop { if account.validate().is_err() { break; } tokio::time::sleep(std::time::Duration::from_millis(200)).await; } } => return Err("ACCOUNT_CHANGED".into()),
            _ = &mut *cancel_rx => {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err("Cancelled".to_string());
            }
            result = download_iter.next() => {
                match result {
                    Ok(Some(chunk)) => {
                        tokio::select! {result=transport.network.pacer.wait(&transport.network,crate::traffic::Direction::Download,chunk.len(),||account.validate())=>result?,_=&mut *cancel_rx=>return Err("Cancelled".into())}
                        account.validate()?;
                        if downloaded.saturating_add(chunk.len() as u64) > total_size { return Err("Incomplete download: source exceeded expected size".into()); }
                        file.write_all(&chunk).await.map_err(|e| format!("Write error: {}", e))?;
                        downloaded += chunk.len() as u64;
                        progress_callback(downloaded as f32 / total_size as f32);
                    }
                    Ok(None) => break,
                    Err(e) => {
                        let _ = tokio::fs::remove_file(&tmp_path).await;
                        return Err(format!("Download error: {}", e));
                    }
                }
            }
        }
    }

    file.flush()
        .await
        .map_err(|e| format!("Flush error: {}", e))?;
    drop(file);

    // Validate file size matches expected size
    let actual_size = tokio::fs::metadata(&tmp_path)
        .await
        .map_err(|e| format!("Metadata error: {}", e))?
        .len();

    if actual_size == 0 {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err("Downloaded zero bytes".to_string());
    }

    if actual_size != total_size {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(format!(
            "Incomplete download: expected {} bytes, received {} bytes",
            total_size, actual_size
        ));
    }

    account.validate()?;
    // Rename .part → .mp4
    tokio::fs::rename(&tmp_path, dest_path)
        .await
        .map_err(|e| format!("Rename error: {}", e))?;

    reservation.commit();
    log::info!(
        "Transcode: Cached original to {:?} ({} bytes)",
        dest_path,
        actual_size
    );
    Ok(actual_size)
}

// ── HLS Transcode (Phase 3) ─────────────────────────────────────────────

fn validate_hls_output(output_dir: &Path) -> Result<(), String> {
    let playlist_path = output_dir.join("index.m3u8");
    let playlist = std::fs::read_to_string(&playlist_path)
        .map_err(|error| format!("Failed to read HLS playlist: {error}"))?;

    if !playlist
        .lines()
        .any(|line| line.trim().starts_with("#EXTINF:"))
    {
        return Err("HLS playlist has no segments".to_string());
    }

    if !playlist.lines().any(|line| line.trim() == "#EXT-X-ENDLIST") {
        return Err("HLS playlist is incomplete".into());
    }

    let mut segment_count = 0usize;
    for line in playlist.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let segment_name = line.split('?').next().unwrap_or(line);
        let segment_path = Path::new(segment_name);
        if segment_path.components().count() != 1
            || segment_path.extension().and_then(|value| value.to_str()) != Some("ts")
        {
            return Err(format!(
                "HLS playlist contains an invalid segment path: {line}"
            ));
        }

        let metadata = std::fs::metadata(output_dir.join(segment_path))
            .map_err(|error| format!("HLS segment {segment_name} is unavailable: {error}"))?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(format!("HLS segment {segment_name} is empty"));
        }
        segment_count += 1;
    }

    if segment_count == 0 {
        return Err("HLS playlist references no segment files".to_string());
    }

    Ok(())
}

/// Run FFmpeg to generate a single HLS variant.
#[allow(clippy::too_many_arguments)] // The account epoch travels with the source, output, conversion policy and cancellation channel.
pub async fn run_transcode(
    ffmpeg_path: &Path,
    input_path: &Path,
    output_dir: &Path,
    quality: &QualityPreset,
    duration_secs: Option<f64>,
    cancel_rx: &mut tokio::sync::oneshot::Receiver<()>,
    progress_callback: impl Fn(f32),
    account: &AccountGuard,
) -> Result<(), String> {
    account.validate()?;
    // Create output directory
    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("Failed to create HLS output dir: {}", e))?;

    let playlist_path = output_dir.join("index.m3u8");
    let segment_pattern = output_dir.join("segment_%03d.ts");

    let mut cmd = tokio::process::Command::new(ffmpeg_path);
    crate::process_util::hide_console(&mut cmd);
    cmd.arg("-y") // Overwrite
        .arg("-i")
        .arg(input_path)
        // Explicit stream mapping: first video, optional audio, no subtitles/data
        .arg("-map")
        .arg("0:v:0")
        .arg("-map")
        .arg("0:a:0?")
        .arg("-sn") // No subtitles
        .arg("-dn") // No data streams
        .arg("-vf")
        .arg(quality.scale_filter)
        .arg("-c:v")
        .arg("libx264")
        .arg("-preset")
        .arg("veryfast")
        .arg("-crf")
        .arg("23")
        .arg("-c:a")
        .arg("aac")
        .arg("-b:a")
        .arg(format!("{}k", quality.audio_bitrate_k))
        .arg("-maxrate")
        .arg(format!("{}k", quality.video_bitrate_k))
        .arg("-bufsize")
        .arg(format!("{}k", quality.video_bitrate_k * 2))
        .arg("-f")
        .arg("hls")
        .arg("-hls_time")
        .arg(HLS_SEGMENT_TIME.to_string())
        .arg("-hls_playlist_type")
        .arg("vod")
        .arg("-hls_segment_filename")
        .arg(&segment_pattern)
        .arg(&playlist_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn FFmpeg: {}", e))?;
    let stderr = child.stderr.take().ok_or("No stderr pipe")?;

    // Parse FFmpeg progress from stderr, filter error lines inline for memory efficiency
    let stderr_reader = tokio::io::BufReader::new(stderr);
    let mut lines = stderr_reader.lines();
    let mut last_progress = 0f32;
    let mut stderr_error_lines: Vec<String> = Vec::new();

    let parse_result: Result<(), String> = loop {
        tokio::select! {
            _ = async { loop { if account.validate().is_err() { break; } tokio::time::sleep(std::time::Duration::from_millis(200)).await; } } => {
                let _ = child.kill().await; let _ = child.wait().await;
                let _ = std::fs::remove_dir_all(output_dir);
                break Err("ACCOUNT_CHANGED".into());
            }
            _ = &mut *cancel_rx => {
                // Kill the FFmpeg process
                let _ = child.kill().await;
                let _ = child.wait().await;
                // Clean up partial output
                let _ = std::fs::remove_dir_all(output_dir);
                break Err("Cancelled".to_string());
            }
            line_result = lines.next_line() => {
                match line_result {
                    Ok(Some(line)) => {
                        // Only store lines containing 'error' (case-insensitive) — avoids
                        // collecting thousands of progress lines for successful transcodes
                        if line.to_lowercase().contains("error") {
                            if stderr_error_lines.len() == 32 { stderr_error_lines.remove(0); }
                            stderr_error_lines.push(line.chars().take(2048).collect());
                        }

                        // Parse time=HH:MM:SS.MS from FFmpeg stderr
                        if let Some(time_str) = line.split("time=").nth(1) {
                            let time_str = time_str.split_whitespace().next().unwrap_or("0");
                            let secs = parse_time_to_secs(time_str);
                            if let Some(dur) = duration_secs {
                                if dur > 0.0 {
                                    let pct = (secs / dur) as f32;
                                    if (pct - last_progress).abs() > 0.01 {
                                        last_progress = pct.clamp(0.0, 0.99);
                                        progress_callback(last_progress);
                                    }
                                }
                            }
                        }
                    }
                    Ok(None) => break Ok(()),
                    Err(e) => {
                        log::warn!("Transcode: stderr read error: {}", e);
                        break Ok(());
                    }
                }
            }
        }
    };

    // Wait for the process to finish (if not cancelled)
    let status = child
        .wait()
        .await
        .map_err(|e| format!("FFmpeg wait error: {}", e))?;

    // Check for cancellation or earlier error
    parse_result?;

    if !status.success() {
        let _ = std::fs::remove_dir_all(output_dir);
        let tail_msg = if stderr_error_lines.is_empty() {
            String::new()
        } else {
            format!("\nFFmpeg error lines:\n{}", stderr_error_lines.join("\n"))
        };
        return Err(format!(
            "FFmpeg exited with code {:?}{}",
            status.code(),
            tail_msg
        ));
    }

    account.validate()?;
    if let Err(error) = validate_hls_output(output_dir) {
        let _ = std::fs::remove_dir_all(output_dir);
        return Err(error);
    }

    log::info!("Transcode: Generated HLS variant at {:?}", output_dir);
    Ok(())
}

fn parse_time_to_secs(time: &str) -> f64 {
    let parts: Vec<&str> = time.split(':').collect();
    if parts.len() == 3 {
        let h: f64 = parts[0].parse().unwrap_or(0.0);
        let m: f64 = parts[1].parse().unwrap_or(0.0);
        let s: f64 = parts[2].parse().unwrap_or(0.0);
        h * 3600.0 + m * 60.0 + s
    } else {
        0.0
    }
}

// ── Execute Full Transcode Pipeline ─────────────────────────────────────

/// Run the full pipeline: cache original → transcode HLS.
/// Runs entirely on the async runtime (FFmpeg runs in its own OS process).
#[allow(clippy::too_many_arguments)] // Carries the prepared Telegram media and its account epoch into the owned worker.
pub async fn execute_transcode_pipeline(
    manager: &TranscodeManager,
    key: &TranscodeKey,
    quality_preset: &QualityPreset,
    client: grammers_client::Client,
    media: Media,
    duration_secs: Option<f64>,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
    account: AccountGuard,
) {
    let job_arc = {
        let jobs = manager.jobs.lock().await;
        jobs.get(&key.job_id()).cloned()
    };

    let job_arc = match job_arc {
        Some(j) => j,
        None => return,
    };

    let ffmpeg_path = { manager.ffmpeg_path.lock().await.clone() };

    let ffmpeg_path = match ffmpeg_path {
        Some(p) => p,
        None => {
            let mut job = job_arc.lock().await;
            job.phase = JobPhase::Error("FFmpeg not available".to_string());
            return;
        }
    };

    if account.validate().is_err() {
        job_arc.lock().await.phase = JobPhase::Error("ACCOUNT_CHANGED".into());
        return;
    }
    let source_lock = crate::workspace::assets::file_lock(format!(
        "transcode-source:{}:{}",
        manager.cache_root.display(),
        key.file_key()
    ))
    .await;
    let source_guard = tokio::select! {
        guard = source_lock.lock() => guard,
        _ = &mut cancel_rx => { job_arc.lock().await.phase = JobPhase::Cancelled; return; }
    };
    let file_key = key.file_key();
    let original_path = manager.original_path(&file_key);
    let output_dir = manager.hls_output_dir(&file_key, &key.quality);

    // ── Step 1: Cache original if needed ────────────────────────────
    if !original_path.exists() {
        {
            let mut job = job_arc.lock().await;
            job.phase = JobPhase::CachingOriginal { progress: 0.0 };
        }

        let job_arc_clone = job_arc.clone();
        match cache_original(
            &client,
            &media,
            &original_path,
            &mut cancel_rx,
            |progress| {
                let job_arc = job_arc_clone.clone();
                tauri::async_runtime::spawn(async move {
                    let mut job = job_arc.lock().await;
                    if job.has_live_writer()
                        && !job.worker.cancelled.load(Ordering::Acquire)
                        && matches!(job.phase, JobPhase::CachingOriginal { .. })
                    {
                        job.phase = JobPhase::CachingOriginal { progress };
                    }
                });
            },
            &account,
            manager,
        )
        .await
        {
            Ok(size) => {
                log::info!(
                    "Transcode: Cached original ({} bytes), starting transcode...",
                    size
                );
            }
            Err(e) => {
                let mut job = job_arc.lock().await;
                job.phase = if job.worker.cancelled.load(Ordering::Acquire) {
                    JobPhase::Cancelled
                } else {
                    JobPhase::Error(format!("Cache failed: {}", e))
                };
                return;
            }
        }
    }

    drop(source_guard);
    if account.validate().is_err() {
        job_arc.lock().await.phase = JobPhase::Error("ACCOUNT_CHANGED".into());
        return;
    }
    // ── Step 2: Detect source resolution ────────────────────────────
    let source_height = {
        let data = bounded_source_prefix(&original_path).unwrap_or_default();
        if data.len() > 1024 {
            mp4_utils::scan_video_tkhd_dimensions(
                &data[..std::cmp::min(2 * 1024 * 1024, data.len())],
            )
            .1
        } else {
            None
        }
    };

    {
        let mut job = job_arc.lock().await;
        job.source_height = source_height;
    }

    // Check if source is lower than requested quality — skip if so
    if let Some(src_h) = source_height {
        if src_h < quality_preset.height {
            let mut job = job_arc.lock().await;
            job.phase = JobPhase::Error(format!(
                "Source is {}p, cannot transcode to {}p",
                src_h, quality_preset.height
            ));
            return;
        }
    }

    // ── Step 3: Transcode ───────────────────────────────────────────
    {
        let mut job = job_arc.lock().await;
        job.phase = JobPhase::Transcoding { progress: 0.0 };
    }

    let job_arc_clone = job_arc.clone();
    let result = run_transcode(
        &ffmpeg_path,
        &original_path,
        &output_dir,
        quality_preset,
        duration_secs,
        &mut cancel_rx,
        |progress| {
            let job_arc = job_arc_clone.clone();
            tauri::async_runtime::spawn(async move {
                let mut job = job_arc.lock().await;
                if job.has_live_writer()
                    && !job.worker.cancelled.load(Ordering::Acquire)
                    && matches!(job.phase, JobPhase::Transcoding { .. })
                {
                    job.phase = JobPhase::Transcoding { progress };
                }
            });
        },
        &account,
    )
    .await;

    match result {
        Ok(()) => {
            let mut job = job_arc.lock().await;
            job.phase = JobPhase::Ready;
            log::info!("Transcode: Job {} completed successfully", key.job_id());
        }
        Err(e) => {
            let mut job = job_arc.lock().await;
            job.phase = if job.worker.cancelled.load(Ordering::Acquire) {
                JobPhase::Cancelled
            } else {
                JobPhase::Error(e)
            };
        }
    }
}

// ── Tauri Commands ──────────────────────────────────────────────────────

#[tauri::command]
pub async fn cmd_get_transcode_capabilities(
    manager: tauri::State<'_, Arc<TranscodeManager>>,
    app_handle: tauri::AppHandle,
) -> Result<TranscodeCapabilities, String> {
    // Lazy detection: if FFmpeg hasn't been detected yet, try now.
    // This fixes the race where the background detection hasn't completed
    // by the time the UI first asks for capabilities.
    let ffmpeg_available = {
        let path_guard = manager.ffmpeg_path.lock().await;
        if path_guard.is_some() {
            true
        } else {
            drop(path_guard);
            // Attempt lazy detection on first call
            if let Some(ffmpeg) = detect_ffmpeg(&app_handle).await {
                *manager.ffmpeg_path.lock().await = Some(ffmpeg);
                true
            } else {
                false
            }
        }
    };

    let variants: Vec<QualityVariant> = QUALITY_PRESETS
        .iter()
        .map(|p| QualityVariant {
            label: p.label.to_string(),
            height: p.height,
            available: ffmpeg_available,
        })
        .collect();

    Ok(TranscodeCapabilities {
        available: ffmpeg_available,
        variants,
        mode: if ffmpeg_available {
            "hls".to_string()
        } else {
            "original".to_string()
        },
    })
}

#[tauri::command]
pub async fn cmd_prepare_transcoded_stream(
    message_id: i32,
    folder_id: Option<i64>,
    quality: String,
    state: tauri::State<'_, TelegramState>,
    manager: tauri::State<'_, Arc<TranscodeManager>>,
) -> Result<TranscodePrepareResult, String> {
    let account = manager.account(None)?;
    let folder_id = folder_id.unwrap_or(0);
    let key = TranscodeKey {
        owner_id: account.owner,
        folder_id,
        message_id,
        quality: quality.clone(),
    };
    let preset = QUALITY_PRESETS
        .iter()
        .find(|p| p.label == quality)
        .ok_or_else(|| format!("Unknown quality: {quality}"))?;
    let job_arc = loop {
        let (job, is_new) = manager.get_or_create_job(&key).await;
        if is_new {
            break job;
        }
        let existing = job.lock().await;
        if existing.worker.cancelled.load(Ordering::Acquire) {
            let worker = existing.worker.clone();
            drop(existing);
            wait_for_worker(&worker).await?;
            account.validate()?;
            continue;
        }
        let (status, progress) = match &existing.phase {
            JobPhase::CachingOriginal { progress } => ("caching", *progress),
            JobPhase::Transcoding { progress } => ("transcoding", *progress),
            _ => ("pending", 0.0),
        };
        account.validate()?;
        return Ok(TranscodePrepareResult {
            job_id: key.job_id(),
            status: status.into(),
            progress,
            playlist_url: None,
            error: None,
        });
    };
    let worker = job_arc.lock().await.worker.clone();
    let lease = WorkerLease(worker.clone());
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel();
    job_arc.lock().await.cancel_tx = Some(cancel_tx);
    let output_dir = manager.hls_output_dir(&key.file_key(), &quality);
    if output_dir.join("index.m3u8").exists() {
        if validate_hls_output(&output_dir).is_ok() {
            account.validate()?;
            job_arc.lock().await.phase = JobPhase::Ready;
            return Ok(TranscodePrepareResult {
                job_id: key.job_id(),
                status: "ready".into(),
                progress: 1.0,
                playlist_url: Some(format!("/hls/{}/{}/index.m3u8", key.file_key(), quality)),
                error: None,
            });
        }
        std::fs::remove_dir_all(&output_dir).map_err(|e| e.to_string())?;
    }
    let preparation = async {
        let client = state
            .client
            .lock()
            .await
            .clone()
            .ok_or("Not connected to Telegram")?;
        let actual = client.get_me().await.map_err(|e| e.to_string())?;
        if actual.bare_id() != account.owner {
            return Err("ACCOUNT_CHANGED".into());
        }
        account.validate()?;
        let peer = crate::commands::utils::resolve_peer(
            &client,
            if folder_id == 0 {
                None
            } else {
                Some(folder_id)
            },
            &state.peer_cache,
        )
        .await?;
        let message = client
            .get_messages_by_id(&peer, &[message_id])
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .flatten()
            .next()
            .ok_or("Message not found")?;
        let media = message.media().ok_or("No media")?;
        if message.text() == "TDENC2"
            || matches!(&media, Media::Document(document) if document.name().to_ascii_lowercase().ends_with(".tdenc"))
        {
            return Err("ENCRYPTED_PREVIEW_UNAVAILABLE".into());
        }
        let duration = get_duration_from_media(&client, message_id, folder_id, &state)
            .await
            .ok();
        account.validate()?;
        Ok::<_, String>((client, media, duration))
    };
    let (client, media, duration_secs) = tokio::select! {
        prepared = preparation => prepared?,
        _ = &mut cancel_rx => { job_arc.lock().await.phase = JobPhase::Cancelled; return Err("Cancelled".into()); },
        _ = async {loop {if account.validate().is_err() {break;} tokio::time::sleep(std::time::Duration::from_millis(200)).await;}} => return Err("ACCOUNT_CHANGED".into()),
    };
    let manager = manager.inner().clone();
    let preset = preset.clone();
    let spawned_key = key.clone();
    tauri::async_runtime::spawn(async move {
        execute_transcode_pipeline(
            &manager,
            &spawned_key,
            &preset,
            client,
            media,
            duration_secs,
            cancel_rx,
            account,
        )
        .await;
        // The lease still protects this newly completed output during eviction.
        manager.evict_lru().await;
        drop(lease);
        manager.start_cache_reconciliation(false);
    });
    Ok(TranscodePrepareResult {
        job_id: key.job_id(),
        status: "started".into(),
        progress: 0.0,
        playlist_url: None,
        error: None,
    })
}

async fn get_duration_from_media(
    client: &grammers_client::Client,
    message_id: i32,
    folder_id: i64,
    state: &TelegramState,
) -> Result<f64, String> {
    let peer = crate::commands::utils::resolve_peer(
        client,
        if folder_id == 0 {
            None
        } else {
            Some(folder_id)
        },
        &state.peer_cache,
    )
    .await?;

    let messages = client
        .get_messages_by_id(&peer, &[message_id])
        .await
        .map_err(|e| e.to_string())?;

    let msg = messages
        .into_iter()
        .flatten()
        .next()
        .ok_or_else(|| "Message not found".to_string())?;

    let media = msg.media().ok_or_else(|| "No media".to_string())?;

    let size = match &media {
        Media::Document(d) => d.size() as u64,
        _ => return Err("Not a document".to_string()),
    };

    // Download first 2MB and parse moov
    let max_bytes = std::cmp::min(2 * 1024 * 1024, size) as usize;
    let mut buffer: Vec<u8> = Vec::with_capacity(max_bytes);
    let mut download_iter = client.iter_download(&media);
    download_iter = download_iter.chunk_size(65536);

    while buffer.len() < max_bytes {
        match download_iter.next().await {
            Ok(Some(chunk)) => {
                let remaining = max_bytes.saturating_sub(buffer.len());
                let take = std::cmp::min(chunk.len(), remaining);
                buffer.extend_from_slice(&chunk[..take]);
            }
            Ok(None) => break,
            Err(e) => return Err(format!("Download error: {}", e)),
        }
    }

    // Parse with mp4parse
    let mut cursor = std::io::Cursor::new(&buffer);
    let context = mp4parse::read_mp4(&mut cursor).map_err(|e| format!("MP4 parse error: {}", e))?;

    let video_track = context
        .tracks
        .iter()
        .find(|t| t.track_type == mp4parse::TrackType::Video);

    video_track
        .and_then(|t| {
            let d = t.duration.as_ref()?;
            let ts = t.timescale.as_ref()?;
            Some((d.0 as f64) / (ts.0 as f64))
        })
        .ok_or_else(|| "No video track duration".to_string())
}

#[tauri::command]
pub async fn cmd_get_transcode_status(
    job_id: String,
    manager: tauri::State<'_, Arc<TranscodeManager>>,
) -> Result<TranscodeStatusResult, String> {
    let account = manager.account(None)?;
    let jobs = manager.jobs.lock().await;
    let job_arc = jobs
        .get(&job_id)
        .ok_or_else(|| format!("Job {} not found", job_id))?;

    let job = job_arc.lock().await;
    if job.key.owner_id != account.owner {
        return Err("ACCOUNT_CHANGED".into());
    }
    account.validate()?;
    let (status_str, progress, error, playlist_url) = match &job.phase {
        JobPhase::NotStarted => ("pending".to_string(), 0.0, None, None),
        JobPhase::CachingOriginal { progress } => ("caching".to_string(), *progress, None, None),
        JobPhase::Transcoding { progress } => ("transcoding".to_string(), *progress, None, None),
        JobPhase::Ready => (
            "ready".to_string(),
            1.0,
            None,
            Some(format!(
                "/hls/{}/{}/index.m3u8",
                job.key.file_key(),
                job.key.quality
            )),
        ),
        JobPhase::Error(e) => ("error".to_string(), 0.0, Some(e.clone()), None),
        JobPhase::Cancelled => ("cancelled".to_string(), 0.0, None, None),
    };

    Ok(TranscodeStatusResult {
        job_id,
        status: status_str,
        progress,
        error,
        playlist_url,
    })
}

#[tauri::command]
pub async fn cmd_cancel_transcode(
    job_id: String,
    manager: tauri::State<'_, Arc<TranscodeManager>>,
) -> Result<(), String> {
    let account = manager.account(None)?;
    let worker = {
        let jobs = manager.jobs.lock().await;
        let job = jobs.get(&job_id).ok_or("Transcode job not found")?;
        let mut job = job.lock().await;
        if job.key.owner_id != account.owner {
            return Err("ACCOUNT_CHANGED".into());
        }
        job.worker.cancelled.store(true, Ordering::Release);
        if let Some(tx) = job.cancel_tx.take() {
            let _ = tx.send(());
        }
        job.phase = JobPhase::Cancelled;
        job.worker.clone()
    };
    wait_for_worker(&worker).await?;
    account.validate()
}

// ── Cache management commands ───────────────────────────────────────

#[tauri::command]
pub async fn cmd_set_transcode_cache_limit(
    max_gb: u32,
    manager: tauri::State<'_, Arc<TranscodeManager>>,
) -> Result<(), String> {
    let gb = max_gb.clamp(1, 50);
    let max_bytes = (gb as u64) * 1024 * 1024 * 1024;
    manager.set_max_cache_bytes(max_bytes).await;
    log::info!(
        "Transcode: Cache limit set to {} GB ({} bytes)",
        gb,
        max_bytes
    );
    Ok(())
}

// ── Cached variants command ─────────────────────────────────────────

#[derive(serde::Serialize)]
pub struct CachedVariantInfo {
    pub quality: String,
    pub available: bool,
}

#[tauri::command]
pub async fn cmd_get_cached_variants(
    message_id: i32,
    folder_id: Option<i64>,
    manager: tauri::State<'_, Arc<TranscodeManager>>,
) -> Result<Vec<CachedVariantInfo>, String> {
    let account = manager.account(None)?;
    let folder_id = folder_id.unwrap_or(0);
    let file_key = format!("{}_{}_{}", account.owner, folder_id, message_id);

    let variants: Vec<CachedVariantInfo> = QUALITY_PRESETS
        .iter()
        .map(|p| {
            let output_dir = manager.hls_output_dir(&file_key, p.label);
            CachedVariantInfo {
                quality: p.label.to_string(),
                available: validate_hls_output(&output_dir).is_ok(),
            }
        })
        .collect();

    account.validate()?;
    Ok(variants)
}

// ── Detailed cache info (per-file per-quality with sizes) ──────────

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct CacheEntry {
    pub file_key: String,
    pub quality: String,
    pub size_bytes: u64,
    pub playlist_exists: bool,
}

#[derive(Clone, serde::Serialize)]
pub struct DetailedCacheInfo {
    pub entries: Vec<CacheEntry>,
    pub total_bytes: u64,
    pub max_bytes: u64,
    pub scan_in_progress: bool,
    pub last_scanned_at: Option<i64>,
    pub last_error: Option<String>,
}

fn scan_transcode_cache(cache_root: &Path) -> Result<Vec<CacheEntry>, String> {
    let mut entries = Vec::new();
    let hls_root = cache_root.join(HLS_DIR);

    match std::fs::read_dir(&hls_root) {
        Ok(file_dirs) => {
            for file_entry in file_dirs {
                let file_entry = match file_entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        log::warn!("Transcode: Could not inspect an HLS cache entry: {error}");
                        continue;
                    }
                };
                let file_type = file_entry.file_type().map_err(|error| {
                    format!("Could not inspect an HLS cache item type: {error}")
                })?;
                if !file_type.is_dir() {
                    continue;
                }
                let file_key = file_entry.file_name().to_string_lossy().into_owned();
                let quality_dirs = std::fs::read_dir(file_entry.path())
                    .map_err(|error| format!("Could not read cached variants: {error}"))?;

                for quality_entry in quality_dirs {
                    let quality_entry = match quality_entry {
                        Ok(entry) => entry,
                        Err(error) => {
                            log::warn!(
                                "Transcode: Could not inspect a cached quality entry: {error}"
                            );
                            continue;
                        }
                    };
                    let quality_type = quality_entry.file_type().map_err(|error| {
                        format!("Could not inspect a cached variant type: {error}")
                    })?;
                    if !quality_type.is_dir() {
                        continue;
                    }
                    let quality_path = quality_entry.path();
                    let mut size_bytes = 0u64;
                    let files = std::fs::read_dir(&quality_path)
                        .map_err(|error| format!("Could not read a cached variant: {error}"))?;
                    let mut observed_files = HashMap::new();
                    for file in files {
                        let file = match file {
                            Ok(entry) => entry,
                            Err(error) => {
                                log::warn!(
                                    "Transcode: Could not inspect a cached segment: {error}"
                                );
                                continue;
                            }
                        };
                        let file_type = file.file_type().map_err(|error| {
                            format!("Could not inspect a cached segment type: {error}")
                        })?;
                        if file_type.is_file() {
                            let metadata = file.metadata().map_err(|error| {
                                format!("Could not read cached segment metadata: {error}")
                            })?;
                            size_bytes = size_bytes.saturating_add(metadata.len());
                            observed_files.insert(
                                file.file_name().to_string_lossy().into_owned(),
                                metadata.len(),
                            );
                        }
                    }
                    entries.push(CacheEntry {
                        file_key: file_key.clone(),
                        quality: quality_entry.file_name().to_string_lossy().into_owned(),
                        size_bytes,
                        playlist_exists: validate_hls_inventory(&quality_path, &observed_files)
                            .is_ok(),
                    });
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Could not read the HLS cache directory: {error}")),
    }

    let originals_root = cache_root.join(ORIGINALS_DIR);
    match std::fs::read_dir(&originals_root) {
        Ok(originals) => {
            for original in originals {
                let original = match original {
                    Ok(entry) => entry,
                    Err(error) => {
                        log::warn!("Transcode: Could not inspect a cached original: {error}");
                        continue;
                    }
                };
                let file_type = original.file_type().map_err(|error| {
                    format!("Could not inspect a cached original type: {error}")
                })?;
                if !file_type.is_file() {
                    continue;
                }
                let metadata = original
                    .metadata()
                    .map_err(|error| format!("Could not read cached original metadata: {error}"))?;
                let path = original.path();
                entries.push(CacheEntry {
                    file_key: path
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    quality: "original".to_string(),
                    size_bytes: metadata.len(),
                    playlist_exists: true,
                });
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Could not read the original media cache directory: {error}"
            ))
        }
    }

    entries.sort_by(|left, right| {
        left.file_key
            .cmp(&right.file_key)
            .then_with(|| left.quality.cmp(&right.quality))
    });
    Ok(entries)
}

fn validate_hls_inventory(
    output_dir: &Path,
    observed_files: &HashMap<String, u64>,
) -> Result<(), String> {
    let playlist = std::fs::read_to_string(output_dir.join("index.m3u8"))
        .map_err(|error| format!("Failed to read HLS playlist: {error}"))?;
    if !playlist
        .lines()
        .any(|line| line.trim().starts_with("#EXTINF:"))
    {
        return Err("HLS playlist has no segments".to_string());
    }
    if !playlist.lines().any(|line| line.trim() == "#EXT-X-ENDLIST") {
        return Err("HLS playlist is incomplete".into());
    }

    let mut segment_count = 0usize;
    for line in playlist.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let segment_name = line.split('?').next().unwrap_or(line);
        let segment_path = Path::new(segment_name);
        if segment_path.components().count() != 1
            || segment_path.extension().and_then(|value| value.to_str()) != Some("ts")
        {
            return Err(format!(
                "HLS playlist contains an invalid segment path: {line}"
            ));
        }
        if observed_files.get(segment_name).copied().unwrap_or(0) == 0 {
            return Err(format!(
                "HLS segment {segment_name} is unavailable or empty"
            ));
        }
        segment_count += 1;
    }
    if segment_count == 0 {
        return Err("HLS playlist references no segment files".to_string());
    }
    Ok(())
}

#[tauri::command]
pub async fn cmd_get_detailed_transcode_cache(
    refresh: Option<bool>,
    manager: tauri::State<'_, Arc<TranscodeManager>>,
) -> Result<DetailedCacheInfo, String> {
    let manager = manager.inner().clone();
    let has_snapshot = manager
        .cache_snapshot
        .read()
        .await
        .last_scanned_at
        .is_some();
    if !manager.cache_scan_in_flight.load(Ordering::Acquire)
        && (refresh.unwrap_or(false) || !has_snapshot)
    {
        manager.start_cache_reconciliation(false);
    }
    Ok(manager.detailed_cache_snapshot().await)
}

// ── Clear transcode cache (all, per-file, or per-variant) ──────────

fn validate_cache_component(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(format!("Invalid {label} for transcode cache operation"));
    }
    Ok(())
}

fn remove_cache_path(path: &Path) -> Result<bool, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("Could not inspect a cache item: {error}")),
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        std::fs::remove_dir_all(path)
            .map_err(|error| format!("Could not remove a cache directory: {error}"))?;
    } else {
        std::fs::remove_file(path)
            .map_err(|error| format!("Could not remove a cache file: {error}"))?;
    }
    Ok(true)
}

fn cache_directory_is_empty(path: &Path) -> Result<bool, String> {
    match std::fs::read_dir(path) {
        Ok(mut entries) => Ok(entries.next().is_none()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(format!(
            "Could not inspect the remaining cache entries: {error}"
        )),
    }
}

fn clear_all_transcode_cache(cache_root: &Path) -> Result<String, String> {
    let mut removed_count = 0u64;
    let mut failures = Vec::new();
    for directory in [
        cache_root.join(HLS_DIR),
        cache_root.join(ORIGINALS_DIR),
        cache_root.join("fmp4"),
    ] {
        match std::fs::read_dir(&directory) {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(entry) => match remove_cache_path(&entry.path()) {
                            Ok(true) => removed_count += 1,
                            Ok(false) => {}
                            Err(error) => failures.push(error),
                        },
                        Err(error) => {
                            failures.push(format!("Could not read a cache item: {error}"))
                        }
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => failures.push(format!("Could not read a cache directory: {error}")),
        }
    }
    if failures.is_empty() {
        log::info!("Transcode: Cleared all cache ({} entries)", removed_count);
        Ok(format!(
            "Cleared all transcode cache ({} entries)",
            removed_count
        ))
    } else {
        Err(format!(
            "Transcode cache was only partially cleared ({} entries removed): {}",
            removed_count, failures[0]
        ))
    }
}

fn clear_file_transcode_cache(cache_root: &Path, file_key: &str) -> Result<String, String> {
    validate_cache_component(file_key, "file key")?;
    remove_cache_path(&cache_root.join(HLS_DIR).join(file_key))?;
    remove_cache_path(&cache_root.join("fmp4").join(file_key))?;
    remove_cache_path(
        &cache_root
            .join(ORIGINALS_DIR)
            .join(format!("{file_key}.mp4")),
    )?;
    log::info!("Transcode: Cleared cache for a file");
    Ok("Cleared cache for the selected file".to_string())
}

fn clear_variant_transcode_cache(
    cache_root: &Path,
    file_key: &str,
    quality: &str,
) -> Result<String, String> {
    validate_cache_component(file_key, "file key")?;
    validate_cache_component(quality, "quality")?;
    if !QUALITY_PRESETS.iter().any(|preset| preset.label == quality) {
        return Err("Unknown transcode quality".to_string());
    }

    let file_directory = cache_root.join(HLS_DIR).join(file_key);
    remove_cache_path(&file_directory.join(quality))?;
    if cache_directory_is_empty(&file_directory)? {
        remove_cache_path(&file_directory)?;
        remove_cache_path(
            &cache_root
                .join(ORIGINALS_DIR)
                .join(format!("{file_key}.mp4")),
        )?;
    }
    log::info!("Transcode: Cleared one cached quality variant");
    Ok(format!("Cleared {quality} variant for the selected file"))
}

#[tauri::command]
pub async fn cmd_clear_transcode_cache(
    file_key: Option<String>,
    quality: Option<String>,
    manager: tauri::State<'_, Arc<TranscodeManager>>,
) -> Result<String, String> {
    let manager = manager.inner().clone();
    let account = if let Some(key) = &file_key {
        manager.account_for_key(key)?
    } else {
        manager.account(None)?
    };
    let cache_root = manager.cache_root.clone();
    // Keep registration locked until removal completes so a newly queued job
    // cannot start writing into a directory that is being cleared.
    let jobs = manager.jobs.lock().await;
    for job in jobs.values() {
        let job = job.lock().await;
        let selected = file_key
            .as_ref()
            .is_none_or(|key| key == &job.key.file_key());
        if selected && job.has_live_writer() {
            return Err("CACHE_BUSY: Wait for active video conversions to finish before clearing their cache".into());
        }
    }
    account.validate()?;
    let result = tokio::task::spawn_blocking(move || {
        account.validate()?;
        match (file_key, quality) {
            (None, None) => clear_all_transcode_cache(&cache_root),
            (Some(file_key), None) => clear_file_transcode_cache(&cache_root, &file_key),
            (Some(file_key), Some(quality)) => {
                clear_variant_transcode_cache(&cache_root, &file_key, &quality)
            }
            (None, Some(_)) => Err("Cannot clear quality without specifying file key".to_string()),
        }
    })
    .await
    .map_err(|error| format!("Transcode cache clear task failed: {error}"))?;
    drop(jobs);
    if result.is_ok() {
        manager.start_cache_reconciliation(false);
    }
    result
}

#[tauri::command]
pub async fn cmd_get_master_playlist_info(
    message_id: i32,
    folder_id: Option<i64>,
    manager: tauri::State<'_, Arc<TranscodeManager>>,
) -> Result<MasterPlaylistInfo, String> {
    let account = manager.account(None)?;
    let folder_id = folder_id.unwrap_or(0);
    let file_key = format!("{}_{}_{}", account.owner, folder_id, message_id);

    let mut variants: Vec<MasterVariant> = Vec::new();

    for preset in QUALITY_PRESETS {
        let output_dir = manager.hls_output_dir(&file_key, preset.label);
        if validate_hls_output(&output_dir).is_ok() {
            // Try to read the playlist to get bandwidth info
            let bandwidth =
                estimate_bandwidth(&output_dir).unwrap_or(preset.video_bitrate_k * 1000);

            variants.push(MasterVariant {
                bandwidth,
                resolution: format!("{}x{}", preset.height * 16 / 9, preset.height),
                quality: preset.label.to_string(),
                playlist_path: format!("/hls/{}/{}/index.m3u8", file_key, preset.label),
            });
        }
    }

    let has_variants = !variants.is_empty();
    let master_url = if has_variants {
        Some(format!("/hls/{}/master.m3u8", file_key))
    } else {
        None
    };

    account.validate()?;
    Ok(MasterPlaylistInfo {
        file_key: file_key.clone(),
        variants,
        master_playlist_url: master_url,
    })
}

#[derive(serde::Serialize, Clone)]
pub struct MasterPlaylistInfo {
    pub file_key: String,
    pub variants: Vec<MasterVariant>,
    pub master_playlist_url: Option<String>,
}

#[derive(serde::Serialize, Clone)]
pub struct MasterVariant {
    pub bandwidth: u32,
    pub resolution: String,
    pub quality: String,
    pub playlist_path: String,
}

fn estimate_bandwidth(output_dir: &Path) -> Option<u32> {
    let playlist = output_dir.join("index.m3u8");
    let content = std::fs::read_to_string(&playlist).ok()?;

    // Sum up segment sizes and find total duration from EXTINF tags
    let mut total_bytes = 0u64;
    let mut total_duration = 0f64;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("#EXTINF:") {
            let dur_str = line
                .trim_start_matches("#EXTINF:")
                .split(',')
                .next()
                .unwrap_or("0");
            total_duration += dur_str.parse::<f64>().unwrap_or(0.0);
        } else if line.ends_with(".ts") {
            let seg_path = output_dir.join(line);
            if let Ok(meta) = std::fs::metadata(&seg_path) {
                total_bytes += meta.len();
            }
        }
    }

    if total_duration > 0.0 {
        Some(((total_bytes as f64 * 8.0) / total_duration) as u32)
    } else {
        None
    }
}

// ── HLS Serving Routes (Phase 4) ────────────────────────────────────────

use crate::server::StreamTokenData;

#[derive(serde::Deserialize)]
struct HlsQuery {
    token: Option<String>,
}

fn playlist_with_stream_token(playlist: &str, token: &str) -> String {
    let mut authenticated = String::with_capacity(playlist.len() + token.len() * 4);
    for line in playlist.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.contains("token=") {
            authenticated.push_str(line);
        } else {
            authenticated.push_str(line);
            authenticated.push(if line.contains('?') { '&' } else { '?' });
            authenticated.push_str("token=");
            authenticated.push_str(token);
        }
        authenticated.push('\n');
    }
    authenticated
}

/// Serve an HLS playlist (.m3u8) or segment (.ts).
async fn serve_hls_file(
    _req: HttpRequest,
    file_key: &str,
    quality: &str,
    segment: Option<&str>,
    query: &HlsQuery,
    manager: &TranscodeManager,
    token_data: &StreamTokenData,
) -> impl Responder {
    // Validate token
    match &query.token {
        Some(t) if t == &token_data.token => {}
        _ => return HttpResponse::Forbidden().body("Invalid or missing stream token"),
    }

    let account = match manager.account_for_key(file_key) {
        Ok(account) => account,
        Err(_) => return HttpResponse::Forbidden().body("Account changed"),
    };
    // Validate path
    let file_path = match manager.validate_hls_path(file_key, quality, segment) {
        Some(p) => p,
        None => return HttpResponse::NotFound().body("File not found"),
    };

    if !file_path.exists() {
        return HttpResponse::NotFound().body("File not found");
    }

    // Determine MIME type
    let is_playlist = file_path.extension().map(|e| e == "m3u8").unwrap_or(false);
    let mime = if is_playlist {
        "application/vnd.apple.mpegurl"
    } else if file_path.extension().map(|e| e == "ts").unwrap_or(false) {
        "video/mp2t"
    } else {
        "application/octet-stream"
    };

    match std::fs::read(&file_path) {
        Ok(data) => {
            if account.validate().is_err() {
                return HttpResponse::Forbidden().body("Account changed");
            }
            let body = if is_playlist {
                match String::from_utf8(data) {
                    Ok(playlist) => {
                        playlist_with_stream_token(&playlist, &token_data.token).into_bytes()
                    }
                    Err(error) => {
                        log::error!(
                            "Transcode: Invalid UTF-8 HLS playlist {:?}: {}",
                            file_path,
                            error
                        );
                        return HttpResponse::InternalServerError().body("Invalid HLS playlist");
                    }
                }
            } else {
                data
            };
            let mut resp = HttpResponse::Ok()
                .content_type(mime)
                .insert_header(("Accept-Ranges", "bytes"))
                .body(body);

            // A session switch must re-authorize every cached media response.
            resp.headers_mut().insert(
                actix_web::http::header::CACHE_CONTROL,
                actix_web::http::header::HeaderValue::from_static("private, no-store"),
            );

            resp
        }
        Err(e) => {
            log::error!("Transcode: Failed to read HLS file {:?}: {}", file_path, e);
            HttpResponse::InternalServerError().body("Failed to read file")
        }
    }
}

/// GET /hls/{file_key}/master.m3u8
#[actix_web::get("/hls/{file_key}/master.m3u8")]
async fn hls_master_playlist(
    _req: HttpRequest,
    path: web::Path<String>,
    query: web::Query<HlsQuery>,
    manager: web::Data<Arc<TranscodeManager>>,
    token_data: web::Data<StreamTokenData>,
) -> impl Responder {
    let file_key = path.into_inner();

    // Validate token
    match &query.token {
        Some(t) if t == &token_data.token => {}
        _ => return HttpResponse::Forbidden().body("Invalid or missing stream token"),
    }

    let account = match manager.account_for_key(&file_key) {
        Ok(account) => account,
        Err(_) => return HttpResponse::Forbidden().body("Account changed"),
    };
    // Build master playlist from available variants
    let mut playlist = String::from("#EXTM3U\n#EXT-X-VERSION:3\n");

    for preset in QUALITY_PRESETS {
        let hls_dir = manager.hls_output_dir(&file_key, preset.label);
        if validate_hls_output(&hls_dir).is_ok() {
            let bandwidth = estimate_bandwidth(&hls_dir).unwrap_or(preset.video_bitrate_k * 1000);
            let width = preset.height * 16 / 9;
            playlist.push_str(&format!(
                "#EXT-X-STREAM-INF:BANDWIDTH={},RESOLUTION={}x{}\n{}/index.m3u8\n",
                bandwidth, width, preset.height, preset.label
            ));
        }
    }

    if playlist.lines().count() <= 2 {
        return HttpResponse::NotFound().body("No HLS variants available");
    }

    if account.validate().is_err() {
        return HttpResponse::Forbidden().body("Account changed");
    }
    HttpResponse::Ok()
        .content_type("application/vnd.apple.mpegurl")
        .insert_header(("Cache-Control", "private, no-store"))
        .body(playlist_with_stream_token(&playlist, &token_data.token))
}

/// GET /hls/{file_key}/{quality}/index.m3u8
#[actix_web::get("/hls/{file_key}/{quality}/index.m3u8")]
async fn hls_playlist(
    req: HttpRequest,
    path: web::Path<(String, String)>,
    query: web::Query<HlsQuery>,
    manager: web::Data<Arc<TranscodeManager>>,
    token_data: web::Data<StreamTokenData>,
) -> impl Responder {
    let (file_key, quality) = path.into_inner();
    serve_hls_file(
        req,
        &file_key,
        &quality,
        None,
        &query,
        &manager,
        &token_data,
    )
    .await
}

/// GET /hls/{file_key}/{quality}/{segment}
#[actix_web::get("/hls/{file_key}/{quality}/{segment}")]
async fn hls_segment(
    req: HttpRequest,
    path: web::Path<(String, String, String)>,
    query: web::Query<HlsQuery>,
    manager: web::Data<Arc<TranscodeManager>>,
    token_data: web::Data<StreamTokenData>,
) -> impl Responder {
    let (file_key, quality, segment) = path.into_inner();
    serve_hls_file(
        req,
        &file_key,
        &quality,
        Some(&segment),
        &query,
        &manager,
        &token_data,
    )
    .await
}

/// Register HLS routes on an Actix ServiceConfig.
pub fn configure_hls_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(hls_master_playlist)
        .service(hls_playlist)
        .service(hls_segment);
}
