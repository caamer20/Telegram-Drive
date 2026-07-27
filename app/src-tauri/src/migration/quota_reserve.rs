use sqlite::{Connection, State};
use std::time::SystemTime;

pub const DAILY_SAFETY_BUDGET_LIMIT: i64 = 250_000_000_000; // 250 GB hard cap

/// Lấy tổng số byte đã dùng (committed + active reserved) trong ngày
pub fn get_daily_used_bytes(conn: &Connection, date_string: &str) -> Result<i64, String> {
    // 1. Committed bytes từ daily_migration_quota
    let mut stmt_quota = conn
        .prepare("SELECT used_bytes FROM daily_migration_quota WHERE date_string = ? LIMIT 1;")
        .map_err(|e| e.to_string())?;
    stmt_quota
        .bind((1, date_string))
        .map_err(|e| e.to_string())?;
    let committed_bytes: i64 = if let Ok(State::Row) = stmt_quota.next() {
        stmt_quota.read(0).unwrap_or(0)
    } else {
        0
    };

    // 2. Active reserved bytes
    let mut stmt_reserve = conn
        .prepare("SELECT COALESCE(SUM(reserved_bytes), 0) FROM quota_reservations WHERE status = 'active';")
        .map_err(|e| e.to_string())?;
    let reserved_bytes: i64 = if let Ok(State::Row) = stmt_reserve.next() {
        stmt_reserve.read(0).unwrap_or(0)
    } else {
        0
    };

    Ok(committed_bytes + reserved_bytes)
}

/// Atomic reserve quota trong transaction.
/// Ngăn race condition giữa check và insert.
pub fn reserve_quota(
    conn: &Connection,
    item_id: i64,
    job_id: i64,
    date_string: &str,
    bytes: i64,
    expires_in_secs: i64,
) -> Result<(), String> {
    conn.execute("BEGIN IMMEDIATE TRANSACTION;")
        .map_err(|e| format!("reserve_quota: BEGIN failed: {}", e))?;

    // 1. Release expired reservations trước
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let mut release_stmt = conn
        .prepare("UPDATE quota_reservations SET status = 'released' WHERE status = 'active' AND expires_at < ?;")
        .map_err(|e| e.to_string())?;
    release_stmt.bind((1, now)).map_err(|e| e.to_string())?;
    release_stmt.next().map_err(|e| e.to_string())?;

    // 2. Đọc committed + active reserved
    let used = get_daily_used_bytes(conn, date_string)?;
    if used + bytes > DAILY_SAFETY_BUDGET_LIMIT {
        conn.execute("ROLLBACK;").map_err(|e| e.to_string())?;
        return Err(format!(
            "Daily safety budget exceeded. Limit: {}, Used: {}, Requested: {}",
            DAILY_SAFETY_BUDGET_LIMIT, used, bytes
        ));
    }

    // 3. Xóa reservation cũ của item này nếu có (đảm bảo clean)
    let mut del_stmt = conn
        .prepare("DELETE FROM quota_reservations WHERE item_id = ? AND status = 'active';")
        .map_err(|e| e.to_string())?;
    del_stmt.bind((1, item_id)).map_err(|e| e.to_string())?;
    del_stmt.next().map_err(|e| e.to_string())?;

    // 4. Insert reservation mới
    let expires_at = now + expires_in_secs;
    let mut stmt = conn
        .prepare(
            "INSERT INTO quota_reservations (job_id, item_id, reserved_bytes, reserved_at, expires_at, status)
             VALUES (?, ?, ?, ?, ?, 'active');"
        )
        .map_err(|e| e.to_string())?;
    stmt.bind((1, job_id)).map_err(|e| e.to_string())?;
    stmt.bind((2, item_id)).map_err(|e| e.to_string())?;
    stmt.bind((3, bytes)).map_err(|e| e.to_string())?;
    stmt.bind((4, now)).map_err(|e| e.to_string())?;
    stmt.bind((5, expires_at)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;

    conn.execute("COMMIT;")
        .map_err(|e| format!("reserve_quota: COMMIT failed: {}", e))?;
    Ok(())
}

/// Atomic commit quota — xác nhận upload thành công, cập nhật daily_quota
pub fn commit_quota(conn: &Connection, item_id: i64, date_string: &str) -> Result<(), String> {
    conn.execute("BEGIN IMMEDIATE TRANSACTION;")
        .map_err(|e| format!("commit_quota: BEGIN failed: {}", e))?;

    // Đọc reservation active
    let mut stmt_read = conn
        .prepare("SELECT id, reserved_bytes, status FROM quota_reservations WHERE item_id = ? AND status = 'active' ORDER BY id DESC LIMIT 1;")
        .map_err(|e| e.to_string())?;
    stmt_read.bind((1, item_id)).map_err(|e| e.to_string())?;

    if let Ok(State::Row) = stmt_read.next() {
        let reservation_id: i64 = stmt_read.read(0).unwrap_or(0);
        let bytes: i64 = stmt_read.read(1).unwrap_or(0);

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        // 1. Update reservation → committed
        let mut upd_res = conn
            .prepare("UPDATE quota_reservations SET status = 'committed' WHERE id = ?;")
            .map_err(|e| e.to_string())?;
        upd_res
            .bind((1, reservation_id))
            .map_err(|e| e.to_string())?;
        upd_res.next().map_err(|e| e.to_string())?;

        // 2. Upsert daily_migration_quota
        let mut check_stmt = conn
            .prepare("SELECT used_bytes FROM daily_migration_quota WHERE date_string = ? LIMIT 1;")
            .map_err(|e| e.to_string())?;
        check_stmt
            .bind((1, date_string))
            .map_err(|e| e.to_string())?;

        if let Ok(State::Row) = check_stmt.next() {
            let current: i64 = check_stmt.read(0).unwrap_or(0);
            let mut upd_q = conn
                .prepare("UPDATE daily_migration_quota SET used_bytes = ?, reset_at = ? WHERE date_string = ?;")
                .map_err(|e| e.to_string())?;
            upd_q
                .bind((1, current + bytes))
                .map_err(|e| e.to_string())?;
            upd_q.bind((2, now)).map_err(|e| e.to_string())?;
            upd_q.bind((3, date_string)).map_err(|e| e.to_string())?;
            upd_q.next().map_err(|e| e.to_string())?;
        } else {
            let mut ins_q = conn
                .prepare("INSERT INTO daily_migration_quota (date_string, used_bytes, reset_at) VALUES (?, ?, ?);")
                .map_err(|e| e.to_string())?;
            ins_q.bind((1, date_string)).map_err(|e| e.to_string())?;
            ins_q.bind((2, bytes)).map_err(|e| e.to_string())?;
            ins_q.bind((3, now)).map_err(|e| e.to_string())?;
            ins_q.next().map_err(|e| e.to_string())?;
        }
    }

    conn.execute("COMMIT;")
        .map_err(|e| format!("commit_quota: COMMIT failed: {}", e))?;
    Ok(())
}

/// Atomic release quota — hoàn trả bytes vào daily_migration_quota
pub fn release_quota(conn: &Connection, item_id: i64, date_string: &str) -> Result<(), String> {
    conn.execute("BEGIN IMMEDIATE TRANSACTION;")
        .map_err(|e| format!("release_quota: BEGIN failed: {}", e))?;

    // Đọc reservation active
    let mut stmt_read = conn
        .prepare("SELECT id, reserved_bytes FROM quota_reservations WHERE item_id = ? AND status = 'active' ORDER BY id DESC LIMIT 1;")
        .map_err(|e| e.to_string())?;
    stmt_read.bind((1, item_id)).map_err(|e| e.to_string())?;

    if let Ok(State::Row) = stmt_read.next() {
        let reservation_id: i64 = stmt_read.read(0).unwrap_or(0);
        let bytes: i64 = stmt_read.read(1).unwrap_or(0);

        // 1. Update reservation → released
        let mut upd = conn
            .prepare("UPDATE quota_reservations SET status = 'released' WHERE id = ?;")
            .map_err(|e| e.to_string())?;
        upd.bind((1, reservation_id)).map_err(|e| e.to_string())?;
        upd.next().map_err(|e| e.to_string())?;

        // 2. Hoàn trả bytes vào daily_migration_quota (nếu đã được commit trước đó)
        // Chỉ hoàn trả nếu có bản ghi daily_migration_quota
        let mut check_stmt = conn
            .prepare("SELECT used_bytes FROM daily_migration_quota WHERE date_string = ? LIMIT 1;")
            .map_err(|e| e.to_string())?;
        check_stmt
            .bind((1, date_string))
            .map_err(|e| e.to_string())?;

        if let Ok(State::Row) = check_stmt.next() {
            let current: i64 = check_stmt.read(0).unwrap_or(0);
            let new_val = (current - bytes).max(0);
            let mut upd_q = conn
                .prepare("UPDATE daily_migration_quota SET used_bytes = ? WHERE date_string = ?;")
                .map_err(|e| e.to_string())?;
            upd_q.bind((1, new_val)).map_err(|e| e.to_string())?;
            upd_q.bind((2, date_string)).map_err(|e| e.to_string())?;
            upd_q.next().map_err(|e| e.to_string())?;
        }
    }

    conn.execute("COMMIT;")
        .map_err(|e| format!("release_quota: COMMIT failed: {}", e))?;
    Ok(())
}

/// Quét dọn dẹp (recovery) các quota reservation bị kẹt khi startup hoặc qua ngày mới
pub fn run_quota_recovery(
    conn: &Connection,
    date_string: &str,
    now_secs: i64,
) -> Result<(), String> {
    conn.execute("BEGIN IMMEDIATE TRANSACTION;")
        .map_err(|e| format!("quota_recovery: BEGIN failed: {}", e))?;

    // 1. Đọc các reservation hết hạn
    let mut read_stmt = conn
        .prepare("SELECT id, reserved_bytes FROM quota_reservations WHERE status = 'active' AND expires_at < ?;")
        .map_err(|e| e.to_string())?;
    read_stmt.bind((1, now_secs)).map_err(|e| e.to_string())?;

    let mut released_bytes: i64 = 0;
    let mut ids_to_release: Vec<i64> = Vec::new();
    while let Ok(State::Row) = read_stmt.next() {
        let id: i64 = read_stmt.read(0).unwrap_or(0);
        let bytes: i64 = read_stmt.read(1).unwrap_or(0);
        ids_to_release.push(id);
        released_bytes += bytes;
    }

    // 2. Release chúng
    for id in &ids_to_release {
        let mut upd = conn
            .prepare("UPDATE quota_reservations SET status = 'released' WHERE id = ?;")
            .map_err(|e| e.to_string())?;
        upd.bind((1, *id)).map_err(|e| e.to_string())?;
        upd.next().map_err(|e| e.to_string())?;
    }

    // 3. Hoàn trả bytes vào daily_migration_quota
    if released_bytes > 0 {
        let mut check_stmt = conn
            .prepare("SELECT used_bytes FROM daily_migration_quota WHERE date_string = ? LIMIT 1;")
            .map_err(|e| e.to_string())?;
        check_stmt
            .bind((1, date_string))
            .map_err(|e| e.to_string())?;

        if let Ok(State::Row) = check_stmt.next() {
            let current: i64 = check_stmt.read(0).unwrap_or(0);
            let new_val = (current - released_bytes).max(0);
            let mut upd_q = conn
                .prepare("UPDATE daily_migration_quota SET used_bytes = ? WHERE date_string = ?;")
                .map_err(|e| e.to_string())?;
            upd_q.bind((1, new_val)).map_err(|e| e.to_string())?;
            upd_q.bind((2, date_string)).map_err(|e| e.to_string())?;
            upd_q.next().map_err(|e| e.to_string())?;
        }
    }

    conn.execute("COMMIT;")
        .map_err(|e| format!("quota_recovery: COMMIT failed: {}", e))?;
    Ok(())
}
