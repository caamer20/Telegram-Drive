use sqlite::Connection;
use std::time::SystemTime;

pub fn migrate_to_v2(conn: &Connection) -> Result<(), String> {
    // Chạy trong transaction an toàn
    conn.execute("BEGIN IMMEDIATE TRANSACTION;")
        .map_err(|e| e.to_string())?;

    let res = (|| -> Result<(), String> {
        // 1. Thêm các cột cho migration_jobs
        ensure_column(
            conn,
            "migration_jobs",
            "pipeline_version",
            "INTEGER NOT NULL DEFAULT 1",
        )?;
        ensure_column(conn, "migration_jobs", "local_backup_dir", "TEXT")?;
        ensure_column(conn, "migration_jobs", "workspace_dir", "TEXT")?;
        ensure_column(
            conn,
            "migration_jobs",
            "manifest_state",
            "TEXT DEFAULT 'pending'",
        )?;
        ensure_column(conn, "migration_jobs", "manifest_json_path", "TEXT")?;
        ensure_column(conn, "migration_jobs", "manifest_csv_path", "TEXT")?;
        ensure_column(conn, "migration_jobs", "manifest_last_error", "TEXT")?;
        ensure_column(conn, "migration_jobs", "manifest_exported_at", "INTEGER")?;

        // 2. Thêm các cột cho migration_items
        ensure_column(
            conn,
            "migration_items",
            "route_kind",
            "TEXT NOT NULL DEFAULT 'other_to_local'",
        )?;
        ensure_column(conn, "migration_items", "duplicate_of_item_id", "INTEGER")?;
        ensure_column(conn, "migration_items", "artifact_size_bytes", "INTEGER")?;
        ensure_column(conn, "migration_items", "local_dest_path", "TEXT")?;
        ensure_column(conn, "migration_items", "telegram_random_id", "TEXT")?;
        ensure_column(conn, "migration_items", "upload_attempt_id", "TEXT")?;
        ensure_column(conn, "migration_items", "original_sha256", "TEXT")?;
        ensure_column(conn, "migration_items", "processed_sha256", "TEXT")?;
        ensure_column(conn, "migration_items", "video_decision", "TEXT")?;
        ensure_column(
            conn,
            "migration_items",
            "pipeline_stage",
            "TEXT NOT NULL DEFAULT 'discovered'",
        )?;

        // 3. Thêm các cột cho migrated_fingerprints
        ensure_column(conn, "migrated_fingerprints", "artifact_target_key", "TEXT")?;
        ensure_column(conn, "migrated_fingerprints", "local_absolute_path", "TEXT")?;

        // Tạo Index unique v2 cho migrated_fingerprints
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_migrated_fingerprints_v2
             ON migrated_fingerprints(fingerprint_type, fingerprint_value, file_size, artifact_target_key);"
        )
        .map_err(|e| format!("Failed to create idx_migrated_fingerprints_v2: {}", e))?;

        // 4. Tạo các bảng mới
        conn.execute(
            "CREATE TABLE IF NOT EXISTS migration_pacing_state (
                key                     TEXT PRIMARY KEY,
                last_success_timestamp  INTEGER,
                sent_count_since_cooldown INTEGER,
                next_allowed_at         INTEGER,
                batch_cooldown_until    INTEGER,
                flood_wait_until        INTEGER,
                updated_at              INTEGER NOT NULL
            );",
        )
        .map_err(|e| format!("Failed to create migration_pacing_state table: {}", e))?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS quota_reservations (
                item_id                 INTEGER PRIMARY KEY,
                job_id                  INTEGER NOT NULL,
                date_string             TEXT NOT NULL,
                reserved_bytes          INTEGER NOT NULL,
                status                  TEXT NOT NULL DEFAULT 'reserved' CHECK(status IN ('reserved', 'committed', 'released')),
                created_at              INTEGER NOT NULL,
                expires_at              INTEGER NOT NULL
            );"
        )
        .map_err(|e| format!("Failed to create quota_reservations table: {}", e))?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS disk_reservations (
                reservation_id          TEXT PRIMARY KEY,
                job_id                  INTEGER NOT NULL,
                item_id                 INTEGER NOT NULL,
                owner_lease             TEXT NOT NULL,
                reserved_bytes          INTEGER NOT NULL,
                purpose                 TEXT NOT NULL CHECK(purpose IN ('download', 'transcode_input', 'transcode_output')),
                created_at              INTEGER NOT NULL,
                expires_at              INTEGER NOT NULL,
                released_at             INTEGER
            );"
        )
        .map_err(|e| format!("Failed to create disk_reservations table: {}", e))?;

        // 5. Logic chuyển trạng thái Job v1 đang running -> paused để tương thích ngược an toàn
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        conn.execute(format!(
            "UPDATE migration_jobs
             SET state = 'paused', pause_reason = 'recovery_interrupted', updated_at = {}
             WHERE pipeline_version = 1 AND state = 'running';",
            now
        ))
        .map_err(|e| format!("Failed to pause running v1 jobs: {}", e))?;

        Ok(())
    })();

    match res {
        Ok(_) => {
            conn.execute("COMMIT;").map_err(|e| e.to_string())?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute("ROLLBACK;");
            Err(e)
        }
    }
}

fn ensure_column(
    conn: &Connection,
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
