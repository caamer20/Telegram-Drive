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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use actix_web::{web, HttpRequest, HttpResponse, Responder};
use tokio::io::AsyncBufReadExt;
use tokio::sync::Mutex;

use crate::commands::TelegramState;
use crate::mp4_utils;
use grammers_client::types::Media;
use tauri::Manager;

// ── Constants ───────────────────────────────────────────────────────────

/// Maximum total cache size in bytes (5 GB).
pub const MAX_CACHE_BYTES: u64 = 5 * 1024 * 1024 * 1024;

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
    pub folder_id: i64, // 0 = root/me
    pub message_id: i32,
    pub quality: String,
}

impl TranscodeKey {
    pub fn file_key(&self) -> String {
        format!("{}_{}", self.folder_id, self.message_id)
    }

    pub fn job_id(&self) -> String {
        format!("{}_{}_{}", self.folder_id, self.message_id, self.quality)
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
}

// ── TranscodeManager ────────────────────────────────────────────────────

#[derive(Clone)]
pub struct TranscodeManager {
    pub cache_root: PathBuf,
    pub ffmpeg_path: Arc<Mutex<Option<PathBuf>>>,
    jobs: Arc<Mutex<HashMap<String, Arc<Mutex<TranscodeJob>>>>>,
    max_cache_bytes: Arc<Mutex<u64>>,
}

impl TranscodeManager {
    pub fn new(cache_root: PathBuf) -> Self {
        // Ensure subdirectories exist
        let _ = std::fs::create_dir_all(cache_root.join(ORIGINALS_DIR));
        let _ = std::fs::create_dir_all(cache_root.join(HLS_DIR));

        // Clean up partial output from previous sessions
        Self::clean_partial_outputs(&cache_root);

        Self {
            cache_root,
            ffmpeg_path: Arc::new(Mutex::new(None)),
            jobs: Arc::new(Mutex::new(HashMap::new())),
            max_cache_bytes: Arc::new(Mutex::new(MAX_CACHE_BYTES)),
        }
    }

    /// Clean up incomplete HLS directories from previous runs.
    fn clean_partial_outputs(cache_root: &Path) {
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
                    if !has_playlist {
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
                    if should_remove {
                        log::info!("Transcode: Removing incomplete file: {:?}", path);
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
        }
    }

    /// Get or create a job entry. Returns (job_arc, is_new).
    pub async fn get_or_create_job(&self, key: &TranscodeKey) -> (Arc<Mutex<TranscodeJob>>, bool) {
        let mut jobs = self.jobs.lock().await;
        let job_id = key.job_id();
        if let Some(job) = jobs.get(&job_id) {
            let mut j = job.lock().await;
            j.last_access = Instant::now();
            drop(j);
            (job.clone(), false)
        } else {
            let job = Arc::new(Mutex::new(TranscodeJob {
                key: key.clone(),
                phase: JobPhase::NotStarted,
                cancel_tx: None,
                last_access: Instant::now(),
                source_height: None,
            }));
            jobs.insert(job_id, job.clone());
            (job, true)
        }
    }

    /// Remove a job from the map.
    pub async fn remove_job(&self, job_id: &str) {
        let mut jobs = self.jobs.lock().await;
        jobs.remove(job_id);
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
        for entry_result in walker {
            if let Ok(entry) = entry_result {
                if entry.file_type().is_file() {
                    if let Ok(meta) = entry.metadata() {
                        total += meta.len();
                    }
                }
            }
        }
        total
    }

    /// Evict oldest files until cache is under the limit.
    /// Never evict files that belong to active jobs.
    pub async fn evict_lru(&self) {
        let max = *self.max_cache_bytes.lock().await;
        let current = self.total_cache_size();
        if current <= max {
            return;
        }

        // Collect all files with their modification times
        let mut files: Vec<(PathBuf, u64, SystemTime)> = Vec::new();
        let walker = walkdir::WalkDir::new(&self.cache_root).min_depth(1);
        for entry_result in walker {
            if let Ok(entry) = entry_result {
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
        }

        // Sort by modification time (oldest first)
        files.sort_by_key(|(_, _, mtime)| *mtime);

        let mut freed: u64 = 0;
        let target = current.saturating_sub(max);

        for (path, size, _) in &files {
            if freed >= target {
                break;
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
    let output = tokio::process::Command::new(path)
        .arg("-version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("Failed to run ffmpeg: {}", e))?;

    Ok(output.status.success())
}

// ── Source Cache (Phase 2) ──────────────────────────────────────────────

/// Download the original MP4 file from Telegram to local cache.
/// Returns the total file size on success.
pub async fn cache_original(
    client: &grammers_client::Client,
    media: &Media,
    dest_path: &Path,
    cancel_rx: &mut tokio::sync::oneshot::Receiver<()>,
    progress_callback: impl Fn(f32),
) -> Result<u64, String> {
    let total_size = match media {
        Media::Document(d) => d.size() as u64,
        _ => return Err("Not a document".to_string()),
    };

    // Ensure parent directory exists
    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create cache dir: {}", e))?;
    }

    let tmp_path = dest_path.with_extension("mp4.part");
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .map_err(|e| format!("Failed to create cache file: {}", e))?;

    use tokio::io::AsyncWriteExt;

    let mut download_iter = client.iter_download(media);
    download_iter = download_iter.chunk_size(65536);
    let mut downloaded: u64 = 0;

    loop {
        tokio::select! {
            _ = &mut *cancel_rx => {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err("Cancelled".to_string());
            }
            result = download_iter.next() => {
                match result {
                    Ok(Some(chunk)) => {
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

    // Rename .part → .mp4
    tokio::fs::rename(&tmp_path, dest_path)
        .await
        .map_err(|e| format!("Rename error: {}", e))?;

    log::info!(
        "Transcode: Cached original to {:?} ({} bytes)",
        dest_path,
        actual_size
    );
    Ok(actual_size)
}

// ── HLS Transcode (Phase 3) ─────────────────────────────────────────────

/// Run FFmpeg to generate a single HLS variant.
pub async fn run_transcode(
    ffmpeg_path: &Path,
    input_path: &Path,
    output_dir: &Path,
    quality: &QualityPreset,
    duration_secs: Option<f64>,
    cancel_rx: &mut tokio::sync::oneshot::Receiver<()>,
    progress_callback: impl Fn(f32),
) -> Result<(), String> {
    // Create output directory
    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("Failed to create HLS output dir: {}", e))?;

    let playlist_path = output_dir.join("index.m3u8");
    let segment_pattern = output_dir.join("segment_%03d.ts");

    let mut cmd = tokio::process::Command::new(ffmpeg_path);
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
                            stderr_error_lines.push(line.clone());
                        }

                        // Parse time=HH:MM:SS.MS from FFmpeg stderr
                        if let Some(time_str) = line.split("time=").nth(1) {
                            let time_str = time_str.split_whitespace().next().unwrap_or("0");
                            let secs = parse_time_to_secs(time_str);
                            if let Some(dur) = duration_secs {
                                if dur > 0.0 {
                                    let pct = (secs / dur as f64) as f32;
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

    // Verify the playlist was created
    if !playlist_path.exists() {
        return Err("FFmpeg completed but no playlist was produced".to_string());
    }

    // ── Validate HLS output before marking ready ────────────────────
    let playlist_content = std::fs::read_to_string(&playlist_path)
        .map_err(|e| format!("Failed to read playlist: {}", e))?;

    // Check for at least one #EXTINF tag
    let has_extinf = playlist_content
        .lines()
        .any(|l| l.trim().starts_with("#EXTINF:"));
    if !has_extinf {
        let _ = std::fs::remove_dir_all(output_dir);
        return Err("HLS playlist has no segments (no #EXTINF tags)".to_string());
    }

    // Check at least one .ts segment exists and is non-empty
    let has_valid_segment = playlist_content
        .lines()
        .filter(|l| l.trim().ends_with(".ts"))
        .any(|l| {
            let seg_path = output_dir.join(l.trim());
            seg_path.exists()
                && std::fs::metadata(&seg_path)
                    .map(|m| m.len() > 0)
                    .unwrap_or(false)
        });

    if !has_valid_segment {
        let _ = std::fs::remove_dir_all(output_dir);
        return Err("HLS playlist references no valid segment files".to_string());
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

/// Detect video resolution from a cached original MP4 file.
pub fn get_source_height(cached_path: &std::path::Path) -> Option<u32> {
    let data = std::fs::read(cached_path).ok()?;
    let buffer = &data[..std::cmp::min(2 * 1024 * 1024, data.len())];
    mp4_utils::scan_video_tkhd_dimensions(buffer).1
}

// ── Execute Full Transcode Pipeline ─────────────────────────────────────

/// Run the full pipeline: cache original → transcode HLS.
/// Runs entirely on the async runtime (FFmpeg runs in its own OS process).
pub async fn execute_transcode_pipeline(
    manager: &TranscodeManager,
    key: &TranscodeKey,
    quality_preset: &QualityPreset,
    client: grammers_client::Client,
    media: Media,
    duration_secs: Option<f64>,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
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
                    job.phase = JobPhase::CachingOriginal { progress };
                });
            },
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
                job.phase = JobPhase::Error(format!("Cache failed: {}", e));
                return;
            }
        }
    }

    // ── Step 2: Detect source resolution ────────────────────────────
    let source_height = {
        let data = std::fs::read(&original_path).unwrap_or_default();
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
                job.phase = JobPhase::Transcoding { progress };
            });
        },
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
            job.phase = JobPhase::Error(e);
        }
    }
}

// ── Tauri Commands ──────────────────────────────────────────────────────

#[tauri::command]
pub async fn cmd_get_transcode_capabilities(
    manager: tauri::State<'_, TranscodeManager>,
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
    manager: tauri::State<'_, TranscodeManager>,
) -> Result<TranscodePrepareResult, String> {
    let folder_id = folder_id.unwrap_or(0);
    let key = TranscodeKey {
        folder_id,
        message_id,
        quality: quality.clone(),
    };

    // Validate quality
    let preset = QUALITY_PRESETS
        .iter()
        .find(|p| p.label == quality)
        .ok_or_else(|| format!("Unknown quality: {}", quality))?;

    // Check if already ready
    let output_dir = manager.hls_output_dir(&key.file_key(), &quality);
    if output_dir.join("index.m3u8").exists() {
        return Ok(TranscodePrepareResult {
            job_id: key.job_id(),
            status: "ready".to_string(),
            progress: 1.0,
            playlist_url: Some(format!("/hls/{}/{}/index.m3u8", key.file_key(), quality)),
        });
    }

    // Check if job already exists
    let (job_arc, is_new) = manager.get_or_create_job(&key).await;
    let phase = {
        let job = job_arc.lock().await;
        job.phase.clone()
    };

    if !is_new {
        // Job exists — return its current status
        return match &phase {
            JobPhase::NotStarted => Ok(TranscodePrepareResult {
                job_id: key.job_id(),
                status: "pending".to_string(),
                progress: 0.0,
                playlist_url: None,
            }),
            JobPhase::CachingOriginal { progress } => Ok(TranscodePrepareResult {
                job_id: key.job_id(),
                status: "caching".to_string(),
                progress: *progress,
                playlist_url: None,
            }),
            JobPhase::Transcoding { progress } => Ok(TranscodePrepareResult {
                job_id: key.job_id(),
                status: "transcoding".to_string(),
                progress: *progress,
                playlist_url: None,
            }),
            JobPhase::Ready => Ok(TranscodePrepareResult {
                job_id: key.job_id(),
                status: "ready".to_string(),
                progress: 1.0,
                playlist_url: Some(format!("/hls/{}/{}/index.m3u8", key.file_key(), quality)),
            }),
            JobPhase::Error(_e) => Ok(TranscodePrepareResult {
                job_id: key.job_id(),
                status: "error".to_string(),
                progress: 0.0,
                playlist_url: None,
            }),
            JobPhase::Cancelled => Ok(TranscodePrepareResult {
                job_id: key.job_id(),
                status: "cancelled".to_string(),
                progress: 0.0,
                playlist_url: None,
            }),
        };
    }

    // New job — start the pipeline
    let client = { state.client.lock().await.clone() };
    let client = client.ok_or_else(|| "Not connected to Telegram".to_string())?;

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

    let messages = client
        .get_messages_by_id(&peer, &[message_id])
        .await
        .map_err(|e| e.to_string())?;

    let msg = messages
        .into_iter()
        .flatten()
        .next()
        .ok_or_else(|| format!("Message {} not found", message_id))?;

    let media = msg.media().ok_or_else(|| "No media".to_string())?;

    // Get duration from mp4parse (quick moov chunk)
    let duration_secs = get_duration_from_media(&client, message_id, folder_id, &state)
        .await
        .ok();

    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();

    {
        let mut job = job_arc.lock().await;
        job.cancel_tx = Some(cancel_tx);
    }

    let manager_clone = manager.inner().clone();
    let key_clone = key.clone();
    let preset_clone = preset.clone();

    // Spawn the pipeline on a background task
    tauri::async_runtime::spawn(async move {
        execute_transcode_pipeline(
            &manager_clone,
            &key_clone,
            &preset_clone,
            client,
            media,
            duration_secs,
            cancel_rx,
        )
        .await;

        // LRU eviction after job completes
        manager_clone.evict_lru().await;
    });

    Ok(TranscodePrepareResult {
        job_id: key.job_id(),
        status: "started".to_string(),
        progress: 0.0,
        playlist_url: None,
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
    manager: tauri::State<'_, TranscodeManager>,
) -> Result<TranscodeStatusResult, String> {
    let jobs = manager.jobs.lock().await;
    let job_arc = jobs
        .get(&job_id)
        .ok_or_else(|| format!("Job {} not found", job_id))?;

    let job = job_arc.lock().await;
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
    manager: tauri::State<'_, TranscodeManager>,
) -> Result<(), String> {
    let jobs = manager.jobs.lock().await;
    let job_arc = jobs
        .get(&job_id)
        .ok_or_else(|| format!("Job {} not found", job_id))?;

    let mut job = job_arc.lock().await;
    if let Some(tx) = job.cancel_tx.take() {
        let _ = tx.send(());
    }
    job.phase = JobPhase::Cancelled;

    Ok(())
}

// ── Cache management commands ───────────────────────────────────────

#[derive(serde::Serialize)]
pub struct TranscodeCacheInfo {
    pub current_bytes: u64,
    pub max_bytes: u64,
    pub cached_variants: Vec<String>,
}
#[tauri::command]
pub async fn cmd_get_transcode_cache_info(
    manager: tauri::State<'_, TranscodeManager>,
) -> Result<TranscodeCacheInfo, String> {
    let current = manager.total_cache_size();
    let max = manager.get_max_cache_bytes().await;
    Ok(TranscodeCacheInfo {
        current_bytes: current,
        max_bytes: max,
        cached_variants: vec![],
    })
}

#[tauri::command]
pub async fn cmd_set_transcode_cache_limit(
    max_gb: u32,
    manager: tauri::State<'_, TranscodeManager>,
) -> Result<(), String> {
    let gb = std::cmp::max(1, std::cmp::min(50, max_gb));
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
    manager: tauri::State<'_, TranscodeManager>,
) -> Result<Vec<CachedVariantInfo>, String> {
    let folder_id = folder_id.unwrap_or(0);
    let file_key = format!("{}_{}", folder_id, message_id);

    let variants: Vec<CachedVariantInfo> = QUALITY_PRESETS
        .iter()
        .map(|p| {
            let output_dir = manager.hls_output_dir(&file_key, p.label);
            CachedVariantInfo {
                quality: p.label.to_string(),
                available: output_dir.join("index.m3u8").exists(),
            }
        })
        .collect();

    Ok(variants)
}

// ── Detailed cache info (per-file per-quality with sizes) ──────────

#[derive(serde::Serialize)]
pub struct CacheEntry {
    pub file_key: String,
    pub quality: String,
    pub size_bytes: u64,
    pub playlist_exists: bool,
}

#[derive(serde::Serialize)]
pub struct DetailedCacheInfo {
    pub entries: Vec<CacheEntry>,
    pub total_bytes: u64,
    pub max_bytes: u64,
}

#[tauri::command]
pub async fn cmd_get_detailed_transcode_cache(
    manager: tauri::State<'_, TranscodeManager>,
) -> Result<DetailedCacheInfo, String> {
    let mut entries: Vec<CacheEntry> = Vec::new();
    let hls_root = manager.cache_root.join(HLS_DIR);

    if let Ok(file_dirs) = std::fs::read_dir(&hls_root) {
        for file_entry in file_dirs.flatten() {
            let file_path = file_entry.path();
            if !file_path.is_dir() {
                continue;
            }
            let file_key = file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            if let Ok(quality_dirs) = std::fs::read_dir(&file_path) {
                for q_entry in quality_dirs.flatten() {
                    let q_path = q_entry.path();
                    if !q_path.is_dir() {
                        continue;
                    }
                    let quality = q_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();

                    let playlist_exists = q_path.join("index.m3u8").exists();

                    // Sum file sizes in this quality directory
                    let mut size_bytes = 0u64;
                    if let Ok(files) = std::fs::read_dir(&q_path) {
                        for f in files.flatten() {
                            if let Ok(meta) = f.metadata() {
                                size_bytes += meta.len();
                            }
                        }
                    }

                    entries.push(CacheEntry {
                        file_key: file_key.clone(),
                        quality,
                        size_bytes,
                        playlist_exists,
                    });
                }
            }
        }
    }

    // Also count originals
    let orig_root = manager.cache_root.join(ORIGINALS_DIR);
    if let Ok(orig_files) = std::fs::read_dir(&orig_root) {
        for of in orig_files.flatten() {
            let path = of.path();
            if path.is_file() {
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                entries.push(CacheEntry {
                    file_key: stem.to_string(),
                    quality: "original".to_string(),
                    size_bytes,
                    playlist_exists: path.exists(),
                });
            }
        }
    }

    let total_bytes: u64 = entries.iter().map(|e| e.size_bytes).sum();
    let max_bytes = manager.get_max_cache_bytes().await;

    Ok(DetailedCacheInfo {
        entries,
        total_bytes,
        max_bytes,
    })
}

// ── Clear transcode cache (all, per-file, or per-variant) ──────────

#[tauri::command]
pub async fn cmd_clear_transcode_cache(
    file_key: Option<String>,
    quality: Option<String>,
    manager: tauri::State<'_, TranscodeManager>,
) -> Result<String, String> {
    match (file_key, quality) {
        // Clear everything
        (None, None) => {
            let hls_root = manager.cache_root.join(HLS_DIR);
            let orig_root = manager.cache_root.join(ORIGINALS_DIR);
            let mut removed_count = 0u64;

            if hls_root.exists() {
                if let Ok(entries) = std::fs::read_dir(&hls_root) {
                    for entry in entries.flatten() {
                        let _ = std::fs::remove_dir_all(entry.path());
                        removed_count += 1;
                    }
                }
            }
            if orig_root.exists() {
                if let Ok(entries) = std::fs::read_dir(&orig_root) {
                    for entry in entries.flatten() {
                        let _ = std::fs::remove_file(entry.path());
                        removed_count += 1;
                    }
                }
            }

            log::info!("Transcode: Cleared all cache ({} entries)", removed_count);
            Ok(format!(
                "Cleared all transcode cache ({} entries)",
                removed_count
            ))
        }
        // Clear all variants for a specific file
        (Some(fk), None) => {
            let hls_path = manager.cache_root.join(HLS_DIR).join(&fk);
            let orig_path = manager
                .cache_root
                .join(ORIGINALS_DIR)
                .join(format!("{}.mp4", fk));

            if hls_path.exists() {
                let _ = std::fs::remove_dir_all(&hls_path);
            }
            if orig_path.exists() {
                let _ = std::fs::remove_file(&orig_path);
            }

            log::info!("Transcode: Cleared cache for file {}", fk);
            Ok(format!("Cleared cache for {}", fk))
        }
        // Clear a specific quality variant for a file
        (Some(fk), Some(q)) => {
            let variant_path = manager.hls_output_dir(&fk, &q);
            if variant_path.exists() {
                let _ = std::fs::remove_dir_all(&variant_path);
            }
            // If no more qualities remain for this file, also remove the parent directory
            // and the orphaned original file.
            let file_dir = manager.cache_root.join(HLS_DIR).join(&fk);
            if file_dir.exists() {
                let has_other_variants = std::fs::read_dir(&file_dir)
                    .map(|mut d| d.any(|e| e.ok().map(|e| e.path().is_dir()).unwrap_or(false)))
                    .unwrap_or(false);
                if !has_other_variants {
                    let _ = std::fs::remove_dir_all(&file_dir);
                    // Clean up orphaned original so it doesn't linger on disk
                    let orig_path = manager.original_path(&fk);
                    if orig_path.exists() {
                        let _ = std::fs::remove_file(&orig_path);
                        log::info!("Transcode: Removed orphaned original {:?}", orig_path);
                    }
                }
            }

            log::info!("Transcode: Cleared variant {} for file {}", q, fk);
            Ok(format!("Cleared {} variant for {}", q, fk))
        }
        (None, Some(_)) => Err("Cannot clear quality without specifying file_key".to_string()),
    }
}

#[tauri::command]
pub async fn cmd_get_master_playlist_info(
    message_id: i32,
    folder_id: Option<i64>,
    manager: tauri::State<'_, TranscodeManager>,
) -> Result<MasterPlaylistInfo, String> {
    let folder_id = folder_id.unwrap_or(0);
    let file_key = format!("{}_{}", folder_id, message_id);

    let mut variants: Vec<MasterVariant> = Vec::new();

    for preset in QUALITY_PRESETS {
        let output_dir = manager.hls_output_dir(&file_key, preset.label);
        let playlist_path = output_dir.join("index.m3u8");

        if playlist_path.exists() {
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

    // Validate path
    let file_path = match manager.validate_hls_path(file_key, quality, segment) {
        Some(p) => p,
        None => return HttpResponse::NotFound().body("File not found"),
    };

    if !file_path.exists() {
        return HttpResponse::NotFound().body("File not found");
    }

    // Determine MIME type
    let mime = if file_path.extension().map(|e| e == "m3u8").unwrap_or(false) {
        "application/vnd.apple.mpegurl"
    } else if file_path.extension().map(|e| e == "ts").unwrap_or(false) {
        "video/mp2t"
    } else {
        "application/octet-stream"
    };

    match std::fs::read(&file_path) {
        Ok(data) => {
            let mut resp = HttpResponse::Ok()
                .content_type(mime)
                .insert_header(("Accept-Ranges", "bytes"))
                .body(data);

            // Cache headers: segments can be cached longer, playlists shorter
            if mime == "video/mp2t" {
                resp.headers_mut().insert(
                    actix_web::http::header::CACHE_CONTROL,
                    actix_web::http::header::HeaderValue::from_static("public, max-age=3600"),
                );
            } else {
                resp.headers_mut().insert(
                    actix_web::http::header::CACHE_CONTROL,
                    actix_web::http::header::HeaderValue::from_static("private, max-age=10"),
                );
            }

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

    // Build master playlist from available variants
    let mut playlist = String::from("#EXTM3U\n#EXT-X-VERSION:3\n");

    for preset in QUALITY_PRESETS {
        let hls_dir = manager.hls_output_dir(&file_key, preset.label);
        if hls_dir.join("index.m3u8").exists() {
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

    HttpResponse::Ok()
        .content_type("application/vnd.apple.mpegurl")
        .insert_header(("Cache-Control", "private, max-age=5"))
        .body(playlist)
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
