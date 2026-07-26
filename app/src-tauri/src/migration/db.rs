use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager};

use crate::migration::models::*;

pub type MigrationDb = Arc<Mutex<sqlite::Connection>>;

const MAX_DB_INIT_RETRIES: u32 = 5;

pub fn init_migration_db(app: &AppHandle) -> Result<MigrationDb, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let db_path = dir.join("migration.db");
    
    // Check if we need to reset the schema from legacy V1/V2 to canonical.
    // We can do this by checking if a legacy table like `migrated_fingerprints` exists.
    let mut needs_reset = false;
    if db_path.exists() {
        if let Ok(conn) = sqlite::open(&db_path) {
            let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='migrated_fingerprints';").unwrap();
            if let Ok(sqlite::State::Row) = stmt.next() {
                needs_reset = true;
            }
        }
    }
    
    let db = open_migration_db_at_path(db_path)?;
    
    if needs_reset {
        log::info!("Legacy schema detected. Resetting to canonical schema.");
        reset_database(&db.lock().unwrap())?;
    } else {
        create_schema(&db.lock().unwrap())?;
    }
    
    Ok(db)
}

pub fn open_migration_db_at_path(db_path: PathBuf) -> Result<MigrationDb, String> {
    let conn = {
        let mut last_err = String::new();
        let mut opened = None;
        for attempt in 0..MAX_DB_INIT_RETRIES {
            match sqlite::open(&db_path) {
                Ok(c) => {
                    opened = Some(c);
                    break;
                }
                Err(e) => {
                    last_err = e.to_string();
                    if attempt < MAX_DB_INIT_RETRIES - 1 {
                        let wait_ms = 100 * 2u64.pow(attempt);
                        std::thread::sleep(Duration::from_millis(wait_ms));
                    }
                }
            }
        }
        opened.ok_or_else(|| {
            format!(
                "Failed to open migration SQLite database after {} attempts: {}",
                MAX_DB_INIT_RETRIES, last_err
            )
        })?
    };

    // PRAGMA
    conn.execute("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
        .map_err(|e| e.to_string())?;

    Ok(Arc::new(Mutex::new(conn)))
}

pub fn reset_database(conn: &sqlite::Connection) -> Result<(), String> {
    log::warn!("Resetting migration database to canonical schema");
    conn.execute("PRAGMA writable_schema = 1; DELETE FROM sqlite_master; PRAGMA writable_schema = 0; VACUUM;").map_err(|e| e.to_string())?;
    create_schema(conn)
}

fn create_schema(conn: &sqlite::Connection) -> Result<(), String> {
    // 1. Migration Jobs
    conn.execute(
        "CREATE TABLE IF NOT EXISTS migration_jobs (
            id                          INTEGER PRIMARY KEY AUTOINCREMENT,
            source_folder_id            TEXT NOT NULL,
            source_folder_path          TEXT NOT NULL,
            telegram_destination_id     INTEGER,
            telegram_destination_name   TEXT NOT NULL,
            local_backup_dir            TEXT NOT NULL,
            workspace_dir               TEXT NOT NULL,
            state                       TEXT NOT NULL DEFAULT 'ready',
            started_at                  INTEGER,
            completed_at                INTEGER,
            last_error                  TEXT,
            flood_wait_until            INTEGER,
            total_folders               INTEGER NOT NULL DEFAULT 0,
            total_files                 INTEGER NOT NULL DEFAULT 0,
            total_bytes                 INTEGER NOT NULL DEFAULT 0,
            processed_files             INTEGER NOT NULL DEFAULT 0,
            processed_bytes             INTEGER NOT NULL DEFAULT 0
        );"
    ).map_err(|e| format!("Failed to create migration_jobs: {}", e))?;

    // 2. Folder Queue
    conn.execute(
        "CREATE TABLE IF NOT EXISTS folder_queue (
            id                  INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id              INTEGER NOT NULL,
            folder_id           TEXT NOT NULL,
            parent_id           TEXT,
            folder_path         TEXT NOT NULL,
            state               TEXT NOT NULL DEFAULT 'pending',
            next_page_token     TEXT,
            has_more            INTEGER NOT NULL DEFAULT 1,
            files_discovered    INTEGER NOT NULL DEFAULT 0,
            files_completed     INTEGER NOT NULL DEFAULT 0,
            folders_discovered  INTEGER NOT NULL DEFAULT 0,
            last_error          TEXT,
            UNIQUE(job_id, folder_id)
        );"
    ).map_err(|e| format!("Failed to create folder_queue: {}", e))?;

    // 3. Migration Items
    conn.execute(
        "CREATE TABLE IF NOT EXISTS migration_items (
            id                          INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id                      INTEGER NOT NULL,
            folder_id                   TEXT NOT NULL,
            source_item_id              TEXT NOT NULL,
            name                        TEXT NOT NULL,
            source_path                 TEXT NOT NULL,
            size_bytes                  INTEGER NOT NULL,
            item_type                   TEXT NOT NULL,
            pipeline_stage              TEXT NOT NULL DEFAULT 'pending',
            original_artifact_path      TEXT,
            processed_artifact_path     TEXT,
            original_sha256             TEXT,
            processed_sha256            TEXT,
            video_decision              TEXT,
            telegram_random_id          INTEGER,
            telegram_message_id         INTEGER,
            retry_count                 INTEGER NOT NULL DEFAULT 0,
            last_error                  TEXT,
            updated_at                  INTEGER NOT NULL,
            UNIQUE(job_id, source_item_id)
        );"
    ).map_err(|e| format!("Failed to create migration_items: {}", e))?;
    
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_items_job_stage ON migration_items(job_id, pipeline_stage);"
    ).map_err(|e| format!("Failed to create idx_items_job_stage: {}", e))?;

    // 4. Daily Quota & Reservations
    conn.execute(
        "CREATE TABLE IF NOT EXISTS daily_migration_quota (
            date_string         TEXT PRIMARY KEY,
            uploaded_bytes      INTEGER NOT NULL DEFAULT 0,
            updated_at          INTEGER NOT NULL DEFAULT 0
        );"
    ).map_err(|e| format!("Failed to create daily_migration_quota: {}", e))?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS quota_reservations (
            item_id         INTEGER PRIMARY KEY,
            job_id          INTEGER NOT NULL,
            date_string     TEXT NOT NULL,
            reserved_bytes  INTEGER NOT NULL,
            status          TEXT NOT NULL DEFAULT 'reserved',
            created_at      INTEGER NOT NULL,
            expires_at      INTEGER NOT NULL
        );"
    ).map_err(|e| format!("Failed to create quota_reservations: {}", e))?;

    // 5. Pacing
    conn.execute(
        "CREATE TABLE IF NOT EXISTS pacing (
            id                      INTEGER PRIMARY KEY AUTOINCREMENT,
            next_allowed_at         INTEGER NOT NULL DEFAULT 0,
            flood_wait_until        INTEGER NOT NULL DEFAULT 0,
            last_upload_success_at  INTEGER NOT NULL DEFAULT 0
        );"
    ).map_err(|e| format!("Failed to create pacing: {}", e))?;
    
    // Ensure pacing has exactly 1 row
    conn.execute("INSERT OR IGNORE INTO pacing (id, next_allowed_at, flood_wait_until, last_upload_success_at) VALUES (1, 0, 0, 0);").map_err(|e| e.to_string())?;

    Ok(())
}

pub fn create_job(
    conn: &sqlite::Connection,
    source_folder_id: &str,
    source_folder_path: &str,
    telegram_destination_id: Option<i64>,
    telegram_destination_name: &str,
    local_backup_dir: &str,
    workspace_dir: &str,
) -> Result<i64, String> {
    let mut stmt = conn.prepare(
        "INSERT INTO migration_jobs (
            source_folder_id, source_folder_path, telegram_destination_id, 
            telegram_destination_name, local_backup_dir, workspace_dir, state
        ) VALUES (?, ?, ?, ?, ?, ?, 'ready')"
    ).map_err(|e| e.to_string())?;

    stmt.bind((1, source_folder_id)).map_err(|e| e.to_string())?;
    stmt.bind((2, source_folder_path)).map_err(|e| e.to_string())?;
    
    match telegram_destination_id {
        Some(id) => stmt.bind((3, id)).map_err(|e| e.to_string())?,
        None => stmt.bind((3, sqlite::Value::Null)).map_err(|e| e.to_string())?,
    }
    
    stmt.bind((4, telegram_destination_name)).map_err(|e| e.to_string())?;
    stmt.bind((5, local_backup_dir)).map_err(|e| e.to_string())?;
    stmt.bind((6, workspace_dir)).map_err(|e| e.to_string())?;
    
    stmt.next().map_err(|e| e.to_string())?;
    
    let mut last_id_stmt = conn.prepare("SELECT last_insert_rowid();").unwrap();
    if let Ok(sqlite::State::Row) = last_id_stmt.next() {
        Ok(last_id_stmt.read::<i64, _>(0).unwrap())
    } else {
        Err("Failed to get last insert rowid".into())
    }
}
