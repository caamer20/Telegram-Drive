use sqlite::{Connection, State};
use std::time::SystemTime;

/// Thực hiện giữ chỗ dung lượng đĩa làm việc cho một tác vụ cụ thể
pub fn reserve_disk_space(
    conn: &Connection,
    reservation_id: &str,
    job_id: i64,
    item_id: i64,
    owner_lease: &str,
    bytes: i64,
    purpose: &str,
    expires_in_secs: i64,
) -> Result<(), String> {
    if bytes < 0 {
        return Err("Reserved bytes cannot be negative".into());
    }

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let expires_at = now + expires_in_secs;

    let mut stmt = conn
        .prepare(
            "INSERT OR REPLACE INTO disk_reservations (reservation_id, job_id, item_id, owner_lease, reserved_bytes, purpose, created_at, expires_at, released_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL);"
        )
        .map_err(|e| e.to_string())?;
    stmt.bind((1, reservation_id)).map_err(|e| e.to_string())?;
    stmt.bind((2, job_id)).map_err(|e| e.to_string())?;
    stmt.bind((3, item_id)).map_err(|e| e.to_string())?;
    stmt.bind((4, owner_lease)).map_err(|e| e.to_string())?;
    stmt.bind((5, bytes)).map_err(|e| e.to_string())?;
    stmt.bind((6, purpose)).map_err(|e| e.to_string())?;
    stmt.bind((7, now)).map_err(|e| e.to_string())?;
    stmt.bind((8, expires_at)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;

    Ok(())
}

/// Giải phóng dung lượng đĩa đã giữ chỗ sau khi tác vụ hoàn tất
pub fn release_disk_space(conn: &Connection, reservation_id: &str) -> Result<(), String> {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let mut stmt = conn
        .prepare("UPDATE disk_reservations SET released_at = ?, reserved_bytes = 0 WHERE reservation_id = ?;")
        .map_err(|e| e.to_string())?;
    stmt.bind((1, now)).map_err(|e| e.to_string())?;
    stmt.bind((2, reservation_id)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;

    Ok(())
}

/// Thu hồi các reservation đã hết hạn hoặc bị mồ côi (orphaned) khi startup
pub fn run_disk_space_recovery(conn: &Connection, now_secs: i64) -> Result<(), String> {
    // Thu hồi bằng cách set reserved_bytes = 0 và set released_at
    let mut stmt = conn
        .prepare("UPDATE disk_reservations SET released_at = ?, reserved_bytes = 0 WHERE released_at IS NULL AND expires_at < ?;")
        .map_err(|e| e.to_string())?;
    stmt.bind((1, now_secs)).map_err(|e| e.to_string())?;
    stmt.bind((2, now_secs)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;

    Ok(())
}

/// Tính tổng dung lượng đang giữ chỗ hiện tại của job
pub fn get_total_reserved_disk_space(conn: &Connection, job_id: i64) -> Result<i64, String> {
    let mut stmt = conn
        .prepare("SELECT SUM(reserved_bytes) FROM disk_reservations WHERE job_id = ? AND released_at IS NULL;")
        .map_err(|e| e.to_string())?;
    stmt.bind((1, job_id)).map_err(|e| e.to_string())?;

    let total: i64 = if let Ok(State::Row) = stmt.next() {
        stmt.read(0).unwrap_or(0)
    } else {
        0
    };

    Ok(total)
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
            let path = env::temp_dir().join(format!("temp_disk_res_dir_{}", rand::random::<u64>()));
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
    fn test_disk_reservation_store() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_disk_reserve.db");
        let db = open_migration_db_at_path(db_path).unwrap();
        let conn = db.lock().unwrap();

        // 1. Reserve thành công
        reserve_disk_space(
            &conn,
            "res_1",
            1,
            42,
            "downloader",
            10_000_000,
            "download",
            3600,
        )
        .unwrap();
        assert_eq!(get_total_reserved_disk_space(&conn, 1).unwrap(), 10_000_000);

        // 2. Không cho phép reserve bytes âm
        let res_neg =
            reserve_disk_space(&conn, "res_2", 1, 43, "downloader", -500, "download", 3600);
        assert!(res_neg.is_err());

        // 3. Release space
        release_disk_space(&conn, "res_1").unwrap();
        assert_eq!(get_total_reserved_disk_space(&conn, 1).unwrap(), 0);

        // 4. Release hai lần là idempotent
        release_disk_space(&conn, "res_1").unwrap();
        assert_eq!(get_total_reserved_disk_space(&conn, 1).unwrap(), 0);

        // 5. Test recovery
        reserve_disk_space(
            &conn,
            "res_expired",
            1,
            44,
            "downloader",
            5_000_000,
            "download",
            -10,
        )
        .unwrap();
        assert_eq!(get_total_reserved_disk_space(&conn, 1).unwrap(), 5_000_000);

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        run_disk_space_recovery(&conn, now + 1).unwrap();
        assert_eq!(get_total_reserved_disk_space(&conn, 1).unwrap(), 0);

        // 6. Reservation của job khác không bị release nhầm
        reserve_disk_space(
            &conn,
            "res_job_2",
            2,
            45,
            "downloader",
            2_000_000,
            "download",
            3600,
        )
        .unwrap();
        assert_eq!(get_total_reserved_disk_space(&conn, 1).unwrap(), 0);
        assert_eq!(get_total_reserved_disk_space(&conn, 2).unwrap(), 2_000_000);
    }
}
