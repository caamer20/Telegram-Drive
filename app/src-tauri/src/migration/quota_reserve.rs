use sqlite::{Connection, State};
use std::time::SystemTime;

pub const DAILY_SAFETY_BUDGET_LIMIT: i64 = 250_000_000_000; // 250 GB hard cap

/// Lấy tổng số byte đã dùng (committed + reserved) trong ngày
pub fn get_daily_used_bytes(conn: &Connection, date_string: &str) -> Result<i64, String> {
    // 1. Tính tổng từ daily_migration_quota (đã committed)
    let mut stmt_quota = conn
        .prepare("SELECT uploaded_bytes FROM daily_migration_quota WHERE date_string = ? LIMIT 1;")
        .map_err(|e| e.to_string())?;
    stmt_quota
        .bind((1, date_string))
        .map_err(|e| e.to_string())?;
    let committed_bytes: i64 = if let Ok(State::Row) = stmt_quota.next() {
        stmt_quota.read(0).unwrap_or(0)
    } else {
        0
    };

    // 2. Tính tổng từ quota_reservations đang ở trạng thái 'reserved'
    let mut stmt_reserve = conn
        .prepare("SELECT SUM(reserved_bytes) FROM quota_reservations WHERE date_string = ? AND status = 'reserved';")
        .map_err(|e| e.to_string())?;
    stmt_reserve
        .bind((1, date_string))
        .map_err(|e| e.to_string())?;
    let reserved_bytes: i64 = if let Ok(State::Row) = stmt_reserve.next() {
        stmt_reserve.read(0).unwrap_or(0)
    } else {
        0
    };

    Ok(committed_bytes + reserved_bytes)
}

/// Giữ trước quota cho tệp tin trước khi upload
pub fn reserve_quota(
    conn: &Connection,
    item_id: i64,
    job_id: i64,
    date_string: &str,
    bytes: i64,
    expires_in_secs: i64,
) -> Result<(), String> {
    let used = get_daily_used_bytes(conn, date_string)?;
    if used + bytes > DAILY_SAFETY_BUDGET_LIMIT {
        return Err(format!(
            "Daily safety budget exceeded. Limit: {}, Used: {}, Requested: {}",
            DAILY_SAFETY_BUDGET_LIMIT, used, bytes
        ));
    }

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let expires_at = now + expires_in_secs;

    let mut stmt = conn
        .prepare(
            "INSERT OR REPLACE INTO quota_reservations (item_id, job_id, date_string, reserved_bytes, status, created_at, expires_at)
             VALUES (?, ?, ?, ?, 'reserved', ?, ?);"
        )
        .map_err(|e| e.to_string())?;
    stmt.bind((1, item_id)).map_err(|e| e.to_string())?;
    stmt.bind((2, job_id)).map_err(|e| e.to_string())?;
    stmt.bind((3, date_string)).map_err(|e| e.to_string())?;
    stmt.bind((4, bytes)).map_err(|e| e.to_string())?;
    stmt.bind((5, now)).map_err(|e| e.to_string())?;
    stmt.bind((6, expires_at)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;

    Ok(())
}

/// Xác nhận tệp upload thành công, cộng vào database quota chính thức và giải phóng reservation
pub fn commit_quota(conn: &Connection, item_id: i64) -> Result<(), String> {
    // Đọc reservation
    let mut stmt_read = conn
        .prepare("SELECT job_id, date_string, reserved_bytes, status FROM quota_reservations WHERE item_id = ? LIMIT 1;")
        .map_err(|e| e.to_string())?;
    stmt_read.bind((1, item_id)).map_err(|e| e.to_string())?;

    if let Ok(State::Row) = stmt_read.next() {
        let status: String = stmt_read.read(3).unwrap_or_default();
        if status != "reserved" {
            return Ok(()); // Đã committed hoặc released rồi
        }

        let date_string: String = stmt_read.read(1).unwrap_or_default();
        let bytes: i64 = stmt_read.read(2).unwrap_or(0);

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        // 1. Cập nhật status của reservation thành 'committed'
        let mut stmt_upd_res = conn
            .prepare("UPDATE quota_reservations SET status = 'committed' WHERE item_id = ?;")
            .map_err(|e| e.to_string())?;
        stmt_upd_res.bind((1, item_id)).map_err(|e| e.to_string())?;
        stmt_upd_res.next().map_err(|e| e.to_string())?;

        // 2. Cộng dồn vào daily_migration_quota chính thức
        let mut stmt_quota_check = conn
            .prepare(
                "SELECT uploaded_bytes FROM daily_migration_quota WHERE date_string = ? LIMIT 1;",
            )
            .map_err(|e| e.to_string())?;
        stmt_quota_check
            .bind((1, date_string.as_str()))
            .map_err(|e| e.to_string())?;

        if let Ok(State::Row) = stmt_quota_check.next() {
            let current_val: i64 = stmt_quota_check.read(0).unwrap_or(0);
            let mut stmt_upd_q = conn
                .prepare("UPDATE daily_migration_quota SET uploaded_bytes = ?, updated_at = ? WHERE date_string = ?;")
                .map_err(|e| e.to_string())?;
            stmt_upd_q
                .bind((1, current_val + bytes))
                .map_err(|e| e.to_string())?;
            stmt_upd_q.bind((2, now)).map_err(|e| e.to_string())?;
            stmt_upd_q
                .bind((3, date_string.as_str()))
                .map_err(|e| e.to_string())?;
            stmt_upd_q.next().map_err(|e| e.to_string())?;
        } else {
            let mut stmt_ins_q = conn
                .prepare("INSERT INTO daily_migration_quota (date_string, uploaded_bytes, updated_at) VALUES (?, ?, ?);")
                .map_err(|e| e.to_string())?;
            stmt_ins_q
                .bind((1, date_string.as_str()))
                .map_err(|e| e.to_string())?;
            stmt_ins_q.bind((2, bytes)).map_err(|e| e.to_string())?;
            stmt_ins_q.bind((3, now)).map_err(|e| e.to_string())?;
            stmt_ins_q.next().map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

/// Giải phóng quota nếu upload thất bại
pub fn release_quota(conn: &Connection, item_id: i64) -> Result<(), String> {
    let mut stmt = conn
        .prepare("UPDATE quota_reservations SET status = 'released' WHERE item_id = ?;")
        .map_err(|e| e.to_string())?;
    stmt.bind((1, item_id)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;
    Ok(())
}

/// Quét dọn dẹp (recovery) các quota reservation bị kẹt khi startup hoặc qua ngày mới
pub fn run_quota_recovery(
    conn: &Connection,
    current_date_string: &str,
    now_secs: i64,
) -> Result<(), String> {
    // 1. Chuyển các reservation hết hạn hoặc khác ngày sang 'released'
    let mut stmt = conn
        .prepare(
            "UPDATE quota_reservations
             SET status = 'released'
             WHERE status = 'reserved' AND (expires_at < ?1 OR date_string <> ?2);",
        )
        .map_err(|e| e.to_string())?;
    stmt.bind((1, now_secs)).map_err(|e| e.to_string())?;
    stmt.bind((2, current_date_string))
        .map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;

    Ok(())
}
