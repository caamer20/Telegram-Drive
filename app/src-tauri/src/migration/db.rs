use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager};

use crate::migration::models::*;

pub type MigrationDb = Arc<Mutex<sqlite::Connection>>;

const MAX_DB_INIT_RETRIES: u32 = 5;

#[derive(Debug, Clone)]
pub struct AutoScanCheckpoint {
    pub next_link: Option<String>,
    pub pages_scanned: usize,
    pub discovered_files: usize,
    pub discovered_folders: usize,
    pub elapsed_ms: u64,
    pub status: String,
}

fn ensure_column(
    conn: &sqlite::Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(format!("PRAGMA table_info({table});"))
        .map_err(|e| e.to_string())?;
    while let Ok(sqlite::State::Row) = stmt.next() {
        let name: String = stmt.read(1).unwrap_or_default();
        if name == column {
            return Ok(());
        }
    }
    conn.execute(format!(
        "ALTER TABLE {table} ADD COLUMN {column} {definition};"
    ))
    .map_err(|e| e.to_string())
}

pub fn init_migration_db(app: &AppHandle) -> Result<MigrationDb, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let db_path = dir.join("migration.db");
    open_migration_db_at_path(db_path)
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

    // PRAGMA & Schema
    conn.execute("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
        .map_err(|e| e.to_string())?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS migration_jobs (
            id                      INTEGER PRIMARY KEY AUTOINCREMENT,
            state                   TEXT NOT NULL DEFAULT 'draft'
                                    CHECK(state IN ('draft','ready','running','paused','completed','cancelled','failed')),
            onedrive_folder_id      TEXT,
            onedrive_folder_path    TEXT,
            telegram_destination_id INTEGER,
            telegram_destination_name TEXT,
            local_dir               TEXT,
            cooldown_until          INTEGER,
            total_folders           INTEGER NOT NULL DEFAULT 0,
            total_files             INTEGER NOT NULL DEFAULT 0,
            total_bytes             INTEGER NOT NULL DEFAULT 0,
            completed_files         INTEGER NOT NULL DEFAULT 0,
            completed_bytes         INTEGER NOT NULL DEFAULT 0,
            failed_files            INTEGER NOT NULL DEFAULT 0,
            skipped_duplicates      INTEGER NOT NULL DEFAULT 0,
            pending_files           INTEGER NOT NULL DEFAULT 0,
            created_at              INTEGER NOT NULL,
            started_at              INTEGER,
            completed_at            INTEGER,
            updated_at              INTEGER NOT NULL
        );",
    )
    .map_err(|e| format!("Failed to create migration_jobs table: {}", e))?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS migration_items (
            id                      INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id                  INTEGER NOT NULL,
            item_type               TEXT NOT NULL DEFAULT 'file'
                                    CHECK(item_type IN ('file', 'folder')),
            name                    TEXT NOT NULL,
            source_path             TEXT NOT NULL,
            source_item_id          TEXT,
            size_bytes              INTEGER NOT NULL DEFAULT 0,
            source_etag             TEXT,
            source_last_modified    TEXT,
            source_fingerprint_type TEXT,
            source_fingerprint_value TEXT,
            state                   TEXT NOT NULL DEFAULT 'pending'
                                    CHECK(state IN ('pending', 'downloading', 'uploading',
                                         'completed', 'skipped_duplicate', 'failed')),
            last_error_code         TEXT
                                    CHECK(last_error_code IS NULL OR last_error_code IN (
                                        'source_changed', 'network', 'auth',
                                        'telegram_file_too_large', 'insufficient_disk',
                                        'working_directory_unavailable', 'recovery_interrupted',
                                        'download_failed', 'upload_failed', 'unknown')),
            last_error_message      TEXT,
            attempt_count           INTEGER NOT NULL DEFAULT 0,
            computed_sha256         TEXT,
            telegram_message_id     INTEGER,
            created_at              INTEGER NOT NULL,
            completed_at            INTEGER,
            UNIQUE(job_id, source_path)
        );",
    )
    .map_err(|e| format!("Failed to create migration_items table: {}", e))?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_items_job_state ON migration_items(job_id, state);
         CREATE INDEX IF NOT EXISTS idx_items_job_type ON migration_items(job_id, item_type);",
    )
    .map_err(|e| format!("Failed to create item indexes: {}", e))?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS migrated_fingerprints (
            id                  INTEGER PRIMARY KEY AUTOINCREMENT,
            fingerprint_type    TEXT NOT NULL,
            fingerprint_value   TEXT NOT NULL,
            file_size           INTEGER NOT NULL,
            first_job_id        INTEGER NOT NULL,
            first_item_id       INTEGER NOT NULL,
            telegram_destination_id INTEGER,
            telegram_message_id INTEGER,
            completed_at        INTEGER NOT NULL,
            UNIQUE(fingerprint_type, fingerprint_value, file_size)
        );",
    )
    .map_err(|e| format!("Failed to create migrated_fingerprints table: {}", e))?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_fingerprints_type_value ON migrated_fingerprints(fingerprint_type, fingerprint_value);",
    )
    .map_err(|e| format!("Failed to create fingerprint index: {}", e))?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS auto_migration_profiles (
            id                      INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id              TEXT UNIQUE NOT NULL,
            enabled                 INTEGER NOT NULL DEFAULT 1,
            default_telegram_dest_id INTEGER,
            default_telegram_dest_name TEXT,
            local_temp_dir          TEXT,
            last_auto_scan_at       INTEGER,
            created_at              INTEGER NOT NULL,
            updated_at              INTEGER NOT NULL
        );",
    )
    .map_err(|e| format!("Failed to create auto_migration_profiles table: {}", e))?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS daily_migration_quota (
            date_string             TEXT PRIMARY KEY,
            uploaded_bytes          INTEGER NOT NULL DEFAULT 0,
            updated_at              INTEGER NOT NULL
        );",
    )
    .map_err(|e| format!("Failed to create daily_migration_quota table: {}", e))?;

    ensure_column(
        &conn,
        "migration_jobs",
        "job_origin",
        "TEXT NOT NULL DEFAULT 'manual'",
    )?;
    ensure_column(&conn, "migration_jobs", "pause_reason", "TEXT")?;
    ensure_column(
        &conn,
        "migration_items",
        "queue_position",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        &conn,
        "migration_items",
        "action_type",
        "TEXT DEFAULT 'sync'",
    )?;
    ensure_column(&conn, "auto_migration_profiles", "active_job_id", "INTEGER")?;
    ensure_column(&conn, "auto_migration_profiles", "pause_reason", "TEXT")?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS migration_activity (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id      INTEGER NOT NULL,
            item_id     INTEGER,
            item_name   TEXT,
            phase       TEXT NOT NULL,
            status      TEXT NOT NULL,
            attempt     INTEGER NOT NULL DEFAULT 0,
            revision    INTEGER NOT NULL DEFAULT 0,
            message     TEXT,
            created_at  INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_activity_job_created
            ON migration_activity(job_id, created_at DESC, id DESC);
        CREATE INDEX IF NOT EXISTS idx_items_job_queue
            ON migration_items(job_id, queue_position, id);",
    )
    .map_err(|e| format!("Failed to create Auto Migration indexes: {e}"))?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS auto_scan_checkpoints (
            account_id          TEXT PRIMARY KEY,
            next_link           TEXT,
            pages_scanned       INTEGER NOT NULL DEFAULT 0,
            discovered_files    INTEGER NOT NULL DEFAULT 0,
            discovered_folders  INTEGER NOT NULL DEFAULT 0,
            elapsed_ms          INTEGER NOT NULL DEFAULT 0,
            status              TEXT NOT NULL DEFAULT 'enumerating',
            updated_at          INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS auto_scan_items (
            account_id          TEXT NOT NULL,
            item_id             TEXT NOT NULL,
            payload_json        TEXT NOT NULL,
            PRIMARY KEY(account_id, item_id)
        );
        CREATE INDEX IF NOT EXISTS idx_auto_scan_items_account
            ON auto_scan_items(account_id);",
    )
    .map_err(|e| format!("Failed to create resumable scan tables: {e}"))?;
    ensure_column(
        &conn,
        "migration_activity",
        "attempt",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        &conn,
        "migration_activity",
        "revision",
        "INTEGER NOT NULL DEFAULT 0",
    )?;

    crate::migration::schema_v2::migrate_to_v2(&conn)?;

    let db = Arc::new(Mutex::new(conn));

    // Run startup recovery
    if let Err(e) = run_startup_recovery(&db) {
        log::error!("Migration startup recovery error: {}", e);
    }

    Ok(db)
}

/// Startup recovery mapping (Rule: downloading/uploading -> pending + recovery_interrupted, cleanup temp files, attempt_count unchanged)
pub fn run_startup_recovery(db: &MigrationDb) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut local_dirs = Vec::new();
    {
        let mut statement = conn
            .prepare(
                "SELECT DISTINCT local_dir FROM migration_jobs
                 WHERE local_dir IS NOT NULL AND TRIM(local_dir) <> '';",
            )
            .map_err(|e| e.to_string())?;
        loop {
            match statement.next().map_err(|e| e.to_string())? {
                sqlite::State::Row => {
                    local_dirs.push(statement.read::<String, _>(0).map_err(|e| e.to_string())?);
                }
                sqlite::State::Done => break,
            }
        }
    }

    conn.execute(
        "UPDATE migration_items
         SET state = 'pending',
             last_error_code = 'recovery_interrupted',
             last_error_message = 'Interrupted by application restart'
         WHERE state IN ('downloading', 'uploading');",
    )
    .map_err(|e| e.to_string())?;

    // Also update any running job back to paused or ready
    conn.execute(
        "UPDATE migration_jobs
         SET state = 'paused',
             updated_at = strftime('%s', 'now')
         WHERE state = 'running';",
    )
    .map_err(|e| e.to_string())?;

    drop(conn);
    for local_dir in local_dirs {
        let removed =
            crate::migration::media_processor::cleanup_orphaned_outputs(Path::new(&local_dir));
        if removed > 0 {
            log::info!(
                "Migration recovery removed {} orphaned video output(s)",
                removed
            );
        }
    }

    Ok(())
}

/// Create a new migration job in 'draft' state
pub fn create_job(db: &MigrationDb) -> Result<MigrationJob, String> {
    let last_id = {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp();

        conn.execute(format!(
            "INSERT INTO migration_jobs (state, created_at, updated_at) VALUES ('draft', {}, {});",
            now, now
        ))
        .map_err(|e| e.to_string())?;

        let mut stmt = conn
            .prepare("SELECT last_insert_rowid();")
            .map_err(|e| e.to_string())?;
        if let Ok(sqlite::State::Row) = stmt.next() {
            stmt.read::<i64, _>(0).unwrap_or(0)
        } else {
            0
        }
    };

    get_job(db, last_id)
}

/// Get job by ID
pub fn get_job(db: &MigrationDb, job_id: i64) -> Result<MigrationJob, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id, state, onedrive_folder_id, onedrive_folder_path, telegram_destination_id, telegram_destination_name, local_dir, cooldown_until, created_at, started_at, completed_at, updated_at, job_origin, pause_reason FROM migration_jobs WHERE id = ?;")
        .map_err(|e| e.to_string())?;

    stmt.bind((1, job_id)).map_err(|e| e.to_string())?;

    if let Ok(sqlite::State::Row) = stmt.next() {
        Ok(MigrationJob {
            id: stmt.read::<i64, _>(0).unwrap_or(0),
            state: stmt.read::<String, _>(1).unwrap_or_else(|_| "draft".into()),
            onedrive_folder_id: stmt.read::<Option<String>, _>(2).unwrap_or(None),
            onedrive_folder_path: stmt.read::<Option<String>, _>(3).unwrap_or(None),
            telegram_destination_id: stmt.read::<Option<i64>, _>(4).unwrap_or(None),
            telegram_destination_name: stmt.read::<Option<String>, _>(5).unwrap_or(None),
            local_dir: stmt.read::<Option<String>, _>(6).unwrap_or(None),
            cooldown_until: stmt.read::<Option<i64>, _>(7).unwrap_or(None),
            created_at: stmt.read::<i64, _>(8).unwrap_or(0),
            started_at: stmt.read::<Option<i64>, _>(9).unwrap_or(None),
            completed_at: stmt.read::<Option<i64>, _>(10).unwrap_or(None),
            updated_at: stmt.read::<i64, _>(11).unwrap_or(0),
            job_origin: stmt
                .read::<String, _>(12)
                .unwrap_or_else(|_| "manual".into()),
            pause_reason: stmt.read::<Option<String>, _>(13).unwrap_or(None),
        })
    } else {
        Err(format!("Job {} not found", job_id))
    }
}

/// Get all jobs summaries
pub fn get_jobs(db: &MigrationDb) -> Result<Vec<MigrationJobSummary>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id, state, onedrive_folder_path, total_files, completed_files, created_at FROM migration_jobs ORDER BY id DESC;")
        .map_err(|e| e.to_string())?;

    let mut list = Vec::new();
    while let Ok(sqlite::State::Row) = stmt.next() {
        list.push(MigrationJobSummary {
            id: stmt.read::<i64, _>(0).unwrap_or(0),
            state: stmt.read::<String, _>(1).unwrap_or_else(|_| "draft".into()),
            onedrive_folder_path: stmt.read::<Option<String>, _>(2).unwrap_or(None),
            total_files: stmt.read::<i64, _>(3).unwrap_or(0),
            completed_files: stmt.read::<i64, _>(4).unwrap_or(0),
            created_at: stmt.read::<i64, _>(5).unwrap_or(0),
        });
    }

    Ok(list)
}

/// Delete job and items
pub fn delete_job(db: &MigrationDb, job_id: i64) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;

    // Check if running
    let mut check = conn
        .prepare("SELECT state FROM migration_jobs WHERE id = ?;")
        .map_err(|e| e.to_string())?;
    check.bind((1, job_id)).map_err(|e| e.to_string())?;
    if let Ok(sqlite::State::Row) = check.next() {
        let state: String = check.read(0).unwrap_or_default();
        if state == "running" {
            return Err("Cannot delete a running migration job".into());
        }
    }

    conn.execute(format!(
        "DELETE FROM migration_items WHERE job_id = {};",
        job_id
    ))
    .map_err(|e| e.to_string())?;
    conn.execute(format!("DELETE FROM migration_jobs WHERE id = {};", job_id))
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// Set source folder
pub fn set_onedrive_folder(
    db: &MigrationDb,
    job_id: i64,
    folder_id: String,
    folder_path: String,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp();
    let mut stmt = conn.prepare("UPDATE migration_jobs SET onedrive_folder_id = ?, onedrive_folder_path = ?, updated_at = ? WHERE id = ?;").map_err(|e| e.to_string())?;
    stmt.bind((1, folder_id.as_str()))
        .map_err(|e| e.to_string())?;
    stmt.bind((2, folder_path.as_str()))
        .map_err(|e| e.to_string())?;
    stmt.bind((3, now)).map_err(|e| e.to_string())?;
    stmt.bind((4, job_id)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;
    Ok(())
}

/// Set destination
pub fn set_telegram_destination(
    db: &MigrationDb,
    job_id: i64,
    dest_id: Option<i64>,
    dest_name: String,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp();
    let mut stmt = conn.prepare("UPDATE migration_jobs SET telegram_destination_id = ?, telegram_destination_name = ?, updated_at = ? WHERE id = ?;").map_err(|e| e.to_string())?;
    match dest_id {
        Some(id) => stmt.bind((1, id)).map_err(|e| e.to_string())?,
        None => stmt
            .bind((1, sqlite::Value::Null))
            .map_err(|e| e.to_string())?,
    }
    stmt.bind((2, dest_name.as_str()))
        .map_err(|e| e.to_string())?;
    stmt.bind((3, now)).map_err(|e| e.to_string())?;
    stmt.bind((4, job_id)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;
    Ok(())
}

/// Set local working dir
pub fn set_local_dir(db: &MigrationDb, job_id: i64, local_dir: String) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp();
    let mut stmt = conn
        .prepare("UPDATE migration_jobs SET local_dir = ?, updated_at = ? WHERE id = ?;")
        .map_err(|e| e.to_string())?;
    stmt.bind((1, local_dir.as_str()))
        .map_err(|e| e.to_string())?;
    stmt.bind((2, now)).map_err(|e| e.to_string())?;
    stmt.bind((3, job_id)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;
    Ok(())
}

/// Get stats for a job
pub fn get_job_stats(db: &MigrationDb, job_id: i64) -> Result<MigrationStats, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT total_folders, total_files, total_bytes, completed_files, completed_bytes, failed_files, skipped_duplicates, pending_files FROM migration_jobs WHERE id = ?;"
    ).map_err(|e| e.to_string())?;
    stmt.bind((1, job_id)).map_err(|e| e.to_string())?;

    if let Ok(sqlite::State::Row) = stmt.next() {
        Ok(MigrationStats {
            total_folders: stmt.read(0).unwrap_or(0),
            total_files: stmt.read(1).unwrap_or(0),
            total_bytes: stmt.read(2).unwrap_or(0),
            completed_files: stmt.read(3).unwrap_or(0),
            completed_bytes: stmt.read(4).unwrap_or(0),
            failed_files: stmt.read(5).unwrap_or(0),
            skipped_duplicates: stmt.read(6).unwrap_or(0),
            pending_files: stmt.read(7).unwrap_or(0),
        })
    } else {
        Err(format!("Job {} stats not found", job_id))
    }
}

/// Update stats query according to F-01 & F-05 rules
pub fn update_job_stats(db: &MigrationDb, job_id: i64) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let sql = "UPDATE migration_jobs SET
        completed_files = (SELECT COUNT(*) FROM migration_items WHERE job_id = ? AND state = 'completed'),
        completed_bytes = (SELECT COALESCE(SUM(size_bytes), 0) FROM migration_items WHERE job_id = ? AND state IN ('completed', 'skipped_duplicate')),
        failed_files = (SELECT COUNT(*) FROM migration_items WHERE job_id = ? AND state = 'failed'),
        skipped_duplicates = (SELECT COUNT(*) FROM migration_items WHERE job_id = ? AND state = 'skipped_duplicate'),
        pending_files = (SELECT COUNT(*) FROM migration_items WHERE job_id = ? AND state IN ('pending', 'downloading', 'uploading')),
        updated_at = strftime('%s', 'now')
    WHERE id = ?;";

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    stmt.bind((1, job_id)).map_err(|e| e.to_string())?;
    stmt.bind((2, job_id)).map_err(|e| e.to_string())?;
    stmt.bind((3, job_id)).map_err(|e| e.to_string())?;
    stmt.bind((4, job_id)).map_err(|e| e.to_string())?;
    stmt.bind((5, job_id)).map_err(|e| e.to_string())?;
    stmt.bind((6, job_id)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;

    Ok(())
}

/// Batch insert items during scan
pub fn batch_insert_items(
    db: &MigrationDb,
    job_id: i64,
    items: &[OneDriveItem],
) -> Result<MigrationStats, String> {
    batch_insert_items_with_action(db, job_id, items, Some("sync"))
}

pub fn batch_insert_items_with_action(
    db: &MigrationDb,
    job_id: i64,
    items: &[OneDriveItem],
    action_type: Option<&str>,
) -> Result<MigrationStats, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp();
    let act_str = action_type.unwrap_or("sync");

    // Clear previous snapshot
    conn.execute(format!(
        "DELETE FROM migration_items WHERE job_id = {};",
        job_id
    ))
    .map_err(|e| e.to_string())?;

    let mut total_folders = 0i64;
    let mut total_files = 0i64;
    let mut total_bytes = 0i64;

    conn.execute("BEGIN TRANSACTION;")
        .map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "INSERT INTO migration_items (
            job_id, item_type, name, source_path, source_item_id, size_bytes,
            source_etag, source_last_modified, source_fingerprint_type, source_fingerprint_value,
            state, created_at, queue_position, action_type
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?, ?);",
        )
        .map_err(|e| e.to_string())?;

    for (queue_position, item) in items.iter().enumerate() {
        if item.item_type == "folder" {
            total_folders += 1;
        } else {
            total_files += 1;
            total_bytes += item.size;
        }

        let rel_path = item.path.as_deref().unwrap_or(&item.name);
        let fingerprint_type = if item.quickxor_hash.is_some() {
            Some("onedrive_quickxor")
        } else if item.sha1_hash.is_some() {
            Some("onedrive_sha1")
        } else {
            None
        };
        let fingerprint_val = item.quickxor_hash.as_deref().or(item.sha1_hash.as_deref());

        stmt.bind((1, job_id)).map_err(|e| e.to_string())?;
        stmt.bind((2, item.item_type.as_str()))
            .map_err(|e| e.to_string())?;
        stmt.bind((3, item.name.as_str()))
            .map_err(|e| e.to_string())?;
        stmt.bind((4, rel_path)).map_err(|e| e.to_string())?;
        stmt.bind((5, item.id.as_str()))
            .map_err(|e| e.to_string())?;
        stmt.bind((6, item.size)).map_err(|e| e.to_string())?;
        match &item.etag {
            Some(e) => stmt.bind((7, e.as_str())).map_err(|e| e.to_string())?,
            None => stmt
                .bind((7, sqlite::Value::Null))
                .map_err(|e| e.to_string())?,
        }
        match &item.last_modified {
            Some(lm) => stmt.bind((8, lm.as_str())).map_err(|e| e.to_string())?,
            None => stmt
                .bind((8, sqlite::Value::Null))
                .map_err(|e| e.to_string())?,
        }
        match fingerprint_type {
            Some(ft) => stmt.bind((9, ft)).map_err(|e| e.to_string())?,
            None => stmt
                .bind((9, sqlite::Value::Null))
                .map_err(|e| e.to_string())?,
        }
        match fingerprint_val {
            Some(fv) => stmt.bind((10, fv)).map_err(|e| e.to_string())?,
            None => stmt
                .bind((10, sqlite::Value::Null))
                .map_err(|e| e.to_string())?,
        }
        stmt.bind((11, now)).map_err(|e| e.to_string())?;
        stmt.bind((12, queue_position as i64))
            .map_err(|e| e.to_string())?;
        stmt.bind((13, act_str)).map_err(|e| e.to_string())?;

        stmt.next().map_err(|e| e.to_string())?;
        stmt.reset().map_err(|e| e.to_string())?;
    }
    drop(stmt);

    conn.execute("COMMIT;").map_err(|e| e.to_string())?;

    // Update job header totals
    let mut update = conn.prepare(
        "UPDATE migration_jobs SET state = 'ready', total_folders = ?, total_files = ?, total_bytes = ?, pending_files = ?, completed_files = 0, completed_bytes = 0, failed_files = 0, skipped_duplicates = 0, updated_at = ? WHERE id = ?;"
    ).map_err(|e| e.to_string())?;

    update.bind((1, total_folders)).map_err(|e| e.to_string())?;
    update.bind((2, total_files)).map_err(|e| e.to_string())?;
    update.bind((3, total_bytes)).map_err(|e| e.to_string())?;
    update.bind((4, total_files)).map_err(|e| e.to_string())?;
    update.bind((5, now)).map_err(|e| e.to_string())?;
    update.bind((6, job_id)).map_err(|e| e.to_string())?;
    update.next().map_err(|e| e.to_string())?;

    Ok(MigrationStats {
        total_folders,
        total_files,
        total_bytes,
        completed_files: 0,
        completed_bytes: 0,
        failed_files: 0,
        skipped_duplicates: 0,
        pending_files: total_files,
    })
}

/// Append newly discovered OneDrive entries while an auto scan is still running.
/// Existing paths are ignored so the worker can safely consume the queue in order.
pub fn append_scan_items(
    db: &MigrationDb,
    job_id: i64,
    items: &[OneDriveItem],
) -> Result<(), String> {
    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp();
        let mut stmt = conn
            .prepare(
                "INSERT OR IGNORE INTO migration_items (
                    job_id, item_type, name, source_path, source_item_id, size_bytes,
                    source_etag, source_last_modified, source_fingerprint_type,
                    source_fingerprint_value, state, created_at, queue_position
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?,
                    COALESCE((SELECT MAX(queue_position) + 1 FROM migration_items WHERE job_id = ?), 0));",
            )
            .map_err(|e| e.to_string())?;

        for item in items {
            let path = item.path.as_deref().unwrap_or(&item.name);
            let fingerprint_type = if item.quickxor_hash.is_some() {
                Some("onedrive_quickxor")
            } else if item.sha1_hash.is_some() {
                Some("onedrive_sha1")
            } else {
                None
            };
            let fingerprint_value = item.quickxor_hash.as_deref().or(item.sha1_hash.as_deref());
            stmt.bind((1, job_id)).map_err(|e| e.to_string())?;
            stmt.bind((2, item.item_type.as_str()))
                .map_err(|e| e.to_string())?;
            stmt.bind((3, item.name.as_str()))
                .map_err(|e| e.to_string())?;
            stmt.bind((4, path)).map_err(|e| e.to_string())?;
            stmt.bind((5, item.id.as_str()))
                .map_err(|e| e.to_string())?;
            stmt.bind((6, item.size)).map_err(|e| e.to_string())?;
            match item.etag.as_deref() {
                Some(value) => stmt.bind((7, value)).map_err(|e| e.to_string())?,
                None => stmt
                    .bind((7, sqlite::Value::Null))
                    .map_err(|e| e.to_string())?,
            }
            match item.last_modified.as_deref() {
                Some(value) => stmt.bind((8, value)).map_err(|e| e.to_string())?,
                None => stmt
                    .bind((8, sqlite::Value::Null))
                    .map_err(|e| e.to_string())?,
            }
            match fingerprint_type {
                Some(value) => stmt.bind((9, value)).map_err(|e| e.to_string())?,
                None => stmt
                    .bind((9, sqlite::Value::Null))
                    .map_err(|e| e.to_string())?,
            }
            match fingerprint_value {
                Some(value) => stmt.bind((10, value)).map_err(|e| e.to_string())?,
                None => stmt
                    .bind((10, sqlite::Value::Null))
                    .map_err(|e| e.to_string())?,
            }
            stmt.bind((11, now)).map_err(|e| e.to_string())?;
            stmt.bind((12, job_id)).map_err(|e| e.to_string())?;
            stmt.next().map_err(|e| e.to_string())?;
            stmt.reset().map_err(|e| e.to_string())?;
        }
        drop(stmt);
    }
    update_job_stats(db, job_id)
}

/// Get items for a job
pub fn get_items_by_job(db: &MigrationDb, job_id: i64) -> Result<Vec<MigrationItem>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT id, job_id, item_type, name, source_path, source_item_id, size_bytes, source_etag, source_last_modified, source_fingerprint_type, source_fingerprint_value, state, last_error_code, last_error_message, attempt_count, computed_sha256, telegram_message_id, created_at, completed_at, queue_position, action_type FROM migration_items WHERE job_id = ? ORDER BY queue_position ASC, id ASC;"
    ).map_err(|e| e.to_string())?;
    stmt.bind((1, job_id)).map_err(|e| e.to_string())?;

    let mut items = Vec::new();
    while let Ok(sqlite::State::Row) = stmt.next() {
        items.push(MigrationItem {
            id: stmt.read(0).unwrap_or(0),
            job_id: stmt.read(1).unwrap_or(0),
            item_type: stmt.read(2).unwrap_or_else(|_| "file".into()),
            name: stmt.read(3).unwrap_or_default(),
            source_path: stmt.read(4).unwrap_or_default(),
            source_item_id: stmt.read(5).unwrap_or(None),
            size_bytes: stmt.read(6).unwrap_or(0),
            source_etag: stmt.read(7).unwrap_or(None),
            source_last_modified: stmt.read(8).unwrap_or(None),
            source_fingerprint_type: stmt.read(9).unwrap_or(None),
            source_fingerprint_value: stmt.read(10).unwrap_or(None),
            state: stmt.read(11).unwrap_or_else(|_| "pending".into()),
            last_error_code: stmt.read(12).unwrap_or(None),
            last_error_message: stmt.read(13).unwrap_or(None),
            attempt_count: stmt.read(14).unwrap_or(0),
            computed_sha256: stmt.read(15).unwrap_or(None),
            telegram_message_id: stmt.read(16).unwrap_or(None),
            created_at: stmt.read(17).unwrap_or(0),
            completed_at: stmt.read(18).unwrap_or(None),
            queue_position: stmt.read(19).unwrap_or(0),
            action_type: stmt.read(20).unwrap_or(None),
        });
    }

    Ok(items)
}

pub fn get_next_pending_item(
    db: &MigrationDb,
    job_id: i64,
) -> Result<Option<MigrationItem>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT id, job_id, item_type, name, source_path, source_item_id, size_bytes, source_etag, source_last_modified, source_fingerprint_type, source_fingerprint_value, state, last_error_code, last_error_message, attempt_count, computed_sha256, telegram_message_id, created_at, completed_at, queue_position, action_type FROM migration_items WHERE job_id = ? AND state = 'pending' AND item_type = 'file' ORDER BY queue_position ASC, id ASC LIMIT 1;"
    ).map_err(|e| e.to_string())?;
    stmt.bind((1, job_id)).map_err(|e| e.to_string())?;

    if let Ok(sqlite::State::Row) = stmt.next() {
        Ok(Some(MigrationItem {
            id: stmt.read(0).unwrap_or(0),
            job_id: stmt.read(1).unwrap_or(0),
            item_type: stmt.read(2).unwrap_or_else(|_| "file".into()),
            name: stmt.read(3).unwrap_or_default(),
            source_path: stmt.read(4).unwrap_or_default(),
            source_item_id: stmt.read(5).unwrap_or(None),
            size_bytes: stmt.read(6).unwrap_or(0),
            source_etag: stmt.read(7).unwrap_or(None),
            source_last_modified: stmt.read(8).unwrap_or(None),
            source_fingerprint_type: stmt.read(9).unwrap_or(None),
            source_fingerprint_value: stmt.read(10).unwrap_or(None),
            state: stmt.read(11).unwrap_or_else(|_| "pending".into()),
            last_error_code: stmt.read(12).unwrap_or(None),
            last_error_message: stmt.read(13).unwrap_or(None),
            attempt_count: stmt.read(14).unwrap_or(0),
            computed_sha256: stmt.read(15).unwrap_or(None),
            telegram_message_id: stmt.read(16).unwrap_or(None),
            created_at: stmt.read(17).unwrap_or(0),
            completed_at: stmt.read(18).unwrap_or(None),
            queue_position: stmt.read(19).unwrap_or(0),
            action_type: stmt.read(20).unwrap_or(None),
        }))
    } else {
        Ok(None)
    }
}

pub fn get_next_pending_non_code_item(
    db: &MigrationDb,
    job_id: i64,
) -> Result<Option<MigrationItem>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT id, job_id, item_type, name, source_path, source_item_id, size_bytes, source_etag, source_last_modified, source_fingerprint_type, source_fingerprint_value, state, last_error_code, last_error_message, attempt_count, computed_sha256, telegram_message_id, created_at, completed_at, queue_position, action_type FROM migration_items WHERE job_id = ? AND state = 'pending' AND item_type = 'file' ORDER BY queue_position ASC, id ASC;"
    ).map_err(|e| e.to_string())?;
    stmt.bind((1, job_id)).map_err(|e| e.to_string())?;

    while let Ok(sqlite::State::Row) = stmt.next() {
        let name: String = stmt.read(3).unwrap_or_default();
        if crate::migration::worker::is_video_file(&name) {
            return Ok(Some(MigrationItem {
                id: stmt.read(0).unwrap_or(0),
                job_id: stmt.read(1).unwrap_or(0),
                item_type: stmt.read(2).unwrap_or_else(|_| "file".into()),
                name,
                source_path: stmt.read(4).unwrap_or_default(),
                source_item_id: stmt.read(5).unwrap_or(None),
                size_bytes: stmt.read(6).unwrap_or(0),
                source_etag: stmt.read(7).unwrap_or(None),
                source_last_modified: stmt.read(8).unwrap_or(None),
                source_fingerprint_type: stmt.read(9).unwrap_or(None),
                source_fingerprint_value: stmt.read(10).unwrap_or(None),
                state: stmt.read(11).unwrap_or_else(|_| "pending".into()),
                last_error_code: stmt.read(12).unwrap_or(None),
                last_error_message: stmt.read(13).unwrap_or(None),
                attempt_count: stmt.read(14).unwrap_or(0),
                computed_sha256: stmt.read(15).unwrap_or(None),
                telegram_message_id: stmt.read(16).unwrap_or(None),
                created_at: stmt.read(17).unwrap_or(0),
                completed_at: stmt.read(18).unwrap_or(None),
                queue_position: stmt.read(19).unwrap_or(0),
                action_type: stmt.read(20).unwrap_or(None),
            }));
        }
    }
    Ok(None)
}

pub fn get_next_pending_video_item(
    db: &MigrationDb,
    job_id: i64,
) -> Result<Option<MigrationItem>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT id, job_id, item_type, name, source_path, source_item_id, size_bytes, source_etag, source_last_modified, source_fingerprint_type, source_fingerprint_value, state, last_error_code, last_error_message, attempt_count, computed_sha256, telegram_message_id, created_at, completed_at, queue_position, action_type FROM migration_items WHERE job_id = ? AND state = 'pending' AND item_type = 'file' ORDER BY queue_position ASC, id ASC;"
    ).map_err(|e| e.to_string())?;
    stmt.bind((1, job_id)).map_err(|e| e.to_string())?;

    while let Ok(sqlite::State::Row) = stmt.next() {
        let name: String = stmt.read(3).unwrap_or_default();
        if crate::migration::worker::is_video_file(&name) {
            return Ok(Some(MigrationItem {
                id: stmt.read(0).unwrap_or(0),
                job_id: stmt.read(1).unwrap_or(0),
                item_type: stmt.read(2).unwrap_or_else(|_| "file".into()),
                name,
                source_path: stmt.read(4).unwrap_or_default(),
                source_item_id: stmt.read(5).unwrap_or(None),
                size_bytes: stmt.read(6).unwrap_or(0),
                source_etag: stmt.read(7).unwrap_or(None),
                source_last_modified: stmt.read(8).unwrap_or(None),
                source_fingerprint_type: stmt.read(9).unwrap_or(None),
                source_fingerprint_value: stmt.read(10).unwrap_or(None),
                state: stmt.read(11).unwrap_or_else(|_| "pending".into()),
                last_error_code: stmt.read(12).unwrap_or(None),
                last_error_message: stmt.read(13).unwrap_or(None),
                attempt_count: stmt.read(14).unwrap_or(0),
                computed_sha256: stmt.read(15).unwrap_or(None),
                telegram_message_id: stmt.read(16).unwrap_or(None),
                created_at: stmt.read(17).unwrap_or(0),
                completed_at: stmt.read(18).unwrap_or(None),
                queue_position: stmt.read(19).unwrap_or(0),
                action_type: stmt.read(20).unwrap_or(None),
            }));
        }
    }
    Ok(None)
}

pub fn get_next_pending_media_item(
    db: &MigrationDb,
    job_id: i64,
) -> Result<Option<MigrationItem>, String> {
    get_next_pending_video_item(db, job_id)
}

pub fn get_item_by_id(db: &MigrationDb, item_id: i64) -> Result<Option<MigrationItem>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT id, job_id, item_type, name, source_path, source_item_id, size_bytes, source_etag, source_last_modified, source_fingerprint_type, source_fingerprint_value, state, last_error_code, last_error_message, attempt_count, computed_sha256, telegram_message_id, created_at, completed_at, queue_position, action_type FROM migration_items WHERE id = ?;"
    ).map_err(|e| e.to_string())?;
    stmt.bind((1, item_id)).map_err(|e| e.to_string())?;

    if let Ok(sqlite::State::Row) = stmt.next() {
        Ok(Some(MigrationItem {
            id: stmt.read(0).unwrap_or(0),
            job_id: stmt.read(1).unwrap_or(0),
            item_type: stmt.read(2).unwrap_or_else(|_| "file".into()),
            name: stmt.read(3).unwrap_or_default(),
            source_path: stmt.read(4).unwrap_or_default(),
            source_item_id: stmt.read(5).unwrap_or(None),
            size_bytes: stmt.read(6).unwrap_or(0),
            source_etag: stmt.read(7).unwrap_or(None),
            source_last_modified: stmt.read(8).unwrap_or(None),
            source_fingerprint_type: stmt.read(9).unwrap_or(None),
            source_fingerprint_value: stmt.read(10).unwrap_or(None),
            state: stmt.read(11).unwrap_or_else(|_| "pending".into()),
            last_error_code: stmt.read(12).unwrap_or(None),
            last_error_message: stmt.read(13).unwrap_or(None),
            attempt_count: stmt.read(14).unwrap_or(0),
            computed_sha256: stmt.read(15).unwrap_or(None),
            telegram_message_id: stmt.read(16).unwrap_or(None),
            created_at: stmt.read(17).unwrap_or(0),
            completed_at: stmt.read(18).unwrap_or(None),
            queue_position: stmt.read(19).unwrap_or(0),
            action_type: stmt.read(20).unwrap_or(None),
        }))
    } else {
        Ok(None)
    }
}

pub fn get_pending_items_by_job(
    db: &MigrationDb,
    job_id: i64,
) -> Result<Vec<MigrationItem>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT id, job_id, item_type, name, source_path, source_item_id, size_bytes, source_etag, source_last_modified, source_fingerprint_type, source_fingerprint_value, state, last_error_code, last_error_message, attempt_count, computed_sha256, telegram_message_id, created_at, completed_at, queue_position, action_type FROM migration_items WHERE job_id = ? AND state = 'pending' ORDER BY queue_position ASC, id ASC;"
    ).map_err(|e| e.to_string())?;
    stmt.bind((1, job_id)).map_err(|e| e.to_string())?;

    let mut items = Vec::new();
    while let Ok(sqlite::State::Row) = stmt.next() {
        items.push(MigrationItem {
            id: stmt.read(0).unwrap_or(0),
            job_id: stmt.read(1).unwrap_or(0),
            item_type: stmt.read(2).unwrap_or_else(|_| "file".into()),
            name: stmt.read(3).unwrap_or_default(),
            source_path: stmt.read(4).unwrap_or_default(),
            source_item_id: stmt.read(5).unwrap_or(None),
            size_bytes: stmt.read(6).unwrap_or(0),
            source_etag: stmt.read(7).unwrap_or(None),
            source_last_modified: stmt.read(8).unwrap_or(None),
            source_fingerprint_type: stmt.read(9).unwrap_or(None),
            source_fingerprint_value: stmt.read(10).unwrap_or(None),
            state: stmt.read(11).unwrap_or_else(|_| "pending".into()),
            last_error_code: stmt.read(12).unwrap_or(None),
            last_error_message: stmt.read(13).unwrap_or(None),
            attempt_count: stmt.read(14).unwrap_or(0),
            computed_sha256: stmt.read(15).unwrap_or(None),
            telegram_message_id: stmt.read(16).unwrap_or(None),
            created_at: stmt.read(17).unwrap_or(0),
            completed_at: stmt.read(18).unwrap_or(None),
            queue_position: stmt.read(19).unwrap_or(0),
            action_type: stmt.read(20).unwrap_or(None),
        });
    }
    Ok(items)
}

/// Check duplicate in `migrated_fingerprints`
pub fn check_fingerprint(
    db: &MigrationDb,
    fingerprint_type: &str,
    fingerprint_value: &str,
    file_size: i64,
) -> Result<bool, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT 1 FROM migrated_fingerprints WHERE fingerprint_type = ? AND fingerprint_value = ? AND file_size = ?;"
    ).map_err(|e| e.to_string())?;
    stmt.bind((1, fingerprint_type))
        .map_err(|e| e.to_string())?;
    stmt.bind((2, fingerprint_value))
        .map_err(|e| e.to_string())?;
    stmt.bind((3, file_size)).map_err(|e| e.to_string())?;

    if let Ok(sqlite::State::Row) = stmt.next() {
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Success Transaction: atomic mark completed + insert fingerprint(s) + update stats
pub fn record_item_success(
    db: &MigrationDb,
    job_id: i64,
    item_id: i64,
    sha256_hash: &str,
    provider_fingerprint: Option<(&str, &str)>, // (type, val)
    file_size: i64,
    telegram_dest_id: Option<i64>,
    telegram_msg_id: Option<i32>,
    count_auto_quota: bool,
) -> Result<(), String> {
    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp();

        conn.execute("BEGIN TRANSACTION;")
            .map_err(|e| e.to_string())?;

        // 1. Update item
        let mut stmt = conn.prepare(
            "UPDATE migration_items SET state = 'completed', computed_sha256 = ?, telegram_message_id = ?, completed_at = ?, last_error_code = NULL, last_error_message = NULL WHERE id = ?;"
        ).map_err(|e| e.to_string())?;
        stmt.bind((1, sha256_hash)).map_err(|e| e.to_string())?;
        match telegram_msg_id {
            Some(msg_id) => stmt.bind((2, msg_id as i64)).map_err(|e| e.to_string())?,
            None => stmt
                .bind((2, sqlite::Value::Null))
                .map_err(|e| e.to_string())?,
        }
        stmt.bind((3, now)).map_err(|e| e.to_string())?;
        stmt.bind((4, item_id)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;

        // 2. Insert SHA-256 fingerprint
        let mut fp_stmt = conn.prepare(
            "INSERT OR IGNORE INTO migrated_fingerprints (fingerprint_type, fingerprint_value, file_size, first_job_id, first_item_id, telegram_destination_id, telegram_message_id, completed_at) VALUES ('sha256', ?, ?, ?, ?, ?, ?, ?);"
        ).map_err(|e| e.to_string())?;
        fp_stmt.bind((1, sha256_hash)).map_err(|e| e.to_string())?;
        fp_stmt.bind((2, file_size)).map_err(|e| e.to_string())?;
        fp_stmt.bind((3, job_id)).map_err(|e| e.to_string())?;
        fp_stmt.bind((4, item_id)).map_err(|e| e.to_string())?;
        match telegram_dest_id {
            Some(did) => fp_stmt.bind((5, did)).map_err(|e| e.to_string())?,
            None => fp_stmt
                .bind((5, sqlite::Value::Null))
                .map_err(|e| e.to_string())?,
        }
        match telegram_msg_id {
            Some(msg_id) => fp_stmt
                .bind((6, msg_id as i64))
                .map_err(|e| e.to_string())?,
            None => fp_stmt
                .bind((6, sqlite::Value::Null))
                .map_err(|e| e.to_string())?,
        }
        fp_stmt.bind((7, now)).map_err(|e| e.to_string())?;
        fp_stmt.next().map_err(|e| e.to_string())?;

        // 3. Insert Provider fingerprint if present
        if let Some((fp_type, fp_val)) = provider_fingerprint {
            let mut pfp_stmt = conn.prepare(
                "INSERT OR IGNORE INTO migrated_fingerprints (fingerprint_type, fingerprint_value, file_size, first_job_id, first_item_id, telegram_destination_id, telegram_message_id, completed_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?);"
            ).map_err(|e| e.to_string())?;
            pfp_stmt.bind((1, fp_type)).map_err(|e| e.to_string())?;
            pfp_stmt.bind((2, fp_val)).map_err(|e| e.to_string())?;
            pfp_stmt.bind((3, file_size)).map_err(|e| e.to_string())?;
            pfp_stmt.bind((4, job_id)).map_err(|e| e.to_string())?;
            pfp_stmt.bind((5, item_id)).map_err(|e| e.to_string())?;
            match telegram_dest_id {
                Some(did) => pfp_stmt.bind((6, did)).map_err(|e| e.to_string())?,
                None => pfp_stmt
                    .bind((6, sqlite::Value::Null))
                    .map_err(|e| e.to_string())?,
            }
            match telegram_msg_id {
                Some(msg_id) => pfp_stmt
                    .bind((7, msg_id as i64))
                    .map_err(|e| e.to_string())?,
                None => pfp_stmt
                    .bind((7, sqlite::Value::Null))
                    .map_err(|e| e.to_string())?,
            }
            pfp_stmt.bind((8, now)).map_err(|e| e.to_string())?;
            pfp_stmt.next().map_err(|e| e.to_string())?;
        }

        if count_auto_quota {
            let today = get_today_date_string();
            let mut quota_stmt = conn
                .prepare(
                    "INSERT INTO daily_migration_quota (date_string, uploaded_bytes, updated_at)
                     VALUES (?, ?, ?)
                     ON CONFLICT(date_string) DO UPDATE SET
                         uploaded_bytes = uploaded_bytes + excluded.uploaded_bytes,
                         updated_at = excluded.updated_at;",
                )
                .map_err(|e| e.to_string())?;
            quota_stmt
                .bind((1, today.as_str()))
                .map_err(|e| e.to_string())?;
            quota_stmt.bind((2, file_size)).map_err(|e| e.to_string())?;
            quota_stmt.bind((3, now)).map_err(|e| e.to_string())?;
            quota_stmt.next().map_err(|e| e.to_string())?;
        }

        conn.execute("COMMIT;").map_err(|e| e.to_string())?;
    }

    update_job_stats(db, job_id)?;
    Ok(())
}

pub fn create_migration_job(
    db: &MigrationDb,
    folder_id: Option<&str>,
    folder_path: Option<&str>,
    dest_id: Option<i64>,
    dest_name: Option<&str>,
    local_dir: &str,
    items: &[OneDriveItem],
) -> Result<(i64, MigrationStats), String> {
    let job = create_job(db)?;
    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        conn.execute(format!(
            "UPDATE migration_jobs SET job_origin = 'auto' WHERE id = {};",
            job.id
        ))
        .map_err(|e| e.to_string())?;
    }
    if let (Some(fid), Some(fpath)) = (folder_id, folder_path) {
        set_onedrive_folder(db, job.id, fid.to_string(), fpath.to_string())?;
    }
    if let Some(dn) = dest_name {
        set_telegram_destination(db, job.id, dest_id, dn.to_string())?;
    }
    set_local_dir(db, job.id, local_dir.to_string())?;
    let stats = batch_insert_items(db, job.id, items)?;
    Ok((job.id, stats))
}

pub fn create_migration_job_with_action(
    db: &MigrationDb,
    folder_id: Option<&str>,
    folder_path: Option<&str>,
    dest_id: Option<i64>,
    dest_name: Option<&str>,
    local_dir: &str,
    items: &[OneDriveItem],
    action_type: Option<&str>,
) -> Result<(i64, MigrationStats), String> {
    let job = create_job(db)?;
    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        conn.execute(format!(
            "UPDATE migration_jobs SET job_origin = 'auto' WHERE id = {};",
            job.id
        ))
        .map_err(|e| e.to_string())?;
    }
    if let (Some(fid), Some(fpath)) = (folder_id, folder_path) {
        set_onedrive_folder(db, job.id, fid.to_string(), fpath.to_string())?;
    }
    if let Some(dn) = dest_name {
        set_telegram_destination(db, job.id, dest_id, dn.to_string())?;
    }
    set_local_dir(db, job.id, local_dir.to_string())?;
    let stats = batch_insert_items_with_action(db, job.id, items, action_type)?;
    Ok((job.id, stats))
}

/// Mark item skipped as duplicate
pub fn record_item_skipped_duplicate(
    db: &MigrationDb,
    job_id: i64,
    item_id: i64,
    sha256_hash: Option<&str>,
) -> Result<(), String> {
    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp();

        let mut stmt = conn.prepare(
            "UPDATE migration_items SET state = 'skipped_duplicate', computed_sha256 = COALESCE(?, computed_sha256), completed_at = ?, last_error_code = NULL, last_error_message = NULL WHERE id = ?;"
        ).map_err(|e| e.to_string())?;

        match sha256_hash {
            Some(h) => stmt.bind((1, h)).map_err(|e| e.to_string())?,
            None => stmt
                .bind((1, sqlite::Value::Null))
                .map_err(|e| e.to_string())?,
        }
        stmt.bind((2, now)).map_err(|e| e.to_string())?;
        stmt.bind((3, item_id)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;
    }

    update_job_stats(db, job_id)?;
    Ok(())
}

/// Mark item skipped as non-media file
pub fn record_item_skipped_non_media(
    db: &MigrationDb,
    job_id: i64,
    item_id: i64,
) -> Result<(), String> {
    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp();

        let mut stmt = conn.prepare(
            "UPDATE migration_items SET state = 'skipped_non_media', completed_at = ?, last_error_code = NULL, last_error_message = NULL WHERE id = ?;"
        ).map_err(|e| e.to_string())?;

        stmt.bind((1, now)).map_err(|e| e.to_string())?;
        stmt.bind((2, item_id)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;
    }

    update_job_stats(db, job_id)?;
    Ok(())
}

/// Mark item failed
pub fn record_item_failed(
    db: &MigrationDb,
    job_id: i64,
    item_id: i64,
    error_code: &str,
    error_message: &str,
    increment_attempt: bool,
) -> Result<(), String> {
    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp();

        let attempt_sql = if increment_attempt {
            "attempt_count = attempt_count + 1,"
        } else {
            ""
        };

        let sql = format!(
            "UPDATE migration_items SET state = 'failed', last_error_code = ?, last_error_message = ?, {} completed_at = ? WHERE id = ?;",
            attempt_sql
        );

        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        stmt.bind((1, error_code)).map_err(|e| e.to_string())?;
        stmt.bind((2, error_message)).map_err(|e| e.to_string())?;
        stmt.bind((3, now)).map_err(|e| e.to_string())?;
        stmt.bind((4, item_id)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;
    }

    update_job_stats(db, job_id)?;
    Ok(())
}

/// Reset failed item(s) for retry
pub fn retry_item(db: &MigrationDb, job_id: i64, item_id: i64) -> Result<(), String> {
    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "UPDATE migration_items SET state = 'pending', attempt_count = 0, last_error_code = NULL, last_error_message = NULL WHERE id = ? AND job_id = ?;"
        ).map_err(|e| e.to_string())?;
        stmt.bind((1, item_id)).map_err(|e| e.to_string())?;
        stmt.bind((2, job_id)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;
    }

    update_job_stats(db, job_id)?;
    Ok(())
}

pub fn retry_all_failed(db: &MigrationDb, job_id: i64) -> Result<i64, String> {
    let count = {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "UPDATE migration_items SET state = 'pending', attempt_count = 0, last_error_code = NULL, last_error_message = NULL WHERE job_id = ? AND state = 'failed';"
        ).map_err(|e| e.to_string())?;
        stmt.bind((1, job_id)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;

        conn.change_count() as i64
    };

    update_job_stats(db, job_id)?;
    Ok(count)
}

pub fn delete_item(db: &MigrationDb, job_id: i64, item_id: i64) -> Result<(), String> {
    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("DELETE FROM migration_items WHERE id = ? AND job_id = ?;")
            .map_err(|e| e.to_string())?;
        stmt.bind((1, item_id)).map_err(|e| e.to_string())?;
        stmt.bind((2, job_id)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;
    }
    update_job_stats(db, job_id)?;
    Ok(())
}

pub fn rename_item(
    db: &MigrationDb,
    job_id: i64,
    item_id: i64,
    new_name: &str,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("UPDATE migration_items SET name = ? WHERE id = ? AND job_id = ?;")
        .map_err(|e| e.to_string())?;
    stmt.bind((1, new_name)).map_err(|e| e.to_string())?;
    stmt.bind((2, item_id)).map_err(|e| e.to_string())?;
    stmt.bind((3, job_id)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_auto_profile(
    db: &MigrationDb,
    account_id: &str,
) -> Result<Option<AutoMigrationProfile>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, account_id, enabled, default_telegram_dest_id, default_telegram_dest_name, local_temp_dir, last_auto_scan_at, created_at, updated_at, active_job_id, pause_reason FROM auto_migration_profiles WHERE account_id = ?;"
        )
        .map_err(|e| e.to_string())?;
    stmt.bind((1, account_id)).map_err(|e| e.to_string())?;

    if let Ok(sqlite::State::Row) = stmt.next() {
        let enabled_val: i64 = stmt.read(2).unwrap_or(1);
        Ok(Some(AutoMigrationProfile {
            id: stmt.read(0).unwrap_or(0),
            account_id: stmt.read::<String, _>(1).unwrap_or_default(),
            enabled: enabled_val != 0,
            default_telegram_dest_id: stmt.read(3).ok(),
            default_telegram_dest_name: stmt.read(4).ok(),
            local_temp_dir: stmt.read(5).ok(),
            last_auto_scan_at: stmt.read(6).ok(),
            created_at: stmt.read(7).unwrap_or(0),
            updated_at: stmt.read(8).unwrap_or(0),
            active_job_id: stmt.read(9).ok(),
            pause_reason: stmt.read(10).ok(),
        }))
    } else {
        Ok(None)
    }
}

pub fn upsert_auto_profile(
    db: &MigrationDb,
    account_id: &str,
    enabled: bool,
    dest_id: Option<i64>,
    dest_name: Option<&str>,
    temp_dir: Option<&str>,
) -> Result<AutoMigrationProfile, String> {
    let now = chrono::Utc::now().timestamp();
    let enabled_val = if enabled { 1 } else { 0 };

    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "INSERT INTO auto_migration_profiles (account_id, enabled, default_telegram_dest_id, default_telegram_dest_name, local_temp_dir, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(account_id) DO UPDATE SET
                     enabled = excluded.enabled,
                     default_telegram_dest_id = COALESCE(excluded.default_telegram_dest_id, auto_migration_profiles.default_telegram_dest_id),
                     default_telegram_dest_name = COALESCE(excluded.default_telegram_dest_name, auto_migration_profiles.default_telegram_dest_name),
                     local_temp_dir = COALESCE(excluded.local_temp_dir, auto_migration_profiles.local_temp_dir),
                     updated_at = excluded.updated_at;",
            )
            .map_err(|e| e.to_string())?;

        stmt.bind((1, account_id)).map_err(|e| e.to_string())?;
        stmt.bind((2, enabled_val)).map_err(|e| e.to_string())?;
        stmt.bind((3, dest_id)).map_err(|e| e.to_string())?;
        stmt.bind((4, dest_name)).map_err(|e| e.to_string())?;
        stmt.bind((5, temp_dir)).map_err(|e| e.to_string())?;
        stmt.bind((6, now)).map_err(|e| e.to_string())?;
        stmt.bind((7, now)).map_err(|e| e.to_string())?;

        stmt.next().map_err(|e| e.to_string())?;
    }

    get_auto_profile(db, account_id)?
        .ok_or_else(|| "Failed to retrieve saved auto profile".to_string())
}

pub fn get_today_date_string() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub const DAILY_UPLOAD_LIMIT_BYTES: u64 = 250 * 1024 * 1024 * 1024; // 250 GB

pub fn would_exceed_daily_quota(uploaded_bytes: u64, next_file_bytes: i64) -> bool {
    uploaded_bytes.saturating_add(next_file_bytes.max(0) as u64) > DAILY_UPLOAD_LIMIT_BYTES
}

pub fn get_daily_quota(db: &MigrationDb) -> Result<DailyMigrationQuota, String> {
    let today = get_today_date_string();
    let conn = db.lock().map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare("SELECT uploaded_bytes FROM daily_migration_quota WHERE date_string = ?;")
        .map_err(|e| e.to_string())?;
    stmt.bind((1, today.as_str())).map_err(|e| e.to_string())?;

    let uploaded: u64 = if let Ok(sqlite::State::Row) = stmt.next() {
        stmt.read::<i64, _>(0).unwrap_or(0) as u64
    } else {
        0
    };

    use chrono::TimeZone;
    let now = chrono::Local::now();
    let tomorrow = now.date_naive().succ_opt().unwrap_or(now.date_naive());
    let resets_at = tomorrow
        .and_hms_opt(0, 0, 0)
        .and_then(|value| chrono::Local.from_local_datetime(&value).earliest())
        .map(|value| value.timestamp())
        .unwrap_or_else(|| now.timestamp() + 86_400);

    Ok(DailyMigrationQuota {
        date_string: today,
        uploaded_bytes: uploaded,
        limit_bytes: DAILY_UPLOAD_LIMIT_BYTES,
        remaining_bytes: DAILY_UPLOAD_LIMIT_BYTES.saturating_sub(uploaded),
        resets_at,
    })
}

pub fn add_daily_uploaded_bytes(db: &MigrationDb, bytes: u64) -> Result<u64, String> {
    let today = get_today_date_string();
    let now = chrono::Utc::now().timestamp();
    let conn = db.lock().map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare(
            "INSERT INTO daily_migration_quota (date_string, uploaded_bytes, updated_at)
             VALUES (?, ?, ?)
             ON CONFLICT(date_string) DO UPDATE SET
                 uploaded_bytes = uploaded_bytes + excluded.uploaded_bytes,
                 updated_at = excluded.updated_at;",
        )
        .map_err(|e| e.to_string())?;

    stmt.bind((1, today.as_str())).map_err(|e| e.to_string())?;
    stmt.bind((2, bytes as i64)).map_err(|e| e.to_string())?;
    stmt.bind((3, now)).map_err(|e| e.to_string())?;

    stmt.next().map_err(|e| e.to_string())?;

    let mut stmt_sel = conn
        .prepare("SELECT uploaded_bytes FROM daily_migration_quota WHERE date_string = ?;")
        .map_err(|e| e.to_string())?;
    stmt_sel
        .bind((1, today.as_str()))
        .map_err(|e| e.to_string())?;

    if let Ok(sqlite::State::Row) = stmt_sel.next() {
        Ok(stmt_sel.read::<i64, _>(0).unwrap_or(0) as u64)
    } else {
        Ok(0)
    }
}

pub fn set_auto_active_job(db: &MigrationDb, account_id: &str, job_id: i64) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "UPDATE auto_migration_profiles
             SET active_job_id = ?, last_auto_scan_at = ?, pause_reason = NULL, updated_at = ?
             WHERE account_id = ?;",
        )
        .map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp();
    stmt.bind((1, job_id)).map_err(|e| e.to_string())?;
    stmt.bind((2, now)).map_err(|e| e.to_string())?;
    stmt.bind((3, now)).map_err(|e| e.to_string())?;
    stmt.bind((4, account_id)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn pause_auto_job(db: &MigrationDb, account_id: &str, reason: &str) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp();
    let mut profile = conn
        .prepare(
            "UPDATE auto_migration_profiles SET pause_reason = ?, updated_at = ? WHERE account_id = ?;",
        )
        .map_err(|e| e.to_string())?;
    profile.bind((1, reason)).map_err(|e| e.to_string())?;
    profile.bind((2, now)).map_err(|e| e.to_string())?;
    profile.bind((3, account_id)).map_err(|e| e.to_string())?;
    profile.next().map_err(|e| e.to_string())?;
    conn.execute(format!(
        "UPDATE migration_jobs SET state = 'paused', pause_reason = '{}', updated_at = {}
         WHERE id = (SELECT active_job_id FROM auto_migration_profiles WHERE account_id = '{}')
           AND state IN ('ready', 'running');",
        reason.replace('\'', "''"),
        now,
        account_id.replace('\'', "''")
    ))
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn record_activity(
    db: &MigrationDb,
    job_id: i64,
    item_id: Option<i64>,
    item_name: Option<&str>,
    phase: &str,
    status: &str,
    message: Option<&str>,
) -> Result<MigrationActivity, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp();
    let mut stmt = conn
        .prepare(
            "INSERT INTO migration_activity
                (job_id, item_id, item_name, phase, status, attempt, revision, message, created_at)
             VALUES (?, ?, ?, ?, ?, 0, 0, ?, ?);",
        )
        .map_err(|e| e.to_string())?;
    stmt.bind((1, job_id)).map_err(|e| e.to_string())?;
    stmt.bind((2, item_id)).map_err(|e| e.to_string())?;
    stmt.bind((3, item_name)).map_err(|e| e.to_string())?;
    stmt.bind((4, phase)).map_err(|e| e.to_string())?;
    stmt.bind((5, status)).map_err(|e| e.to_string())?;
    stmt.bind((6, message)).map_err(|e| e.to_string())?;
    stmt.bind((7, now)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;
    let activity_id = {
        let mut id_stmt = conn
            .prepare("SELECT last_insert_rowid();")
            .map_err(|e| e.to_string())?;
        if let Ok(sqlite::State::Row) = id_stmt.next() {
            id_stmt.read(0).unwrap_or(0)
        } else {
            0
        }
    };
    Ok(MigrationActivity {
        id: activity_id,
        job_id,
        item_id,
        item_name: item_name.map(str::to_string),
        phase: phase.to_string(),
        status: status.to_string(),
        attempt: 0,
        revision: 0,
        message: message.map(str::to_string),
        created_at: now,
    })
}

pub fn get_activity(
    db: &MigrationDb,
    job_id: i64,
    limit: i64,
) -> Result<Vec<MigrationActivity>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, job_id, item_id, item_name, phase, status, attempt, revision, message, created_at
             FROM migration_activity WHERE job_id = ?
             ORDER BY created_at DESC, id DESC LIMIT ?;",
        )
        .map_err(|e| e.to_string())?;
    stmt.bind((1, job_id)).map_err(|e| e.to_string())?;
    stmt.bind((2, limit.clamp(1, 500)))
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    while let Ok(sqlite::State::Row) = stmt.next() {
        entries.push(MigrationActivity {
            id: stmt.read(0).unwrap_or(0),
            job_id: stmt.read(1).unwrap_or(0),
            item_id: stmt.read(2).unwrap_or(None),
            item_name: stmt.read(3).unwrap_or(None),
            phase: stmt.read(4).unwrap_or_default(),
            status: stmt.read(5).unwrap_or_default(),
            attempt: stmt.read(6).unwrap_or(0),
            revision: stmt.read(7).unwrap_or(0),
            message: stmt.read(8).unwrap_or(None),
            created_at: stmt.read(9).unwrap_or(0),
        });
    }
    Ok(entries)
}

pub fn is_auto_job(db: &MigrationDb, job_id: i64) -> Result<bool, String> {
    Ok(get_job(db, job_id)?.job_origin == "auto")
}

pub fn get_auto_scan_checkpoint(
    db: &MigrationDb,
    account_id: &str,
) -> Result<Option<AutoScanCheckpoint>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT next_link, pages_scanned, discovered_files, discovered_folders,
                    elapsed_ms, status
             FROM auto_scan_checkpoints WHERE account_id = ?;",
        )
        .map_err(|e| e.to_string())?;
    stmt.bind((1, account_id)).map_err(|e| e.to_string())?;
    if let Ok(sqlite::State::Row) = stmt.next() {
        Ok(Some(AutoScanCheckpoint {
            next_link: stmt.read(0).ok(),
            pages_scanned: stmt.read::<i64, _>(1).unwrap_or(0).max(0) as usize,
            discovered_files: stmt.read::<i64, _>(2).unwrap_or(0).max(0) as usize,
            discovered_folders: stmt.read::<i64, _>(3).unwrap_or(0).max(0) as usize,
            elapsed_ms: stmt.read::<i64, _>(4).unwrap_or(0).max(0) as u64,
            status: stmt
                .read::<String, _>(5)
                .unwrap_or_else(|_| "enumerating".into()),
        }))
    } else {
        Ok(None)
    }
}

pub fn load_auto_scan_items(
    db: &MigrationDb,
    account_id: &str,
) -> Result<Vec<serde_json::Value>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT payload_json FROM auto_scan_items
             WHERE account_id = ? ORDER BY item_id;",
        )
        .map_err(|e| e.to_string())?;
    stmt.bind((1, account_id)).map_err(|e| e.to_string())?;
    let mut values = Vec::new();
    while let Ok(sqlite::State::Row) = stmt.next() {
        let payload = stmt.read::<String, _>(0).unwrap_or_default();
        values.push(
            serde_json::from_str(&payload)
                .map_err(|e| format!("Invalid persisted OneDrive scan item: {e}"))?,
        );
    }
    Ok(values)
}

pub fn save_auto_scan_page(
    db: &MigrationDb,
    account_id: &str,
    values: &[serde_json::Value],
    next_link: Option<&str>,
    progress: &ScanProgressPayload,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    conn.execute("BEGIN IMMEDIATE TRANSACTION;")
        .map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        let mut upsert = conn
            .prepare(
                "INSERT INTO auto_scan_items (account_id, item_id, payload_json)
                 VALUES (?, ?, ?)
                 ON CONFLICT(account_id, item_id) DO UPDATE SET
                    payload_json = excluded.payload_json;",
            )
            .map_err(|e| e.to_string())?;
        let mut delete = conn
            .prepare("DELETE FROM auto_scan_items WHERE account_id = ? AND item_id = ?;")
            .map_err(|e| e.to_string())?;

        for value in values {
            let Some(item_id) = value["id"].as_str().filter(|id| !id.is_empty()) else {
                continue;
            };
            if value.get("deleted").is_some() {
                delete.bind((1, account_id)).map_err(|e| e.to_string())?;
                delete.bind((2, item_id)).map_err(|e| e.to_string())?;
                delete.next().map_err(|e| e.to_string())?;
                delete.reset().map_err(|e| e.to_string())?;
            } else {
                let payload = serde_json::to_string(value).map_err(|e| e.to_string())?;
                upsert.bind((1, account_id)).map_err(|e| e.to_string())?;
                upsert.bind((2, item_id)).map_err(|e| e.to_string())?;
                upsert
                    .bind((3, payload.as_str()))
                    .map_err(|e| e.to_string())?;
                upsert.next().map_err(|e| e.to_string())?;
                upsert.reset().map_err(|e| e.to_string())?;
            }
        }
        drop(upsert);
        drop(delete);

        let mut checkpoint = conn
            .prepare(
                "INSERT INTO auto_scan_checkpoints (
                    account_id, next_link, pages_scanned, discovered_files,
                    discovered_folders, elapsed_ms, status, updated_at
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(account_id) DO UPDATE SET
                    next_link = excluded.next_link,
                    pages_scanned = excluded.pages_scanned,
                    discovered_files = excluded.discovered_files,
                    discovered_folders = excluded.discovered_folders,
                    elapsed_ms = excluded.elapsed_ms,
                    status = CASE
                        WHEN auto_scan_checkpoints.status = 'stopped' THEN 'stopped'
                        ELSE excluded.status
                    END,
                    updated_at = excluded.updated_at;",
            )
            .map_err(|e| e.to_string())?;
        checkpoint
            .bind((1, account_id))
            .map_err(|e| e.to_string())?;
        checkpoint.bind((2, next_link)).map_err(|e| e.to_string())?;
        checkpoint
            .bind((3, progress.pages_scanned as i64))
            .map_err(|e| e.to_string())?;
        checkpoint
            .bind((4, progress.discovered_files as i64))
            .map_err(|e| e.to_string())?;
        checkpoint
            .bind((5, progress.discovered_folders as i64))
            .map_err(|e| e.to_string())?;
        checkpoint
            .bind((6, progress.elapsed_ms as i64))
            .map_err(|e| e.to_string())?;
        checkpoint
            .bind((7, progress.phase.as_str()))
            .map_err(|e| e.to_string())?;
        checkpoint
            .bind((8, chrono::Utc::now().timestamp()))
            .map_err(|e| e.to_string())?;
        checkpoint.next().map_err(|e| e.to_string())?;
        Ok(())
    })();

    match result {
        Ok(()) => conn.execute("COMMIT;").map_err(|e| e.to_string()),
        Err(error) => {
            let _ = conn.execute("ROLLBACK;");
            Err(error)
        }
    }
}

pub fn set_auto_scan_status(
    db: &MigrationDb,
    account_id: &str,
    status: &str,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "UPDATE auto_scan_checkpoints SET status = ?, updated_at = ?
             WHERE account_id = ?;",
        )
        .map_err(|e| e.to_string())?;
    stmt.bind((1, status)).map_err(|e| e.to_string())?;
    stmt.bind((2, chrono::Utc::now().timestamp()))
        .map_err(|e| e.to_string())?;
    stmt.bind((3, account_id)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn clear_auto_scan_checkpoint(db: &MigrationDb, account_id: &str) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    conn.execute("BEGIN IMMEDIATE TRANSACTION;")
        .map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        for sql in [
            "DELETE FROM auto_scan_items WHERE account_id = ?;",
            "DELETE FROM auto_scan_checkpoints WHERE account_id = ?;",
        ] {
            let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
            stmt.bind((1, account_id)).map_err(|e| e.to_string())?;
            stmt.next().map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => conn.execute("COMMIT;").map_err(|e| e.to_string()),
        Err(error) => {
            let _ = conn.execute("ROLLBACK;");
            Err(error)
        }
    }
}

pub fn finalize_auto_scan(db: &MigrationDb, account_id: &str, job_id: i64) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    conn.execute("BEGIN IMMEDIATE TRANSACTION;")
        .map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        let now = chrono::Utc::now().timestamp();
        let mut profile = conn
            .prepare(
                "UPDATE auto_migration_profiles
                 SET active_job_id = ?, last_auto_scan_at = ?, pause_reason = NULL, updated_at = ?
                 WHERE account_id = ?;",
            )
            .map_err(|e| e.to_string())?;
        profile.bind((1, job_id)).map_err(|e| e.to_string())?;
        profile.bind((2, now)).map_err(|e| e.to_string())?;
        profile.bind((3, now)).map_err(|e| e.to_string())?;
        profile.bind((4, account_id)).map_err(|e| e.to_string())?;
        profile.next().map_err(|e| e.to_string())?;
        drop(profile);

        for sql in [
            "DELETE FROM auto_scan_items WHERE account_id = ?;",
            "DELETE FROM auto_scan_checkpoints WHERE account_id = ?;",
        ] {
            let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
            stmt.bind((1, account_id)).map_err(|e| e.to_string())?;
            stmt.next().map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => conn.execute("COMMIT;").map_err(|e| e.to_string()),
        Err(error) => {
            let _ = conn.execute("ROLLBACK;");
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db(name: &str) -> (PathBuf, MigrationDb) {
        let path = std::env::temp_dir().join(format!(
            "telegram-drive-{name}-{}-{}.db",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let db = open_migration_db_at_path(path.clone()).expect("open test migration db");
        (path, db)
    }

    fn cleanup(path: &PathBuf, db: MigrationDb) {
        drop(db);
        for candidate in [
            path.clone(),
            PathBuf::from(format!("{}-wal", path.display())),
            PathBuf::from(format!("{}-shm", path.display())),
        ] {
            let _ = std::fs::remove_file(candidate);
        }
    }

    #[test]
    fn projected_quota_pauses_before_overshoot() {
        assert!(!would_exceed_daily_quota(DAILY_UPLOAD_LIMIT_BYTES - 10, 10));
        assert!(would_exceed_daily_quota(DAILY_UPLOAD_LIMIT_BYTES - 10, 11));
        assert_eq!(
            get_today_date_string(),
            chrono::Local::now().format("%Y-%m-%d").to_string()
        );
    }

    #[test]
    fn auto_snapshot_persists_order_and_ownership_even_when_empty() {
        let (path, db) = test_db("snapshot");
        upsert_auto_profile(&db, "test@example.com", true, None, None, None).unwrap();
        let (job_id, stats) = create_migration_job(
            &db,
            Some("root"),
            Some("/ (Root)"),
            None,
            Some("Saved Messages"),
            std::env::temp_dir().to_string_lossy().as_ref(),
            &[],
        )
        .unwrap();
        set_auto_active_job(&db, "test@example.com", job_id).unwrap();

        assert_eq!(stats.total_files, 0);
        assert_eq!(get_job(&db, job_id).unwrap().job_origin, "auto");
        let manual_job = create_job(&db).unwrap();
        assert!(!is_auto_job(&db, manual_job.id).unwrap());
        assert_eq!(
            get_auto_profile(&db, "test@example.com")
                .unwrap()
                .unwrap()
                .active_job_id,
            Some(job_id)
        );
        cleanup(&path, db);
    }

    #[test]
    fn auto_success_accounts_quota_and_activity_persists() {
        let (path, db) = test_db("quota-activity");
        let source = OneDriveItem {
            id: "source-1".into(),
            name: "one.bin".into(),
            item_type: "file".into(),
            size: 42,
            path: Some("one.bin".into()),
            child_count: None,
            etag: None,
            quickxor_hash: None,
            sha1_hash: None,
            last_modified: None,
        };
        let (job_id, _) = create_migration_job(
            &db,
            Some("root"),
            Some("/ (Root)"),
            None,
            Some("Saved Messages"),
            std::env::temp_dir().to_string_lossy().as_ref(),
            &[source],
        )
        .unwrap();
        let item = get_next_pending_item(&db, job_id).unwrap().unwrap();
        record_item_success(
            &db,
            job_id,
            item.id,
            "sha256",
            None,
            item.size_bytes,
            None,
            Some(1),
            true,
        )
        .unwrap();
        assert_eq!(get_daily_quota(&db).unwrap().uploaded_bytes, 42);

        record_activity(
            &db,
            job_id,
            Some(item.id),
            Some(&item.name),
            "completed",
            "completed",
            None,
        )
        .unwrap();
        let activity = get_activity(&db, job_id, 10).unwrap();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].item_id, Some(item.id));
        cleanup(&path, db);
    }

    #[test]
    fn auto_scan_checkpoint_saves_pages_and_survives_stop() {
        let (path, db) = test_db("scan-checkpoint");
        let account = "checkpoint@example.com";
        upsert_auto_profile(&db, account, true, None, None, None).unwrap();
        let first_page = vec![
            serde_json::json!({
                "id": "root",
                "name": "OneDrive",
                "folder": { "childCount": 1 },
                "root": {}
            }),
            serde_json::json!({
                "id": "file-1",
                "name": "one.txt",
                "size": 10,
                "file": {},
                "parentReference": { "id": "root" }
            }),
        ];
        let first_progress = ScanProgressPayload {
            phase: "enumerating".into(),
            pages_scanned: 1,
            discovered_files: 1,
            discovered_folders: 0,
            elapsed_ms: 100,
        };
        save_auto_scan_page(
            &db,
            account,
            &first_page,
            Some("https://graph.microsoft.com/next"),
            &first_progress,
        )
        .unwrap();
        assert_eq!(load_auto_scan_items(&db, account).unwrap().len(), 2);

        set_auto_scan_status(&db, account, "stopped").unwrap();
        let second_progress = ScanProgressPayload {
            phase: "enumerating".into(),
            pages_scanned: 2,
            discovered_files: 0,
            discovered_folders: 0,
            elapsed_ms: 200,
        };
        save_auto_scan_page(
            &db,
            account,
            &[serde_json::json!({ "id": "file-1", "deleted": {} })],
            Some("https://graph.microsoft.com/next-2"),
            &second_progress,
        )
        .unwrap();
        let checkpoint = get_auto_scan_checkpoint(&db, account).unwrap().unwrap();
        assert_eq!(checkpoint.pages_scanned, 2);
        assert_eq!(checkpoint.status, "stopped");
        assert_eq!(load_auto_scan_items(&db, account).unwrap().len(), 1);

        let (job_id, _) = create_migration_job(
            &db,
            Some("root"),
            Some("/ (Root)"),
            None,
            Some("Saved Messages"),
            std::env::temp_dir().to_string_lossy().as_ref(),
            &[],
        )
        .unwrap();
        finalize_auto_scan(&db, account, job_id).unwrap();
        assert!(get_auto_scan_checkpoint(&db, account).unwrap().is_none());
        assert!(load_auto_scan_items(&db, account).unwrap().is_empty());
        assert_eq!(
            get_auto_profile(&db, account)
                .unwrap()
                .unwrap()
                .active_job_id,
            Some(job_id)
        );
        cleanup(&path, db);
    }

    #[test]
    fn clearing_auto_scan_checkpoint_removes_progress_and_staged_items() {
        let (path, db) = test_db("clear-scan-checkpoint");
        let account = "clear-checkpoint@example.com";
        let progress = ScanProgressPayload {
            phase: "stopped".into(),
            pages_scanned: 3,
            discovered_files: 1,
            discovered_folders: 0,
            elapsed_ms: 300,
        };
        save_auto_scan_page(
            &db,
            account,
            &[serde_json::json!({
                "id": "file-1",
                "name": "one.txt",
                "size": 10,
                "file": {}
            })],
            Some("https://graph.microsoft.com/next"),
            &progress,
        )
        .unwrap();

        clear_auto_scan_checkpoint(&db, account).unwrap();

        assert!(get_auto_scan_checkpoint(&db, account).unwrap().is_none());
        assert!(load_auto_scan_items(&db, account).unwrap().is_empty());
        cleanup(&path, db);
    }
}
