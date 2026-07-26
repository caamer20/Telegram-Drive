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
        
        let mut stmt = conn.prepare(
            "INSERT INTO folder_queue (job_id, folder_id, folder_path, state) VALUES (?, ?, ?, 'pending')"
        ).map_err(|e| e.to_string())?;
        stmt.bind((1, jid)).unwrap();
        stmt.bind((2, source_folder_id.as_str())).unwrap();
        stmt.bind((3, source_folder_path.as_str())).unwrap();
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
) -> Result<serde_json::Value, String> {
    // Check if pipeline is running
    let is_running = {
        let active_guard = state.active_pipeline.lock().await;
        active_guard.as_ref().map_or(false, |p| p.job_id == job_id)
    };
    
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    
    // Get stats from migration_items
    let mut stmt = conn.prepare(
        "SELECT pipeline_stage, COUNT(*) FROM migration_items WHERE job_id = ? GROUP BY pipeline_stage;"
    ).map_err(|e| e.to_string())?;
    stmt.bind((1, job_id)).map_err(|e| e.to_string())?;
    
    let mut item_stats = serde_json::Map::new();
    let mut total_items = 0;
    while let Ok(sqlite::State::Row) = stmt.next() {
        let stage: String = stmt.read(0).unwrap_or_default();
        let count: i64 = stmt.read(1).unwrap_or(0);
        item_stats.insert(stage, serde_json::json!(count));
        total_items += count;
    }
    
    // Get stats from folder_queue
    let mut stmt = conn.prepare(
        "SELECT state, COUNT(*) FROM folder_queue WHERE job_id = ? GROUP BY state;"
    ).map_err(|e| e.to_string())?;
    stmt.bind((1, job_id)).map_err(|e| e.to_string())?;
    
    let mut folder_stats = serde_json::Map::new();
    let mut total_folders = 0;
    while let Ok(sqlite::State::Row) = stmt.next() {
        let folder_state: String = stmt.read(0).unwrap_or_default();
        let count: i64 = stmt.read(1).unwrap_or(0);
        folder_stats.insert(folder_state, serde_json::json!(count));
        total_folders += count;
    }
    
    Ok(serde_json::json!({
        "job_id": job_id,
        "is_running": is_running,
        "total_items": total_items,
        "item_stats": item_stats,
        "total_folders": total_folders,
        "folder_stats": folder_stats,
    }))
}

#[tauri::command]
pub async fn cmd_migration_retry_failed(
    state: State<'_, MigrationState>,
    job_id: i64,
) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "UPDATE migration_items SET pipeline_stage = 'discovered' WHERE job_id = ? AND pipeline_stage = 'failed';"
    ).map_err(|e| e.to_string())?;
    stmt.bind((1, job_id)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;
    
    let mut stmt2 = conn.prepare(
        "UPDATE folder_queue SET state = 'pending' WHERE job_id = ? AND state = 'failed';"
    ).map_err(|e| e.to_string())?;
    stmt2.bind((1, job_id)).map_err(|e| e.to_string())?;
    stmt2.next().map_err(|e| e.to_string())?;
    
    Ok(())
}

#[tauri::command]
pub async fn cmd_migration_reset_database(
    state: State<'_, MigrationState>,
) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    crate::migration::db::reset_database(&conn)?;
    Ok(())
}
