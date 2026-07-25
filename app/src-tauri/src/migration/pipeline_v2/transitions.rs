use crate::migration::db::MigrationDb;
use crate::migration::pipeline_v2::stages::PipelineStage;
use sqlite::State;
use std::time::SystemTime;

/// Xác thực các bước chuyển stage hợp lệ
pub fn validate_stage_transition(from: PipelineStage, to: PipelineStage) -> bool {
    // Cho phép nhảy về Failed từ bất kỳ trạng thái nào
    if to == PipelineStage::Failed || to == PipelineStage::ReconciliationRequired {
        return true;
    }

    match from {
        PipelineStage::Discovered => {
            to == PipelineStage::DedupeCheck || to == PipelineStage::QueuedDownload
        }
        PipelineStage::DedupeCheck => {
            to == PipelineStage::WaitingForCanonical
                || to == PipelineStage::SkippedDuplicate
                || to == PipelineStage::QueuedDownload
        }
        PipelineStage::WaitingForCanonical => {
            to == PipelineStage::SkippedDuplicate || to == PipelineStage::QueuedDownload
        }
        PipelineStage::QueuedDownload => to == PipelineStage::Downloading,
        PipelineStage::Downloading => {
            to == PipelineStage::Downloaded || to == PipelineStage::RetryWait
        }
        PipelineStage::Downloaded => {
            to == PipelineStage::QueuedProcessing
                || to == PipelineStage::QueuedUpload
                || to == PipelineStage::SavingLocal
        }
        PipelineStage::QueuedProcessing => to == PipelineStage::Processing,
        PipelineStage::Processing => {
            to == PipelineStage::QueuedUpload || to == PipelineStage::RetryWait
        }
        PipelineStage::QueuedUpload => to == PipelineStage::Uploading,
        PipelineStage::Uploading => {
            to == PipelineStage::CompletedTelegram
                || to == PipelineStage::RetryWait
                || to == PipelineStage::ReconciliationRequired
        }
        PipelineStage::SavingLocal => {
            to == PipelineStage::CompletedLocal || to == PipelineStage::RetryWait
        }
        PipelineStage::RetryWait => {
            to == PipelineStage::QueuedDownload
                || to == PipelineStage::QueuedProcessing
                || to == PipelineStage::QueuedUpload
        }
        PipelineStage::CompletedTelegram
        | PipelineStage::CompletedLocal
        | PipelineStage::SkippedDuplicate
        | PipelineStage::ReconciliationRequired
        | PipelineStage::Failed => {
            false // Các trạng thái terminal không được chuyển đổi tiếp
        }
    }
}

/// Cập nhật stage của item trong database v2
pub fn update_item_pipeline_stage(
    db: &MigrationDb,
    item_id: i64,
    new_stage: PipelineStage,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;

    // 1. Kiểm tra stage hiện tại
    let mut check_stmt = conn
        .prepare("SELECT pipeline_stage, job_id FROM migration_items WHERE id = ? LIMIT 1;")
        .map_err(|e| e.to_string())?;
    check_stmt.bind((1, item_id)).map_err(|e| e.to_string())?;

    let (current_stage_str, job_id): (String, i64) = if let Ok(State::Row) = check_stmt.next() {
        (
            check_stmt.read(0).unwrap_or_else(|_| "discovered".into()),
            check_stmt.read(1).unwrap_or(0),
        )
    } else {
        return Err("Item not found".into());
    };

    // 2. Bảo vệ Job v1 không bị ghi đè stage v2
    let mut check_job = conn
        .prepare("SELECT pipeline_version FROM migration_jobs WHERE id = ? LIMIT 1;")
        .map_err(|e| e.to_string())?;
    check_job.bind((1, job_id)).map_err(|e| e.to_string())?;
    let pipeline_version: i64 = if let Ok(State::Row) = check_job.next() {
        check_job.read(0).unwrap_or(1)
    } else {
        1
    };

    if pipeline_version == 1 {
        return Err("Cannot update pipeline stage for v1 jobs".into());
    }

    let current_stage = PipelineStage::from_str(&current_stage_str);

    // 3. Validate chuyển đổi trạng thái
    if !validate_stage_transition(current_stage, new_stage) {
        return Err(format!(
            "Invalid stage transition from {:?} to {:?}",
            current_stage, new_stage
        ));
    }

    // 4. Thực thi update
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let state_mapped = match new_stage {
        PipelineStage::CompletedTelegram | PipelineStage::CompletedLocal => "completed",
        PipelineStage::SkippedDuplicate => "skipped_duplicate",
        PipelineStage::Failed => "failed",
        _ => "downloading", // Giữ nguyên state v1 tương thích trong CHECK constraint
    };

    let mut upd = conn
        .prepare("UPDATE migration_items SET pipeline_stage = ?, state = ?, completed_at = ? WHERE id = ?;")
        .map_err(|e| e.to_string())?;
    upd.bind((1, new_stage.as_str()))
        .map_err(|e| e.to_string())?;
    upd.bind((2, state_mapped)).map_err(|e| e.to_string())?;
    if state_mapped == "completed"
        || state_mapped == "skipped_duplicate"
        || state_mapped == "failed"
    {
        upd.bind((3, now)).map_err(|e| e.to_string())?;
    } else {
        upd.bind((3, None::<i64>)).map_err(|e| e.to_string())?;
    }
    upd.bind((4, item_id)).map_err(|e| e.to_string())?;
    upd.next().map_err(|e| e.to_string())?;

    Ok(())
}

/// Promote a waiting duplicate to canonical if the current canonical permanently fails
pub fn promote_canonical(db: &MigrationDb, canonical_item_id: i64) -> Result<Option<i64>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare("SELECT id FROM migration_items WHERE duplicate_of_item_id = ? AND pipeline_stage = 'waiting_for_canonical' LIMIT 1;").unwrap();
    stmt.bind((1, canonical_item_id)).unwrap();
    
    if let Ok(State::Row) = stmt.next() {
        let promoted_id: i64 = stmt.read(0).unwrap();
        
        // Update promoted to canonical
        let mut upd_promoted = conn.prepare("UPDATE migration_items SET duplicate_of_item_id = NULL, pipeline_stage = 'queued_download', state = 'pending' WHERE id = ?;").unwrap();
        upd_promoted.bind((1, promoted_id)).unwrap();
        upd_promoted.next().unwrap();

        // Update others to point to new canonical
        let mut upd_others = conn.prepare("UPDATE migration_items SET duplicate_of_item_id = ? WHERE duplicate_of_item_id = ?;").unwrap();
        upd_others.bind((1, promoted_id)).unwrap();
        upd_others.bind((2, canonical_item_id)).unwrap();
        upd_others.next().unwrap();
        
        return Ok(Some(promoted_id));
    }
    Ok(None)
}

/// Dedupe based on SHA-256 after download finishes
pub fn post_download_dedupe(db: &MigrationDb, item_id: i64, sha256: &str) -> Result<bool, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare("SELECT id FROM migration_items WHERE original_sha256 = ? AND id != ? AND (pipeline_stage = 'completed_telegram' OR pipeline_stage = 'completed_local') LIMIT 1;").unwrap();
    stmt.bind((1, sha256)).unwrap();
    stmt.bind((2, item_id)).unwrap();
    
    if let Ok(State::Row) = stmt.next() {
        let canonical_id: i64 = stmt.read(0).unwrap();
        
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
            
        let mut upd = conn.prepare("UPDATE migration_items SET duplicate_of_item_id = ?, pipeline_stage = 'skipped_duplicate', state = 'skipped_duplicate', completed_at = ? WHERE id = ?;").unwrap();
        upd.bind((1, canonical_id)).unwrap();
        upd.bind((2, now)).unwrap();
        upd.bind((3, item_id)).unwrap();
        upd.next().unwrap();
        
        return Ok(true);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::db::open_migration_db_at_path;
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let path =
                env::temp_dir().join(format!("temp_transition_dir_{}", rand::random::<u64>()));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn test_stage_transitions() {
        // Valid transitions
        assert!(validate_stage_transition(
            PipelineStage::Discovered,
            PipelineStage::DedupeCheck
        ));
        assert!(validate_stage_transition(
            PipelineStage::Downloading,
            PipelineStage::Downloaded
        ));
        assert!(validate_stage_transition(
            PipelineStage::Downloading,
            PipelineStage::Failed
        ));

        // Invalid transitions
        assert!(!validate_stage_transition(
            PipelineStage::CompletedTelegram,
            PipelineStage::Downloading
        ));
        assert!(!validate_stage_transition(
            PipelineStage::Discovered,
            PipelineStage::Processing
        ));
    }

    #[test]
    fn test_update_stage_in_db() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_transitions.db");
        let db = open_migration_db_at_path(db_path).unwrap();
        let conn = db.lock().unwrap();

        // Chèn Job v2
        conn.execute("INSERT INTO migration_jobs (id, state, pipeline_version, created_at, updated_at) VALUES (1, 'running', 2, 0, 0);").unwrap();
        // Chèn Item v2
        conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, state, pipeline_stage, created_at) VALUES (100, 1, 'file.mp4', 'path.mp4', 'pending', 'discovered', 0);").unwrap();

        drop(conn);

        // Chuyển hợp lệ: discovered -> dedupe_check
        update_item_pipeline_stage(&db, 100, PipelineStage::DedupeCheck).unwrap();

        // Chuyển không hợp lệ: dedupe_check -> processing
        let err = update_item_pipeline_stage(&db, 100, PipelineStage::Processing);
        assert!(err.is_err());
    }
}
