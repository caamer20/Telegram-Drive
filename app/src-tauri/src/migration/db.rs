use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager};


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

    create_schema(&conn)?;

    Ok(Arc::new(Mutex::new(conn)))
}

pub fn reset_database(conn: &sqlite::Connection) -> Result<(), String> {
    log::warn!("Resetting migration database to canonical schema");
    let mut tables = Vec::new();
    let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table';").unwrap();
    while let Ok(sqlite::State::Row) = stmt.next() {
        let table_name = stmt.read::<String, _>(0).unwrap();
        if table_name != "sqlite_sequence" {
            tables.push(table_name);
        }
    }
    
    conn.execute("BEGIN EXCLUSIVE TRANSACTION;").map_err(|e| e.to_string())?;
    for table in tables {
        if let Err(e) = conn.execute(format!("DROP TABLE IF EXISTS {};", table)) {
            let _ = conn.execute("ROLLBACK;");
            return Err(e.to_string());
        }
    }
    conn.execute("COMMIT;").map_err(|e| e.to_string())?;
    
    conn.execute("VACUUM;").map_err(|e| e.to_string())?;
    create_schema(conn)
}

pub fn create_schema(conn: &sqlite::Connection) -> Result<(), String> {
    // 1. Migration Jobs
    conn.execute(
        "CREATE TABLE IF NOT EXISTS migration_jobs (
            id                          INTEGER PRIMARY KEY AUTOINCREMENT,
            source_folder_id            TEXT NOT NULL,
            source_folder_path          TEXT NOT NULL,
            telegram_destination_id     TEXT NOT NULL,
            telegram_destination_name   TEXT NOT NULL,
            local_backup_dir            TEXT NOT NULL,
            workspace_dir               TEXT NOT NULL,
            state                       TEXT NOT NULL DEFAULT 'running',
            started_at                  INTEGER NOT NULL,
            completed_at                INTEGER,
            last_error                  TEXT,
            flood_wait_until            INTEGER,
            discovered_folders          INTEGER NOT NULL DEFAULT 0,
            completed_folders           INTEGER NOT NULL DEFAULT 0,
            discovered_items            INTEGER NOT NULL DEFAULT 0,
            completed_items             INTEGER NOT NULL DEFAULT 0,
            failed_items                INTEGER NOT NULL DEFAULT 0,
            waiting_items               INTEGER NOT NULL DEFAULT 0,
            created_at                  INTEGER NOT NULL,
            updated_at                  INTEGER NOT NULL
        );"
    ).map_err(|e| format!("Failed to create migration_jobs: {}", e))?;

    // 2. Folder Queue
    conn.execute(
        "CREATE TABLE IF NOT EXISTS folder_queue (
            id                          INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id                      INTEGER NOT NULL,
            folder_id                   TEXT NOT NULL,
            parent_id                   TEXT,
            folder_path                 TEXT NOT NULL,
            state                       TEXT NOT NULL DEFAULT 'pending',
            next_page_link              TEXT,
            has_more                    INTEGER NOT NULL DEFAULT 1,
            discovered_files_count      INTEGER NOT NULL DEFAULT 0,
            discovered_folders_count    INTEGER NOT NULL DEFAULT 0,
            completed_files_count       INTEGER NOT NULL DEFAULT 0,
            last_error                  TEXT,
            created_at                  INTEGER NOT NULL,
            updated_at                  INTEGER NOT NULL,
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
            path                        TEXT NOT NULL,
            size                        INTEGER NOT NULL,
            item_category               TEXT NOT NULL,
            pipeline_stage              TEXT NOT NULL DEFAULT 'discovered',
            original_artifact_path      TEXT,
            processed_artifact_path     TEXT,
            original_sha256             TEXT,
            processed_sha256            TEXT,
            video_decision              TEXT,
            artifact_size               INTEGER,
            telegram_attempt_id         TEXT,
            telegram_random_id          INTEGER,
            telegram_message_id         INTEGER,
            retry_count                 INTEGER NOT NULL DEFAULT 0,
            last_error                  TEXT,
            created_at                  INTEGER NOT NULL,
            updated_at                  INTEGER NOT NULL,
            completed_at                INTEGER,
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
            used_bytes          INTEGER NOT NULL DEFAULT 0,
            reset_at            INTEGER NOT NULL DEFAULT 0
        );"
    ).map_err(|e| format!("Failed to create daily_migration_quota: {}", e))?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS quota_reservations (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id          INTEGER NOT NULL,
            item_id         TEXT NOT NULL,
            reserved_bytes  INTEGER NOT NULL,
            reserved_at     INTEGER NOT NULL,
            expires_at      INTEGER NOT NULL,
            status          TEXT NOT NULL DEFAULT 'active'
        );"
    ).map_err(|e| format!("Failed to create quota_reservations: {}", e))?;

    Ok(())
}

pub fn create_job(
    conn: &sqlite::Connection,
    source_folder_id: &str,
    source_folder_path: &str,
    telegram_destination_id: &str,
    telegram_destination_name: &str,
    local_backup_dir: &str,
    workspace_dir: &str,
) -> Result<i64, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
        
    let mut stmt = conn.prepare(
        "INSERT INTO migration_jobs (
            source_folder_id, source_folder_path, telegram_destination_id, 
            telegram_destination_name, local_backup_dir, workspace_dir, state,
            started_at, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, 'running', ?, ?, ?)"
    ).map_err(|e| e.to_string())?;

    stmt.bind((1, source_folder_id)).map_err(|e| e.to_string())?;
    stmt.bind((2, source_folder_path)).map_err(|e| e.to_string())?;
    stmt.bind((3, telegram_destination_id)).map_err(|e| e.to_string())?;
    stmt.bind((4, telegram_destination_name)).map_err(|e| e.to_string())?;
    stmt.bind((5, local_backup_dir)).map_err(|e| e.to_string())?;
    stmt.bind((6, workspace_dir)).map_err(|e| e.to_string())?;
    stmt.bind((7, now)).map_err(|e| e.to_string())?;
    stmt.bind((8, now)).map_err(|e| e.to_string())?;
    stmt.bind((9, now)).map_err(|e| e.to_string())?;
    
    stmt.next().map_err(|e| e.to_string())?;
    
    let mut last_id_stmt = conn.prepare("SELECT last_insert_rowid();").unwrap();
    if let Ok(sqlite::State::Row) = last_id_stmt.next() {
        Ok(last_id_stmt.read::<i64, _>(0).unwrap())
    } else {
        Err("Failed to get last insert rowid".into())
    }
}

