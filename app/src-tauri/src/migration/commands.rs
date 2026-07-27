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
        let mut stmt = conn.prepare(
            "INSERT INTO folder_queue (job_id, folder_id, folder_path, state, created_at, updated_at) VALUES (?, ?, ?, 'pending', ?, ?)"
        ).map_err(|e| e.to_string())?;
        stmt.bind((1, jid)).unwrap();
        stmt.bind((2, source_folder_id.as_str())).unwrap();
        stmt.bind((3, source_folder_path.as_str())).unwrap();
        stmt.bind((4, now)).unwrap();
        stmt.bind((5, now)).unwrap();
        stmt.next().unwrap();
        jid
    };

    // 4. Start pipeline
    let mig_state = state.inner().clone_state();
    let tg_client_arc = tg_state.client.clone();
    let tg_peer_cache_arc = tg_state.peer_cache.clone();

    tauri::async_runtime::spawn(async move {
        use std::path::PathBuf;

        let (runner, downloader, media_adapter, uploader, finalizer, cancel_token) =
            match crate::migration::adapters::factory::build_pipeline_services(
                mig_state.db.clone(),
                mig_state.ms_session.clone(),
                tg_client_arc,
                tg_peer_cache_arc,
                job_id,
                PathBuf::from(&workspace_dir),
                PathBuf::from(&local_backup_dir),
                telegram_destination_id,
                Some(app_handle.clone()),
            ) {
                Ok(services) => services,
                Err(e) => {
                    log::error!("Failed to build pipeline services: {}", e);
                    return;
                }
            };

        runner.clone().start(
            downloader,
            media_adapter.clone(),
            media_adapter,
            uploader,
            finalizer,
        );

        let mut active_guard = mig_state.active_pipeline.lock().await;
        *active_guard = Some(crate::migration::ActivePipeline {
            job_id,
            runner: runner.clone(),
            cancel_token,
        });
        drop(active_guard);

        if let Err(e) = runner.run_to_completion().await {
            log::error!("Pipeline failed for job {}: {}", job_id, e);
        }

        let mut active_guard = mig_state.active_pipeline.lock().await;
        *active_guard = None;
    });

    Ok(job_id)
}

#[tauri::command]
pub async fn cmd_migration_stop(state: State<'_, MigrationState>) -> Result<(), String> {
    let guard = state.active_pipeline.lock().await;
    if let Some(active) = guard.as_ref() {
        // Set stopped_by_user flag before cancelling
        active
            .runner
            .stopped_by_user
            .store(true, std::sync::atomic::Ordering::Relaxed);
        active.cancel_token.cancel();
        Ok(())
    } else {
        Err("No active migration".into())
    }
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
    {
        let active_guard = state.active_pipeline.lock().await;
        if let Some(active) = active_guard.as_ref() {
            return Ok(Some(active.job_id));
        }
    }

    let conn = state.db.lock().map_err(|e| e.to_string())?;
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

#[tauri::command]
pub async fn cmd_migration_resume(
    state: State<'_, MigrationState>,
    tg_state: State<'_, crate::commands::TelegramState>,
    app_handle: tauri::AppHandle,
    job_id: i64,
) -> Result<(), String> {
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

    let mut active_guard = state.active_pipeline.lock().await;
    if let Some(active) = active_guard.as_ref() {
        if active.job_id == job_id {
            return Err("This migration is already running".into());
        }
        return Err("Another migration pipeline is active".into());
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

    let mig_state = state.inner().clone_state();
    let (runner, downloader, media_adapter, uploader, finalizer, cancel_token) =
        crate::migration::adapters::factory::build_pipeline_services(
            mig_state.db.clone(),
            mig_state.ms_session.clone(),
            tg_state.client.clone(),
            tg_state.peer_cache.clone(),
            job_id,
            std::path::PathBuf::from(&workspace_dir),
            std::path::PathBuf::from(&backup_dir),
            telegram_destination_id,
            Some(app_handle),
        )?;

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

    runner.clone().start(
        downloader,
        media_adapter.clone(),
        media_adapter,
        uploader,
        finalizer,
    );
    *active_guard = Some(crate::migration::ActivePipeline {
        job_id,
        runner: runner.clone(),
        cancel_token,
    });
    drop(active_guard);

    tauri::async_runtime::spawn(async move {
        if let Err(error) = runner.run_to_completion().await {
            log::error!("Resumed pipeline failed for job {}: {}", job_id, error);
        }

        let mut active_guard = mig_state.active_pipeline.lock().await;
        if active_guard.as_ref().map(|active| active.job_id) == Some(job_id) {
            *active_guard = None;
        }
    });

    Ok(())
}

#[tauri::command]
pub async fn cmd_migration_retry_failed(
    state: State<'_, MigrationState>,
    tg_state: State<'_, crate::commands::TelegramState>,
    app_handle: tauri::AppHandle,
    job_id: i64,
) -> Result<(), String> {
    // Check if a pipeline is already active
    {
        let guard = state.active_pipeline.lock().await;
        if let Some(active) = guard.as_ref() {
            if active.job_id == job_id {
                return Err(
                    "A pipeline is already active for this job. Stop it first before retrying."
                        .into(),
                );
            }
            return Err("Another migration pipeline is active. Stop it first.".into());
        }
    }

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
    let tg_client_arc = tg_state.client.clone();
    let tg_peer_cache_arc = tg_state.peer_cache.clone();

    tauri::async_runtime::spawn(async move {
        let (runner, downloader, media_adapter, uploader, finalizer, cancel_token) =
            match crate::migration::adapters::factory::build_pipeline_services(
                mig_state.db.clone(),
                mig_state.ms_session.clone(),
                tg_client_arc,
                tg_peer_cache_arc,
                job_id,
                std::path::PathBuf::from(&workspace_dir),
                std::path::PathBuf::from(&backup_dir),
                telegram_destination_id,
                Some(app_handle.clone()),
            ) {
                Ok(services) => services,
                Err(e) => {
                    log::error!("Retry: Failed to build pipeline services: {}", e);
                    return;
                }
            };

        runner.clone().start(
            downloader,
            media_adapter.clone(),
            media_adapter,
            uploader,
            finalizer,
        );

        let mut active_guard = mig_state.active_pipeline.lock().await;
        *active_guard = Some(crate::migration::ActivePipeline {
            job_id,
            runner: runner.clone(),
            cancel_token,
        });
        drop(active_guard);

        if let Err(e) = runner.run_to_completion().await {
            log::error!("Retry: Pipeline failed for job {}: {}", job_id, e);
        }

        let mut active_guard = mig_state.active_pipeline.lock().await;
        *active_guard = None;
    });

    Ok(())
}

#[tauri::command]
pub async fn cmd_migration_reset_database(state: State<'_, MigrationState>) -> Result<(), String> {
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
