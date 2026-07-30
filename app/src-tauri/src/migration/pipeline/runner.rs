use crate::migration::db::MigrationDb;
use crate::migration::events::{emit_item_complete, now_millis, ItemCompletePayload};

use crate::migration::pipeline::classifier::{classify_file, FileCategory};
use crate::migration::pipeline::config::PipelineConfig;
use crate::migration::pipeline::stages::{
    is_below_optimization_target, validate_canonical_output, CanonicalVideoProfile, LocalFinalizer,
    MediaInspector, PipelineItem, PipelineStage, SourceDownloader, TelegramMediaKind,
    TelegramUploadRequest, TelegramUploadResult, TelegramUploader, VideoProcessRequest,
    VideoProcessor,
};
use crate::migration::pipeline::transitions::update_item_pipeline_stage;
use crate::migration::quota_reserve::{commit_quota, release_quota, reserve_quota};
use crate::migration::telegram_idempotency::get_deterministic_random_id;
use chrono::Utc;
use futures::stream::{FuturesUnordered, StreamExt};

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

fn sanitize_path(path: &str) -> PathBuf {
    let mut safe = PathBuf::new();
    for component in std::path::Path::new(path).components() {
        if let std::path::Component::Normal(c) = component {
            let s = c.to_string_lossy().to_ascii_uppercase();
            if matches!(
                s.as_ref(),
                "CON"
                    | "PRN"
                    | "AUX"
                    | "NUL"
                    | "COM1"
                    | "COM2"
                    | "COM3"
                    | "COM4"
                    | "COM5"
                    | "COM6"
                    | "COM7"
                    | "COM8"
                    | "COM9"
                    | "LPT1"
                    | "LPT2"
                    | "LPT3"
                    | "LPT4"
                    | "LPT5"
                    | "LPT6"
                    | "LPT7"
                    | "LPT8"
                    | "LPT9"
            ) {
                safe.push(format!("{}_safe", c.to_string_lossy()));
            } else {
                safe.push(c);
            }
        }
    }
    safe
}

/// Parse flood wait seconds from Telegram error string
fn parse_flood_wait_seconds(err_str: &str) -> Option<i64> {
    if let Some(idx) = err_str.find("FLOOD_WAIT_") {
        let rest = &err_str[idx + "FLOOD_WAIT_".len()..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse::<i64>().ok()
    } else if let Some(idx) = err_str.to_ascii_lowercase().find("flood wait") {
        let digits: String = err_str[idx..]
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect();
        digits.parse::<i64>().ok()
    } else if let Some(idx) = err_str.to_ascii_lowercase().find("floodwait") {
        let digits: String = err_str[idx..]
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect();
        digits.parse::<i64>().ok()
    } else {
        None
    }
}

fn persist_item_error(db: &MigrationDb, item_id: i64, error: &str) -> Result<(), String> {
    let conn = db.lock().map_err(|value| value.to_string())?;
    let mut stmt = conn
        .prepare("UPDATE migration_items SET last_error = ?, updated_at = ? WHERE id = ?")
        .map_err(|value| value.to_string())?;
    stmt.bind((1, error)).map_err(|value| value.to_string())?;
    stmt.bind((2, now_millis()))
        .map_err(|value| value.to_string())?;
    stmt.bind((3, item_id)).map_err(|value| value.to_string())?;
    stmt.next().map_err(|value| value.to_string())?;
    Ok(())
}

fn transition_item(db: &MigrationDb, item_id: i64, stage: PipelineStage) -> Result<(), String> {
    update_item_pipeline_stage(db, item_id, stage).map_err(|error| {
        let message = format!("Pipeline transition to {:?} failed: {}", stage, error);
        if let Err(persist_error) = persist_item_error(db, item_id, &message) {
            log::error!(
                "Failed to persist transition error for item {}: {}",
                item_id,
                persist_error
            );
        }
        message
    })
}

fn reset_item_stage(db: &MigrationDb, item_id: i64, stage: PipelineStage) -> Result<(), String> {
    let conn = db.lock().map_err(|value| value.to_string())?;
    let mut stmt = conn
        .prepare(
            "UPDATE migration_items SET pipeline_stage = ?, completed_at = NULL, updated_at = ? WHERE id = ?",
        )
        .map_err(|value| value.to_string())?;
    stmt.bind((1, stage.as_str()))
        .map_err(|value| value.to_string())?;
    stmt.bind((2, now_millis()))
        .map_err(|value| value.to_string())?;
    stmt.bind((3, item_id)).map_err(|value| value.to_string())?;
    stmt.next().map_err(|value| value.to_string())?;
    Ok(())
}

pub(crate) fn artifact_is_valid_file(path: Option<&str>) -> bool {
    path.map(std::path::Path::new)
        .and_then(|path| path.metadata().ok())
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false)
}

fn expected_profile(decision: Option<&str>) -> Option<CanonicalVideoProfile> {
    match decision {
        Some("canonical_transcode_main8") => Some(CanonicalVideoProfile::Main8),
        Some("canonical_transcode_main10") => Some(CanonicalVideoProfile::Main10),
        _ => None,
    }
}

pub(crate) async fn validate_processed_artifact(
    inspector: &dyn MediaInspector,
    item: &PipelineItem,
) -> bool {
    let Some(processed_path) = item.processed_artifact_path.as_deref() else {
        return false;
    };
    if !artifact_is_valid_file(Some(processed_path)) {
        return false;
    }
    let Some(profile) = expected_profile(item.video_decision.as_deref()) else {
        return false;
    };
    let output = match inspector
        .inspect_file(std::path::Path::new(processed_path))
        .await
    {
        Ok(metadata) => metadata,
        Err(error) => {
            log::warn!(
                "Processed artifact validation failed for item {}: {}",
                item.id,
                error
            );
            return false;
        }
    };
    let source = if artifact_is_valid_file(item.local_artifact_path.as_deref()) {
        match inspector
            .inspect_file(std::path::Path::new(
                item.local_artifact_path.as_deref().unwrap_or_default(),
            ))
            .await
        {
            Ok(metadata) => metadata,
            Err(_) => output.clone(),
        }
    } else {
        output.clone()
    };
    validate_canonical_output(&source, &output, profile).is_ok()
}

pub(crate) async fn validate_passthrough_artifact(
    inspector: &dyn MediaInspector,
    item: &PipelineItem,
) -> Result<bool, String> {
    let original_path = item
        .local_artifact_path
        .as_deref()
        .ok_or_else(|| "Passthrough original artifact is missing".to_string())?;
    if !artifact_is_valid_file(Some(original_path)) {
        return Err("Passthrough original artifact is missing or empty".to_string());
    }
    let metadata = inspector
        .inspect_file(std::path::Path::new(original_path))
        .await
        .map_err(|error| format!("Passthrough FFprobe validation failed: {}", error))?;
    if item.video_decision.as_deref() == Some("size_passthrough_original") {
        return Ok(metadata.is_valid);
    }
    if item.video_decision.as_deref() == Some("quality_passthrough_original") {
        let source_size = std::fs::metadata(original_path)
            .map_err(|error| format!("Passthrough metadata failed: {}", error))?
            .len();
        return Ok(is_below_optimization_target(&metadata, source_size));
    }
    let canonical = match item.video_decision.as_deref() {
        Some("canonical_passthrough_main8") => metadata.is_canonical_main8(),
        Some("canonical_passthrough_main10") => metadata.is_canonical_main10(),
        _ => return Ok(false),
    };
    Ok(canonical && metadata.is_mp4_source(std::path::Path::new(original_path)))
}

pub(crate) fn clear_processed_checkpoint(
    db: &MigrationDb,
    item: &PipelineItem,
) -> Result<(), String> {
    if let Some(path) = item.processed_artifact_path.as_deref() {
        if let Err(error) = std::fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(format!(
                    "Failed to remove invalid processed artifact: {}",
                    error
                ));
            }
        }
    }
    let conn = db.lock().map_err(|value| value.to_string())?;
    let mut stmt = conn
        .prepare(
            "UPDATE migration_items SET processed_artifact_path = NULL, processed_sha256 = NULL WHERE id = ?",
        )
        .map_err(|value| value.to_string())?;
    stmt.bind((1, item.id)).map_err(|value| value.to_string())?;
    stmt.next().map_err(|value| value.to_string())?;
    Ok(())
}

fn emit_terminal(
    app: Option<&tauri::AppHandle>,
    item: &PipelineItem,
    phase: &str,
    status: PipelineStage,
    error_type: Option<&str>,
    error_message: Option<&str>,
) {
    if let Some(app) = app {
        emit_item_complete(
            app,
            ItemCompletePayload {
                job_id: item.job_id,
                item_id: item.id,
                item_name: item.name.clone(),
                phase: phase.to_string(),
                status: status.as_str().to_string(),
                error_type: error_type.map(ToString::to_string),
                error_message: error_message.map(ToString::to_string),
                timestamp: now_millis(),
            },
        );
    }
}

fn fail_item(
    db: &MigrationDb,
    app: Option<&tauri::AppHandle>,
    item: &PipelineItem,
    phase: &str,
    error_type: &str,
    error: &str,
) -> Result<(), String> {
    persist_item_error(db, item.id, error)?;
    transition_item(db, item.id, PipelineStage::Failed)?;
    emit_terminal(
        app,
        item,
        phase,
        PipelineStage::Failed,
        Some(error_type),
        Some(error),
    );
    Ok(())
}

fn persist_passthrough_checkpoint(
    db: &MigrationDb,
    item_id: i64,
    decision: &str,
    artifact_size: i64,
) -> Result<(), String> {
    let conn = db.lock().map_err(|value| value.to_string())?;
    let mut stmt = conn
        .prepare(
            "UPDATE migration_items SET video_decision = ?, processed_artifact_path = NULL, processed_sha256 = NULL, artifact_size = ?, last_error = NULL WHERE id = ?",
        )
        .map_err(|value| value.to_string())?;
    stmt.bind((1, decision))
        .map_err(|value| value.to_string())?;
    stmt.bind((2, artifact_size))
        .map_err(|value| value.to_string())?;
    stmt.bind((3, item_id)).map_err(|value| value.to_string())?;
    stmt.next().map_err(|value| value.to_string())?;
    Ok(())
}

fn persist_processed_checkpoint(
    db: &MigrationDb,
    item_id: i64,
    decision: &str,
    sha256: &str,
    path: &std::path::Path,
) -> Result<(), String> {
    let size = path
        .metadata()
        .map_err(|error| format!("Processed artifact metadata failed: {}", error))?
        .len() as i64;
    let conn = db.lock().map_err(|value| value.to_string())?;
    let mut stmt = conn
        .prepare(
            "UPDATE migration_items SET video_decision = ?, processed_sha256 = ?, processed_artifact_path = ?, artifact_size = ?, last_error = NULL WHERE id = ?",
        )
        .map_err(|value| value.to_string())?;
    stmt.bind((1, decision))
        .map_err(|value| value.to_string())?;
    stmt.bind((2, sha256)).map_err(|value| value.to_string())?;
    stmt.bind((3, path.to_string_lossy().as_ref()))
        .map_err(|value| value.to_string())?;
    stmt.bind((4, size)).map_err(|value| value.to_string())?;
    stmt.bind((5, item_id)).map_err(|value| value.to_string())?;
    stmt.next().map_err(|value| value.to_string())?;
    Ok(())
}

fn persist_telegram_confirmation(
    db: &MigrationDb,
    item_id: i64,
    artifact_size: i64,
    message_id: i64,
    random_id: i64,
) -> Result<(), String> {
    let conn = db.lock().map_err(|value| value.to_string())?;
    let mut stmt = conn
        .prepare(
            "UPDATE migration_items SET artifact_size = ?, telegram_message_id = ?, telegram_random_id = ?, last_error = NULL WHERE id = ?",
        )
        .map_err(|value| value.to_string())?;
    stmt.bind((1, artifact_size))
        .map_err(|value| value.to_string())?;
    stmt.bind((2, message_id))
        .map_err(|value| value.to_string())?;
    stmt.bind((3, random_id))
        .map_err(|value| value.to_string())?;
    stmt.bind((4, item_id)).map_err(|value| value.to_string())?;
    stmt.next().map_err(|value| value.to_string())?;
    Ok(())
}

fn is_cancellation_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("cancelled") || error.contains("canceled")
}

/// Trạng thái hoạt động nội bộ của Pipeline Runner
pub struct PipelineRunner {
    pub config: PipelineConfig,
    pub db: MigrationDb,
    pub job_id: i64,
    pub workspace_dir: PathBuf,
    pub backup_dir: PathBuf,
    pub cancel_token: CancellationToken,
    pub stopped_by_user: Arc<std::sync::atomic::AtomicBool>,
    pub active_tasks: std::sync::Mutex<Vec<JoinHandle<Result<(), String>>>>,
    pub ms_session: Arc<tokio::sync::Mutex<Option<crate::migration::microsoft::MicrosoftSession>>>,
    pub telegram_destination_id: Option<i64>,
    pub app_handle: Option<tauri::AppHandle>,
}

impl PipelineRunner {
    pub fn new(
        config: PipelineConfig,
        db: MigrationDb,
        job_id: i64,
        workspace_dir: PathBuf,
        backup_dir: PathBuf,
        ms_session: Arc<tokio::sync::Mutex<Option<crate::migration::microsoft::MicrosoftSession>>>,
        cancel_token: CancellationToken,
        telegram_destination_id: Option<i64>,
        app_handle: Option<tauri::AppHandle>,
    ) -> Self {
        let _ = std::fs::create_dir_all(&workspace_dir);
        let _ = std::fs::create_dir_all(&backup_dir);
        Self {
            config,
            db,
            job_id,
            workspace_dir,
            backup_dir,
            cancel_token,
            stopped_by_user: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            active_tasks: std::sync::Mutex::new(vec![]),
            ms_session,
            telegram_destination_id,
            app_handle,
        }
    }

    /// Khởi chạy toàn bộ bounded pipeline
    pub fn start(
        self: Arc<Self>,
        downloader: Arc<dyn SourceDownloader>,
        inspector: Arc<dyn MediaInspector>,
        processor: Arc<dyn VideoProcessor>,
        uploader: Arc<dyn TelegramUploader>,
        finalizer: Arc<dyn LocalFinalizer>,
    ) {
        let (download_tx, download_rx) = mpsc::channel(self.config.download_queue_capacity);
        let (process_tx, process_rx) = mpsc::channel(self.config.processing_queue_capacity);
        let (upload_tx, upload_rx) = mpsc::channel(self.config.upload_queue_capacity);
        let (local_tx, local_rx) = mpsc::channel(self.config.local_finalizer_queue_capacity);

        let _cancel = self.cancel_token.clone();

        let crawler = crate::migration::pipeline::crawler::StreamingCrawler {
            db: self.db.clone(),
            job_id: self.job_id,
            ms_session: self.ms_session.clone(),
            cancel_token: self.cancel_token.clone(),
            config: self.config.clone(),
        };
        let crawler_arc = Arc::new(crawler);

        let runner_for_loader = self.clone();
        let inspector_for_recovery = inspector.clone();
        let download_tx_loader = download_tx.clone();
        let process_tx_loader = process_tx.clone();
        let upload_tx_loader = upload_tx.clone();
        let local_tx_loader = local_tx.clone();

        let planner_handle: JoinHandle<Result<(), String>> = tokio::spawn(async move {
            runner_for_loader
                .dispatch_recovering_items(
                    download_tx_loader,
                    process_tx_loader,
                    upload_tx_loader,
                    local_tx_loader,
                    inspector_for_recovery,
                )
                .await?;
            crawler_arc.run(download_tx).await
        });

        // 2. Task Downloader (tải tệp từ nguồn)
        let runner_clone = self.clone();
        let upload_tx_clone = upload_tx.clone();
        let download_handle: JoinHandle<Result<(), String>> = tokio::spawn(async move {
            runner_clone
                .run_downloader(
                    download_rx,
                    process_tx,
                    upload_tx_clone,
                    local_tx,
                    downloader,
                )
                .await
        });
        // 3. Task Processor (inspect và xử lý video bằng ffmpeg)
        let runner_clone = self.clone();
        let process_handle: JoinHandle<Result<(), String>> = tokio::spawn(async move {
            runner_clone
                .run_processor(process_rx, upload_tx.clone(), inspector, processor)
                .await
        });

        // 4. Task Uploader (tải tệp lên Telegram)
        let runner_clone = self.clone();
        let upload_handle: JoinHandle<Result<(), String>> =
            tokio::spawn(async move { runner_clone.run_uploader(upload_rx, uploader).await });

        // 5. Task Local Finalizer (di chuyển tệp backup local)
        let runner_clone = self.clone();
        let local_handle: JoinHandle<Result<(), String>> =
            tokio::spawn(
                async move { runner_clone.run_local_finalizer(local_rx, finalizer).await },
            );

        let mut guard = self.active_tasks.lock().unwrap();
        guard.push(planner_handle);
        guard.push(download_handle);
        guard.push(process_handle);
        guard.push(upload_handle);
        guard.push(local_handle);
    }

    pub async fn run_to_completion(&self) -> Result<(), String> {
        // Drain active tasks (5 main loop tasks)
        let main_tasks: Vec<JoinHandle<Result<(), String>>> = {
            let mut guard = self.active_tasks.lock().unwrap();
            std::mem::take(&mut *guard)
        };

        let mut tasks: FuturesUnordered<_> = main_tasks.into_iter().collect();
        let mut fatal_error: Option<String> = None;
        while let Some(result) = tasks.next().await {
            match result {
                Ok(Ok(())) => {} // Task completed normally
                Ok(Err(e)) => {
                    if self
                        .stopped_by_user
                        .load(std::sync::atomic::Ordering::Relaxed)
                        && is_cancellation_error(&e)
                    {
                        continue;
                    }
                    let err_msg = format!("Worker error: {}", e);
                    log::error!("Pipeline task error: {}", err_msg);
                    if fatal_error.is_none() {
                        fatal_error = Some(err_msg);
                    }
                    // Cancel other tasks
                    self.cancel_token.cancel();
                }
                Err(e) => {
                    if self
                        .stopped_by_user
                        .load(std::sync::atomic::Ordering::Relaxed)
                        && e.is_cancelled()
                    {
                        continue;
                    }
                    let err_msg = if e.is_panic() {
                        format!("Worker panicked: {:?}", e)
                    } else {
                        format!("Worker cancelled: {}", e)
                    };
                    log::error!("Pipeline task error: {}", err_msg);
                    if fatal_error.is_none() {
                        fatal_error = Some(err_msg);
                    }
                    self.cancel_token.cancel();
                }
            }
        }

        if let Some(err) = fatal_error {
            let conn = self.db.lock().map_err(|e| e.to_string())?;
            let mut upd = conn
                .prepare(
                    "UPDATE migration_jobs SET state = 'failed', last_error = ?, completed_at = ?, updated_at = ? WHERE id = ?",
                )
                .map_err(|e| e.to_string())?;
            let now = now_millis();
            upd.bind((1, err.as_str())).map_err(|e| e.to_string())?;
            upd.bind((2, now)).map_err(|e| e.to_string())?;
            upd.bind((3, now)).map_err(|e| e.to_string())?;
            upd.bind((4, self.job_id)).map_err(|e| e.to_string())?;
            upd.next().map_err(|e| e.to_string())?;
            return Err(err);
        }

        // Finalize job state
        self.finalize_job().await
    }

    /// Finalize job state based on item outcomes
    async fn finalize_job(&self) -> Result<(), String> {
        let conn = self.db.lock().map_err(|e| e.to_string())?;

        // Count items by stage
        let mut stmt = conn.prepare(
            "SELECT pipeline_stage, COUNT(*) FROM migration_items WHERE job_id = ? GROUP BY pipeline_stage"
        ).map_err(|e| e.to_string())?;
        stmt.bind((1, self.job_id)).map_err(|e| e.to_string())?;

        let mut failed_count: i64 = 0;
        let mut completed_telegram: i64 = 0;
        let mut completed_local: i64 = 0;
        let mut waiting_quota: i64 = 0;
        let mut reconciliation: i64 = 0;
        let mut pending_count: i64 = 0;

        while let Ok(sqlite::State::Row) = stmt.next() {
            let stage: String = stmt.read(0).unwrap_or_default();
            let count: i64 = stmt.read(1).unwrap_or(0);
            match stage.as_str() {
                "completed_telegram" => completed_telegram = count,
                "completed_local" => completed_local = count,
                "failed" => failed_count = count,
                "waiting_for_quota" => waiting_quota = count,
                "reconciliation_required" => reconciliation = count,
                _ => pending_count += count,
            }
        }

        drop(stmt);
        let (failed_folders, failed_root_folders, folder_error) = {
            let mut folder_stmt = conn
                .prepare(
                    "SELECT COUNT(*), \
                            COALESCE(SUM(CASE WHEN parent_id IS NULL THEN 1 ELSE 0 END), 0), \
                            GROUP_CONCAT(COALESCE(last_error, folder_path), '; ') \
                     FROM folder_queue WHERE job_id = ? AND state = 'failed'",
                )
                .map_err(|error| error.to_string())?;
            folder_stmt
                .bind((1, self.job_id))
                .map_err(|error| error.to_string())?;
            if let Ok(sqlite::State::Row) = folder_stmt.next() {
                (
                    folder_stmt.read::<i64, _>(0).unwrap_or(0),
                    folder_stmt.read::<i64, _>(1).unwrap_or(0),
                    folder_stmt.read::<Option<String>, _>(2).ok().flatten(),
                )
            } else {
                (0, 0, None)
            }
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let stopped_by_user = self
            .stopped_by_user
            .load(std::sync::atomic::Ordering::Relaxed);
        let invariant_error = if pending_count > 0
            && !stopped_by_user
            && !(waiting_quota > 0 && pending_count == 0)
        {
            Some("Pipeline drained with non-terminal items remaining")
        } else {
            None
        };

        let job_state = if stopped_by_user {
            "stopped"
        } else if failed_root_folders > 0 {
            "failed"
        } else if waiting_quota > 0 && pending_count == 0 {
            "waiting_for_quota"
        } else if invariant_error.is_some() {
            "failed"
        } else if failed_count > 0 || reconciliation > 0 || failed_folders > 0 {
            "completed_with_errors"
        } else if pending_count == 0 {
            "completed"
        } else {
            "failed"
        };

        let mut upd = conn.prepare(
            "UPDATE migration_jobs SET state = ?, completed_items = ?, failed_items = ?, waiting_items = ?, completed_at = ?, updated_at = ?, last_error = ? WHERE id = ?"
        ).map_err(|e| e.to_string())?;
        upd.bind((1, job_state)).map_err(|e| e.to_string())?;
        upd.bind((2, completed_telegram + completed_local))
            .map_err(|e| e.to_string())?;
        upd.bind((3, failed_count)).map_err(|e| e.to_string())?;
        upd.bind((4, waiting_quota + reconciliation))
            .map_err(|e| e.to_string())?;
        let completed_at = if matches!(job_state, "completed" | "completed_with_errors" | "failed")
        {
            Some(now)
        } else {
            None
        };
        upd.bind((5, completed_at)).map_err(|e| e.to_string())?;
        upd.bind((6, now)).map_err(|e| e.to_string())?;
        let final_error = invariant_error
            .map(ToString::to_string)
            .or_else(|| folder_error.map(|error| format!("Folder crawl failure: {}", error)));
        upd.bind((7, final_error.as_deref()))
            .map_err(|e| e.to_string())?;
        upd.bind((8, self.job_id)).map_err(|e| e.to_string())?;
        upd.next().map_err(|e| e.to_string())?;

        Ok(())
    }

    async fn dispatch_recovering_items(
        &self,
        download_tx: mpsc::Sender<PipelineItem>,
        process_tx: mpsc::Sender<PipelineItem>,
        upload_tx: mpsc::Sender<PipelineItem>,
        local_tx: mpsc::Sender<PipelineItem>,
        inspector: Arc<dyn MediaInspector>,
    ) -> Result<(), String> {
        // Recover all non-terminal items on pipeline start/restart
        let terminal_stages =
            "'completed_telegram', 'completed_local', 'failed', 'reconciliation_required'";
        let items = {
            let conn = self.db.lock().map_err(|e| e.to_string())?;
            let query = format!(
                "SELECT id, job_id, name, path, source_item_id, size, item_category, \
                 original_sha256, processed_sha256, video_decision, pipeline_stage, \
                 original_artifact_path, processed_artifact_path, retry_count \
                 FROM migration_items \
                 WHERE job_id = ? AND pipeline_stage NOT IN ({})",
                terminal_stages
            );
            let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;

            stmt.bind((1, self.job_id)).map_err(|e| e.to_string())?;

            let mut items = Vec::new();
            while let Ok(sqlite::State::Row) = stmt.next() {
                let id: i64 = stmt.read(0).unwrap();
                let job_id: i64 = stmt.read(1).unwrap();
                let name: String = stmt.read(2).unwrap();
                let path: String = stmt.read(3).unwrap();
                let source_item_id: String = stmt.read(4).unwrap();
                let size_bytes: i64 = stmt.read(5).unwrap();
                let _category: String = stmt.read(6).unwrap_or_default();
                let original_sha256: Option<String> = stmt.read(7).unwrap_or_default();
                let processed_sha256: Option<String> = stmt.read(8).unwrap_or_default();
                let video_decision: Option<String> = stmt.read(9).unwrap_or_default();
                let state: String = stmt.read(10).unwrap();
                let original_path: Option<String> = stmt.read(11).unwrap_or_default();
                let processed_path: Option<String> = stmt.read(12).unwrap_or_default();
                let retry_count: i64 = stmt.read(13).unwrap_or(0);

                items.push(PipelineItem {
                    id,
                    job_id,
                    name,
                    source_path: path,
                    source_item_id: Some(source_item_id),
                    size_bytes,
                    source_etag: None,
                    source_last_modified: None,
                    source_fingerprint_type: None,
                    source_fingerprint_value: None,
                    state: state.clone(),
                    original_sha256,
                    processed_sha256,
                    local_artifact_path: original_path,
                    processed_artifact_path: processed_path,
                    telegram_random_id: None,
                    video_decision,
                    retry_count,
                });
            }
            items
        };

        for mut item in items {
            if self.cancel_token.is_cancelled() {
                break;
            }
            let stage = item.state.as_str();
            match stage {
                // Items that need (re-)download
                "discovered" | "queued_download" | "downloading" => {
                    if stage != "queued_download" {
                        if stage == "downloading" {
                            transition_item(&self.db, item.id, PipelineStage::QueuedDownload)?;
                        } else {
                            transition_item(&self.db, item.id, PipelineStage::QueuedDownload)?;
                        }
                    }
                    download_tx
                        .send(item)
                        .await
                        .map_err(|_| "Recovery download queue closed".to_string())?;
                }
                // Items already downloaded — verify and route
                "downloaded" | "queued_processing" | "processing" | "processed"
                | "queued_upload" | "uploading" => {
                    let category = classify_file(&item.name);

                    let processed_valid = matches!(category, FileCategory::Video)
                        && validate_processed_artifact(inspector.as_ref(), &item).await;
                    let original_valid =
                        artifact_is_valid_file(item.local_artifact_path.as_deref());
                    let is_passthrough_checkpoint = item
                        .video_decision
                        .as_deref()
                        .map(|decision| {
                            decision.starts_with("canonical_passthrough")
                                || decision == "size_passthrough_original"
                                || decision == "quality_passthrough_original"
                        })
                        .unwrap_or(false);
                    let passthrough_validation = if is_passthrough_checkpoint {
                        Some(validate_passthrough_artifact(inspector.as_ref(), &item).await)
                    } else {
                        None
                    };
                    let passthrough_valid =
                        matches!(passthrough_validation.as_ref(), Some(Ok(true)));
                    let passthrough_unreadable =
                        matches!(passthrough_validation.as_ref(), Some(Err(_)));

                    if processed_valid || passthrough_valid {
                        match stage {
                            "downloaded" => {
                                transition_item(
                                    &self.db,
                                    item.id,
                                    PipelineStage::QueuedProcessing,
                                )?;
                                transition_item(&self.db, item.id, PipelineStage::Processing)?;
                                transition_item(&self.db, item.id, PipelineStage::Processed)?;
                                transition_item(&self.db, item.id, PipelineStage::QueuedUpload)?;
                            }
                            "queued_processing" => {
                                transition_item(&self.db, item.id, PipelineStage::Processing)?;
                                transition_item(&self.db, item.id, PipelineStage::Processed)?;
                                transition_item(&self.db, item.id, PipelineStage::QueuedUpload)?;
                            }
                            "processing" => {
                                transition_item(&self.db, item.id, PipelineStage::Processed)?;
                                transition_item(&self.db, item.id, PipelineStage::QueuedUpload)?;
                            }
                            "processed" => {
                                transition_item(&self.db, item.id, PipelineStage::QueuedUpload)?;
                            }
                            "uploading" => {
                                transition_item(&self.db, item.id, PipelineStage::QueuedUpload)?;
                            }
                            "queued_upload" => {}
                            _ => reset_item_stage(&self.db, item.id, PipelineStage::QueuedUpload)?,
                        }
                        upload_tx
                            .send(item)
                            .await
                            .map_err(|_| "Recovery upload queue closed".to_string())?;
                    } else if passthrough_unreadable {
                        clear_processed_checkpoint(&self.db, &item)?;
                        reset_item_stage(&self.db, item.id, PipelineStage::QueuedDownload)?;
                        download_tx
                            .send(item)
                            .await
                            .map_err(|_| "Recovery download queue closed".to_string())?;
                    } else if original_valid {
                        // Has original file — route based on category
                        match category {
                            FileCategory::Video => {
                                // Clear stale processed path
                                clear_processed_checkpoint(&self.db, &item)?;
                                item.processed_artifact_path = None;
                                item.processed_sha256 = None;
                                reset_item_stage(
                                    &self.db,
                                    item.id,
                                    PipelineStage::QueuedProcessing,
                                )?;
                                process_tx
                                    .send(item)
                                    .await
                                    .map_err(|_| "Recovery processing queue closed".to_string())?;
                            }
                            FileCategory::Image => {
                                reset_item_stage(&self.db, item.id, PipelineStage::QueuedUpload)?;
                                upload_tx
                                    .send(item)
                                    .await
                                    .map_err(|_| "Recovery upload queue closed".to_string())?;
                            }
                            FileCategory::Other => {
                                reset_item_stage(&self.db, item.id, PipelineStage::SavingLocal)?;
                                local_tx
                                    .send(item)
                                    .await
                                    .map_err(|_| "Recovery local queue closed".to_string())?;
                            }
                        }
                    } else {
                        // No artifacts — re-download
                        clear_processed_checkpoint(&self.db, &item)?;
                        reset_item_stage(&self.db, item.id, PipelineStage::QueuedDownload)?;
                        download_tx
                            .send(item)
                            .await
                            .map_err(|_| "Recovery download queue closed".to_string())?;
                    }
                }
                // Items needing local finalization
                "saving_local" => {
                    let original_valid = item
                        .local_artifact_path
                        .as_deref()
                        .map(|p| {
                            let path = std::path::Path::new(p);
                            path.exists()
                                && path.is_file()
                                && path.metadata().map(|m| m.len() > 0).unwrap_or(false)
                        })
                        .unwrap_or(false);
                    if original_valid {
                        local_tx
                            .send(item)
                            .await
                            .map_err(|_| "Recovery local queue closed".to_string())?;
                    } else {
                        reset_item_stage(&self.db, item.id, PipelineStage::QueuedDownload)?;
                        download_tx
                            .send(item)
                            .await
                            .map_err(|_| "Recovery download queue closed".to_string())?;
                    }
                }
                // Re-check quota/flood-wait when a stopped pipeline is resumed.
                "waiting_for_quota" => {
                    let flood_wait_until = {
                        let conn = self.db.lock().map_err(|error| error.to_string())?;
                        let mut stmt = conn
                            .prepare("SELECT flood_wait_until FROM migration_jobs WHERE id = ?")
                            .map_err(|error| error.to_string())?;
                        stmt.bind((1, item.job_id))
                            .map_err(|error| error.to_string())?;
                        if let Ok(sqlite::State::Row) = stmt.next() {
                            stmt.read::<i64, _>(0).unwrap_or(0)
                        } else {
                            0
                        }
                    };
                    if flood_wait_until <= Utc::now().timestamp() {
                        transition_item(&self.db, item.id, PipelineStage::QueuedUpload)?;
                        upload_tx
                            .send(item)
                            .await
                            .map_err(|_| "Recovery upload queue closed".to_string())?;
                    }
                }
                // Reconciliation items — terminal, do not re-queue to upload
                "reconciliation_required" => {
                    log::info!(
                        "Recovery: item {} is reconciliation_required — skipping",
                        item.id
                    );
                }
                _ => {
                    log::warn!(
                        "Recovery: unknown stage '{}' for item {} — queuing for download",
                        stage,
                        item.id
                    );
                    reset_item_stage(&self.db, item.id, PipelineStage::QueuedDownload)?;
                    download_tx
                        .send(item)
                        .await
                        .map_err(|_| "Recovery download queue closed".to_string())?;
                }
            }
        }
        Ok(())
    }

    /// Check if job has any non-terminal items remaining
    pub fn has_pending_items(&self) -> bool {
        let conn = match self.db.lock() {
            Ok(c) => c,
            Err(_) => return true, // assume pending on error
        };
        let mut stmt = match conn.prepare(
            "SELECT COUNT(*) FROM migration_items WHERE job_id = ? AND pipeline_stage NOT IN ('completed_telegram', 'completed_local', 'failed', 'reconciliation_required');"
        ) {
            Ok(s) => s,
            Err(_) => return true,
        };
        stmt.bind((1, self.job_id)).ok();
        if let Ok(sqlite::State::Row) = stmt.next() {
            let count: i64 = stmt.read(0).unwrap_or(1);
            count > 0
        } else {
            true
        }
    }

    /// Task Downloader tải tệp và định tuyến
    async fn run_downloader(
        &self,
        rx: mpsc::Receiver<PipelineItem>,
        process_tx: mpsc::Sender<PipelineItem>,
        upload_tx: mpsc::Sender<PipelineItem>,
        local_tx: mpsc::Sender<PipelineItem>,
        downloader: Arc<dyn SourceDownloader>,
    ) -> Result<(), String> {
        let rx = Arc::new(tokio::sync::Mutex::new(rx));
        let mut workers = Vec::new();
        log::debug!(
            "Downloader loop started with {} workers",
            self.config.download_concurrency
        );

        for _ in 0..self.config.download_concurrency {
            let rx_clone = rx.clone();
            let db_clone = self.db.clone();
            let downloader_clone = downloader.clone();
            let process_tx_clone = process_tx.clone();
            let upload_tx_clone = upload_tx.clone();
            let local_tx_clone = local_tx.clone();
            let workspace = self.workspace_dir.clone();
            let cancel = self.cancel_token.clone();
            let app_handle = self.app_handle.clone();

            let handle = tokio::spawn(async move {
                loop {
                    let item_opt = tokio::select! {
                        _ = cancel.cancelled() => break,
                        item = async {
                            let mut guard = rx_clone.lock().await;
                            guard.recv().await
                        } => item,
                    };
                    let mut item = match item_opt {
                        Some(i) => i,
                        None => break, // Channel closed
                    };

                    log::debug!("Downloader task active for item {}", item.id);

                    transition_item(&db_clone, item.id, PipelineStage::Downloading)?;

                    let category = classify_file(&item.name);

                    // 3. Thực hiện tải (ghi tệp tạm .part rồi rename)
                    let part_path = workspace.join(format!("{}.part", item.id));
                    let final_path = workspace.join(format!("{}", item.id));

                    if let Some(source_id) = &item.source_item_id {
                        log::debug!("Downloader starting download_file for item {}", item.id);
                        match downloader_clone
                            .download_file(item.id, source_id, &part_path)
                            .await
                        {
                            Ok(sha256) => {
                                log::debug!(
                                    "Downloader download_file OK for item {}, sha256={}",
                                    item.id,
                                    sha256
                                );
                                // Đảm bảo tạo directory đích
                                if let Some(parent) = final_path.parent() {
                                    std::fs::create_dir_all(parent).map_err(|error| {
                                        format!("Downloader cannot create workspace: {}", error)
                                    })?;
                                }
                                // Atomic Rename
                                if let Err(e) = std::fs::rename(&part_path, &final_path) {
                                    log::error!(
                                        "Downloader rename failed for item {}: {}",
                                        item.id,
                                        e
                                    );
                                    fail_item(
                                        &db_clone,
                                        app_handle.as_ref(),
                                        &item,
                                        "downloading",
                                        "filesystem_error",
                                        &format!("Atomic rename failed: {}", e),
                                    )?;
                                } else {
                                    log::debug!("Downloader rename OK for item {}", item.id);
                                    item.original_sha256 = Some(sha256.clone());
                                    item.local_artifact_path =
                                        Some(final_path.to_string_lossy().into_owned());
                                    // Update original hash + path + size in DB
                                    {
                                        let conn =
                                            db_clone.lock().map_err(|value| value.to_string())?;
                                        let file_size = std::fs::metadata(&final_path)
                                            .map_err(|error| error.to_string())?
                                            .len()
                                            as i64;
                                        let mut upd_hash = conn.prepare(
                                        "UPDATE migration_items SET original_sha256 = ?, original_artifact_path = ?, artifact_size = ? WHERE id = ?;"
                                    ).map_err(|value| value.to_string())?;
                                        upd_hash
                                            .bind((1, sha256.as_str()))
                                            .map_err(|value| value.to_string())?;
                                        upd_hash
                                            .bind((2, final_path.to_str().unwrap_or_default()))
                                            .map_err(|value| value.to_string())?;
                                        upd_hash
                                            .bind((3, file_size))
                                            .map_err(|value| value.to_string())?;
                                        upd_hash
                                            .bind((4, item.id))
                                            .map_err(|value| value.to_string())?;
                                        upd_hash.next().map_err(|value| value.to_string())?;
                                    }

                                    transition_item(&db_clone, item.id, PipelineStage::Downloaded)?;

                                    // 4. Giải phóng disk reservation
                                    {
                                        let _conn = db_clone.lock().unwrap();
                                    }

                                    log::debug!(
                                        "Downloader routing item {} as {:?}",
                                        item.id,
                                        category
                                    );

                                    // 5. Định tuyến (routing)
                                    match category {
                                        FileCategory::Video => {
                                            transition_item(
                                                &db_clone,
                                                item.id,
                                                PipelineStage::QueuedProcessing,
                                            )?;
                                            process_tx_clone.send(item).await.map_err(|_| {
                                                "Downloader processing queue closed".to_string()
                                            })?;
                                        }
                                        FileCategory::Image => {
                                            transition_item(
                                                &db_clone,
                                                item.id,
                                                PipelineStage::QueuedUpload,
                                            )?;
                                            upload_tx_clone.send(item).await.map_err(|_| {
                                                "Downloader upload queue closed".to_string()
                                            })?;
                                        }
                                        FileCategory::Other => {
                                            transition_item(
                                                &db_clone,
                                                item.id,
                                                PipelineStage::SavingLocal,
                                            )?;
                                            local_tx_clone.send(item).await.map_err(|_| {
                                                "Downloader local queue closed".to_string()
                                            })?;
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                log::debug!(
                                    "Downloader download_file Err for item {}: {}",
                                    item.id,
                                    e
                                );
                                if let Err(remove_error) = std::fs::remove_file(&part_path) {
                                    if remove_error.kind() != std::io::ErrorKind::NotFound {
                                        log::warn!(
                                            "Failed to remove partial download: {}",
                                            remove_error
                                        );
                                    }
                                }
                                if cancel.is_cancelled() {
                                    transition_item(
                                        &db_clone,
                                        item.id,
                                        PipelineStage::QueuedDownload,
                                    )?;
                                    break;
                                }
                                fail_item(
                                    &db_clone,
                                    app_handle.as_ref(),
                                    &item,
                                    "downloading",
                                    "download_error",
                                    &e,
                                )?;
                            }
                        }
                    } else {
                        log::error!("Downloader source_item_id is None for item {}", item.id);
                        fail_item(
                            &db_clone,
                            app_handle.as_ref(),
                            &item,
                            "downloading",
                            "source_error",
                            "Source item ID is missing",
                        )?;
                    }
                }
                Ok::<(), String>(())
            });
            workers.push(handle);
        }

        for w in workers {
            match w.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => return Err(error),
                Err(error) => {
                    let err_msg = if error.is_panic() {
                        format!("Downloader worker panicked: {:?}", error)
                    } else {
                        format!("Downloader worker cancelled: {}", error)
                    };
                    log::error!("{}", err_msg);
                    return Err(err_msg);
                }
            }
        }
        Ok(())
    }
    async fn run_processor(
        &self,
        rx: mpsc::Receiver<PipelineItem>,
        upload_tx: mpsc::Sender<PipelineItem>,
        inspector: Arc<dyn MediaInspector>,
        processor: Arc<dyn VideoProcessor>,
    ) -> Result<(), String> {
        let rx = Arc::new(tokio::sync::Mutex::new(rx));
        let mut workers = Vec::new();

        for _ in 0..self.config.processing_concurrency {
            let rx_clone = rx.clone();
            let db_clone = self.db.clone();
            let inspector_clone = inspector.clone();
            let processor_clone = processor.clone();
            let upload_tx_clone = upload_tx.clone();
            let workspace = self.workspace_dir.clone();
            let cancel = self.cancel_token.clone();
            let app_handle = self.app_handle.clone();

            let handle = tokio::spawn(async move {
                loop {
                    let item_opt = tokio::select! {
                        _ = cancel.cancelled() => break,
                        item = async {
                            let mut guard = rx_clone.lock().await;
                            guard.recv().await
                        } => item,
                    };
                    let mut item = match item_opt {
                        Some(i) => i,
                        None => break,
                    };

                    transition_item(&db_clone, item.id, PipelineStage::Processing)?;

                    let input_path = match &item.local_artifact_path {
                        Some(p) => std::path::PathBuf::from(p),
                        None => workspace.join(format!("{}", item.id)),
                    };
                    let output_path = workspace.join(format!("{}.processed.mp4", item.id));

                    // Inspect video
                    match inspector_clone.inspect_file(&input_path).await {
                        Ok(meta) => {
                            let original_size = input_path
                                .metadata()
                                .map_err(|error| {
                                    format!("Original artifact metadata failed: {}", error)
                                })?
                                .len();
                            // Determine decision using VideoMetadata helpers
                            let decision = if is_below_optimization_target(&meta, original_size) {
                                "quality_passthrough_original"
                            } else if meta.is_hdr() || meta.is_10bit() {
                                if meta.is_canonical_main10() && meta.is_mp4_source(&input_path) {
                                    "canonical_passthrough_main10"
                                } else {
                                    "canonical_transcode_main10"
                                }
                            } else {
                                if meta.is_canonical_main8() && meta.is_mp4_source(&input_path) {
                                    "canonical_passthrough_main8"
                                } else {
                                    "canonical_transcode_main8"
                                }
                            };

                            item.video_decision = Some(decision.to_string());

                            if decision.starts_with("canonical_passthrough")
                                || decision == "quality_passthrough_original"
                            {
                                // Passthrough: use original artifact
                                item.processed_artifact_path = None;
                                item.processed_sha256 = None;
                                persist_passthrough_checkpoint(
                                    &db_clone,
                                    item.id,
                                    decision,
                                    original_size as i64,
                                )?;
                                transition_item(&db_clone, item.id, PipelineStage::Processed)?;
                                transition_item(&db_clone, item.id, PipelineStage::QueuedUpload)?;
                                upload_tx_clone
                                    .send(item)
                                    .await
                                    .map_err(|_| "Processor upload queue closed".to_string())?;
                            } else {
                                {
                                    let conn =
                                        db_clone.lock().map_err(|value| value.to_string())?;
                                    let mut stmt = conn
                                        .prepare("UPDATE migration_items SET video_decision = ? WHERE id = ?")
                                        .map_err(|value| value.to_string())?;
                                    stmt.bind((1, decision))
                                        .map_err(|value| value.to_string())?;
                                    stmt.bind((2, item.id)).map_err(|value| value.to_string())?;
                                    stmt.next().map_err(|value| value.to_string())?;
                                }
                                // Gọi FFmpeg processor
                                match processor_clone
                                    .process_video(VideoProcessRequest {
                                        input_path: input_path.clone(),
                                        output_path: output_path.clone(),
                                        decision: decision.to_string(),
                                        item_id: item.id,
                                        job_id: item.job_id,
                                        item_name: item.name.clone(),
                                        metadata: meta.clone(),
                                    })
                                    .await
                                {
                                    Ok(proc_hash) => {
                                        let original_size = input_path
                                            .metadata()
                                            .map_err(|error| {
                                                format!(
                                                    "Original artifact metadata failed: {}",
                                                    error
                                                )
                                            })?
                                            .len();
                                        let processed_size = output_path
                                            .metadata()
                                            .map_err(|error| {
                                                format!(
                                                    "Processed artifact metadata failed: {}",
                                                    error
                                                )
                                            })?
                                            .len();
                                        if processed_size >= original_size {
                                            std::fs::remove_file(&output_path).map_err(|error| {
                                                format!(
                                                    "Failed to discard oversized processed artifact: {}",
                                                    error
                                                )
                                            })?;
                                            let size_decision = "size_passthrough_original";
                                            item.video_decision = Some(size_decision.to_string());
                                            item.processed_sha256 = None;
                                            item.processed_artifact_path = None;
                                            persist_passthrough_checkpoint(
                                                &db_clone,
                                                item.id,
                                                size_decision,
                                                original_size as i64,
                                            )?;
                                        } else {
                                            item.processed_sha256 = Some(proc_hash.clone());
                                            item.processed_artifact_path =
                                                Some(output_path.to_string_lossy().into_owned());
                                            persist_processed_checkpoint(
                                                &db_clone,
                                                item.id,
                                                decision,
                                                &proc_hash,
                                                &output_path,
                                            )?;
                                        }
                                        transition_item(
                                            &db_clone,
                                            item.id,
                                            PipelineStage::Processed,
                                        )?;
                                        transition_item(
                                            &db_clone,
                                            item.id,
                                            PipelineStage::QueuedUpload,
                                        )?;
                                        upload_tx_clone.send(item).await.map_err(|_| {
                                            "Processor upload queue closed".to_string()
                                        })?;
                                    }
                                    Err(e) => {
                                        let _ = std::fs::remove_file(&output_path);
                                        if cancel.is_cancelled() {
                                            transition_item(
                                                &db_clone,
                                                item.id,
                                                PipelineStage::QueuedProcessing,
                                            )?;
                                            break;
                                        }
                                        log::error!(
                                            "Processor: transcode failed for item {}: {}",
                                            item.id,
                                            e
                                        );
                                        fail_item(
                                            &db_clone,
                                            app_handle.as_ref(),
                                            &item,
                                            "processing",
                                            "processing_error",
                                            &e,
                                        )?;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            if cancel.is_cancelled() {
                                transition_item(
                                    &db_clone,
                                    item.id,
                                    PipelineStage::QueuedProcessing,
                                )?;
                                break;
                            }
                            log::error!("Processor: inspect failed for item {}: {}", item.id, e);
                            fail_item(
                                &db_clone,
                                app_handle.as_ref(),
                                &item,
                                "processing",
                                "inspection_error",
                                &e,
                            )?;
                        }
                    }
                }
                Ok::<(), String>(())
            });
            workers.push(handle);
        }

        for w in workers {
            match w.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    log::error!("Processor worker error: {}", e);
                    return Err(e);
                }
                Err(e) => {
                    let err_msg = if e.is_panic() {
                        format!("Processor worker panicked: {:?}", e)
                    } else {
                        format!("Processor worker cancelled: {}", e)
                    };
                    log::error!("{}", err_msg);
                    return Err(err_msg);
                }
            }
        }
        Ok(())
    }

    /// Task Uploader upload tệp tin lên Telegram
    async fn run_uploader(
        &self,
        rx: mpsc::Receiver<PipelineItem>,
        uploader: Arc<dyn TelegramUploader>,
    ) -> Result<(), String> {
        let rx = Arc::new(tokio::sync::Mutex::new(rx));
        let mut workers = Vec::new();
        let destination_id = self.telegram_destination_id;

        for _ in 0..self.config.upload_concurrency {
            let rx_clone = rx.clone();
            let db_clone = self.db.clone();
            let uploader_clone = uploader.clone();
            let cancel = self.cancel_token.clone();
            let app_handle = self.app_handle.clone();

            let handle: JoinHandle<Result<(), String>> = tokio::spawn(async move {
                loop {
                    let item_opt = tokio::select! {
                        _ = cancel.cancelled() => break,
                        item = async {
                            let mut guard = rx_clone.lock().await;
                            guard.recv().await
                        } => item,
                    };
                    let item = match item_opt {
                        Some(i) => i,
                        None => break,
                    };

                    log::debug!("Uploader loop received item {}", item.id);

                    // Enforce flood wait check before uploading
                    let wait_until = {
                        let conn = db_clone.lock().map_err(|value| value.to_string())?;
                        let mut stmt = conn
                            .prepare(
                                "SELECT flood_wait_until FROM migration_jobs WHERE id = ? LIMIT 1",
                            )
                            .map_err(|value| value.to_string())?;
                        stmt.bind((1, item.job_id))
                            .map_err(|value| value.to_string())?;
                        if let Ok(sqlite::State::Row) = stmt.next() {
                            stmt.read::<i64, _>(0).unwrap_or(0)
                        } else {
                            0
                        }
                    };

                    let now = Utc::now().timestamp();
                    if wait_until > now {
                        log::info!("Upload: Job {} is under FloodWait until {} — marking item {} waiting_for_quota", item.job_id, wait_until, item.id);
                        persist_item_error(
                            &db_clone,
                            item.id,
                            &format!("Flood wait active until {}", wait_until),
                        )?;
                        transition_item(&db_clone, item.id, PipelineStage::WaitingForQuota)?;
                        // Do NOT spawn a sleep task — item stays in DB as waiting_for_quota
                        // Pipeline will end when all items are terminal/waiting
                        continue;
                    }

                    log::debug!("Uploader spawned task active for item {}", item.id);

                    // Artifact selection based on video_decision
                    let file_path = Self::select_artifact_path(&item);
                    let artifact_size = match std::fs::metadata(&file_path) {
                        Ok(m) => m.len() as i64,
                        Err(error) => {
                            log::error!(
                                "Upload: artifact missing for item {} at {:?}",
                                item.id,
                                file_path
                            );
                            fail_item(
                                &db_clone,
                                app_handle.as_ref(),
                                &item,
                                "uploading",
                                "artifact_missing",
                                &format!("Artifact file missing: {}", error),
                            )?;
                            continue;
                        }
                    };

                    // Quota check — atomic reserve before upload
                    let date_string = Utc::now().format("%Y-%m-%d").to_string();
                    let reserved = {
                        let conn = db_clone.lock().map_err(|value| value.to_string())?;
                        reserve_quota(
                            &conn,
                            item.id,
                            item.job_id,
                            &date_string,
                            artifact_size,
                            7200,
                        )
                    };

                    match reserved {
                        Ok(_) => {}
                        Err(e) => {
                            log::warn!("Upload: quota reserve failed for item {}: {} — marking waiting_for_quota", item.id, e);
                            persist_item_error(&db_clone, item.id, &e)?;
                            transition_item(&db_clone, item.id, PipelineStage::WaitingForQuota)?;
                            // Do NOT spawn a sleep task
                            continue;
                        }
                    }

                    if cancel.is_cancelled() {
                        let conn = db_clone.lock().map_err(|value| value.to_string())?;
                        if let Err(error) = release_quota(&conn, item.id, &date_string) {
                            log::warn!("Failed to release quota on cancellation: {}", error);
                        }
                        break;
                    }

                    transition_item(&db_clone, item.id, PipelineStage::Uploading)?;

                    // Sinh deterministic random_id
                    let upload_attempt_id =
                        format!("job_{}_item_{}_attempt_1", item.job_id, item.id);
                    let random_id = get_deterministic_random_id(&upload_attempt_id);

                    // Determine media kind
                    let media_kind = match classify_file(&item.name) {
                        FileCategory::Video => TelegramMediaKind::Video,
                        FileCategory::Image => TelegramMediaKind::Image,
                        FileCategory::Other => TelegramMediaKind::Other,
                    };

                    let request = TelegramUploadRequest {
                        job_id: item.job_id,
                        item_id: item.id,
                        path: file_path.clone(),
                        filename: if media_kind == TelegramMediaKind::Video {
                            let mut name = item.name.clone();
                            if let Some(idx) = name.rfind('.') {
                                name.truncate(idx);
                            }
                            format!("{}.mp4", name)
                        } else {
                            item.name.clone()
                        },
                        random_id,
                        destination_id,
                        media_kind,
                    };

                    let upload_result = tokio::select! {
                        _ = cancel.cancelled() => Err("Upload: cancelled".to_string()),
                        result = uploader_clone.upload_file(request) => result,
                    };
                    match upload_result {
                        Ok(result) => match result {
                            TelegramUploadResult::Confirmed {
                                message_id,
                                random_id: confirmed_random_id,
                            } => {
                                persist_telegram_confirmation(
                                    &db_clone,
                                    item.id,
                                    artifact_size,
                                    message_id,
                                    confirmed_random_id,
                                )?;
                                if let Err(error) = transition_item(
                                    &db_clone,
                                    item.id,
                                    PipelineStage::CompletedTelegram,
                                ) {
                                    if update_item_pipeline_stage(
                                        &db_clone,
                                        item.id,
                                        PipelineStage::Failed,
                                    )
                                    .is_ok()
                                    {
                                        emit_terminal(
                                            app_handle.as_ref(),
                                            &item,
                                            "uploading",
                                            PipelineStage::Failed,
                                            Some("transition_error"),
                                            Some(&error),
                                        );
                                        continue;
                                    }
                                    return Err(error);
                                }

                                {
                                    let conn =
                                        db_clone.lock().map_err(|value| value.to_string())?;
                                    if let Err(error) = commit_quota(&conn, item.id, &date_string) {
                                        log::error!(
                                            "Upload quota commit failed for item {}: {}",
                                            item.id,
                                            error
                                        );
                                    }
                                }

                                emit_terminal(
                                    app_handle.as_ref(),
                                    &item,
                                    "uploading",
                                    PipelineStage::CompletedTelegram,
                                    None,
                                    None,
                                );

                                // Cleanup using DB paths, not hardcoded workspace
                                Self::cleanup_item_artifacts(&item);
                            }
                            TelegramUploadResult::ReconciliationRequired {
                                random_id: rec_random_id,
                                reason,
                            } => {
                                log::warn!(
                                "Upload: reconciliation_required for item {}, random_id={}, reason: {}",
                                item.id, rec_random_id, reason
                            );

                                // Commit quota conservatively (assume sent)
                                {
                                    let conn =
                                        db_clone.lock().map_err(|value| value.to_string())?;
                                    if let Err(error) = commit_quota(&conn, item.id, &date_string) {
                                        log::error!("Upload quota commit failed for reconciliation item {}: {}", item.id, error);
                                    }
                                }

                                // Update item stage
                                {
                                    let conn =
                                        db_clone.lock().map_err(|value| value.to_string())?;
                                    let mut upd = conn
                                    .prepare(
                                        "UPDATE migration_items SET telegram_random_id = ?, last_error = ? WHERE id = ?;",
                                    )
                                    .map_err(|value| value.to_string())?;
                                    upd.bind((1, rec_random_id))
                                        .map_err(|value| value.to_string())?;
                                    upd.bind((2, reason.as_str()))
                                        .map_err(|value| value.to_string())?;
                                    upd.bind((3, item.id)).map_err(|value| value.to_string())?;
                                    upd.next().map_err(|value| value.to_string())?;
                                }

                                transition_item(
                                    &db_clone,
                                    item.id,
                                    PipelineStage::ReconciliationRequired,
                                )?;
                                emit_terminal(
                                    app_handle.as_ref(),
                                    &item,
                                    "uploading",
                                    PipelineStage::ReconciliationRequired,
                                    Some("reconciliation_required"),
                                    Some(&reason),
                                );
                            }
                        },
                        Err(e) => {
                            // Release quota on confirmed failure
                            {
                                let conn = db_clone.lock().map_err(|value| value.to_string())?;
                                if let Err(error) = release_quota(&conn, item.id, &date_string) {
                                    log::warn!("Failed to release upload quota: {}", error);
                                }
                            }

                            if cancel.is_cancelled() || is_cancellation_error(&e) {
                                transition_item(&db_clone, item.id, PipelineStage::QueuedUpload)?;
                                break;
                            }

                            // Check if it's a FloodWait
                            if let Some(seconds) = parse_flood_wait_seconds(&e) {
                                log::warn!("Upload: FloodWait {}s for item {}", seconds, item.id);
                                // Persist flood wait state
                                let now_ts = Utc::now().timestamp();
                                let next_allowed = now_ts + seconds;
                                {
                                    let conn =
                                        db_clone.lock().map_err(|value| value.to_string())?;
                                    let mut upd = conn
                                    .prepare(
                                        "UPDATE migration_jobs SET flood_wait_until = ? WHERE id = ?;",
                                    )
                                    .map_err(|value| value.to_string())?;
                                    upd.bind((1, next_allowed))
                                        .map_err(|value| value.to_string())?;
                                    upd.bind((2, item.job_id))
                                        .map_err(|value| value.to_string())?;
                                    upd.next().map_err(|value| value.to_string())?;
                                }
                                // Mark item as waiting_for_quota, do NOT sleep, do NOT spawn
                                persist_item_error(&db_clone, item.id, &e)?;
                                transition_item(
                                    &db_clone,
                                    item.id,
                                    PipelineStage::WaitingForQuota,
                                )?;
                                continue;
                            }

                            fail_item(
                                &db_clone,
                                app_handle.as_ref(),
                                &item,
                                "uploading",
                                "upload_error",
                                &e,
                            )?;
                        }
                    }
                }
                Ok::<(), String>(())
            });
            workers.push(handle);
        }

        for w in workers {
            match w.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    log::error!("Uploader worker error: {}", e);
                    return Err(e);
                }
                Err(e) => {
                    let err_msg = if e.is_panic() {
                        format!("Uploader worker panicked: {:?}", e)
                    } else {
                        format!("Uploader worker cancelled: {}", e)
                    };
                    log::error!("{}", err_msg);
                    return Err(err_msg);
                }
            }
        }
        Ok(())
    }

    /// Select artifact path based on video_decision policy
    fn select_artifact_path(item: &PipelineItem) -> PathBuf {
        let decision = item.video_decision.as_deref().unwrap_or("");
        match decision {
            "canonical_passthrough_main8" | "canonical_passthrough_main10" => {
                // Passthrough → use original artifact
                if let Some(ref p) = item.local_artifact_path {
                    PathBuf::from(p)
                } else {
                    PathBuf::from(item.local_artifact_path.as_deref().unwrap_or(""))
                }
            }
            "canonical_transcode_main8" | "canonical_transcode_main10" => {
                // Transcode → use processed artifact
                if let Some(ref p) = item.processed_artifact_path {
                    PathBuf::from(p)
                } else {
                    PathBuf::from(item.processed_artifact_path.as_deref().unwrap_or(""))
                }
            }
            _ => {
                // Image or other → use original artifact
                if let Some(ref p) = item.local_artifact_path {
                    PathBuf::from(p)
                } else {
                    PathBuf::from(item.local_artifact_path.as_deref().unwrap_or(""))
                }
            }
        }
    }

    /// Cleanup item artifacts using DB paths (idempotent)
    fn cleanup_item_artifacts(item: &PipelineItem) {
        if let Some(ref p) = item.local_artifact_path {
            if let Err(e) = std::fs::remove_file(p) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("Cleanup: failed to remove original artifact {}: {}", p, e);
                }
            }
        }
        if let Some(ref p) = item.processed_artifact_path {
            if let Err(e) = std::fs::remove_file(p) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("Cleanup: failed to remove processed artifact {}: {}", p, e);
                }
            }
        }
    }

    /// Task Local Finalizer lưu tệp Other vào backup_dir
    async fn run_local_finalizer(
        &self,
        rx: mpsc::Receiver<PipelineItem>,
        finalizer: Arc<dyn LocalFinalizer>,
    ) -> Result<(), String> {
        let rx = Arc::new(tokio::sync::Mutex::new(rx));
        let mut workers = Vec::new();

        for _ in 0..self.config.local_finalizer_concurrency {
            let rx_clone = rx.clone();
            let db_clone = self.db.clone();
            let finalizer_clone = finalizer.clone();
            let workspace = self.workspace_dir.clone();
            let backup = self.backup_dir.clone();
            let cancel = self.cancel_token.clone();
            let app_handle = self.app_handle.clone();

            let handle = tokio::spawn(async move {
                loop {
                    let item_opt = tokio::select! {
                        _ = cancel.cancelled() => break,
                        item = async {
                            let mut guard = rx_clone.lock().await;
                            guard.recv().await
                        } => item,
                    };
                    let item = match item_opt {
                        Some(i) => i,
                        None => break,
                    };
                    // Use actual artifact path from item, not hardcoded workspace/id
                    let input_path = match &item.local_artifact_path {
                        Some(p) => std::path::PathBuf::from(p),
                        None => workspace.join(format!("{}", item.id)),
                    };
                    let safe_source = sanitize_path(&item.source_path);

                    let base_dest = backup.join("OneDrive_Archive").join(&safe_source);
                    let mut dest_path = base_dest.clone();
                    let mut counter = 1;

                    // Collision handle
                    while dest_path.exists() {
                        let file_stem = base_dest.file_stem().unwrap_or_default().to_string_lossy();
                        let extension = base_dest.extension().unwrap_or_default().to_string_lossy();
                        let new_name = if extension.is_empty() {
                            format!("{}_{}", file_stem, counter)
                        } else {
                            format!("{}_{}.{}", file_stem, counter, extension)
                        };
                        dest_path = base_dest.with_file_name(new_name);
                        counter += 1;
                    }

                    // Tạo parent directories của file đích
                    if let Some(parent) = dest_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }

                    match finalizer_clone
                        .finalize_local(&input_path, &dest_path)
                        .await
                    {
                        Ok(_) => {
                            // Cập nhật local artifact path
                            {
                                let conn = db_clone.lock().unwrap();
                                let mut upd = conn
                                    .prepare(
                                        "UPDATE migration_items SET original_artifact_path = ? WHERE id = ?;",
                                    )
                                    .unwrap();
                                upd.bind((1, dest_path.to_str().unwrap_or_default()))
                                    .unwrap();
                                upd.bind((2, item.id)).unwrap();
                                upd.next().unwrap();
                            }

                            transition_item(&db_clone, item.id, PipelineStage::CompletedLocal)?;
                            emit_terminal(
                                app_handle.as_ref(),
                                &item,
                                "saving_local",
                                PipelineStage::CompletedLocal,
                                None,
                                None,
                            );

                            // Dọn dẹp tệp tin workspace
                            // No automatic removal of input_path here; cleanup happens centrally
                        }
                        Err(e) => {
                            log::error!("Local finalizer failed for item {}: {:?}", item.id, e);
                            fail_item(
                                &db_clone,
                                app_handle.as_ref(),
                                &item,
                                "saving_local",
                                "local_finalize_error",
                                &format!("{:?}", e),
                            )?;
                        }
                    }
                }
                Ok::<(), String>(())
            });
            workers.push(handle);
        }

        for w in workers {
            match w.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    log::error!("Local finalizer worker error: {}", e);
                    return Err(e);
                }
                Err(e) => {
                    let err_msg = if e.is_panic() {
                        format!("Local finalizer worker panicked: {:?}", e)
                    } else {
                        format!("Local finalizer worker cancelled: {}", e)
                    };
                    log::error!("{}", err_msg);
                    return Err(err_msg);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::db::open_migration_db_at_path;
    use crate::migration::pipeline::stages::VideoMetadata;
    use std::future::Future;
    use std::path::{Path, PathBuf};
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "telegram-drive-{}-{}-{}",
                label,
                std::process::id(),
                rand::random::<u64>()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn seed_job(db: &MigrationDb, workspace: &Path) {
        let conn = db.lock().unwrap();
        conn.execute(&format!(
            "INSERT INTO migration_jobs (id, source_folder_id, source_folder_path, telegram_destination_name, local_backup_dir, workspace_dir, state, started_at, created_at, updated_at) VALUES (1, 'src', '/', 'Saved Messages', '{}', '{}', 'running', 0, 0, 0)",
            workspace.display(),
            workspace.display()
        ))
        .unwrap();
        conn.execute("CREATE TABLE stage_history (seq INTEGER PRIMARY KEY AUTOINCREMENT, item_id INTEGER, stage TEXT)").unwrap();
        conn.execute("CREATE TRIGGER record_stage AFTER UPDATE OF pipeline_stage ON migration_items WHEN OLD.pipeline_stage <> NEW.pipeline_stage BEGIN INSERT INTO stage_history(item_id, stage) VALUES (NEW.id, NEW.pipeline_stage); END").unwrap();
    }

    fn seed_item(
        db: &MigrationDb,
        stage: &str,
        name: &str,
        original: Option<&Path>,
        processed: Option<&Path>,
        decision: Option<&str>,
    ) {
        let conn = db.lock().unwrap();
        let mut stmt = conn.prepare("INSERT INTO migration_items (id, job_id, folder_id, name, path, source_item_id, size, item_category, pipeline_stage, original_artifact_path, processed_artifact_path, video_decision, created_at, updated_at) VALUES (100, 1, 'f', ?, ?, 'source-100', 100, 'video', ?, ?, ?, ?, 0, 0)").unwrap();
        stmt.bind((1, name)).unwrap();
        stmt.bind((2, name)).unwrap();
        stmt.bind((3, stage)).unwrap();
        let original_path = original.map(|path| path.to_string_lossy().into_owned());
        let processed_path = processed.map(|path| path.to_string_lossy().into_owned());
        stmt.bind((4, original_path.as_deref())).unwrap();
        stmt.bind((5, processed_path.as_deref())).unwrap();
        stmt.bind((6, decision)).unwrap();
        stmt.next().unwrap();
    }

    fn make_runner(
        db: MigrationDb,
        workspace: &Path,
        cancel: CancellationToken,
    ) -> Arc<PipelineRunner> {
        Arc::new(PipelineRunner::new(
            PipelineConfig {
                download_concurrency: 1,
                processing_concurrency: 1,
                upload_concurrency: 1,
                local_finalizer_concurrency: 1,
                ..PipelineConfig::default()
            },
            db,
            1,
            workspace.to_path_buf(),
            workspace.join("backup"),
            Arc::new(tokio::sync::Mutex::new(None)),
            cancel,
            None,
            None,
        ))
    }

    fn metadata_for(marker: &str) -> Result<VideoMetadata, String> {
        if marker.contains("corrupt") {
            return Err("ffprobe parse error".to_string());
        }
        let (codec, profile, pixel_format) = if marker.contains("h264") {
            ("h264", "High", "yuv420p")
        } else if marker.contains("wrong-main10") {
            ("hevc", "Main 10", "yuv422p10le")
        } else {
            ("hevc", "Main", "yuv420p")
        };
        Ok(VideoMetadata {
            container_format_names: "mov,mp4,m4a,3gp,3g2,mj2".to_string(),
            video_codec: codec.to_string(),
            audio_codec: "aac".to_string(),
            duration: 1.0,
            width: 320,
            height: 240,
            is_valid: true,
            profile: profile.to_string(),
            pixel_format: pixel_format.to_string(),
            fps: 30.0,
            bitrate: if marker.contains("h264-low") {
                750_000
            } else if marker.contains("h264") {
                4_000_000
            } else {
                1_000_000
            },
            major_brand: "isom".to_string(),
            ..Default::default()
        })
    }

    struct ContentInspector;
    impl MediaInspector for ContentInspector {
        fn inspect_file(
            &self,
            path: &Path,
        ) -> Pin<Box<dyn Future<Output = Result<VideoMetadata, String>> + Send>> {
            let path = path.to_path_buf();
            Box::pin(async move {
                let marker = tokio::fs::read_to_string(path)
                    .await
                    .map_err(|error| error.to_string())?;
                metadata_for(&marker)
            })
        }
    }

    struct CountingProcessor(Arc<AtomicUsize>);
    impl VideoProcessor for CountingProcessor {
        fn process_video(
            &self,
            request: VideoProcessRequest,
        ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> {
            let output = request.output_path;
            let calls = self.0.clone();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                tokio::fs::write(output, b"processed-main8")
                    .await
                    .map_err(|error| error.to_string())?;
                Ok("processed-sha".to_string())
            })
        }
    }

    struct NeverDownloader;
    impl SourceDownloader for NeverDownloader {
        fn download_file(
            &self,
            _item_id: i64,
            _source_item_id: &str,
            _dest_path: &Path,
        ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> {
            Box::pin(async { Err("unexpected download".to_string()) })
        }
    }

    struct ConfirmingUploader {
        db: MigrationDb,
        observed: Arc<std::sync::Mutex<Vec<String>>>,
    }
    impl TelegramUploader for ConfirmingUploader {
        fn upload_file(
            &self,
            request: TelegramUploadRequest,
        ) -> Pin<Box<dyn Future<Output = Result<TelegramUploadResult, String>> + Send>> {
            let db = self.db.clone();
            let observed = self.observed.clone();
            Box::pin(async move {
                let stage = {
                    let conn = db.lock().unwrap();
                    let mut stmt = conn
                        .prepare("SELECT pipeline_stage FROM migration_items WHERE id = ?")
                        .unwrap();
                    stmt.bind((1, request.item_id)).unwrap();
                    stmt.next().unwrap();
                    stmt.read::<String, _>(0).unwrap()
                };
                observed.lock().unwrap().push(stage);
                Ok(TelegramUploadResult::Confirmed {
                    message_id: 7,
                    random_id: request.random_id,
                })
            })
        }
    }

    struct NoopFinalizer;
    impl LocalFinalizer for NoopFinalizer {
        fn finalize_local(
            &self,
            _source_path: &Path,
            _dest_path: &Path,
        ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> {
            Box::pin(async { Ok(()) })
        }
    }

    async fn run_video_flow(
        source_marker: &str,
        reject_terminal: bool,
    ) -> (
        TempDir,
        MigrationDb,
        Arc<AtomicUsize>,
        Arc<std::sync::Mutex<Vec<String>>>,
    ) {
        let tmp = TempDir::new("video-flow");
        let db = open_migration_db_at_path(tmp.0.join("migration.db")).unwrap();
        seed_job(&db, &tmp.0);
        let original = tmp.0.join("100");
        std::fs::write(&original, source_marker).unwrap();
        seed_item(
            &db,
            "queued_processing",
            "video.mp4",
            Some(&original),
            None,
            None,
        );
        if reject_terminal {
            db.lock().unwrap().execute("CREATE TRIGGER reject_completed BEFORE UPDATE OF pipeline_stage ON migration_items WHEN NEW.pipeline_stage = 'completed_telegram' BEGIN SELECT RAISE(ABORT, 'terminal persist rejected'); END").unwrap();
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
        let runner = make_runner(db.clone(), &tmp.0, CancellationToken::new());
        runner.clone().start(
            Arc::new(NeverDownloader),
            Arc::new(ContentInspector),
            Arc::new(CountingProcessor(calls.clone())),
            Arc::new(ConfirmingUploader {
                db: db.clone(),
                observed: observed.clone(),
            }),
            Arc::new(NoopFinalizer),
        );
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            runner.run_to_completion(),
        )
        .await
        .expect("pipeline lifecycle timed out")
        .unwrap();
        (tmp, db, calls, observed)
    }

    fn stage_history(db: &MigrationDb) -> Vec<String> {
        let conn = db.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT stage FROM stage_history WHERE item_id = 100 ORDER BY seq")
            .unwrap();
        let mut stages = Vec::new();
        while let Ok(sqlite::State::Row) = stmt.next() {
            stages.push(stmt.read(0).unwrap());
        }
        stages
    }

    #[tokio::test]
    async fn passthrough_uses_full_video_state_machine() {
        let (tmp, db, calls, observed) = run_video_flow("canonical-source", false).await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(observed.lock().unwrap().as_slice(), ["uploading"]);
        assert_eq!(
            stage_history(&db),
            [
                "processing",
                "processed",
                "queued_upload",
                "uploading",
                "completed_telegram"
            ]
        );
        assert!(
            !tmp.0.join("100").exists(),
            "cleanup follows terminal persist"
        );
    }

    #[tokio::test]
    async fn transcode_uses_full_video_state_machine() {
        let (tmp, db, calls, observed) = run_video_flow("h264-source", false).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(observed.lock().unwrap().as_slice(), ["uploading"]);
        assert_eq!(
            stage_history(&db),
            [
                "processing",
                "processed",
                "queued_upload",
                "uploading",
                "completed_telegram"
            ]
        );
        {
            let conn = db.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT video_decision, processed_artifact_path IS NULL, artifact_size \
                     FROM migration_items WHERE id = 100",
                )
                .unwrap();
            assert_eq!(stmt.next().unwrap(), sqlite::State::Row);
            assert_eq!(
                stmt.read::<String, _>(0).unwrap(),
                "size_passthrough_original"
            );
            assert_eq!(stmt.read::<i64, _>(1).unwrap(), 1);
            assert_eq!(stmt.read::<i64, _>(2).unwrap(), "h264-source".len() as i64);
        }
        assert!(!tmp.0.join("100").exists());
        assert!(!tmp.0.join("100.processed.mp4").exists());
    }

    #[tokio::test]
    async fn low_quality_noncanonical_video_skips_transcode() {
        let (_tmp, db, calls, observed) = run_video_flow("h264-low-source", false).await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(observed.lock().unwrap().as_slice(), ["uploading"]);
        let conn = db.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT video_decision FROM migration_items WHERE id = 100")
            .unwrap();
        assert_eq!(stmt.next().unwrap(), sqlite::State::Row);
        assert_eq!(
            stmt.read::<String, _>(0).unwrap(),
            "quality_passthrough_original"
        );
    }

    #[tokio::test]
    async fn cleanup_waits_for_completed_telegram_persist() {
        let (tmp, db, _calls, _observed) = run_video_flow("canonical-source", true).await;
        assert!(tmp.0.join("100").exists());
        let conn = db.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT pipeline_stage, last_error FROM migration_items WHERE id = 100")
            .unwrap();
        stmt.next().unwrap();
        assert_eq!(stmt.read::<String, _>(0).unwrap(), "failed");
        assert!(stmt
            .read::<String, _>(1)
            .unwrap()
            .contains("terminal persist rejected"));
    }

    #[tokio::test]
    async fn pipeline_drain_with_pending_item_fails_job_invariant() {
        let tmp = TempDir::new("pending-invariant");
        let db = open_migration_db_at_path(tmp.0.join("migration.db")).unwrap();
        seed_job(&db, &tmp.0);
        seed_item(&db, "queued_processing", "video.mp4", None, None, None);
        let runner = make_runner(db.clone(), &tmp.0, CancellationToken::new());
        runner.finalize_job().await.unwrap();
        let conn = db.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT state, last_error FROM migration_jobs WHERE id = 1")
            .unwrap();
        stmt.next().unwrap();
        assert_eq!(stmt.read::<String, _>(0).unwrap(), "failed");
        assert_eq!(
            stmt.read::<String, _>(1).unwrap(),
            "Pipeline drained with non-terminal items remaining"
        );
    }

    #[tokio::test]
    async fn folder_failures_drive_failed_or_completed_with_errors() {
        for (label, parent_id, expected_state) in [
            ("root-folder-failure", None, "failed"),
            (
                "child-folder-failure",
                Some("root"),
                "completed_with_errors",
            ),
        ] {
            let tmp = TempDir::new(label);
            let db = open_migration_db_at_path(tmp.0.join("migration.db")).unwrap();
            seed_job(&db, &tmp.0);
            {
                let conn = db.lock().unwrap();
                let mut stmt = conn.prepare("INSERT INTO folder_queue (job_id, folder_id, parent_id, folder_path, state, has_more, last_error, created_at, updated_at) VALUES (1, 'folder', ?, '/folder', 'failed', 0, 'permission denied', 0, 0)").unwrap();
                stmt.bind((1, parent_id)).unwrap();
                stmt.next().unwrap();
            }
            let runner = make_runner(db.clone(), &tmp.0, CancellationToken::new());
            runner.finalize_job().await.unwrap();
            let conn = db.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT state, last_error FROM migration_jobs WHERE id = 1")
                .unwrap();
            stmt.next().unwrap();
            assert_eq!(stmt.read::<String, _>(0).unwrap(), expected_state);
            assert!(stmt
                .read::<String, _>(1)
                .unwrap()
                .contains("permission denied"));
        }
    }

    #[tokio::test]
    async fn empty_pipeline_lifecycle_drains_within_timeout() {
        let tmp = TempDir::new("lifecycle");
        let db = open_migration_db_at_path(tmp.0.join("migration.db")).unwrap();
        seed_job(&db, &tmp.0);
        let runner = make_runner(db.clone(), &tmp.0, CancellationToken::new());
        runner.clone().start(
            Arc::new(NeverDownloader),
            Arc::new(ContentInspector),
            Arc::new(CountingProcessor(Arc::new(AtomicUsize::new(0)))),
            Arc::new(ConfirmingUploader {
                db: db.clone(),
                observed: Arc::new(std::sync::Mutex::new(Vec::new())),
            }),
            Arc::new(NoopFinalizer),
        );
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            runner.run_to_completion(),
        )
        .await
        .expect("planner/downloader/processor/uploader/finalizer did not drain")
        .unwrap();
        let conn = db.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT state FROM migration_jobs WHERE id = 1")
            .unwrap();
        stmt.next().unwrap();
        assert_eq!(stmt.read::<String, _>(0).unwrap(), "completed");
    }

    #[tokio::test]
    async fn fatal_worker_error_cancels_pipeline_and_fails_job() {
        let tmp = TempDir::new("fatal");
        let db = open_migration_db_at_path(tmp.0.join("migration.db")).unwrap();
        seed_job(&db, &tmp.0);
        seed_item(&db, "queued_download", "video.mp4", None, None, None);
        db.lock().unwrap().execute("CREATE TRIGGER reject_downloading BEFORE UPDATE OF pipeline_stage ON migration_items WHEN NEW.pipeline_stage = 'downloading' BEGIN SELECT RAISE(ABORT, 'fatal transition'); END").unwrap();
        let runner = make_runner(db.clone(), &tmp.0, CancellationToken::new());
        runner.clone().start(
            Arc::new(NeverDownloader),
            Arc::new(ContentInspector),
            Arc::new(CountingProcessor(Arc::new(AtomicUsize::new(0)))),
            Arc::new(ConfirmingUploader {
                db: db.clone(),
                observed: Arc::new(std::sync::Mutex::new(Vec::new())),
            }),
            Arc::new(NoopFinalizer),
        );
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            runner.run_to_completion(),
        )
        .await
        .expect("fatal pipeline did not cancel peers")
        .unwrap_err();
        assert!(error.contains("fatal transition"));
        let conn = db.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT state, last_error FROM migration_jobs WHERE id = 1")
            .unwrap();
        stmt.next().unwrap();
        assert_eq!(stmt.read::<String, _>(0).unwrap(), "failed");
        assert!(stmt
            .read::<String, _>(1)
            .unwrap()
            .contains("fatal transition"));
    }

    #[tokio::test]
    async fn crawler_permanent_auth_error_fails_job_without_retry_loop() {
        let tmp = TempDir::new("crawler-auth-fatal");
        let db = open_migration_db_at_path(tmp.0.join("migration.db")).unwrap();
        seed_job(&db, &tmp.0);
        {
            let conn = db.lock().unwrap();
            conn.execute("INSERT INTO folder_queue (job_id, folder_id, folder_path, state, has_more, created_at, updated_at) VALUES (1, 'root', '/', 'pending', 1, 0, 0)").unwrap();
        }
        let runner = make_runner(db.clone(), &tmp.0, CancellationToken::new());
        runner.clone().start(
            Arc::new(NeverDownloader),
            Arc::new(ContentInspector),
            Arc::new(CountingProcessor(Arc::new(AtomicUsize::new(0)))),
            Arc::new(ConfirmingUploader {
                db: db.clone(),
                observed: Arc::new(std::sync::Mutex::new(Vec::new())),
            }),
            Arc::new(NoopFinalizer),
        );
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            runner.run_to_completion(),
        )
        .await
        .expect("crawler auth failure entered a retry loop")
        .unwrap_err();
        assert!(error.contains("Microsoft account not connected"));
        let conn = db.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT state, last_error FROM migration_jobs WHERE id = 1")
            .unwrap();
        stmt.next().unwrap();
        assert_eq!(stmt.read::<String, _>(0).unwrap(), "failed");
        assert!(stmt
            .read::<String, _>(1)
            .unwrap()
            .contains("Microsoft account not connected"));
    }

    struct BlockingProcessor {
        cancel: CancellationToken,
        started: Arc<tokio::sync::Notify>,
    }
    impl VideoProcessor for BlockingProcessor {
        fn process_video(
            &self,
            request: VideoProcessRequest,
        ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> {
            let output = request.output_path;
            let cancel = self.cancel.clone();
            let started = self.started.clone();
            Box::pin(async move {
                tokio::fs::write(&output, b"partial-output")
                    .await
                    .map_err(|error| error.to_string())?;
                started.notify_one();
                cancel.cancelled().await;
                Err("Processor: cancelled".to_string())
            })
        }
    }

    struct BlockingDownloader {
        cancel: CancellationToken,
        started: Arc<tokio::sync::Notify>,
    }
    impl SourceDownloader for BlockingDownloader {
        fn download_file(
            &self,
            _item_id: i64,
            _source_item_id: &str,
            dest_path: &Path,
        ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> {
            let dest = dest_path.to_path_buf();
            let cancel = self.cancel.clone();
            let started = self.started.clone();
            Box::pin(async move {
                tokio::fs::write(&dest, b"partial-download")
                    .await
                    .map_err(|error| error.to_string())?;
                started.notify_one();
                cancel.cancelled().await;
                Err("Download: cancelled".to_string())
            })
        }
    }

    struct BlockingUploader {
        cancel: CancellationToken,
        started: Arc<tokio::sync::Notify>,
    }
    impl TelegramUploader for BlockingUploader {
        fn upload_file(
            &self,
            _request: TelegramUploadRequest,
        ) -> Pin<Box<dyn Future<Output = Result<TelegramUploadResult, String>> + Send>> {
            let cancel = self.cancel.clone();
            let started = self.started.clone();
            Box::pin(async move {
                started.notify_one();
                cancel.cancelled().await;
                Err("Upload: cancelled".to_string())
            })
        }
    }

    async fn stop_and_wait(
        runner: Arc<PipelineRunner>,
        cancel: CancellationToken,
        started: Arc<tokio::sync::Notify>,
    ) {
        let completion = tokio::spawn({
            let runner = runner.clone();
            async move { runner.run_to_completion().await }
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), started.notified())
            .await
            .expect("worker did not start");
        runner.stopped_by_user.store(true, Ordering::SeqCst);
        cancel.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(5), completion)
            .await
            .expect("stopped pipeline did not drain")
            .unwrap()
            .unwrap();
    }

    fn assert_item_and_job_state(db: &MigrationDb, item_stage: &str) {
        let conn = db.lock().unwrap();
        let mut item = conn
            .prepare("SELECT pipeline_stage, last_error FROM migration_items WHERE id = 100")
            .unwrap();
        item.next().unwrap();
        assert_eq!(item.read::<String, _>(0).unwrap(), item_stage);
        assert!(item.read::<Option<String>, _>(1).unwrap().is_none());
        let mut job = conn
            .prepare("SELECT state, last_error FROM migration_jobs WHERE id = 1")
            .unwrap();
        job.next().unwrap();
        assert_eq!(job.read::<String, _>(0).unwrap(), "stopped");
        assert!(job.read::<Option<String>, _>(1).unwrap().is_none());
    }

    #[tokio::test]
    async fn stop_mid_encode_removes_partial_and_retains_original() {
        let tmp = TempDir::new("stop-encode");
        let db = open_migration_db_at_path(tmp.0.join("migration.db")).unwrap();
        seed_job(&db, &tmp.0);
        let original = tmp.0.join("100");
        std::fs::write(&original, "h264-source").unwrap();
        seed_item(
            &db,
            "queued_processing",
            "video.mp4",
            Some(&original),
            None,
            None,
        );
        let cancel = CancellationToken::new();
        let started = Arc::new(tokio::sync::Notify::new());
        let runner = make_runner(db.clone(), &tmp.0, cancel.clone());
        runner.clone().start(
            Arc::new(NeverDownloader),
            Arc::new(ContentInspector),
            Arc::new(BlockingProcessor {
                cancel: cancel.clone(),
                started: started.clone(),
            }),
            Arc::new(ConfirmingUploader {
                db: db.clone(),
                observed: Arc::new(std::sync::Mutex::new(Vec::new())),
            }),
            Arc::new(NoopFinalizer),
        );
        stop_and_wait(runner, cancel, started).await;
        assert!(original.exists());
        assert!(!tmp.0.join("100.processed.mp4").exists());
        assert_item_and_job_state(&db, "queued_processing");
    }

    #[tokio::test]
    async fn stop_mid_download_removes_part_and_does_not_fail_item() {
        let tmp = TempDir::new("stop-download");
        let db = open_migration_db_at_path(tmp.0.join("migration.db")).unwrap();
        seed_job(&db, &tmp.0);
        seed_item(&db, "queued_download", "video.mp4", None, None, None);
        let cancel = CancellationToken::new();
        let started = Arc::new(tokio::sync::Notify::new());
        let runner = make_runner(db.clone(), &tmp.0, cancel.clone());
        runner.clone().start(
            Arc::new(BlockingDownloader {
                cancel: cancel.clone(),
                started: started.clone(),
            }),
            Arc::new(ContentInspector),
            Arc::new(CountingProcessor(Arc::new(AtomicUsize::new(0)))),
            Arc::new(ConfirmingUploader {
                db: db.clone(),
                observed: Arc::new(std::sync::Mutex::new(Vec::new())),
            }),
            Arc::new(NoopFinalizer),
        );
        stop_and_wait(runner, cancel, started).await;
        assert!(!tmp.0.join("100.part").exists());
        assert_item_and_job_state(&db, "queued_download");
    }

    #[tokio::test]
    async fn stop_mid_upload_retains_artifact_and_does_not_fail_item() {
        let tmp = TempDir::new("stop-upload");
        let db = open_migration_db_at_path(tmp.0.join("migration.db")).unwrap();
        seed_job(&db, &tmp.0);
        let original = tmp.0.join("100");
        std::fs::write(&original, "image-bytes").unwrap();
        seed_item(
            &db,
            "queued_upload",
            "photo.jpg",
            Some(&original),
            None,
            None,
        );
        let cancel = CancellationToken::new();
        let started = Arc::new(tokio::sync::Notify::new());
        let runner = make_runner(db.clone(), &tmp.0, cancel.clone());
        runner.clone().start(
            Arc::new(NeverDownloader),
            Arc::new(ContentInspector),
            Arc::new(CountingProcessor(Arc::new(AtomicUsize::new(0)))),
            Arc::new(BlockingUploader {
                cancel: cancel.clone(),
                started: started.clone(),
            }),
            Arc::new(NoopFinalizer),
        );
        stop_and_wait(runner, cancel, started).await;
        assert!(original.exists());
        assert_item_and_job_state(&db, "queued_upload");
    }

    struct ErrorUploader {
        error: String,
        calls: Arc<AtomicUsize>,
    }
    impl TelegramUploader for ErrorUploader {
        fn upload_file(
            &self,
            _request: TelegramUploadRequest,
        ) -> Pin<Box<dyn Future<Output = Result<TelegramUploadResult, String>> + Send>> {
            let error = self.error.clone();
            let calls = self.calls.clone();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(error)
            })
        }
    }

    #[tokio::test]
    async fn expired_waiting_for_quota_resumes_same_item_to_upload() {
        let tmp = TempDir::new("quota-resume");
        let db = open_migration_db_at_path(tmp.0.join("migration.db")).unwrap();
        seed_job(&db, &tmp.0);
        let original = tmp.0.join("100");
        std::fs::write(&original, "image").unwrap();
        seed_item(
            &db,
            "waiting_for_quota",
            "photo.jpg",
            Some(&original),
            None,
            None,
        );
        let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
        let runner = make_runner(db.clone(), &tmp.0, CancellationToken::new());
        runner.clone().start(
            Arc::new(NeverDownloader),
            Arc::new(ContentInspector),
            Arc::new(CountingProcessor(Arc::new(AtomicUsize::new(0)))),
            Arc::new(ConfirmingUploader {
                db: db.clone(),
                observed,
            }),
            Arc::new(NoopFinalizer),
        );
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            runner.run_to_completion(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            stage_history(&db),
            ["queued_upload", "uploading", "completed_telegram"]
        );
    }

    #[tokio::test]
    async fn quota_reserve_failure_waits_without_network_upload() {
        let tmp = TempDir::new("quota-reserve");
        let db = open_migration_db_at_path(tmp.0.join("migration.db")).unwrap();
        seed_job(&db, &tmp.0);
        let original = tmp.0.join("100");
        let file = std::fs::File::create(&original).unwrap();
        file.set_len((crate::migration::quota_reserve::DAILY_SAFETY_BUDGET_LIMIT + 1) as u64)
            .unwrap();
        seed_item(
            &db,
            "queued_upload",
            "photo.jpg",
            Some(&original),
            None,
            None,
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let runner = make_runner(db.clone(), &tmp.0, CancellationToken::new());
        runner.clone().start(
            Arc::new(NeverDownloader),
            Arc::new(ContentInspector),
            Arc::new(CountingProcessor(Arc::new(AtomicUsize::new(0)))),
            Arc::new(ErrorUploader {
                error: "must not run".to_string(),
                calls: calls.clone(),
            }),
            Arc::new(NoopFinalizer),
        );
        runner.run_to_completion().await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(stage_history(&db), ["waiting_for_quota"]);
    }

    #[tokio::test]
    async fn flood_wait_after_upload_started_returns_to_waiting_for_quota() {
        let tmp = TempDir::new("flood-wait");
        let db = open_migration_db_at_path(tmp.0.join("migration.db")).unwrap();
        seed_job(&db, &tmp.0);
        let original = tmp.0.join("100");
        std::fs::write(&original, "image").unwrap();
        seed_item(
            &db,
            "queued_upload",
            "photo.jpg",
            Some(&original),
            None,
            None,
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let runner = make_runner(db.clone(), &tmp.0, CancellationToken::new());
        runner.clone().start(
            Arc::new(NeverDownloader),
            Arc::new(ContentInspector),
            Arc::new(CountingProcessor(Arc::new(AtomicUsize::new(0)))),
            Arc::new(ErrorUploader {
                error: "Upload: FloodWait: 2s".to_string(),
                calls: calls.clone(),
            }),
            Arc::new(NoopFinalizer),
        );
        runner.run_to_completion().await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(stage_history(&db), ["uploading", "waiting_for_quota"]);
    }

    #[tokio::test]
    async fn processed_validation_rejects_h264_corrupt_zero_and_wrong_main10() {
        let tmp = TempDir::new("processed-validation");
        let db = open_migration_db_at_path(tmp.0.join("migration.db")).unwrap();
        seed_job(&db, &tmp.0);
        let original = tmp.0.join("original");
        std::fs::write(&original, "canonical-source").unwrap();
        for (index, marker) in ["h264-processed", "corrupt", "wrong-main10"]
            .into_iter()
            .enumerate()
        {
            let processed = tmp.0.join(format!("processed-{}", index));
            std::fs::write(&processed, marker).unwrap();
            let item = PipelineItem {
                id: index as i64 + 1,
                job_id: 1,
                name: "video.mp4".to_string(),
                source_path: "video.mp4".to_string(),
                source_item_id: Some("source".to_string()),
                size_bytes: 1,
                source_etag: None,
                source_last_modified: None,
                source_fingerprint_type: None,
                source_fingerprint_value: None,
                state: "processed".to_string(),
                original_sha256: None,
                processed_sha256: Some("hash".to_string()),
                local_artifact_path: Some(original.to_string_lossy().into_owned()),
                processed_artifact_path: Some(processed.to_string_lossy().into_owned()),
                telegram_random_id: None,
                video_decision: Some(
                    if marker == "wrong-main10" {
                        "canonical_transcode_main10"
                    } else {
                        "canonical_transcode_main8"
                    }
                    .to_string(),
                ),
                retry_count: 0,
            };
            assert!(
                !validate_processed_artifact(&ContentInspector, &item).await,
                "{} must be rejected",
                marker
            );
        }
        let zero = tmp.0.join("zero.processed.mp4");
        std::fs::write(&zero, []).unwrap();
        assert!(!artifact_is_valid_file(Some(zero.to_str().unwrap())));
    }

    #[tokio::test]
    async fn recovery_routes_valid_processed_to_upload_and_invalid_to_processing_or_download() {
        for (label, processed_marker, with_original, expected_prefix, processor_calls) in [
            (
                "valid",
                "processed-main8",
                false,
                vec!["queued_upload", "uploading", "completed_telegram"],
                0,
            ),
            (
                "invalid-original",
                "h264-processed",
                true,
                vec!["queued_processing", "processing", "processed"],
                1,
            ),
            (
                "invalid-missing",
                "corrupt",
                false,
                vec!["queued_download", "downloading", "failed"],
                0,
            ),
        ] {
            let tmp = TempDir::new(label);
            let db = open_migration_db_at_path(tmp.0.join("migration.db")).unwrap();
            seed_job(&db, &tmp.0);
            let processed = tmp.0.join("100.processed.mp4");
            std::fs::write(&processed, processed_marker).unwrap();
            let original = tmp.0.join("100");
            if with_original {
                std::fs::write(&original, "h264-source").unwrap();
            }
            seed_item(
                &db,
                "processed",
                "video.mp4",
                with_original.then_some(original.as_path()),
                Some(&processed),
                Some("canonical_transcode_main8"),
            );
            let calls = Arc::new(AtomicUsize::new(0));
            let runner = make_runner(db.clone(), &tmp.0, CancellationToken::new());
            runner.clone().start(
                Arc::new(NeverDownloader),
                Arc::new(ContentInspector),
                Arc::new(CountingProcessor(calls.clone())),
                Arc::new(ConfirmingUploader {
                    db: db.clone(),
                    observed: Arc::new(std::sync::Mutex::new(Vec::new())),
                }),
                Arc::new(NoopFinalizer),
            );
            runner.run_to_completion().await.unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), processor_calls, "{}", label);
            let history = stage_history(&db);
            assert_eq!(
                &history[..expected_prefix.len()],
                expected_prefix.as_slice(),
                "{}",
                label
            );
        }
    }

    #[tokio::test]
    async fn recovery_reprobes_passthrough_original_before_upload() {
        for (label, original_marker, expected_prefix, processor_calls) in [
            (
                "passthrough-valid",
                "processed-main8",
                vec!["queued_upload", "uploading", "completed_telegram"],
                0,
            ),
            (
                "passthrough-h264",
                "h264-source",
                vec!["queued_processing", "processing", "processed"],
                1,
            ),
            (
                "passthrough-corrupt",
                "corrupt",
                vec!["queued_download", "downloading", "failed"],
                0,
            ),
        ] {
            let tmp = TempDir::new(label);
            let db = open_migration_db_at_path(tmp.0.join("migration.db")).unwrap();
            seed_job(&db, &tmp.0);
            let original = tmp.0.join("original.mp4");
            std::fs::write(&original, original_marker).unwrap();
            seed_item(
                &db,
                "processed",
                "video.mp4",
                Some(&original),
                None,
                Some("canonical_passthrough_main8"),
            );
            let calls = Arc::new(AtomicUsize::new(0));
            let runner = make_runner(db.clone(), &tmp.0, CancellationToken::new());
            runner.clone().start(
                Arc::new(NeverDownloader),
                Arc::new(ContentInspector),
                Arc::new(CountingProcessor(calls.clone())),
                Arc::new(ConfirmingUploader {
                    db: db.clone(),
                    observed: Arc::new(std::sync::Mutex::new(Vec::new())),
                }),
                Arc::new(NoopFinalizer),
            );
            runner.run_to_completion().await.unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), processor_calls, "{}", label);
            let history = stage_history(&db);
            assert_eq!(
                &history[..expected_prefix.len()],
                expected_prefix.as_slice(),
                "{}",
                label
            );
        }
    }
}
