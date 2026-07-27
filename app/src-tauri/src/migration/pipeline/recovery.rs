use crate::migration::db::MigrationDb;
use crate::migration::pipeline::stages::PipelineStage;
use sqlite::State;

/// Khôi phục trạng thái an toàn cho các item bị gián đoạn (crash/shutdown)
pub fn run_crash_recovery(db: &MigrationDb, job_id: i64) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;

    // 1. Quét tìm tất cả các item trung gian của job_id
    let mut stmt = conn
        .prepare("SELECT id, pipeline_stage FROM migration_items WHERE job_id = ?;")
        .map_err(|e| e.to_string())?;
    stmt.bind((1, job_id)).map_err(|e| e.to_string())?;

    let mut items = vec![];
    while let Ok(State::Row) = stmt.next() {
        let id: i64 = stmt.read(0).unwrap_or(0);
        let stage_str: String = stmt.read(1).unwrap_or_else(|_| "discovered".into());
        items.push((id, PipelineStage::from_str(&stage_str)));
    }

    for (id, stage) in items {
        let target_stage = match stage {
            PipelineStage::Downloading | PipelineStage::QueuedDownload => {
                // Trả về Discovered để planner claim lại
                Some(PipelineStage::Discovered)
            }
            PipelineStage::Processing | PipelineStage::QueuedProcessing => {
                // Trả về Downloaded để processor xử lý lại
                Some(PipelineStage::Downloaded)
            }
            PipelineStage::Uploading
            | PipelineStage::QueuedUpload
            | PipelineStage::WaitingForQuota => {
                // Đổi thành ReconciliationRequired để kiểm tra trùng lặp trước khi upload lại
                Some(PipelineStage::ReconciliationRequired)
            }
            PipelineStage::SavingLocal => {
                // Trả về Downloaded để local finalizer lưu lại
                Some(PipelineStage::Downloaded)
            }
            _ => None, // Các trạng thái terminal hoặc chờ không đổi
        };

        if let Some(target) = target_stage {
            let mut upd = conn
                .prepare("UPDATE migration_items SET pipeline_stage = ? WHERE id = ?;")
                .map_err(|e| e.to_string())?;
            upd.bind((1, target.as_str())).map_err(|e| e.to_string())?;
            upd.bind((2, id)).map_err(|e| e.to_string())?;
            upd.next().map_err(|e| e.to_string())?;
        }
    }

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
            let path = env::temp_dir().join(format!("temp_recovery_dir_{}", rand::random::<u64>()));
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
    fn test_crash_recovery_pipeline() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_recovery.db");
        let db = open_migration_db_at_path(db_path).unwrap();
        let conn = db.lock().unwrap();

        // Seed
        conn.execute("INSERT INTO migration_jobs (id, source_folder_id, source_folder_path, telegram_destination_id, telegram_destination_name, local_backup_dir, workspace_dir, state, started_at, created_at, updated_at) VALUES (1, 'src', 'path', 'tg', 'tg', 'loc', 'ws', 'running', 0, 0, 0);").unwrap();
        // Insert items recovering into various stages
        conn.execute("INSERT INTO migration_items (id, job_id, folder_id, name, path, source_item_id, size, item_category, pipeline_stage, created_at, updated_at) VALUES (1, 1, 'f', 'file1.mp4', 'file1.mp4', 's1', 100, 'video', 'downloading', 0, 0);").unwrap();
        conn.execute("INSERT INTO migration_items (id, job_id, folder_id, name, path, source_item_id, size, item_category, pipeline_stage, created_at, updated_at) VALUES (2, 1, 'f', 'file2.mp4', 'file2.mp4', 's2', 100, 'video', 'processing', 0, 0);").unwrap();
        conn.execute("INSERT INTO migration_items (id, job_id, folder_id, name, path, source_item_id, size, item_category, pipeline_stage, created_at, updated_at) VALUES (3, 1, 'f', 'file3.mp4', 'file3.mp4', 's3', 100, 'video', 'uploading', 0, 0);").unwrap();

        drop(conn);

        run_crash_recovery(&db, 1).unwrap();

        let conn = db.lock().unwrap();
        let mut check1 = conn
            .prepare("SELECT pipeline_stage FROM migration_items WHERE id = 1;")
            .unwrap();
        assert_eq!(check1.next().unwrap(), sqlite::State::Row);
        assert_eq!(check1.read::<String, _>(0).unwrap(), "discovered");

        let mut check2 = conn
            .prepare("SELECT pipeline_stage FROM migration_items WHERE id = 2;")
            .unwrap();
        assert_eq!(check2.next().unwrap(), sqlite::State::Row);
        assert_eq!(check2.read::<String, _>(0).unwrap(), "downloaded");

        let mut check3 = conn
            .prepare("SELECT pipeline_stage FROM migration_items WHERE id = 3;")
            .unwrap();
        assert_eq!(check3.next().unwrap(), sqlite::State::Row);
        assert_eq!(
            check3.read::<String, _>(0).unwrap(),
            "reconciliation_required"
        );
    }
}
