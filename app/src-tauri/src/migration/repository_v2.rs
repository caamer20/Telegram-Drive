use crate::migration::db::MigrationDb;
use sqlite::{Connection, State};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DedupeResultV2 {
    CanonicalClaimed,
    AlreadyMigrated {
        telegram_message_id: Option<i64>,
        local_absolute_path: Option<String>,
    },
    SkippedAsDuplicate {
        canonical_item_id: i64,
    },
    WaitingForCanonical {
        canonical_item_id: i64,
    },
}

#[derive(Debug, Clone)]
pub struct MigrationItemV2 {
    pub id: i64,
    pub job_id: i64,
    pub item_type: String, // "file" | "folder"
    pub name: String,
    pub source_path: String,
    pub source_item_id: Option<String>,
    pub size_bytes: i64,
    pub source_etag: Option<String>,
    pub source_last_modified: Option<String>,
    pub source_fingerprint_type: Option<String>,
    pub source_fingerprint_value: Option<String>,
    pub state: String,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    pub attempt_count: i64,
    pub computed_sha256: Option<String>,
    pub telegram_message_id: Option<i64>,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub queue_position: i64,
    pub action_type: Option<String>,

    // v2 fields
    pub route_kind: String,
    pub duplicate_of_item_id: Option<i64>,
    pub artifact_size_bytes: Option<i64>,
    pub local_dest_path: Option<String>,
    pub telegram_random_id: Option<String>,
    pub upload_attempt_id: Option<String>,
    pub original_sha256: Option<String>,
    pub processed_sha256: Option<String>,
    pub video_decision: Option<String>,
}

fn parse_row_v2(stmt: &sqlite::Statement) -> MigrationItemV2 {
    MigrationItemV2 {
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
        route_kind: stmt.read(21).unwrap_or_else(|_| "other_to_local".into()),
        duplicate_of_item_id: stmt.read(22).unwrap_or(None),
        artifact_size_bytes: stmt.read(23).unwrap_or(None),
        local_dest_path: stmt.read(24).unwrap_or(None),
        telegram_random_id: stmt.read(25).unwrap_or(None),
        upload_attempt_id: stmt.read(26).unwrap_or(None),
        original_sha256: stmt.read(27).unwrap_or(None),
        processed_sha256: stmt.read(28).unwrap_or(None),
        video_decision: stmt.read(29).unwrap_or(None),
    }
}

const SELECT_FIELDS: &str = "id, job_id, item_type, name, source_path, source_item_id, size_bytes, source_etag, source_last_modified, source_fingerprint_type, source_fingerprint_value, state, last_error_code, last_error_message, attempt_count, computed_sha256, telegram_message_id, created_at, completed_at, queue_position, action_type, route_kind, duplicate_of_item_id, artifact_size_bytes, local_dest_path, telegram_random_id, upload_attempt_id, original_sha256, processed_sha256, video_decision";

fn execute_with_retry<F, T>(db: &MigrationDb, mut op: F) -> Result<T, String>
where
    F: FnMut(&Connection) -> Result<T, String>,
{
    let mut last_err = String::new();
    for attempt in 0..5 {
        let conn = db.lock().map_err(|e| e.to_string())?;
        match op(&conn) {
            Ok(val) => return Ok(val),
            Err(e) => {
                last_err = e.clone();
                if e.contains("database is locked") || e.contains("busy") {
                    drop(conn); // Release lock before sleep
                    let wait_ms = 50 * 2u64.pow(attempt);
                    std::thread::sleep(Duration::from_millis(wait_ms));
                } else {
                    return Err(e);
                }
            }
        }
    }
    Err(format!(
        "SQLite operation failed after 5 retries: {}",
        last_err
    ))
}

/// Atomic Claim: Lấy tệp pending tiếp theo và đổi trạng thái thành downloading trong transaction an toàn
pub fn claim_next_item(db: &MigrationDb, job_id: i64) -> Result<Option<MigrationItemV2>, String> {
    execute_with_retry(db, |conn| {
        conn.execute("BEGIN IMMEDIATE TRANSACTION;")
            .map_err(|e| e.to_string())?;

        let res = (|| -> Result<Option<MigrationItemV2>, String> {
            let mut stmt = conn
                .prepare(format!(
                    "SELECT {} FROM migration_items
                     WHERE job_id = ? AND state = 'pending' AND item_type = 'file' AND duplicate_of_item_id IS NULL
                     ORDER BY queue_position ASC, id ASC LIMIT 1;",
                    SELECT_FIELDS
                ))
                .map_err(|e| e.to_string())?;
            stmt.bind((1, job_id)).map_err(|e| e.to_string())?;

            if let Ok(State::Row) = stmt.next() {
                let item = parse_row_v2(&stmt);

                // Lease item: Update sang downloading
                let mut upd = conn
                    .prepare("UPDATE migration_items SET state = 'downloading' WHERE id = ?;")
                    .map_err(|e| e.to_string())?;
                upd.bind((1, item.id)).map_err(|e| e.to_string())?;
                upd.next().map_err(|e| e.to_string())?;

                Ok(Some(item))
            } else {
                Ok(None)
            }
        })();

        match res {
            Ok(item) => {
                if let Err(commit_err) = conn.execute("COMMIT;") {
                    let _ = conn.execute("ROLLBACK;");
                    Err(commit_err.to_string())
                } else {
                    Ok(item)
                }
            }
            Err(e) => {
                let _ = conn.execute("ROLLBACK;");
                Err(e)
            }
        }
    })
}

/// Canonical Dedupe Claim: Giao dịch đặt chỗ vân tay độc nhất theo target key
pub fn claim_dedupe_canonical(
    db: &MigrationDb,
    item_id: i64,
    fingerprint_type: &str,
    fingerprint_value: &str,
    source_size: i64,
    artifact_target_key: &str,
) -> Result<DedupeResultV2, String> {
    execute_with_retry(db, |conn| {
        conn.execute("BEGIN IMMEDIATE TRANSACTION;")
            .map_err(|e| e.to_string())?;

        let res = (|| -> Result<DedupeResultV2, String> {
            // Bước 0: Đồng bộ hóa fingerprint hiện tại vào item trước khi check canonical
            let mut upd_fp = conn
                .prepare("UPDATE migration_items SET source_fingerprint_type = ?, source_fingerprint_value = ?, size_bytes = ? WHERE id = ?;")
                .map_err(|e| e.to_string())?;
            upd_fp
                .bind((1, fingerprint_type))
                .map_err(|e| e.to_string())?;
            upd_fp
                .bind((2, fingerprint_value))
                .map_err(|e| e.to_string())?;
            upd_fp.bind((3, source_size)).map_err(|e| e.to_string())?;
            upd_fp.bind((4, item_id)).map_err(|e| e.to_string())?;
            upd_fp.next().map_err(|e| e.to_string())?;

            // Bước a: Check migrated_fingerprints toàn cục
            let mut check_mf = conn
                .prepare(
                    "SELECT telegram_message_id, local_absolute_path FROM migrated_fingerprints
                     WHERE fingerprint_type = ?1 AND fingerprint_value = ?2 AND file_size = ?3 AND artifact_target_key = ?4
                     LIMIT 1;"
                )
                .map_err(|e| e.to_string())?;
            check_mf
                .bind((1, fingerprint_type))
                .map_err(|e| e.to_string())?;
            check_mf
                .bind((2, fingerprint_value))
                .map_err(|e| e.to_string())?;
            check_mf.bind((3, source_size)).map_err(|e| e.to_string())?;
            check_mf
                .bind((4, artifact_target_key))
                .map_err(|e| e.to_string())?;

            if let Ok(State::Row) = check_mf.next() {
                let msg_id: Option<i64> = check_mf.read(0).unwrap_or(None);
                let path: Option<String> = check_mf.read(1).unwrap_or(None);

                // Cập nhật trạng thái item hiện tại sang skipped_duplicate
                let now = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                let mut upd = conn
                    .prepare("UPDATE migration_items SET state = 'skipped_duplicate', completed_at = ?, telegram_message_id = ?, local_dest_path = ? WHERE id = ?;")
                    .map_err(|e| e.to_string())?;
                upd.bind((1, now)).map_err(|e| e.to_string())?;
                upd.bind((2, msg_id)).map_err(|e| e.to_string())?;
                upd.bind((3, path.as_deref())).map_err(|e| e.to_string())?;
                upd.bind((4, item_id)).map_err(|e| e.to_string())?;
                upd.next().map_err(|e| e.to_string())?;

                return Ok(DedupeResultV2::AlreadyMigrated {
                    telegram_message_id: msg_id,
                    local_absolute_path: path,
                });
            }

            // Lấy thông tin job_id của item hiện tại để check canonical cùng job
            let mut check_job = conn
                .prepare("SELECT job_id FROM migration_items WHERE id = ? LIMIT 1;")
                .map_err(|e| e.to_string())?;
            check_job.bind((1, item_id)).map_err(|e| e.to_string())?;
            let job_id: i64 = if let Ok(State::Row) = check_job.next() {
                check_job.read(0).unwrap_or(0)
            } else {
                return Err("Item not found".into());
            };

            // Bước b: Tìm canonical item trùng fingerprint và target key trong cùng job
            let mut check_canonical = conn
                .prepare(
                    "SELECT id, state FROM migration_items
                     WHERE job_id = ?1 AND source_fingerprint_type = ?2 AND source_fingerprint_value = ?3
                       AND size_bytes = ?4 AND duplicate_of_item_id IS NULL AND id <> ?5
                     LIMIT 1;"
                )
                .map_err(|e| e.to_string())?;
            check_canonical
                .bind((1, job_id))
                .map_err(|e| e.to_string())?;
            check_canonical
                .bind((2, fingerprint_type))
                .map_err(|e| e.to_string())?;
            check_canonical
                .bind((3, fingerprint_value))
                .map_err(|e| e.to_string())?;
            check_canonical
                .bind((4, source_size))
                .map_err(|e| e.to_string())?;
            check_canonical
                .bind((5, item_id))
                .map_err(|e| e.to_string())?;

            if let Ok(State::Row) = check_canonical.next() {
                let canonical_id: i64 = check_canonical.read(0).unwrap_or(0);
                let canonical_state: String =
                    check_canonical.read(1).unwrap_or_else(|_| "pending".into());

                if canonical_state == "completed" {
                    // Canonical đã hoàn thành, item này tự động hoàn thành
                    let now = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64;

                    // Lấy kết quả từ canonical
                    let mut get_res = conn
                        .prepare("SELECT telegram_message_id, local_dest_path FROM migration_items WHERE id = ? LIMIT 1;")
                        .map_err(|e| e.to_string())?;
                    get_res.bind((1, canonical_id)).map_err(|e| e.to_string())?;
                    let (msg_id, local_path): (Option<i64>, Option<String>) =
                        if let Ok(State::Row) = get_res.next() {
                            (
                                get_res.read(0).unwrap_or(None),
                                get_res.read(1).unwrap_or(None),
                            )
                        } else {
                            (None, None)
                        };

                    let mut upd = conn
                        .prepare("UPDATE migration_items SET state = 'skipped_duplicate', duplicate_of_item_id = ?, completed_at = ?, telegram_message_id = ?, local_dest_path = ? WHERE id = ?;")
                        .map_err(|e| e.to_string())?;
                    upd.bind((1, canonical_id)).map_err(|e| e.to_string())?;
                    upd.bind((2, now)).map_err(|e| e.to_string())?;
                    upd.bind((3, msg_id)).map_err(|e| e.to_string())?;
                    upd.bind((4, local_path.as_deref()))
                        .map_err(|e| e.to_string())?;
                    upd.bind((5, item_id)).map_err(|e| e.to_string())?;
                    upd.next().map_err(|e| e.to_string())?;

                    Ok(DedupeResultV2::SkippedAsDuplicate {
                        canonical_item_id: canonical_id,
                    })
                } else if canonical_state == "failed" {
                    // Canonical cũ đã chết, tự promote mình làm canonical mới
                    let mut upd = conn
                        .prepare("UPDATE migration_items SET duplicate_of_item_id = NULL, state = 'pending' WHERE id = ?;")
                        .map_err(|e| e.to_string())?;
                    upd.bind((1, item_id)).map_err(|e| e.to_string())?;
                    upd.next().map_err(|e| e.to_string())?;

                    Ok(DedupeResultV2::CanonicalClaimed)
                } else {
                    // Canonical vẫn đang chạy, giữ state pending và đặt duplicate_of_item_id
                    let mut upd = conn
                        .prepare("UPDATE migration_items SET state = 'pending', duplicate_of_item_id = ? WHERE id = ?;")
                        .map_err(|e| e.to_string())?;
                    upd.bind((1, canonical_id)).map_err(|e| e.to_string())?;
                    upd.bind((2, item_id)).map_err(|e| e.to_string())?;
                    upd.next().map_err(|e| e.to_string())?;

                    Ok(DedupeResultV2::WaitingForCanonical {
                        canonical_item_id: canonical_id,
                    })
                }
            } else {
                // Chưa có canonical nào, tự mình làm canonical
                let mut upd = conn
                    .prepare("UPDATE migration_items SET duplicate_of_item_id = NULL WHERE id = ?;")
                    .map_err(|e| e.to_string())?;
                upd.bind((1, item_id)).map_err(|e| e.to_string())?;
                upd.next().map_err(|e| e.to_string())?;

                Ok(DedupeResultV2::CanonicalClaimed)
            }
        })();

        match res {
            Ok(val) => {
                if let Err(commit_err) = conn.execute("COMMIT;") {
                    let _ = conn.execute("ROLLBACK;");
                    Err(commit_err.to_string())
                } else {
                    Ok(val)
                }
            }
            Err(e) => {
                let _ = conn.execute("ROLLBACK;");
                Err(e)
            }
        }
    })
}

/// Khi canonical chính thất bại, promote một duplicate pending đang chờ lên làm canonical mới
pub fn promote_next_duplicate(
    db: &MigrationDb,
    failed_canonical_id: i64,
) -> Result<Option<i64>, String> {
    execute_with_retry(db, |conn| {
        conn.execute("BEGIN IMMEDIATE TRANSACTION;")
            .map_err(|e| e.to_string())?;

        let res = (|| -> Result<Option<i64>, String> {
            // Tìm duplicate đầu tiên đang chờ
            let mut find_dup = conn
                .prepare("SELECT id FROM migration_items WHERE duplicate_of_item_id = ? AND state = 'pending' ORDER BY id ASC LIMIT 1;")
                .map_err(|e| e.to_string())?;
            find_dup
                .bind((1, failed_canonical_id))
                .map_err(|e| e.to_string())?;

            if let Ok(State::Row) = find_dup.next() {
                let new_canonical_id: i64 = find_dup.read(0).unwrap_or(0);

                // Promote duplicate này
                let mut upd = conn
                    .prepare("UPDATE migration_items SET duplicate_of_item_id = NULL, state = 'pending' WHERE id = ?;")
                    .map_err(|e| e.to_string())?;
                upd.bind((1, new_canonical_id)).map_err(|e| e.to_string())?;
                upd.next().map_err(|e| e.to_string())?;

                // Trỏ các duplicates còn lại sang new canonical
                let mut upd_others = conn
                    .prepare("UPDATE migration_items SET duplicate_of_item_id = ? WHERE duplicate_of_item_id = ? AND id <> ?;")
                    .map_err(|e| e.to_string())?;
                upd_others
                    .bind((1, new_canonical_id))
                    .map_err(|e| e.to_string())?;
                upd_others
                    .bind((2, failed_canonical_id))
                    .map_err(|e| e.to_string())?;
                upd_others
                    .bind((3, new_canonical_id))
                    .map_err(|e| e.to_string())?;
                upd_others.next().map_err(|e| e.to_string())?;

                Ok(Some(new_canonical_id))
            } else {
                Ok(None)
            }
        })();

        match res {
            Ok(val) => {
                if let Err(commit_err) = conn.execute("COMMIT;") {
                    let _ = conn.execute("ROLLBACK;");
                    Err(commit_err.to_string())
                } else {
                    Ok(val)
                }
            }
            Err(e) => {
                let _ = conn.execute("ROLLBACK;");
                Err(e)
            }
        }
    })
}
