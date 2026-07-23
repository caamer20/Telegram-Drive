use reqwest::Client;
use tauri::{AppHandle, Manager};

use crate::migration::db::*;
use crate::migration::microsoft::*;
use crate::migration::worker::*;
use crate::migration::MigrationState;

enum AutoJobCheck {
    AlreadyRunning,
    Resume(i64),
    NeedScan,
}

fn check_existing_job(db: &MigrationDb) -> Result<AutoJobCheck, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id, state FROM migration_jobs WHERE state IN ('running', 'ready', 'paused') ORDER BY id DESC LIMIT 1;")
        .map_err(|e| e.to_string())?;

    if let Ok(sqlite::State::Row) = stmt.next() {
        let job_id: i64 = stmt.read(0).unwrap_or(0);
        let state_str: String = stmt.read(1).unwrap_or_default();
        if state_str == "running" {
            Ok(AutoJobCheck::AlreadyRunning)
        } else {
            Ok(AutoJobCheck::Resume(job_id))
        }
    } else {
        Ok(AutoJobCheck::NeedScan)
    }
}

pub async fn start_auto_engine(app_handle: AppHandle) -> Result<(), String> {
    let mig_state = app_handle.state::<MigrationState>();

    // 1. Check MS session
    let (access_token, account_email) = {
        let mut session_guard = mig_state.ms_session.lock().await;
        if let Some(ref mut session) = *session_guard {
            if session.is_expired() {
                if let Err(e) = refresh_access_token(DEFAULT_MS_CLIENT_ID, session).await {
                    log::warn!("Auto engine: failed to refresh token: {}", e);
                    return Ok(());
                }
            }
            (session.access_token.clone(), session.account_info.account_email.clone())
        } else {
            log::info!("Auto engine: no Microsoft account connected.");
            return Ok(());
        }
    };

    if account_email.is_empty() {
        return Ok(());
    }

    // 2. Check profile
    let profile = match get_auto_profile(&mig_state.db, &account_email)? {
        Some(p) if p.enabled => p,
        _ => {
            log::info!("Auto engine: Auto Migration disabled for account {}.", account_email);
            return Ok(());
        }
    };

    // 3. Check 250GB daily quota
    let quota = get_daily_quota(&mig_state.db)?;
    if quota.uploaded_bytes >= quota.limit_bytes {
        log::warn!("Auto engine: Daily quota limit of 250GB reached today ({} bytes).", quota.uploaded_bytes);
        return Ok(());
    }

    // 4. Resolve default temp directory
    let temp_dir = match profile.local_temp_dir {
        Some(dir) if !dir.trim().is_empty() => dir,
        _ => {
            let app_data = app_handle.path().app_data_dir().map_err(|e| e.to_string())?;
            let default_dir = app_data.join("temp_migration");
            std::fs::create_dir_all(&default_dir).map_err(|e| e.to_string())?;
            default_dir.to_string_lossy().to_string()
        }
    };

    // 5. Check existing jobs
    match check_existing_job(&mig_state.db)? {
        AutoJobCheck::AlreadyRunning => {
            log::info!("Auto engine: Job already running.");
            return Ok(());
        }
        AutoJobCheck::Resume(job_id) => {
            log::info!("Auto engine: Resuming job {}.", job_id);
            let app_clone = app_handle.clone();
            tokio::spawn(async move {
                let state_ref = app_clone.state::<MigrationState>();
                let mig_clone = state_ref.inner().clone_state();
                run_migration_worker(mig_clone, job_id, app_clone).await;
            });
            return Ok(());
        }
        AutoJobCheck::NeedScan => {}
    }

    // 6. Scan root folder
    log::info!("Auto engine: Scanning root OneDrive folder for account {}...", account_email);
    let http = Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let root_items = scan_folder_recursive(&http, &access_token, "root", "").await?;

    if root_items.is_empty() {
        log::info!("Auto engine: OneDrive root is empty.");
        return Ok(());
    }

    let dest_id = profile.default_telegram_dest_id;
    let dest_name = profile.default_telegram_dest_name.as_deref();

    let (job_id, _) = create_migration_job(
        &mig_state.db,
        Some("root"),
        Some("/ (Root)"),
        dest_id,
        dest_name,
        &temp_dir,
        &root_items,
    )?;

    // Mark job ready
    {
        let conn = mig_state.db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(format!(
            "UPDATE migration_jobs SET state = 'ready', updated_at = {} WHERE id = {};",
            now, job_id
        ))
        .map_err(|e| e.to_string())?;
    }

    log::info!("Auto engine: Created auto migration job {}. Launching worker...", job_id);
    let app_clone = app_handle.clone();
    tokio::spawn(async move {
        let state_ref = app_clone.state::<MigrationState>();
        let mig_clone = state_ref.inner().clone_state();
        run_migration_worker(mig_clone, job_id, app_clone).await;
    });

    Ok(())
}
