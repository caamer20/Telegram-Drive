use tauri::{Manager, State};

use crate::migration::microsoft;
use crate::migration::models::*;
use crate::migration::MigrationState;

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
        return Err(
            "Local backup directory and workspace directory must be different".into(),
        );
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
        
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
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

        let (runner, downloader, media_adapter, uploader, finalizer, _cancel) = match crate::migration::adapters::factory::build_pipeline_services(
            mig_state.db.clone(),
            mig_state.ms_session.clone(),
            tg_client_arc,
            tg_peer_cache_arc,
            job_id,
            PathBuf::from(&workspace_dir),
            PathBuf::from(&local_backup_dir),
            telegram_destination_id,
        ) {
            Ok(services) => services,
            Err(e) => {
                log::error!("Failed to build pipeline services: {}", e);
                return;
            }
        };

        let cancel_token = runner.clone().start(
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
pub async fn cmd_migration_stop(
    state: State<'_, MigrationState>,
) -> Result<(), String> {
    let guard = state.active_pipeline.lock().await;
    if let Some(active) = guard.as_ref() {
        active.cancel_token.stop();
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
    let mut job_stmt = conn.prepare(
        "SELECT id, source_folder_id, source_folder_path, telegram_destination_id, \
         telegram_destination_name, local_backup_dir, workspace_dir, state, \
         started_at, completed_at, last_error, flood_wait_until, \
         discovered_folders, completed_folders, discovered_items, completed_items, \
         failed_items, waiting_items, created_at, updated_at \
         FROM migration_jobs WHERE id = ?"
    ).map_err(|e| e.to_string())?;
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
    let mut files_stmt = conn.prepare(
        "SELECT id, job_id, folder_id, source_item_id, name, path, size, item_category, \
         pipeline_stage, original_artifact_path, processed_artifact_path, \
         original_sha256, processed_sha256, video_decision, artifact_size, \
         telegram_attempt_id, telegram_random_id, telegram_message_id, \
         retry_count, last_error, created_at, updated_at, completed_at \
         FROM migration_items WHERE job_id = ?"
    ).map_err(|e| e.to_string())?;
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
    let completed_telegram = files.iter().filter(|f| f.pipeline_stage == "completed_telegram").count() as i64;
    let completed_local = files.iter().filter(|f| f.pipeline_stage == "completed_local").count() as i64;
    let completed_bytes: i64 = files.iter()
        .filter(|f| f.pipeline_stage == "completed_telegram" || f.pipeline_stage == "completed_local")
        .map(|f| f.size).sum();
    let failed_files = files.iter().filter(|f| f.pipeline_stage == "failed").count() as i64;
    let waiting_files = files.iter().filter(|f| f.pipeline_stage == "waiting_for_quota").count() as i64;
    let terminal_stages = ["completed_telegram", "completed_local", "failed", "reconciliation_required"];
    let pending_files = files.iter().filter(|f| !terminal_stages.contains(&f.pipeline_stage.as_str())).count() as i64;

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
pub async fn cmd_migration_retry_failed(
    state: State<'_, MigrationState>,
    job_id: i64,
) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    
    // Load failed items
    let mut load_stmt = conn.prepare(
        "SELECT id, pipeline_stage, original_artifact_path, processed_artifact_path, \
         original_sha256, processed_sha256, video_decision, retry_count \
         FROM migration_items WHERE job_id = ? AND pipeline_stage = 'failed'"
    ).map_err(|e| e.to_string())?;
    load_stmt.bind((1, job_id)).map_err(|e| e.to_string())?;
    
    let mut updates: Vec<(i64, String, i64)> = Vec::new();
    while let Ok(sqlite::State::Row) = load_stmt.next() {
        let item_id: i64 = load_stmt.read(0).unwrap_or(0);
        let processed_path: Option<String> = load_stmt.read(3).ok();
        let original_path: Option<String> = load_stmt.read(2).ok();
        let retry_count: i64 = load_stmt.read(7).unwrap_or(0);
        
        let new_stage = if processed_path.is_some() {
            // Has processed artifact -> retry from upload
            "queued_upload"
        } else if original_path.is_some() {
            // Has original artifact -> retry from processing/routing
            "queued_processing"
        } else {
            // No artifacts -> retry from download
            "queued_download"
        };
        
        updates.push((item_id, new_stage.to_string(), retry_count + 1));
    }
    
    // Apply updates
    for (item_id, stage, new_retry) in updates {
        let mut upd = conn.prepare(
            "UPDATE migration_items SET pipeline_stage = ?, retry_count = ?, last_error = NULL, updated_at = ? WHERE id = ?"
        ).map_err(|e| e.to_string())?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        upd.bind((1, stage.as_str())).map_err(|e| e.to_string())?;
        upd.bind((2, new_retry)).map_err(|e| e.to_string())?;
        upd.bind((3, now)).map_err(|e| e.to_string())?;
        upd.bind((4, item_id)).map_err(|e| e.to_string())?;
        upd.next().map_err(|e| e.to_string())?;
    }
    
    // Also retry failed folders in folder_queue
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
    
    Ok(())
}

#[tauri::command]
pub async fn cmd_migration_reset_database(
    state: State<'_, MigrationState>,
) -> Result<(), String> {
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
