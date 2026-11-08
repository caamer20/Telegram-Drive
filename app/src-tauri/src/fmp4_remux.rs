// ── fMP4 Remux Module ───────────────────────────────────────────────────
// Handles on-the-fly conversion of progressive (moov-at-end) MP4 files into
// fragmented MP4 (fMP4) using FFmpeg stream-copy (no re-encoding).  The
// output fMP4 can be parsed by mp4box and fed into the frontend's MediaSource
// Extensions pipeline, eliminating the need to fall back to native <video>.
//
// Cache layout:
//   $APPDATA/streaming/fmp4/{owner}_{folder_id}_{message_id}/output.mp4

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::commands::TelegramState;
use crate::server::StreamTokenData;
use crate::transcode::{JobPhase, TranscodeKey, TranscodeManager};
use crate::workspace::AccountGuard;
use actix_web::{web, HttpRequest, HttpResponse, Responder};

// ── Constants ────────────────────────────────────────────────────────

/// Subdirectory under the streaming cache root for fMP4 outputs.
const FMP4_DIR: &str = "fmp4";

// ── Types ────────────────────────────────────────────────────────────

#[derive(serde::Serialize, Clone)]
pub struct Fmp4StreamInfo {
    pub url: String,
    pub output_file_key: String,
    /// "ready" if the fMP4 is available, "processing" if download/remux is in progress.
    pub status: String,
}

#[derive(serde::Serialize, Clone)]
pub struct Fmp4StatusResult {
    pub status: String,
    pub error: Option<String>,
}

/// Shared state for tracking in-flight fMP4 remux jobs.
/// Managed as Tauri state so both commands and the frontend can query progress.
#[derive(Clone)]
pub struct Fmp4RemuxState {
    /// Maps file_key → job status: None = not started, Some(None) = in progress,
    /// Some(Some(err)) = failed with error. Absent + output exists = ready.
    jobs: Arc<Mutex<HashMap<String, Option<String>>>>,
}

impl Fmp4RemuxState {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for Fmp4RemuxState {
    fn default() -> Self {
        Self::new()
    }
}

// ── FFmpeg Remux ─────────────────────────────────────────────────────

/// Run FFmpeg to remux a progressive MP4 into a fragmented MP4 (fMP4).
///
/// Uses `-c copy` (stream copy) for maximum speed — no re-encoding.
///
/// # Flags
/// - `frag_keyframe`  — start a new fragment at every video keyframe
/// - `empty_moov`     — initial moov is minimal; track metadata lives in moof boxes
/// - `default_base_moof` — ensures each moof has the necessary base offset
///
/// These flags produce an fMP4 that mp4box's `initializeSegmentation()` can
/// handle, enabling full MSE playback.
pub async fn run_fmp4_remux(
    ffmpeg_path: &Path,
    input_path: &Path,
    output_path: &Path,
    cancel_rx: &mut tokio::sync::oneshot::Receiver<()>,
    progress_callback: impl Fn(f32),
    account: &AccountGuard,
) -> Result<(), String> {
    account.validate()?;
    // Ensure output directory exists
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create fMP4 output dir: {}", e))?;
    }

    let mut cmd = tokio::process::Command::new(ffmpeg_path);
    crate::process_util::hide_console(&mut cmd);
    cmd.arg("-y") // Overwrite existing output
        .arg("-i")
        .arg(input_path)
        .arg("-c")
        .arg("copy") // Stream copy — no re-encode
        .arg("-movflags")
        .arg("frag_keyframe+empty_moov+default_base_moof")
        .arg("-f")
        .arg("mp4")
        .arg(output_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn FFmpeg for fMP4 remux: {}", e))?;

    let stderr = child
        .stderr
        .take()
        .ok_or("No stderr pipe for FFmpeg fMP4 remux")?;

    // Read stderr lines for progress (best-effort) and error collection
    let stderr_reader = tokio::io::BufReader::new(stderr);
    let mut lines = tokio::io::AsyncBufReadExt::lines(stderr_reader);
    let input_size = std::fs::metadata(input_path).map(|m| m.len()).unwrap_or(0);

    let mut account_check = tokio::time::interval(std::time::Duration::from_secs(1));
    let parse_result: Result<(), String> = loop {
        tokio::select! {
            _ = account_check.tick() => {
                if let Err(error) = account.validate() {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    let _ = std::fs::remove_file(output_path);
                    break Err(error);
                }
            }
            _ = &mut *cancel_rx => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let _ = std::fs::remove_file(output_path);
                break Err("Cancelled".to_string());
            }
            line_result = lines.next_line() => {
                match line_result {
                    Ok(Some(line)) => {
                        // Parse time= for progress
                        if let Some(time_str) = line.split("time=").nth(1) {
                            let time_str = time_str.split_whitespace().next().unwrap_or("0");
                            if let Ok(secs) = parse_ffmpeg_time(time_str) {
                                // With -c copy, duration is the total input duration.
                                // Report progress based on time position.
                                if input_size > 0 && secs > 0.0 {
                                    // Coarse progress from stderr time markers
                                    progress_callback(0.5); // FFmpeg spends ~50% time reading input
                                }
                            }
                        }
                    }
                    Ok(None) => break Ok(()),
                    Err(e) => {
                        log::warn!("fMP4 remux: stderr read error: {}", e);
                        break Ok(());
                    }
                }
            }
        }
    };

    // Check cancellation
    parse_result?;

    let status = child
        .wait()
        .await
        .map_err(|e| format!("FFmpeg fMP4 wait error: {}", e))?;

    if !status.success() {
        let _ = std::fs::remove_file(output_path);
        return Err(format!(
            "FFmpeg fMP4 remux exited with code {:?}",
            status.code()
        ));
    }
    account.validate()?;

    // Verify output
    if !output_path.exists() {
        return Err("FFmpeg fMP4 remux completed but no output file was produced".to_string());
    }

    let output_size = std::fs::metadata(output_path).map(|m| m.len()).unwrap_or(0);
    if output_size == 0 {
        let _ = std::fs::remove_file(output_path);
        return Err("FFmpeg fMP4 remux produced an empty output file".to_string());
    }

    log::info!(
        "fMP4 remux: output {:?} ({} bytes)",
        output_path,
        output_size
    );

    Ok(())
}

/// Parse an FFmpeg time string like "00:05:30.12" into seconds.
fn parse_ffmpeg_time(time: &str) -> Result<f64, ()> {
    let parts: Vec<&str> = time.split(':').collect();
    if parts.len() == 3 {
        let h: f64 = parts[0].parse().map_err(|_| ())?;
        let m: f64 = parts[1].parse().map_err(|_| ())?;
        let s: f64 = parts[2].parse().map_err(|_| ())?;
        Ok(h * 3600.0 + m * 60.0 + s)
    } else {
        Err(())
    }
}

// ── Tauri Commands ───────────────────────────────────────────────────

/// Prepare a fragmented MP4 stream for a progressive MP4 file.
///
/// Returns immediately with `status: "ready"` if the fMP4 is cached, or
/// `status: "processing"` after spawning the download+remux in the background.
/// The frontend polls `cmd_get_fmp4_status` until the job completes.
#[tauri::command]
pub async fn cmd_prepare_fmp4_stream(
    message_id: i32,
    folder_id: Option<i64>,
    state: tauri::State<'_, TelegramState>,
    manager: tauri::State<'_, Arc<TranscodeManager>>,
    remux_state: tauri::State<'_, Fmp4RemuxState>,
) -> Result<Fmp4StreamInfo, String> {
    let account = manager.account(None)?;
    let key = TranscodeKey {
        owner_id: account.owner,
        folder_id: folder_id.unwrap_or(0),
        message_id,
        quality: "fmp4".into(),
    };
    let file_key = key.file_key();
    let url = format!("/fmp4/{file_key}/output.mp4");
    let (job, is_new) = manager.get_or_create_job(&key).await;
    if !is_new {
        account.validate()?;
        return Ok(Fmp4StreamInfo {
            url,
            output_file_key: file_key,
            status: "processing".into(),
        });
    }
    // The same registration protects originals and remux output from cache
    // clearing/eviction until all writers (including FFmpeg) have terminated.
    let lease = job.lock().await.writer_lease();
    let output_path = manager
        .cache_root
        .join(FMP4_DIR)
        .join(&file_key)
        .join("output.mp4");
    account.validate()?;
    if completed_output(&output_path) {
        job.lock().await.phase = JobPhase::Ready;
        remux_state.jobs.lock().await.remove(&file_key);
        return Ok(Fmp4StreamInfo {
            url,
            output_file_key: file_key,
            status: "ready".into(),
        });
    }

    let setup: Result<_, String> = async {
        let client = state.client.lock().await.clone().ok_or("Not connected to Telegram")?;
        let actual = client.get_me().await.map_err(|e| e.to_string())?;
        if actual.bare_id() != account.owner { return Err("ACCOUNT_CHANGED".into()); }
        account.validate()?;
        let peer = crate::commands::utils::resolve_peer(&client, folder_id, &state.peer_cache).await?;
        let message = client.get_messages_by_id(&peer, &[message_id]).await.map_err(|e| e.to_string())?
            .into_iter().flatten().next().ok_or("Message not found")?;
        let media = message.media().ok_or("No media")?;
        if message.text() == "TDENC2"
            || matches!(&media, grammers_client::types::Media::Document(document) if document.name().to_ascii_lowercase().ends_with(".tdenc")) {
            return Err("ENCRYPTED_PREVIEW_UNAVAILABLE".into());
        }
        let ffmpeg = manager.ffmpeg_path.lock().await.clone().ok_or("FFmpeg is not available")?;
        account.validate()?;
        Ok((client, media, ffmpeg))
    }.await;
    let (client, media, ffmpeg) = match setup {
        Ok(values) => values,
        Err(error) => {
            job.lock().await.phase = JobPhase::Error(error.clone());
            return Err(error);
        }
    };
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel();
    job.lock().await.cancel_tx = Some(cancel_tx);
    remux_state.jobs.lock().await.insert(file_key.clone(), None);
    let manager = manager.inner().clone();
    let remux_state = remux_state.inner().clone();
    let task_key = file_key.clone();
    tokio::spawn(async move {
        let _lease = lease;
        let original_path = manager.original_path(&task_key);
        let partial_path = output_path.with_extension("mp4.part");
        let result: Result<(), String> = async {
            let source_lock = crate::workspace::assets::file_lock(format!(
                "transcode-source:{}:{}",
                manager.cache_root.display(),
                task_key
            ))
            .await;
            let source_guard = tokio::select! {
                guard = source_lock.lock() => guard,
                _ = &mut cancel_rx => return Err("Cancelled".into()),
            };
            account.validate()?;
            if !original_path.is_file() {
                job.lock().await.phase = JobPhase::CachingOriginal { progress: 0.0 };
                crate::transcode::cache_original(
                    &client,
                    &media,
                    &original_path,
                    &mut cancel_rx,
                    |_| {},
                    &account,
                    &manager,
                )
                .await?;
            }
            drop(source_guard);
            account.validate()?;
            job.lock().await.phase = JobPhase::Transcoding { progress: 0.0 };
            run_fmp4_remux(
                &ffmpeg,
                &original_path,
                &partial_path,
                &mut cancel_rx,
                |_| {},
                &account,
            )
            .await?;
            publish_output(&partial_path, &output_path, || account.validate())?;
            Ok(())
        }
        .await;
        if result.is_err() {
            let _ = std::fs::remove_file(&partial_path);
        }
        let mut jobs = remux_state.jobs.lock().await;
        match result {
            Ok(()) => {
                job.lock().await.phase = JobPhase::Ready;
                jobs.remove(&task_key);
            }
            Err(error) => {
                job.lock().await.phase = JobPhase::Error(error.clone());
                jobs.insert(task_key, Some(error));
            }
        }
        drop(jobs);
        drop(_lease);
        manager.evict_lru().await;
    });
    Ok(Fmp4StreamInfo {
        url,
        output_file_key: file_key,
        status: "processing".into(),
    })
}

fn completed_output(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_file() && meta.len() > 0)
}

fn publish_output(
    partial: &Path,
    output: &Path,
    validate: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    validate()?;
    if !completed_output(partial) {
        return Err("Incomplete fMP4 output".into());
    }
    std::fs::rename(partial, output).map_err(|error| error.to_string())
}

/// Poll the status of an fMP4 remux job.
#[tauri::command]
pub async fn cmd_get_fmp4_status(
    file_key: String,
    manager: tauri::State<'_, Arc<TranscodeManager>>,
    remux_state: tauri::State<'_, Fmp4RemuxState>,
) -> Result<Fmp4StatusResult, String> {
    let account = manager.account_for_key(&file_key)?;
    // Check if output file already exists (ready)
    let output_path = manager
        .cache_root
        .join(FMP4_DIR)
        .join(&file_key)
        .join("output.mp4");
    if completed_output(&output_path) {
        account.validate()?;
        // Clean up job entry if still present
        remux_state.jobs.lock().await.remove(&file_key);
        return Ok(Fmp4StatusResult {
            status: "ready".to_string(),
            error: None,
        });
    }

    let jobs = remux_state.jobs.lock().await;
    account.validate()?;
    match jobs.get(&file_key) {
        Some(None) => Ok(Fmp4StatusResult {
            status: "processing".to_string(),
            error: None,
        }),
        Some(Some(err)) => Ok(Fmp4StatusResult {
            status: "error".to_string(),
            error: Some(err.clone()),
        }),
        None => Ok(Fmp4StatusResult {
            status: "not_found".to_string(),
            error: None,
        }),
    }
}

// ── Actix Serving Route ──────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct Fmp4Query {
    token: Option<String>,
}

/// GET /fmp4/{file_key}/output.mp4
///
/// Serves a pre-remuxed fragmented MP4 file. Token validation matches the
/// existing streaming server pattern.
#[actix_web::get("/fmp4/{file_key}/output.mp4")]
async fn serve_fmp4(
    _req: HttpRequest,
    path: web::Path<String>,
    query: web::Query<Fmp4Query>,
    manager: web::Data<std::sync::Arc<TranscodeManager>>,
    token_data: web::Data<StreamTokenData>,
) -> impl Responder {
    let file_key = path.into_inner();

    // Validate token
    match &query.token {
        Some(t) if t == &token_data.token => {}
        _ => return HttpResponse::Forbidden().body("Invalid or missing stream token"),
    }

    // Sanitize file_key to prevent path traversal
    if file_key
        .chars()
        .any(|c| !c.is_alphanumeric() && c != '_' && c != '-')
    {
        return HttpResponse::BadRequest().body("Invalid file key");
    }
    let account = match manager.account_for_key(&file_key) {
        Ok(account) => account,
        Err(_) => return HttpResponse::Forbidden().body("Account changed"),
    };

    // Build path and validate it stays within the cache root
    let fmp4_root = manager.cache_root.join(FMP4_DIR);
    let file_path = fmp4_root.join(&file_key).join("output.mp4");

    // Canonicalize and verify path is safe
    let safe_path = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => return HttpResponse::NotFound().body("File not found"),
    };

    let safe_root = fmp4_root
        .canonicalize()
        .unwrap_or_else(|_| fmp4_root.clone());
    if !safe_path.starts_with(&safe_root) {
        log::error!(
            "fMP4 path traversal attempt: {:?} not under {:?}",
            safe_path,
            safe_root
        );
        return HttpResponse::Forbidden().body("Access denied");
    }

    if !safe_path.exists() {
        return HttpResponse::NotFound().body("fMP4 file not found");
    }

    // Use NamedFile for automatic Range/Content-Range support and
    // streaming from disk (no full-file memory load).
    let opened = async {
        let file = tokio::fs::File::open(&safe_path).await?;
        actix_files::NamedFile::from_file(file.into_std().await, &safe_path)
    }
    .await;
    match opened {
        Ok(f) => {
            if account.validate().is_err() {
                return HttpResponse::Forbidden().body("Account changed");
            }
            f.set_content_type("video/mp4".parse().unwrap())
                .into_response(&_req)
        }
        Err(e) => {
            log::error!("Failed to open fMP4 file {:?}: {}", safe_path, e);
            HttpResponse::InternalServerError().body("Failed to read file")
        }
    }
}

/// Register fMP4 routes on the Actix service config.
pub fn configure_fmp4_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(serve_fmp4);
}
