use crate::migration::db::MigrationDb;
use crate::migration::pipeline::stages::PipelineStage;
use sqlite::State;
use std::time::SystemTime;

/// Xác thực các bước chuyển stage hợp lệ
pub fn validate_stage_transition(from: PipelineStage, to: PipelineStage) -> bool {
    // Cho phép nhảy về Failed / ReconciliationRequired từ bất kỳ trạng thái nào
    if to == PipelineStage::Failed || to == PipelineStage::ReconciliationRequired {
        return true;
    }

    match from {
        PipelineStage::Discovered => to == PipelineStage::QueuedDownload,
        PipelineStage::QueuedDownload => to == PipelineStage::Downloading,
        PipelineStage::Downloading => {
            to == PipelineStage::Downloaded || to == PipelineStage::QueuedDownload
        }
        PipelineStage::Downloaded => {
            to == PipelineStage::QueuedProcessing
                || to == PipelineStage::QueuedUpload
                || to == PipelineStage::SavingLocal
        }
        PipelineStage::QueuedProcessing => to == PipelineStage::Processing,
        PipelineStage::Processing => {
            to == PipelineStage::Processed || to == PipelineStage::QueuedProcessing
        }
        PipelineStage::Processed => to == PipelineStage::QueuedUpload,
        PipelineStage::QueuedUpload => {
            to == PipelineStage::Uploading || to == PipelineStage::WaitingForQuota
        }
        PipelineStage::Uploading => {
            to == PipelineStage::CompletedTelegram
                || to == PipelineStage::WaitingForQuota
                || to == PipelineStage::QueuedUpload
        }
        PipelineStage::WaitingForQuota => to == PipelineStage::QueuedUpload,
        PipelineStage::SavingLocal => to == PipelineStage::CompletedLocal,
        // Recovery transitions:
        // Allow retry from failed back to appropriate queue stage
        PipelineStage::Failed => {
            to == PipelineStage::QueuedDownload
                || to == PipelineStage::QueuedProcessing
                || to == PipelineStage::QueuedUpload
                || to == PipelineStage::SavingLocal
        }
        PipelineStage::CompletedTelegram
        | PipelineStage::CompletedLocal
        | PipelineStage::ReconciliationRequired => {
            false // Các trạng thái terminal không được chuyển đổi tiếp
        }
    }
}

/// Cập nhật stage của item trong database
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

    let (current_stage_str, _job_id): (String, i64) = if let Ok(State::Row) = check_stmt.next() {
        (
            check_stmt.read(0).unwrap_or_else(|_| "discovered".into()),
            check_stmt.read(1).unwrap_or(0),
        )
    } else {
        return Err("Item not found".into());
    };

    let current_stage = PipelineStage::from_str(&current_stage_str);

    // 2. Validate chuyển đổi trạng thái
    if !validate_stage_transition(current_stage, new_stage) {
        return Err(format!(
            "Invalid stage transition from {:?} to {:?}",
            current_stage, new_stage
        ));
    }

    // 3. Thực thi update
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let mut upd = conn
        .prepare("UPDATE migration_items SET pipeline_stage = ?, updated_at = ?, completed_at = ? WHERE id = ?;")
        .map_err(|e| e.to_string())?;
    upd.bind((1, new_stage.as_str()))
        .map_err(|e| e.to_string())?;
    upd.bind((2, now)).map_err(|e| e.to_string())?;
    if new_stage.is_terminal() {
        upd.bind((3, now)).map_err(|e| e.to_string())?;
    } else {
        upd.bind((3, None::<i64>)).map_err(|e| e.to_string())?;
    }
    upd.bind((4, item_id)).map_err(|e| e.to_string())?;
    upd.next().map_err(|e| e.to_string())?;

    Ok(())
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
            PipelineStage::QueuedDownload
        ));
        assert!(validate_stage_transition(
            PipelineStage::Downloading,
            PipelineStage::Downloaded
        ));
        assert!(validate_stage_transition(
            PipelineStage::Downloading,
            PipelineStage::Failed
        ));
        assert!(validate_stage_transition(
            PipelineStage::QueuedUpload,
            PipelineStage::WaitingForQuota
        ));
        assert!(validate_stage_transition(
            PipelineStage::Uploading,
            PipelineStage::WaitingForQuota
        ));
        assert!(validate_stage_transition(
            PipelineStage::WaitingForQuota,
            PipelineStage::QueuedUpload
        ));
        assert!(validate_stage_transition(
            PipelineStage::Downloading,
            PipelineStage::QueuedDownload
        ));
        assert!(validate_stage_transition(
            PipelineStage::Processing,
            PipelineStage::QueuedProcessing
        ));
        assert!(validate_stage_transition(
            PipelineStage::Uploading,
            PipelineStage::QueuedUpload
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

        // Chèn Job
        conn.execute("INSERT INTO migration_jobs (id, source_folder_id, source_folder_path, telegram_destination_id, telegram_destination_name, local_backup_dir, workspace_dir, state, started_at, created_at, updated_at) VALUES (1, 'src', 'path', 'tg', 'tg', 'loc', 'ws', 'running', 0, 0, 0);").unwrap();

        conn.execute("INSERT INTO migration_items (id, job_id, folder_id, name, path, source_item_id, size, item_category, pipeline_stage, created_at, updated_at) VALUES (100, 1, 'f', 'file.mp4', 'file.mp4', 's1', 100, 'video', 'discovered', 0, 0);").unwrap();

        drop(conn);

        // Chuyển hợp lệ: discovered -> queued_download
        update_item_pipeline_stage(&db, 100, PipelineStage::QueuedDownload).unwrap();

        // Chuyển không hợp lệ: queued_download -> processing
        let err = update_item_pipeline_stage(&db, 100, PipelineStage::Processing);
        assert!(err.is_err());
    }
}
