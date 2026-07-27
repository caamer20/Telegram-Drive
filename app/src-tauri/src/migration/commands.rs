use tauri::{Manager, State};

use crate::migration::microsoft;
use crate::migration::models::*;
use crate::migration::MigrationState;

async fn determine_retry_route(
    inspector: &dyn crate::migration::pipeline::stages::MediaInspector,
    item: &crate::migration::pipeline::stages::PipelineItem,
) -> (crate::migration::pipeline::stages::PipelineStage, bool) {
    use crate::migration::pipeline::classifier::{classify_file, FileCategory};
    use crate::migration::pipeline::stages::PipelineStage;

    let category = classify_file(&item.name);
    let processed_valid = matches!(category, FileCategory::Video)
        && crate::migration::pipeline::runner::validate_processed_artifact(inspector, item).await;
    if processed_valid {
        return (PipelineStage::QueuedUpload, true);
    }
    if crate::migration::pipeline::runner::artifact_is_valid_file(
        item.local_artifact_path.as_deref(),
    ) {
        let stage = match category {
            FileCategory::Video => PipelineStage::QueuedProcessing,
            FileCategory::Image => PipelineStage::QueuedUpload,
            FileCategory::Other => PipelineStage::SavingLocal,
        };
        (stage, false)
    } else {
        (PipelineStage::QueuedDownload, false)
    }
}

fn latest_resumable_job(conn: &sqlite::Connection) -> Result<Option<(i64, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, state FROM migration_jobs \
             WHERE id = (SELECT id FROM migration_jobs ORDER BY updated_at DESC, id DESC LIMIT 1) \
               AND state IN ('running', 'stopped', 'waiting_for_quota')",
        )
        .map_err(|e| e.to_string())?;

    if let Ok(sqlite::State::Row) = stmt.next() {
        Ok(Some((
            stmt.read(0).unwrap_or(0),
            stmt.read(1).unwrap_or_default(),
        )))
    } else {
        Ok(None)
    }
}

fn mark_interrupted_job_stopped(conn: &sqlite::Connection, job_id: i64) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let mut stmt = conn
        .prepare(
            "UPDATE migration_jobs \
             SET state = 'stopped', completed_at = NULL, \
                 last_error = COALESCE(last_error, 'Migration interrupted by app shutdown'), \
                 updated_at = ? \
             WHERE id = ? AND state = 'running'",
        )
        .map_err(|e| e.to_string())?;
    stmt.bind((1, now)).map_err(|e| e.to_string())?;
    stmt.bind((2, job_id)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;
    Ok(())
}

async fn ensure_no_active_pipeline(state: &MigrationState) -> Result<(), String> {
    if let Some(active) = state.active_pipeline.lock().await.as_ref() {
        return Err(format!(
            "Migration pipeline {} is already active",
            active.job_id
        ));
    }
    Ok(())
}

async fn clear_active_pipeline_if_matches(state: &MigrationState, completed_job_id: i64) {
    let mut active = state.active_pipeline.lock().await;
    if active.as_ref().map(|pipeline| pipeline.job_id) == Some(completed_job_id) {
        *active = None;
    }
}

async fn get_resumable_job_locked(state: &MigrationState) -> Result<Option<i64>, String> {
    if let Some(active) = state.active_pipeline.lock().await.as_ref() {
        return Ok(Some(active.job_id));
    }
    let conn = state.db.lock().map_err(|error| error.to_string())?;
    let resumable = latest_resumable_job(&conn)?;
    if let Some((job_id, job_state)) = resumable {
        if job_state == "running" {
            mark_interrupted_job_stopped(&conn, job_id)?;
        }
        Ok(Some(job_id))
    } else {
        Ok(None)
    }
}

fn persist_job_setup_failure(
    db: &crate::migration::db::MigrationDb,
    job_id: i64,
    error: &str,
) -> Result<(), String> {
    let conn = db.lock().map_err(|value| value.to_string())?;
    persist_job_setup_failure_on_conn(&conn, job_id, error)
}

fn persist_job_setup_failure_on_conn(
    conn: &sqlite::Connection,
    job_id: i64,
    error: &str,
) -> Result<(), String> {
    let now = crate::migration::events::now_millis();
    let mut stmt = conn
        .prepare(
            "UPDATE migration_jobs SET state = 'failed', last_error = ?, completed_at = ?, updated_at = ? WHERE id = ?",
        )
        .map_err(|value| value.to_string())?;
    stmt.bind((1, error)).map_err(|value| value.to_string())?;
    stmt.bind((2, now)).map_err(|value| value.to_string())?;
    stmt.bind((3, now)).map_err(|value| value.to_string())?;
    stmt.bind((4, job_id)).map_err(|value| value.to_string())?;
    stmt.next().map_err(|value| value.to_string())?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn register_and_spawn_pipeline(
    state: std::sync::Arc<MigrationState>,
    job_id: i64,
    runner: std::sync::Arc<crate::migration::pipeline::runner::PipelineRunner>,
    downloader: std::sync::Arc<crate::migration::adapters::onedrive::OneDriveDownloader>,
    media_adapter: std::sync::Arc<crate::migration::adapters::media::FFmpegMediaAdapter>,
    uploader: std::sync::Arc<crate::migration::adapters::telegram::TelegramProductionAdapter>,
    finalizer: std::sync::Arc<crate::migration::adapters::local::LocalProductionAdapter>,
    cancel_token: tokio_util::sync::CancellationToken,
) -> Result<(), String> {
    ensure_no_active_pipeline(&state).await?;
    runner.clone().start(
        downloader,
        media_adapter.clone(),
        media_adapter,
        uploader,
        finalizer,
    );
    let (completion_tx, completion_rx) = tokio::sync::watch::channel(None);
    {
        let mut active = state.active_pipeline.lock().await;
        *active = Some(crate::migration::ActivePipeline {
            job_id,
            runner: runner.clone(),
            cancel_token,
            completion: completion_rx,
        });
    }
    tauri::async_runtime::spawn(async move {
        let result = runner.run_to_completion().await;
        if let Err(error) = &result {
            log::error!("Pipeline failed for job {}: {}", job_id, error);
        }
        clear_active_pipeline_if_matches(&state, job_id).await;
        let _ = completion_tx.send(Some(result));
    });
    Ok(())
}

async fn wait_for_pipeline_completion(
    completion: &mut tokio::sync::watch::Receiver<Option<Result<(), String>>>,
) -> Result<Result<(), String>, String> {
    let wait = async {
        loop {
            if let Some(result) = completion.borrow().clone() {
                return Ok(result);
            }
            completion
                .changed()
                .await
                .map_err(|_| "Migration completion signal closed unexpectedly".to_string())?;
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(30), wait)
        .await
        .map_err(|_| "Migration stop timed out".to_string())?
}

async fn stop_active_pipeline(state: &MigrationState) -> Result<(), String> {
    let (job_id, runner, cancel_token, mut completion) = {
        let active = state.active_pipeline.lock().await;
        let active = active
            .as_ref()
            .ok_or_else(|| "No active migration".to_string())?;
        (
            active.job_id,
            active.runner.clone(),
            active.cancel_token.clone(),
            active.completion.clone(),
        )
    };
    runner
        .stopped_by_user
        .store(true, std::sync::atomic::Ordering::Relaxed);
    cancel_token.cancel();
    let result = wait_for_pipeline_completion(&mut completion).await?;
    if let Err(error) = result {
        return Err(format!(
            "Migration stop completed with pipeline error: {}",
            error
        ));
    }
    let conn = state.db.lock().map_err(|value| value.to_string())?;
    let mut stmt = conn
        .prepare("SELECT state FROM migration_jobs WHERE id = ?")
        .map_err(|value| value.to_string())?;
    stmt.bind((1, job_id)).map_err(|value| value.to_string())?;
    let job_state = if let Ok(sqlite::State::Row) = stmt.next() {
        stmt.read::<String, _>(0).unwrap_or_default()
    } else {
        return Err("Stopped migration job no longer exists".to_string());
    };
    if job_state != "stopped" {
        return Err(format!(
            "Migration stop completed but job {} is '{}'",
            job_id, job_state
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn cmd_migration_ms_connect(
    state: State<'_, MigrationState>,
    app_handle: tauri::AppHandle,
    client_id: Option<String>,
    tenant: Option<String>,
    redirect_uri: Option<String>,
) -> Result<MsAccountInfo, String> {
    let cid = client_id
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| microsoft::DEFAULT_MS_CLIENT_ID.to_string());
    let t = tenant
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "common".to_string());
    let r = redirect_uri
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| microsoft::DEFAULT_REDIRECT_URI.to_string());

    let session = microsoft::start_oauth_flow(&cid, &t, &r, &app_handle).await?;
    let info = session.account_info.clone();
    crate::migration::session_store::save(&app_handle, &session)?;
    *state.ms_session.lock().await = Some(session);

    Ok(info)
}

#[tauri::command]
pub async fn cmd_migration_ms_disconnect(
    state: State<'_, MigrationState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    *state.ms_session.lock().await = None;
    crate::migration::session_store::delete(&app_handle)?;
    Ok(())
}

#[tauri::command]
pub async fn cmd_migration_ms_status(
    state: State<'_, MigrationState>,
) -> Result<Option<MsAccountInfo>, String> {
    let session_guard = state.ms_session.lock().await;
    if let Some(ref session) = *session_guard {
        Ok(Some(session.account_info.clone()))
    } else {
        Ok(None)
    }
}

#[tauri::command]
pub async fn cmd_migration_get_folder_children(
    state: State<'_, MigrationState>,
    app_handle: tauri::AppHandle,
    parent_id: Option<String>,
) -> Result<Vec<OneDriveItem>, String> {
    let http = reqwest::Client::new();
    let access_token = {
        let mut guard = state.ms_session.lock().await;
        if let Some(ref mut session) = *guard {
            if session.is_expired() {
                microsoft::refresh_access_token(session).await?;
                crate::migration::session_store::save(&app_handle, session)?;
            }
            session.access_token.clone()
        } else {
            return Err("Microsoft account not connected".into());
        }
    };

    microsoft::list_children(&http, &access_token, parent_id.as_deref()).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn cmd_migration_start(
    state: State<'_, MigrationState>,
    tg_state: State<'_, crate::commands::TelegramState>,
    app_handle: tauri::AppHandle,
    source_folder_id: String,
    source_folder_path: String,
    telegram_destination_id: Option<i64>,
    telegram_destination_name: String,
    local_backup_dir: String,
) -> Result<i64, String> {
    let _lifecycle = state.lifecycle_lock.lock().await;
    ensure_no_active_pipeline(&state).await?;

    // 1. Validate OneDrive session
    let _access_token = {
        let mut guard = state.ms_session.lock().await;
        if let Some(ref mut session) = *guard {
            if session.is_expired() {
                microsoft::refresh_access_token(session).await?;
                crate::migration::session_store::save(&app_handle, session)?;
            }
            session.access_token.clone()
        } else {
            return Err("Microsoft account not connected".into());
        }
    };

    // 2. Validate local directories
    let backup_path = std::path::Path::new(&local_backup_dir);
    if !backup_path.exists() {
        return Err(format!(
            "Local backup directory does not exist: {}",
            local_backup_dir
        ));
    }

    let workspace_dir = app_handle
        .path()
        .app_data_dir()
        .unwrap_or_default()
        .join("migration_workspace")
        .to_string_lossy()
        .to_string();
    let workspace_path = std::path::Path::new(&workspace_dir);
    std::fs::create_dir_all(workspace_path)
        .map_err(|e| format!("Cannot create workspace directory: {}", e))?;

    // Ensure workspace and backup don't overlap
    let backup_canon = backup_path
        .canonicalize()
        .unwrap_or_else(|_| backup_path.to_path_buf());
    let workspace_canon = workspace_path
        .canonicalize()
        .unwrap_or_else(|_| workspace_path.to_path_buf());
    if backup_canon == workspace_canon {
        return Err("Local backup directory and workspace directory must be different".into());
    }

    let capabilities = crate::migration::adapters::media::preflight_media().await?;

    // 3. Create job in DB
    let job_id = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let jid = crate::migration::db::create_job(
            &conn,
            &source_folder_id,
            &source_folder_path,
            telegram_destination_id,
            &telegram_destination_name,
            &local_backup_dir,
            &workspace_dir,
        )?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let insert_result = (|| -> Result<(), String> {
            let mut stmt = conn.prepare(
                "INSERT INTO folder_queue (job_id, folder_id, folder_path, state, created_at, updated_at) VALUES (?, ?, ?, 'pending', ?, ?)"
            ).map_err(|e| e.to_string())?;
            stmt.bind((1, jid)).map_err(|error| error.to_string())?;
            stmt.bind((2, source_folder_id.as_str()))
                .map_err(|error| error.to_string())?;
            stmt.bind((3, source_folder_path.as_str()))
                .map_err(|error| error.to_string())?;
            stmt.bind((4, now)).map_err(|error| error.to_string())?;
            stmt.bind((5, now)).map_err(|error| error.to_string())?;
            stmt.next().map_err(|error| error.to_string())?;
            Ok(())
        })();
        if let Err(error) = insert_result {
            persist_job_setup_failure_on_conn(&conn, jid, &error)?;
            return Err(error);
        }
        jid
    };

    // 4. Start pipeline
    let mig_state = state.inner().clone_state();
    let services = crate::migration::adapters::factory::build_pipeline_services(
        mig_state.db.clone(),
        mig_state.ms_session.clone(),
        tg_state.client.clone(),
        tg_state.peer_cache.clone(),
        job_id,
        std::path::PathBuf::from(&workspace_dir),
        std::path::PathBuf::from(&local_backup_dir),
        telegram_destination_id,
        Some(app_handle),
        capabilities.selected_encoder,
    );
    let (runner, downloader, media_adapter, uploader, finalizer, cancel_token) = match services {
        Ok(services) => services,
        Err(error) => {
            persist_job_setup_failure(&state.db, job_id, &error)?;
            return Err(error);
        }
    };
    register_and_spawn_pipeline(
        mig_state,
        job_id,
        runner,
        downloader,
        media_adapter,
        uploader,
        finalizer,
        cancel_token,
    )
    .await
    .inspect_err(|error| {
        let _ = persist_job_setup_failure(&state.db, job_id, error);
    })?;

    Ok(job_id)
}

#[tauri::command]
pub async fn cmd_migration_stop(state: State<'_, MigrationState>) -> Result<(), String> {
    let _lifecycle = state.lifecycle_lock.lock().await;
    stop_active_pipeline(&state).await
}

#[tauri::command]
pub async fn cmd_migration_get_status(
    state: State<'_, MigrationState>,
    job_id: i64,
) -> Result<MigrationJobDetail, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    // 1. Get Job with explicit columns
    let mut job_stmt = conn
        .prepare(
            "SELECT id, source_folder_id, source_folder_path, telegram_destination_id, \
         telegram_destination_name, local_backup_dir, workspace_dir, state, \
         started_at, completed_at, last_error, flood_wait_until, \
         discovered_folders, completed_folders, discovered_items, completed_items, \
         failed_items, waiting_items, created_at, updated_at \
         FROM migration_jobs WHERE id = ?",
        )
        .map_err(|e| e.to_string())?;
    job_stmt.bind((1, job_id)).map_err(|e| e.to_string())?;
    let job = if let Ok(sqlite::State::Row) = job_stmt.next() {
        MigrationJob {
            id: job_stmt.read(0).unwrap_or(0),
            source_folder_id: job_stmt.read(1).unwrap_or_default(),
            source_folder_path: job_stmt.read(2).unwrap_or_default(),
            telegram_destination_id: job_stmt.read(3).ok(),
            telegram_destination_name: job_stmt.read(4).unwrap_or_default(),
            local_backup_dir: job_stmt.read(5).unwrap_or_default(),
            workspace_dir: job_stmt.read(6).unwrap_or_default(),
            state: job_stmt.read(7).unwrap_or_default(),
            started_at: job_stmt.read(8).unwrap_or(0),
            completed_at: job_stmt.read(9).ok(),
            last_error: job_stmt.read(10).ok(),
            flood_wait_until: job_stmt.read(11).ok(),
            discovered_folders: job_stmt.read(12).unwrap_or(0),
            completed_folders: job_stmt.read(13).unwrap_or(0),
            discovered_items: job_stmt.read(14).unwrap_or(0),
            completed_items: job_stmt.read(15).unwrap_or(0),
            failed_items: job_stmt.read(16).unwrap_or(0),
            waiting_items: job_stmt.read(17).unwrap_or(0),
            created_at: job_stmt.read(18).unwrap_or(0),
            updated_at: job_stmt.read(19).unwrap_or(0),
        }
    } else {
        return Err("Job not found".into());
    };

    // 2. Get Files with explicit columns
    let mut files_stmt = conn
        .prepare(
            "SELECT id, job_id, folder_id, source_item_id, name, path, size, item_category, \
         pipeline_stage, original_artifact_path, processed_artifact_path, \
         original_sha256, processed_sha256, video_decision, artifact_size, \
         telegram_attempt_id, telegram_random_id, telegram_message_id, \
         retry_count, last_error, created_at, updated_at, completed_at \
         FROM migration_items WHERE job_id = ?",
        )
        .map_err(|e| e.to_string())?;
    files_stmt.bind((1, job_id)).map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    while let Ok(sqlite::State::Row) = files_stmt.next() {
        files.push(MigrationItem {
            id: files_stmt.read(0).unwrap_or(0),
            job_id: files_stmt.read(1).unwrap_or(0),
            folder_id: files_stmt.read(2).unwrap_or_default(),
            source_item_id: files_stmt.read(3).unwrap_or_default(),
            name: files_stmt.read(4).unwrap_or_default(),
            path: files_stmt.read(5).unwrap_or_default(),
            size: files_stmt.read(6).unwrap_or(0),
            item_category: files_stmt.read(7).unwrap_or_default(),
            pipeline_stage: files_stmt.read(8).unwrap_or_default(),
            original_artifact_path: files_stmt.read(9).ok(),
            processed_artifact_path: files_stmt.read(10).ok(),
            original_sha256: files_stmt.read(11).ok(),
            processed_sha256: files_stmt.read(12).ok(),
            video_decision: files_stmt.read(13).ok(),
            artifact_size: files_stmt.read(14).ok(),
            telegram_attempt_id: files_stmt.read(15).ok(),
            telegram_random_id: files_stmt.read(16).ok(),
            telegram_message_id: files_stmt.read(17).ok(),
            retry_count: files_stmt.read(18).unwrap_or(0),
            last_error: files_stmt.read(19).ok(),
            created_at: files_stmt.read(20).unwrap_or(0),
            updated_at: files_stmt.read(21).unwrap_or(0),
            completed_at: files_stmt.read(22).ok(),
        });
    }

    // 3. Stats - use terminal stages properly
    let total_folders = job.discovered_folders;
    let total_files = files.len() as i64;
    let total_bytes: i64 = files.iter().map(|f| f.size).sum();
    let completed_telegram = files
        .iter()
        .filter(|f| f.pipeline_stage == "completed_telegram")
        .count() as i64;
    let completed_local = files
        .iter()
        .filter(|f| f.pipeline_stage == "completed_local")
        .count() as i64;
    let completed_bytes: i64 = files
        .iter()
        .filter(|f| {
            f.pipeline_stage == "completed_telegram" || f.pipeline_stage == "completed_local"
        })
        .map(|f| f.size)
        .sum();
    let failed_files = files
        .iter()
        .filter(|f| f.pipeline_stage == "failed")
        .count() as i64;
    let waiting_files = files
        .iter()
        .filter(|f| f.pipeline_stage == "waiting_for_quota")
        .count() as i64;
    let terminal_stages = [
        "completed_telegram",
        "completed_local",
        "failed",
        "reconciliation_required",
    ];
    let pending_files = files
        .iter()
        .filter(|f| !terminal_stages.contains(&f.pipeline_stage.as_str()))
        .count() as i64;

    let stats = MigrationStats {
        total_folders,
        total_files,
        total_bytes,
        completed_telegram,
        completed_local,
        completed_bytes,
        failed_files,
        waiting_files,
        pending_files,
    };

    // 4. Folders from folder_queue
    let mut folders = Vec::new();
    let mut fq_stmt = conn.prepare(
        "SELECT fq.folder_path, COUNT(mi.id) as file_count, COALESCE(SUM(mi.size), 0) as total_size \
         FROM folder_queue fq \
         LEFT JOIN migration_items mi ON mi.folder_id = fq.folder_id AND mi.job_id = fq.job_id \
         WHERE fq.job_id = ? \
         GROUP BY fq.folder_path \
         ORDER BY fq.id"
    ).map_err(|e| e.to_string())?;
    fq_stmt.bind((1, job_id)).map_err(|e| e.to_string())?;
    while let Ok(sqlite::State::Row) = fq_stmt.next() {
        let fpath: String = fq_stmt.read(0).unwrap_or_default();
        let fname = fpath.rsplit('/').next().unwrap_or("").to_string();
        folders.push(FolderSummary {
            source_path: fpath,
            name: fname,
            file_count: fq_stmt.read(1).unwrap_or(0),
            total_size: fq_stmt.read(2).unwrap_or(0),
        });
    }

    Ok(MigrationJobDetail {
        job,
        stats,
        folders,
        files,
    })
}

#[tauri::command]
pub async fn cmd_migration_get_resumable_job(
    state: State<'_, MigrationState>,
) -> Result<Option<i64>, String> {
    let _lifecycle = state.lifecycle_lock.lock().await;
    get_resumable_job_locked(&state).await
}

#[tauri::command]
pub async fn cmd_migration_resume(
    state: State<'_, MigrationState>,
    tg_state: State<'_, crate::commands::TelegramState>,
    app_handle: tauri::AppHandle,
    job_id: i64,
) -> Result<(), String> {
    let _lifecycle = state.lifecycle_lock.lock().await;
    ensure_no_active_pipeline(&state).await?;

    {
        let mut session_guard = state.ms_session.lock().await;
        let session = session_guard
            .as_mut()
            .ok_or_else(|| "Microsoft account not connected".to_string())?;
        if session.is_expired() {
            microsoft::refresh_access_token(session).await?;
            crate::migration::session_store::save(&app_handle, session)?;
        }
    }

    let (workspace_dir, backup_dir, telegram_destination_id, job_state) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT workspace_dir, local_backup_dir, telegram_destination_id, state \
                 FROM migration_jobs WHERE id = ?",
            )
            .map_err(|e| e.to_string())?;
        stmt.bind((1, job_id)).map_err(|e| e.to_string())?;
        if let Ok(sqlite::State::Row) = stmt.next() {
            (
                stmt.read::<String, _>(0).unwrap_or_default(),
                stmt.read::<String, _>(1).unwrap_or_default(),
                stmt.read::<Option<i64>, _>(2).ok().flatten(),
                stmt.read::<String, _>(3).unwrap_or_default(),
            )
        } else {
            return Err("Job not found".into());
        }
    };

    if !matches!(
        job_state.as_str(),
        "running" | "stopped" | "waiting_for_quota"
    ) {
        return Err(format!("Job cannot be resumed from state '{}'", job_state));
    }
    if !std::path::Path::new(&backup_dir).is_dir() {
        return Err(format!(
            "Local backup directory is unavailable: {}",
            backup_dir
        ));
    }
    std::fs::create_dir_all(&workspace_dir)
        .map_err(|e| format!("Cannot restore migration workspace: {}", e))?;

    let capabilities = crate::migration::adapters::media::preflight_media().await?;

    let mig_state = state.inner().clone_state();
    let services = crate::migration::adapters::factory::build_pipeline_services(
        mig_state.db.clone(),
        mig_state.ms_session.clone(),
        tg_state.client.clone(),
        tg_state.peer_cache.clone(),
        job_id,
        std::path::PathBuf::from(&workspace_dir),
        std::path::PathBuf::from(&backup_dir),
        telegram_destination_id,
        Some(app_handle),
        capabilities.selected_encoder,
    );
    let (runner, downloader, media_adapter, uploader, finalizer, cancel_token) = match services {
        Ok(services) => services,
        Err(error) => {
            persist_job_setup_failure(&state.db, job_id, &error)?;
            return Err(error);
        }
    };

    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let mut stmt = conn
            .prepare(
                "UPDATE migration_jobs \
                 SET state = 'running', completed_at = NULL, last_error = NULL, updated_at = ? \
                 WHERE id = ?",
            )
            .map_err(|e| e.to_string())?;
        stmt.bind((1, now)).map_err(|e| e.to_string())?;
        stmt.bind((2, job_id)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;
    }

    register_and_spawn_pipeline(
        mig_state,
        job_id,
        runner,
        downloader,
        media_adapter,
        uploader,
        finalizer,
        cancel_token,
    )
    .await?;

    Ok(())
}

#[tauri::command]
pub async fn cmd_migration_retry_failed(
    state: State<'_, MigrationState>,
    tg_state: State<'_, crate::commands::TelegramState>,
    app_handle: tauri::AppHandle,
    job_id: i64,
) -> Result<(), String> {
    let _lifecycle = state.lifecycle_lock.lock().await;
    ensure_no_active_pipeline(&state).await?;

    let (workspace_dir, backup_dir, telegram_destination_id, flood_wait_until, retry_items) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut job_stmt = conn
            .prepare("SELECT workspace_dir, local_backup_dir, telegram_destination_id, flood_wait_until FROM migration_jobs WHERE id = ?")
            .map_err(|e| e.to_string())?;
        job_stmt.bind((1, job_id)).map_err(|e| e.to_string())?;
        let job = if let Ok(sqlite::State::Row) = job_stmt.next() {
            (
                job_stmt.read::<String, _>(0).unwrap_or_default(),
                job_stmt.read::<String, _>(1).unwrap_or_default(),
                job_stmt.read::<Option<i64>, _>(2).ok().flatten(),
                job_stmt.read::<i64, _>(3).unwrap_or(0),
            )
        } else {
            return Err("Job not found".into());
        };
        drop(job_stmt);

        let mut load_stmt = conn
            .prepare(
                "SELECT id, name, path, source_item_id, size, pipeline_stage, original_artifact_path, processed_artifact_path, original_sha256, processed_sha256, video_decision, retry_count \
                 FROM migration_items WHERE job_id = ? AND pipeline_stage IN ('failed', 'waiting_for_quota')",
            )
            .map_err(|e| e.to_string())?;
        load_stmt.bind((1, job_id)).map_err(|e| e.to_string())?;
        let mut items = Vec::new();
        while let Ok(sqlite::State::Row) = load_stmt.next() {
            let stage: String = load_stmt.read(5).unwrap_or_default();
            items.push((
                crate::migration::pipeline::stages::PipelineItem {
                    id: load_stmt.read(0).unwrap_or(0),
                    job_id,
                    name: load_stmt.read(1).unwrap_or_default(),
                    source_path: load_stmt.read(2).unwrap_or_default(),
                    source_item_id: load_stmt.read::<Option<String>, _>(3).ok().flatten(),
                    size_bytes: load_stmt.read(4).unwrap_or(0),
                    source_etag: None,
                    source_last_modified: None,
                    source_fingerprint_type: None,
                    source_fingerprint_value: None,
                    state: stage,
                    original_sha256: load_stmt.read::<Option<String>, _>(8).ok().flatten(),
                    processed_sha256: load_stmt.read::<Option<String>, _>(9).ok().flatten(),
                    local_artifact_path: load_stmt.read::<Option<String>, _>(6).ok().flatten(),
                    processed_artifact_path: load_stmt.read::<Option<String>, _>(7).ok().flatten(),
                    telegram_random_id: None,
                    video_decision: load_stmt.read::<Option<String>, _>(10).ok().flatten(),
                },
                load_stmt.read::<i64, _>(11).unwrap_or(0),
            ));
        }
        (job.0, job.1, job.2, job.3, items)
    };

    let failed_folder_count = {
        let conn = state.db.lock().map_err(|error| error.to_string())?;
        let mut stmt = conn
            .prepare("SELECT COUNT(*) FROM folder_queue WHERE job_id = ? AND state = 'failed'")
            .map_err(|error| error.to_string())?;
        stmt.bind((1, job_id)).map_err(|error| error.to_string())?;
        if let Ok(sqlite::State::Row) = stmt.next() {
            stmt.read::<i64, _>(0).unwrap_or(0)
        } else {
            0
        }
    };
    if retry_items.is_empty() && failed_folder_count == 0 {
        return Err("No retryable migration items".to_string());
    }
    if !std::path::Path::new(&backup_dir).is_dir() {
        return Err(format!(
            "Local backup directory is unavailable: {}",
            backup_dir
        ));
    }
    std::fs::create_dir_all(&workspace_dir)
        .map_err(|error| format!("Cannot restore migration workspace: {}", error))?;

    let capabilities = crate::migration::adapters::media::preflight_media().await?;
    let validator = crate::migration::adapters::media::FFmpegMediaAdapter::new(
        std::path::PathBuf::from(if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        }),
        std::path::PathBuf::from(if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        }),
        tokio_util::sync::CancellationToken::new(),
        None,
        capabilities.selected_encoder.clone(),
    );
    let quota_ready = flood_wait_until <= chrono::Utc::now().timestamp();
    let mut updates = Vec::new();
    for (item, retry_count) in retry_items {
        if item.state == "waiting_for_quota" {
            if quota_ready {
                updates.push((
                    item,
                    crate::migration::pipeline::stages::PipelineStage::QueuedUpload,
                    retry_count,
                ));
            }
            continue;
        }

        let (new_stage, processed_valid) = determine_retry_route(&validator, &item).await;
        if !processed_valid {
            crate::migration::pipeline::runner::clear_processed_checkpoint(&state.db, &item)?;
        }
        updates.push((item, new_stage, retry_count + 1));
    }

    if updates.is_empty() && failed_folder_count == 0 {
        return Err("No retryable migration items".to_string());
    }

    for (item, stage, new_retry) in updates {
        crate::migration::pipeline::transitions::update_item_pipeline_stage(
            &state.db, item.id, stage,
        )?;
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut upd = conn
            .prepare("UPDATE migration_items SET retry_count = ?, last_error = NULL, updated_at = ? WHERE id = ?")
            .map_err(|e| e.to_string())?;
        upd.bind((1, new_retry)).map_err(|e| e.to_string())?;
        upd.bind((2, crate::migration::events::now_millis()))
            .map_err(|e| e.to_string())?;
        upd.bind((3, item.id)).map_err(|e| e.to_string())?;
        upd.next().map_err(|e| e.to_string())?;
    }

    // Also retry failed folders in folder_queue
    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut fq_upd = conn.prepare(
            "UPDATE folder_queue SET state = 'pending', last_error = NULL, updated_at = ? WHERE job_id = ? AND state = 'failed'"
        ).map_err(|e| e.to_string())?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        fq_upd.bind((1, now)).map_err(|e| e.to_string())?;
        fq_upd.bind((2, job_id)).map_err(|e| e.to_string())?;
        fq_upd.next().map_err(|e| e.to_string())?;
    }

    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut job_upd = conn
            .prepare("UPDATE migration_jobs SET state = 'running', completed_at = NULL, last_error = NULL, updated_at = ? WHERE id = ?")
            .map_err(|e| e.to_string())?;
        job_upd
            .bind((1, crate::migration::events::now_millis()))
            .map_err(|e| e.to_string())?;
        job_upd.bind((2, job_id)).map_err(|e| e.to_string())?;
        job_upd.next().map_err(|e| e.to_string())?;
    }

    // Start a new pipeline for this job
    let mig_state = state.inner().clone_state();
    let services = crate::migration::adapters::factory::build_pipeline_services(
        mig_state.db.clone(),
        mig_state.ms_session.clone(),
        tg_state.client.clone(),
        tg_state.peer_cache.clone(),
        job_id,
        std::path::PathBuf::from(&workspace_dir),
        std::path::PathBuf::from(&backup_dir),
        telegram_destination_id,
        Some(app_handle),
        capabilities.selected_encoder,
    );
    let (runner, downloader, media_adapter, uploader, finalizer, cancel_token) = match services {
        Ok(services) => services,
        Err(error) => {
            persist_job_setup_failure(&state.db, job_id, &error)?;
            return Err(error);
        }
    };
    register_and_spawn_pipeline(
        mig_state,
        job_id,
        runner,
        downloader,
        media_adapter,
        uploader,
        finalizer,
        cancel_token,
    )
    .await?;

    Ok(())
}

#[tauri::command]
pub async fn cmd_migration_reset_database(state: State<'_, MigrationState>) -> Result<(), String> {
    let _lifecycle = state.lifecycle_lock.lock().await;
    // Check if pipeline is running
    {
        let guard = state.active_pipeline.lock().await;
        if guard.is_some() {
            return Err("Cannot reset database while a migration pipeline is active. Stop the pipeline first.".into());
        }
    }

    let conn = state.db.lock().map_err(|e| e.to_string())?;
    crate::migration::db::reset_database(&conn)?;
    Ok(())
}

#[cfg(test)]
mod resume_tests {
    use super::{determine_retry_route, latest_resumable_job, mark_interrupted_job_stopped};
    use crate::migration::db::open_migration_db_at_path;
    use crate::migration::pipeline::stages::{
        MediaInspector, PipelineItem, PipelineStage, VideoMetadata,
    };
    use std::future::Future;
    use std::path::Path;
    use std::pin::Pin;

    struct RetryInspector;
    impl MediaInspector for RetryInspector {
        fn inspect_file(
            &self,
            path: &Path,
        ) -> Pin<Box<dyn Future<Output = Result<VideoMetadata, String>> + Send>> {
            let path = path.to_path_buf();
            Box::pin(async move {
                let marker = tokio::fs::read_to_string(path)
                    .await
                    .map_err(|error| error.to_string())?;
                if marker == "corrupt" {
                    return Err("ffprobe parse failed".to_string());
                }
                let (codec, profile, pixel_format) = match marker.as_str() {
                    "h264" => ("h264", "High", "yuv420p"),
                    "wrong-main10" => ("hevc", "Main 10", "yuv422p10le"),
                    _ => ("hevc", "Main", "yuv420p"),
                };
                Ok(VideoMetadata {
                    container_format_names: "mp4".to_string(),
                    video_codec: codec.to_string(),
                    audio_codec: "aac".to_string(),
                    duration: 1.0,
                    width: 320,
                    height: 240,
                    is_valid: true,
                    profile: profile.to_string(),
                    pixel_format: pixel_format.to_string(),
                    fps: 30.0,
                    major_brand: "isom".to_string(),
                    ..Default::default()
                })
            })
        }
    }

    fn retry_item(
        original: Option<&Path>,
        processed: Option<&Path>,
        decision: &str,
    ) -> PipelineItem {
        PipelineItem {
            id: 1,
            job_id: 1,
            name: "video.mp4".to_string(),
            source_path: "video.mp4".to_string(),
            source_item_id: Some("source".to_string()),
            size_bytes: 1,
            source_etag: None,
            source_last_modified: None,
            source_fingerprint_type: None,
            source_fingerprint_value: None,
            state: "failed".to_string(),
            original_sha256: None,
            processed_sha256: processed.map(|_| "hash".to_string()),
            local_artifact_path: original.map(|path| path.to_string_lossy().into_owned()),
            processed_artifact_path: processed.map(|path| path.to_string_lossy().into_owned()),
            telegram_random_id: Some(987654321),
            video_decision: Some(decision.to_string()),
        }
    }

    #[test]
    fn finds_and_marks_latest_interrupted_job() {
        let db_path = std::env::temp_dir().join(format!(
            "telegram-drive-resume-{}-{}.db",
            std::process::id(),
            rand::random::<u64>()
        ));
        let db = open_migration_db_at_path(db_path.clone()).unwrap();
        let conn = db.lock().unwrap();
        conn.execute("INSERT INTO migration_jobs (id, source_folder_id, source_folder_path, telegram_destination_name, local_backup_dir, workspace_dir, state, started_at, created_at, updated_at) VALUES (1, 'done', '/', 'Saved Messages', '/tmp', '/tmp/ws', 'completed', 1, 1, 10)").unwrap();
        conn.execute("INSERT INTO migration_jobs (id, source_folder_id, source_folder_path, telegram_destination_name, local_backup_dir, workspace_dir, state, started_at, created_at, updated_at) VALUES (2, 'active', '/', 'Saved Messages', '/tmp', '/tmp/ws', 'running', 2, 2, 20)").unwrap();

        let resumable = latest_resumable_job(&conn).unwrap();
        assert_eq!(resumable, Some((2, "running".to_string())));
        mark_interrupted_job_stopped(&conn, 2).unwrap();

        let mut stmt = conn
            .prepare(
                "SELECT state, completed_at IS NULL, last_error FROM migration_jobs WHERE id = 2",
            )
            .unwrap();
        assert_eq!(stmt.next().unwrap(), sqlite::State::Row);
        assert_eq!(stmt.read::<String, _>(0).unwrap(), "stopped");
        assert_eq!(stmt.read::<i64, _>(1).unwrap(), 1);
        assert_eq!(
            stmt.read::<String, _>(2).unwrap(),
            "Migration interrupted by app shutdown"
        );

        drop(stmt);
        conn.execute("INSERT INTO migration_jobs (id, source_folder_id, source_folder_path, telegram_destination_name, local_backup_dir, workspace_dir, state, started_at, created_at, updated_at) VALUES (3, 'newer', '/', 'Saved Messages', '/tmp', '/tmp/ws', 'completed', 3, 3, 9000000000000)").unwrap();
        assert_eq!(latest_resumable_job(&conn).unwrap(), None);

        drop(conn);
        drop(db);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn retry_routes_using_canonical_probe_validation_and_preserves_random_id() {
        let root = std::env::temp_dir().join(format!("retry-route-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        let original = root.join("original");
        std::fs::write(&original, "valid").unwrap();

        for (marker, decision, expected) in [
            (
                "valid",
                "canonical_transcode_main8",
                PipelineStage::QueuedUpload,
            ),
            (
                "h264",
                "canonical_transcode_main8",
                PipelineStage::QueuedProcessing,
            ),
            (
                "corrupt",
                "canonical_transcode_main8",
                PipelineStage::QueuedProcessing,
            ),
            (
                "wrong-main10",
                "canonical_transcode_main10",
                PipelineStage::QueuedProcessing,
            ),
        ] {
            let processed = root.join(format!("{}.processed.mp4", marker));
            std::fs::write(&processed, marker).unwrap();
            let item = retry_item(Some(&original), Some(&processed), decision);
            let (stage, processed_valid) = determine_retry_route(&RetryInspector, &item).await;
            assert_eq!(stage, expected, "route for {}", marker);
            assert_eq!(processed_valid, marker == "valid");
            assert_eq!(item.telegram_random_id, Some(987654321));
        }

        let zero = root.join("zero.processed.mp4");
        std::fs::write(&zero, []).unwrap();
        let zero_item = retry_item(Some(&original), Some(&zero), "canonical_transcode_main8");
        assert_eq!(
            determine_retry_route(&RetryInspector, &zero_item).await.0,
            PipelineStage::QueuedProcessing
        );

        let missing = retry_item(None, None, "canonical_transcode_main8");
        assert_eq!(
            determine_retry_route(&RetryInspector, &missing).await.0,
            PipelineStage::QueuedDownload
        );
        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::{
        clear_active_pipeline_if_matches, ensure_no_active_pipeline, get_resumable_job_locked,
        stop_active_pipeline,
    };
    use crate::migration::db::open_migration_db_at_path;
    use crate::migration::pipeline::config::PipelineConfig;
    use crate::migration::pipeline::runner::PipelineRunner;
    use crate::migration::{ActivePipeline, MigrationState};
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::watch;
    use tokio_util::sync::CancellationToken;

    fn test_state(name: &str) -> (Arc<MigrationState>, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "migration-lifecycle-{}-{}-{}.db",
            name,
            std::process::id(),
            rand::random::<u64>()
        ));
        let db = open_migration_db_at_path(path.clone()).unwrap();
        {
            let conn = db.lock().unwrap();
            conn.execute("INSERT INTO migration_jobs (id, source_folder_id, source_folder_path, telegram_destination_name, local_backup_dir, workspace_dir, state, started_at, created_at, updated_at) VALUES (1, 'root', '/', 'Saved Messages', '/tmp', '/tmp', 'running', 0, 0, 0)").unwrap();
            conn.execute("INSERT INTO migration_jobs (id, source_folder_id, source_folder_path, telegram_destination_name, local_backup_dir, workspace_dir, state, started_at, created_at, updated_at) VALUES (2, 'root-2', '/', 'Saved Messages', '/tmp', '/tmp', 'running', 0, 0, 0)").unwrap();
        }
        (Arc::new(MigrationState::new(db)), path)
    }

    fn active_pipeline(
        state: &MigrationState,
        job_id: i64,
    ) -> (ActivePipeline, watch::Sender<Option<Result<(), String>>>) {
        let cancel = CancellationToken::new();
        let runner = Arc::new(PipelineRunner::new(
            PipelineConfig::default(),
            state.db.clone(),
            job_id,
            std::env::temp_dir(),
            std::env::temp_dir(),
            state.ms_session.clone(),
            cancel.clone(),
            None,
            None,
        ));
        let (completion_tx, completion) = watch::channel(None);
        (
            ActivePipeline {
                job_id,
                runner,
                cancel_token: cancel,
                completion,
            },
            completion_tx,
        )
    }

    async fn concurrent_claims_only_one_succeeds(start_name: &str) {
        let (state, db_path) = test_state(start_name);
        let acquired = Arc::new(tokio::sync::Notify::new());
        let first_state = state.clone();
        let first_acquired = acquired.clone();
        let first = tokio::spawn(async move {
            let _lifecycle = first_state.lifecycle_lock.lock().await;
            ensure_no_active_pipeline(&first_state).await.unwrap();
            first_acquired.notify_one();
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            let (active, sender) = active_pipeline(&first_state, 1);
            *first_state.active_pipeline.lock().await = Some(active);
            sender
        });
        acquired.notified().await;
        let second_state = state.clone();
        let second = tokio::spawn(async move {
            let _lifecycle = second_state.lifecycle_lock.lock().await;
            ensure_no_active_pipeline(&second_state).await
        });
        let sender = first.await.unwrap();
        assert!(second.await.unwrap().is_err());
        drop(sender);
        drop(state);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn two_concurrent_starts_create_only_one_active_pipeline() {
        let path = std::env::temp_dir().join(format!(
            "migration-two-starts-{}-{}.db",
            std::process::id(),
            rand::random::<u64>()
        ));
        let state = Arc::new(MigrationState::new(
            open_migration_db_at_path(path.clone()).unwrap(),
        ));
        let acquired = Arc::new(tokio::sync::Notify::new());
        let first_state = state.clone();
        let first_acquired = acquired.clone();
        let first = tokio::spawn(async move {
            let _lifecycle = first_state.lifecycle_lock.lock().await;
            ensure_no_active_pipeline(&first_state).await?;
            first_acquired.notify_one();
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            let job_id = {
                let conn = first_state.db.lock().map_err(|error| error.to_string())?;
                crate::migration::db::create_job(
                    &conn,
                    "root",
                    "/",
                    None,
                    "Saved Messages",
                    "/tmp",
                    "/tmp",
                )?
            };
            let (active, sender) = active_pipeline(&first_state, job_id);
            *first_state.active_pipeline.lock().await = Some(active);
            Ok::<_, String>(sender)
        });
        acquired.notified().await;
        let second_state = state.clone();
        let second = tokio::spawn(async move {
            let _lifecycle = second_state.lifecycle_lock.lock().await;
            ensure_no_active_pipeline(&second_state).await?;
            let conn = second_state.db.lock().map_err(|error| error.to_string())?;
            crate::migration::db::create_job(
                &conn,
                "root-2",
                "/",
                None,
                "Saved Messages",
                "/tmp",
                "/tmp",
            )
        });
        let sender = first.await.unwrap().unwrap();
        assert!(second.await.unwrap().is_err());
        let conn = state.db.lock().unwrap();
        let mut stmt = conn.prepare("SELECT COUNT(*) FROM migration_jobs").unwrap();
        stmt.next().unwrap();
        assert_eq!(stmt.read::<i64, _>(0).unwrap(), 1);
        drop(stmt);
        drop(conn);
        drop(sender);
        drop(state);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn concurrent_start_and_retry_create_only_one_active_pipeline() {
        concurrent_claims_only_one_succeeds("start-retry").await;
    }

    #[tokio::test]
    async fn old_pipeline_completion_does_not_clear_new_active_pipeline() {
        let (state, db_path) = test_state("old-completion");
        let (active, sender) = active_pipeline(&state, 2);
        *state.active_pipeline.lock().await = Some(active);
        clear_active_pipeline_if_matches(&state, 1).await;
        assert_eq!(
            state
                .active_pipeline
                .lock()
                .await
                .as_ref()
                .map(|pipeline| pipeline.job_id),
            Some(2)
        );
        drop(sender);
        drop(state);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn stop_waits_for_completion_and_stopped_job_state() {
        let (state, db_path) = test_state("stop-completion");
        let (active, completion_tx) = active_pipeline(&state, 1);
        let cancel = active.cancel_token.clone();
        *state.active_pipeline.lock().await = Some(active);
        let completion_state = state.clone();
        tokio::spawn(async move {
            cancel.cancelled().await;
            {
                let conn = completion_state.db.lock().unwrap();
                conn.execute("UPDATE migration_jobs SET state = 'stopped' WHERE id = 1")
                    .unwrap();
            }
            clear_active_pipeline_if_matches(&completion_state, 1).await;
            completion_tx.send(Some(Ok(()))).unwrap();
        });

        let _lifecycle = state.lifecycle_lock.lock().await;
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            stop_active_pipeline(&state),
        )
        .await
        .expect("stop command semantics timed out")
        .unwrap();
        assert!(state.active_pipeline.lock().await.is_none());
        drop(_lifecycle);
        drop(state);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn get_resumable_waits_for_start_setup_and_keeps_running_job() {
        let (state, db_path) = test_state("resumable-race");
        let acquired = Arc::new(tokio::sync::Notify::new());
        let setup_state = state.clone();
        let setup_acquired = acquired.clone();
        let setup = tokio::spawn(async move {
            let _lifecycle = setup_state.lifecycle_lock.lock().await;
            setup_acquired.notify_one();
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            let (active, sender) = active_pipeline(&setup_state, 1);
            *setup_state.active_pipeline.lock().await = Some(active);
            sender
        });
        acquired.notified().await;
        let resumable_state = state.clone();
        let resumable = tokio::spawn(async move {
            let _lifecycle = resumable_state.lifecycle_lock.lock().await;
            get_resumable_job_locked(&resumable_state).await
        });
        let sender = setup.await.unwrap();
        assert_eq!(resumable.await.unwrap().unwrap(), Some(1));
        let conn = state.db.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT state FROM migration_jobs WHERE id = 1")
            .unwrap();
        stmt.next().unwrap();
        assert_eq!(stmt.read::<String, _>(0).unwrap(), "running");
        drop(stmt);
        drop(conn);
        drop(sender);
        drop(state);
        let _ = std::fs::remove_file(db_path);
    }
}
