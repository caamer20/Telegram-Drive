// Pipeline V2 Backup Commands
// Preflight, Start, Pause, Resume, Stop, GetStatus, RetryManifest

use std::sync::Arc;
use tauri::{Emitter, Manager, State};

use crate::migration::microsoft;
use crate::migration::MigrationState;

/// Preflight: validate setup, scan folder, classify files, create V2 job
#[tauri::command]
pub async fn cmd_backup_v2_preflight(
    state: State<'_, MigrationState>,
    app_handle: tauri::AppHandle,
    source_folder_id: String,
    source_folder_path: String,
    telegram_destination_id: Option<i64>,
    telegram_destination_name: String,
    local_backup_dir: String,
    workspace_dir: String,
) -> Result<serde_json::Value, String> {
    // 1. Validate OneDrive session
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

    // 2. Validate local directories
    let backup_path = std::path::Path::new(&local_backup_dir);
    if !backup_path.exists() {
        return Err(format!(
            "Local backup directory does not exist: {}",
            local_backup_dir
        ));
    }
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

    // 3. Scan selected folder
    let http = reqwest::Client::new();
    let items = microsoft::scan_folder_recursive(
        &http,
        &access_token,
        &source_folder_id,
        &source_folder_path,
    )
    .await?;

    // 4. Classify and calculate stats
    let mut total_files: i64 = 0;
    let mut total_bytes: i64 = 0;
    let mut video_count: i64 = 0;
    let mut video_bytes: i64 = 0;
    let mut image_count: i64 = 0;
    let mut image_bytes: i64 = 0;
    let mut other_count: i64 = 0;
    let mut other_bytes: i64 = 0;

    for item in &items {
        if item.item_type == "file" {
            total_files += 1;
            total_bytes += item.size;

            let ext = std::path::Path::new(&item.name)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            match ext.as_str() {
                "mp4" | "mkv" | "mov" | "webm" => {
                    video_count += 1;
                    video_bytes += item.size;
                }
                "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" => {
                    image_count += 1;
                    image_bytes += item.size;
                }
                _ => {
                    other_count += 1;
                    other_bytes += item.size;
                }
            }
        }
    }

    let telegram_bytes = video_bytes + image_bytes;

    // 5. Check disk availability (simplified)
    let disk_available: i64 = {
        #[cfg(not(target_os = "windows"))]
        {
            use std::os::unix::fs::MetadataExt;
            std::fs::metadata(&backup_canon)
                .map(|m| (m.blocks() as i64) * 512)
                .unwrap_or(0)
        }
        #[cfg(target_os = "windows")]
        {
            i64::MAX
        }
    };

    // 6. Read remaining quota
    let date_string = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let quota_used = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        crate::migration::quota_reserve::get_daily_used_bytes(&conn, &date_string).unwrap_or(0)
    };
    let quota_remaining =
        crate::migration::quota_reserve::DAILY_SAFETY_BUDGET_LIMIT - quota_used;

    // 7. Create job with pipeline_version = 2
    let job_id = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let mut stmt = conn
            .prepare(
                "INSERT INTO migration_jobs (state, pipeline_version, onedrive_folder_id, onedrive_folder_path, telegram_destination_id, telegram_destination_name, local_dir, local_backup_dir, workspace_dir, created_at, updated_at)
                 VALUES ('pending', 2, ?, ?, ?, ?, ?, ?, ?, ?, ?);",
            )
            .map_err(|e| e.to_string())?;
        stmt.bind((1, source_folder_id.as_str()))
            .map_err(|e| e.to_string())?;
        stmt.bind((2, source_folder_path.as_str()))
            .map_err(|e| e.to_string())?;
        stmt.bind((3, telegram_destination_id.unwrap_or(0)))
            .map_err(|e| e.to_string())?;
        stmt.bind((4, telegram_destination_name.as_str()))
            .map_err(|e| e.to_string())?;
        stmt.bind((5, workspace_dir.as_str()))
            .map_err(|e| e.to_string())?;
        stmt.bind((6, local_backup_dir.as_str()))
            .map_err(|e| e.to_string())?;
        stmt.bind((7, workspace_dir.as_str()))
            .map_err(|e| e.to_string())?;
        stmt.bind((8, now)).map_err(|e| e.to_string())?;
        stmt.bind((9, now)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;
        drop(stmt);

        let mut id_stmt = conn
            .prepare("SELECT last_insert_rowid();")
            .map_err(|e| e.to_string())?;
        id_stmt.next().map_err(|e| e.to_string())?;
        let jid: i64 = id_stmt.read(0).map_err(|e| e.to_string())?;
        drop(id_stmt);

        // 8. Seed items in DB
        for item in &items {
            if item.item_type == "file" {
                let ext = std::path::Path::new(&item.name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                let route_kind = match ext.as_str() {
                    "mp4" | "mkv" | "mov" | "webm" => "video_to_telegram",
                    "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" => "image_to_telegram",
                    _ => "other_to_local",
                };

                let mut ins = conn
                    .prepare(
                        "INSERT INTO migration_items (job_id, name, source_path, source_item_id, size_bytes, item_type, state, pipeline_stage, route_kind, created_at)
                         VALUES (?, ?, ?, ?, ?, 'file', 'pending', 'discovered', ?, ?);",
                    )
                    .map_err(|e| e.to_string())?;
                ins.bind((1, jid)).map_err(|e| e.to_string())?;
                ins.bind((2, item.name.as_str()))
                    .map_err(|e| e.to_string())?;
                ins.bind((3, item.path.as_deref().unwrap_or(&item.name)))
                    .map_err(|e| e.to_string())?;
                ins.bind((4, item.id.as_str()))
                    .map_err(|e| e.to_string())?;
                ins.bind((5, item.size))
                    .map_err(|e| e.to_string())?;
                ins.bind((6, route_kind)).map_err(|e| e.to_string())?;
                ins.bind((7, now)).map_err(|e| e.to_string())?;
                ins.next().map_err(|e| e.to_string())?;
            }
        }
        jid
    };

    Ok(serde_json::json!({
        "job_id": job_id,
        "total_files": total_files,
        "total_bytes": total_bytes,
        "video_count": video_count,
        "video_bytes": video_bytes,
        "image_count": image_count,
        "image_bytes": image_bytes,
        "other_count": other_count,
        "other_bytes": other_bytes,
        "telegram_bytes": telegram_bytes,
        "local_bytes": other_bytes,
        "quota_remaining": quota_remaining,
        "disk_available": disk_available,
        "warnings": [],
        "valid": true,
    }))
}

/// Start V2 backup pipeline
#[tauri::command]
pub async fn cmd_backup_v2_start(
    state: State<'_, MigrationState>,
    app_handle: tauri::AppHandle,
    job_id: i64,
) -> Result<(), String> {
    // Check if already running
    {
        let guard = state.active_pipeline_v2.lock().await;
        if guard.is_some() {
            return Err("A backup is already running".into());
        }
    }

    // Get job info
    let (workspace_dir, backup_dir, dest_folder_id) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT pipeline_version, workspace_dir, local_backup_dir, telegram_destination_id FROM migration_jobs WHERE id = ? LIMIT 1;",
            )
            .map_err(|e| e.to_string())?;
        stmt.bind((1, job_id)).map_err(|e| e.to_string())?;

        if let Ok(sqlite::State::Row) = stmt.next() {
            let v: i64 = stmt.read(0).unwrap_or(1);
            if v != 2 {
                return Err(
                    "This job uses V1 pipeline. Use the legacy migration start instead.".into(),
                );
            }
            let w: String = stmt.read(1).unwrap_or_default();
            let b: String = stmt.read(2).unwrap_or_default();
            let d: Option<i64> = stmt.read(3).unwrap_or(None);
            (w, b, d)
        } else {
            return Err("Job not found".into());
        }
    };

    // Get Telegram state — convert tokio::sync::Mutex → std::sync::Mutex for factory
    let tg_state = app_handle
        .try_state::<crate::commands::TelegramState>()
        .ok_or("Telegram not initialized")?;
    let tg_client: Arc<std::sync::Mutex<Option<grammers_client::Client>>> = {
        let guard = tg_state.client.lock().await;
        Arc::new(std::sync::Mutex::new(guard.clone()))
    };
    let tg_peer_cache = tg_state.peer_cache.clone();

    // Build pipeline services
    let (runner, _downloader, _media, _telegram, _local, cancel_token) =
        crate::migration::adapters_v2::factory::build_pipeline_v2_services(
            state.db.clone(),
            state.ms_session.clone(),
            tg_client,
            tg_peer_cache,
            job_id,
            std::path::PathBuf::from(&workspace_dir),
            std::path::PathBuf::from(&backup_dir),
            dest_folder_id,
        )?;

    // Mark job as running
    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let mut upd = conn
            .prepare(
                "UPDATE migration_jobs SET state = 'running', updated_at = ? WHERE id = ?;",
            )
            .map_err(|e| e.to_string())?;
        upd.bind((1, now)).map_err(|e| e.to_string())?;
        upd.bind((2, job_id)).map_err(|e| e.to_string())?;
        upd.next().map_err(|e| e.to_string())?;
    }

    // Store active handle — use the runner's CancellationToken
    let runner_cancel = runner.cancel_token.clone();
    {
        let mut guard = state.active_pipeline_v2.lock().await;
        *guard = Some(crate::migration::ActivePipelineV2 {
            job_id,
            runner: runner.clone(),
            cancel_token: runner_cancel,
        });
    }

    // Spawn pipeline execution (fire-and-forget with completion handling)
    let mig_state = state.inner().clone_state();
    tauri::async_runtime::spawn(async move {
        let _ = runner.run_to_completion().await;

        // Mark job completed
        if let Ok(conn) = mig_state.db.lock() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let _ = conn.execute(format!(
                "UPDATE migration_jobs SET state = 'completed', completed_at = {}, updated_at = {} WHERE id = {};",
                now, now, job_id
            ));
        }

        // Clear active state
        let mut guard = mig_state.active_pipeline_v2.lock().await;
        *guard = None;

        // Emit completion event
        let _ = app_handle.emit(
            "backup-v2:completed",
            serde_json::json!({ "job_id": job_id }),
        );
    });

    Ok(())
}

/// Pause V2 backup
#[tauri::command]
pub async fn cmd_backup_v2_pause(
    state: State<'_, MigrationState>,
) -> Result<(), String> {
    let guard = state.active_pipeline_v2.lock().await;
    if let Some(active) = guard.as_ref() {
        active.cancel_token.pause();
        Ok(())
    } else {
        Err("No active backup".into())
    }
}

/// Resume V2 backup
#[tauri::command]
pub async fn cmd_backup_v2_resume(
    state: State<'_, MigrationState>,
) -> Result<(), String> {
    let guard = state.active_pipeline_v2.lock().await;
    if let Some(active) = guard.as_ref() {
        active.cancel_token.resume();
        Ok(())
    } else {
        Err("No active backup".into())
    }
}

/// Stop V2 backup
#[tauri::command]
pub async fn cmd_backup_v2_stop(
    state: State<'_, MigrationState>,
) -> Result<(), String> {
    let guard = state.active_pipeline_v2.lock().await;
    if let Some(active) = guard.as_ref() {
        active.cancel_token.stop();
        Ok(())
    } else {
        Err("No active backup".into())
    }
}

/// Get V2 backup job status
#[tauri::command]
pub async fn cmd_backup_v2_get_status(
    state: State<'_, MigrationState>,
    job_id: i64,
) -> Result<serde_json::Value, String> {
    // Gather all DB data in a block, then drop conn before awaits
    let (job_state, stats, quota_used, quota_remaining, flood_wait_secs, manifest_state) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;

        // Job state
        let mut job_stmt = conn
            .prepare("SELECT state FROM migration_jobs WHERE id = ? LIMIT 1;")
            .map_err(|e| e.to_string())?;
        job_stmt.bind((1, job_id)).map_err(|e| e.to_string())?;

        let job_state = if let Ok(sqlite::State::Row) = job_stmt.next() {
            job_stmt.read::<String, _>(0).unwrap_or_default()
        } else {
            return Err("Job not found".into());
        };

        // Stats
        let mut count_stmt = conn
            .prepare(
                "SELECT
                    COUNT(*) as total,
                    SUM(CASE WHEN pipeline_stage = 'completed_telegram' THEN 1 ELSE 0 END) as completed_telegram,
                    SUM(CASE WHEN pipeline_stage = 'completed_local' THEN 1 ELSE 0 END) as completed_local,
                    SUM(CASE WHEN pipeline_stage = 'skipped_duplicate' THEN 1 ELSE 0 END) as skipped_duplicate,
                    SUM(CASE WHEN pipeline_stage = 'failed' THEN 1 ELSE 0 END) as failed,
                    SUM(CASE WHEN pipeline_stage = 'reconciliation_required' THEN 1 ELSE 0 END) as reconciliation_required,
                    SUM(CASE WHEN pipeline_stage = 'waiting_for_quota' THEN 1 ELSE 0 END) as waiting_for_quota
                 FROM migration_items WHERE job_id = ? AND duplicate_of_item_id IS NULL;",
            )
            .map_err(|e| e.to_string())?;
        count_stmt.bind((1, job_id)).map_err(|e| e.to_string())?;

        let mut stats = serde_json::json!({});
        if let Ok(sqlite::State::Row) = count_stmt.next() {
            stats = serde_json::json!({
                "total_items": count_stmt.read::<i64, _>(0).unwrap_or(0),
                "completed_telegram": count_stmt.read::<i64, _>(1).unwrap_or(0),
                "completed_local": count_stmt.read::<i64, _>(2).unwrap_or(0),
                "skipped_duplicates": count_stmt.read::<i64, _>(3).unwrap_or(0),
                "failed_items": count_stmt.read::<i64, _>(4).unwrap_or(0),
                "reconciliation_required": count_stmt.read::<i64, _>(5).unwrap_or(0),
                "waiting_for_quota": count_stmt.read::<i64, _>(6).unwrap_or(0),
            });
        }

        // Quota
        let date_string = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let quota_used =
            crate::migration::quota_reserve::get_daily_used_bytes(&conn, &date_string).unwrap_or(0);
        let quota_remaining =
            crate::migration::quota_reserve::DAILY_SAFETY_BUDGET_LIMIT - quota_used;

        // Flood wait
        let mut flood_stmt = conn
            .prepare("SELECT next_allowed_at FROM migration_pacing_state WHERE key = 'next_allowed_at' LIMIT 1;")
            .map_err(|e| e.to_string())?;
        let flood_wait_secs: Option<i64> = if let Ok(sqlite::State::Row) = flood_stmt.next() {
            let next_allowed: i64 = flood_stmt.read(0).unwrap_or(0);
            let now = chrono::Utc::now().timestamp();
            if next_allowed > now {
                Some(next_allowed - now)
            } else {
                None
            }
        } else {
            None
        };

        // Manifest state
        let mut manifest_stmt = conn
            .prepare("SELECT manifest_state FROM migration_jobs WHERE id = ? LIMIT 1;")
            .map_err(|e| e.to_string())?;
        manifest_stmt.bind((1, job_id)).map_err(|e| e.to_string())?;
        let manifest_state: String = if let Ok(sqlite::State::Row) = manifest_stmt.next() {
            manifest_stmt.read(0).unwrap_or_else(|_| "pending".into())
        } else {
            "pending".into()
        };

        (job_state, stats, quota_used, quota_remaining, flood_wait_secs, manifest_state)
    }; // conn and all statements dropped here

    // Is active — uses async lock
    let is_active = {
        let guard = state.active_pipeline_v2.lock().await;
        guard
            .as_ref()
            .map(|a| a.job_id == job_id)
            .unwrap_or(false)
    };

    Ok(serde_json::json!({
        "job_id": job_id,
        "state": job_state,
        "is_active": is_active,
        "stats": stats,
        "quota_used": quota_used,
        "quota_remaining": quota_remaining,
        "flood_wait_secs": flood_wait_secs,
        "manifest_state": manifest_state,
    }))
}

/// Retry manifest export
#[tauri::command]
pub async fn cmd_backup_v2_retry_manifest(
    state: State<'_, MigrationState>,
    job_id: i64,
) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut upd = conn
        .prepare("UPDATE migration_jobs SET manifest_state = 'export_pending' WHERE id = ?;")
        .map_err(|e| e.to_string())?;
    upd.bind((1, job_id)).map_err(|e| e.to_string())?;
    upd.next().map_err(|e| e.to_string())?;

    Ok(())
}
