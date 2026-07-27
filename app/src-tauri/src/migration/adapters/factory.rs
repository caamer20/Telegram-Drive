// Production adapter factory for Pipeline.
//
// Wires together:
//   - OneDriveDownloader (SourceDownloader trait)
//   - FFmpegMediaAdapter (MediaInspector + VideoProcessor traits)
//   - TelegramProductionAdapter (TelegramUploader trait)
//   - LocalProductionAdapter (LocalFinalizer trait)
//   - PipelineRunner (orchestrator)

use crate::migration::adapters::local::LocalProductionAdapter;
use crate::migration::adapters::media::FFmpegMediaAdapter;
use crate::migration::adapters::onedrive::OneDriveDownloader;
use crate::migration::adapters::telegram::TelegramProductionAdapter;
use crate::migration::db::MigrationDb;
use crate::migration::pipeline::config::PipelineConfig;
use crate::migration::pipeline::runner::PipelineRunner;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Composition root: builds all production adapters and a configured PipelineRunner.
///
/// # Arguments
/// * `db` — shared MigrationDb
/// * `ms_session` — shared Microsoft OAuth session
/// * `tg_client` — shared grammers Client (from TelegramState)
/// * `tg_peer_cache` — shared peer cache (from TelegramState)
/// * `job_id` — the migration job ID to run
/// * `workspace_dir` — temp workspace for downloads and processing
/// * `backup_dir` — final local backup destination
/// * `destination_folder_id` — Telegram destination (None = Saved Messages)
///
/// # Returns
/// A configured `PipelineRunner` (NOT started — call `.start()` separately)
/// and a cancel token.
#[allow(clippy::too_many_arguments)]
pub fn build_pipeline_services(
    db: MigrationDb,
    ms_session: Arc<tokio::sync::Mutex<Option<crate::migration::microsoft::MicrosoftSession>>>,
    tg_client: Arc<tokio::sync::Mutex<Option<grammers_client::Client>>>,
    tg_peer_cache: Arc<tokio::sync::RwLock<HashMap<i64, grammers_client::types::Peer>>>,
    job_id: i64,
    workspace_dir: PathBuf,
    backup_dir: PathBuf,
    destination_folder_id: Option<i64>,
    app_handle: Option<tauri::AppHandle>,
) -> Result<
    (
        Arc<PipelineRunner>,
        Arc<OneDriveDownloader>,
        Arc<FFmpegMediaAdapter>,
        Arc<TelegramProductionAdapter>,
        Arc<LocalProductionAdapter>,
        tokio_util::sync::CancellationToken,
    ),
    String,
> {
    let cancel_token = tokio_util::sync::CancellationToken::new();

    // Determine ffmpeg/ffprobe paths
    let ffmpeg_path = PathBuf::from(if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    });
    let ffprobe_path = PathBuf::from(if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    });

    // Build adapters
    let http_client = reqwest::Client::new();

    let downloader = Arc::new(OneDriveDownloader::new(
        http_client,
        ms_session.clone(),
        db.clone(),
        cancel_token.clone(),
        app_handle.clone(),
    ));

    let media_adapter = Arc::new(FFmpegMediaAdapter::new(
        ffprobe_path,
        ffmpeg_path,
        cancel_token.clone(),
        app_handle.clone(),
    ));

    let telegram_adapter = Arc::new(TelegramProductionAdapter::new(
        tg_client,
        tg_peer_cache,
        cancel_token.clone(),
        destination_folder_id,
        db.clone(),
        app_handle.clone(),
    ));

    let local_adapter = Arc::new(LocalProductionAdapter::new(backup_dir.clone()));

    // Build pipeline runner
    let config = PipelineConfig::default();
    let runner = Arc::new(PipelineRunner::new(
        config,
        db,
        job_id,
        workspace_dir,
        backup_dir,
        ms_session.clone(),
        cancel_token.clone(),
        destination_folder_id,
        app_handle,
    ));

    Ok((
        runner,
        downloader,
        media_adapter,
        telegram_adapter,
        local_adapter,
        cancel_token,
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
