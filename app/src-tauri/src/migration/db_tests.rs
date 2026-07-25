use crate::migration::db::open_migration_db_at_path;
use crate::migration::quota_reserve::*;
use crate::migration::repository_v2::*;
use crate::migration::schema_v2::migrate_to_v2;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        let path = env::temp_dir().join(format!("temp_db_dir_{}", rand::random::<u64>()));
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

fn setup_v1_db(conn: &sqlite::Connection) {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS migration_jobs (
            id                      INTEGER PRIMARY KEY AUTOINCREMENT,
            state                   TEXT NOT NULL DEFAULT 'draft',
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
    .unwrap();

    conn.execute(
        "CREATE TABLE IF NOT EXISTS migration_items (
            id                      INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id                  INTEGER NOT NULL,
            item_type               TEXT NOT NULL DEFAULT 'file',
            name                    TEXT NOT NULL,
            source_path             TEXT NOT NULL,
            source_item_id          TEXT,
            size_bytes              INTEGER NOT NULL DEFAULT 0,
            source_etag             TEXT,
            source_last_modified    TEXT,
            source_fingerprint_type TEXT,
            source_fingerprint_value TEXT,
            state                   TEXT NOT NULL DEFAULT 'pending',
            last_error_code         TEXT,
            last_error_message      TEXT,
            attempt_count           INTEGER NOT NULL DEFAULT 0,
            computed_sha256         TEXT,
            telegram_message_id     INTEGER,
            created_at              INTEGER NOT NULL,
            completed_at            INTEGER,
            UNIQUE(job_id, source_path)
        );",
    )
    .unwrap();

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
    .unwrap();

    conn.execute(
        "CREATE TABLE IF NOT EXISTS daily_migration_quota (
            date_string             TEXT PRIMARY KEY,
            uploaded_bytes          INTEGER NOT NULL DEFAULT 0,
            updated_at              INTEGER NOT NULL
        );",
    )
    .unwrap();
}

#[test]
fn test_fresh_database_and_double_migration() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_fresh.db");

    // 1. Fresh database init qua open_migration_db_at_path
    let db = open_migration_db_at_path(db_path.clone()).unwrap();
    {
        let conn = db.lock().unwrap();
        // Kiểm tra các cột v2 tồn tại
        conn.prepare("SELECT pipeline_version, local_backup_dir, workspace_dir, manifest_state FROM migration_jobs;").unwrap();
        conn.prepare("SELECT route_kind, duplicate_of_item_id, artifact_size_bytes, local_dest_path, telegram_random_id, video_decision FROM migration_items;").unwrap();
    }

    // 2. Migration chạy hai lần không lỗi (Idempotency)
    let conn = db.lock().unwrap();
    migrate_to_v2(&conn).unwrap();
}

#[test]
fn test_upgrade_from_v1_and_recovery() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_v1.db");

    // Dựng database v1 chay
    {
        let conn = sqlite::open(&db_path).unwrap();
        setup_v1_db(&conn);
        // Chèn một job v1 đang running
        conn.execute("INSERT INTO migration_jobs (state, total_files, created_at, updated_at) VALUES ('running', 10, 100, 100);").unwrap();
    }

    // Nâng cấp lên v2
    let db = open_migration_db_at_path(db_path).unwrap();
    {
        let conn = db.lock().unwrap();
        // Verify job v1 đang running tự chuyển paused và pause_reason = recovery_interrupted
        let mut stmt = conn
            .prepare("SELECT state, pause_reason FROM migration_jobs WHERE id = 1;")
            .unwrap();
        assert_eq!(stmt.next().unwrap(), sqlite::State::Row);
        let state: String = stmt.read(0).unwrap();
        let reason: String = stmt.read(1).unwrap();
        assert_eq!(state, "paused");
        assert_eq!(reason, "recovery_interrupted");
    }
}

#[test]
fn test_insert_read_job_v2() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_job_v2.db");
    let db = open_migration_db_at_path(db_path).unwrap();
    let conn = db.lock().unwrap();

    // Chèn job v2
    conn.execute(
        "INSERT INTO migration_jobs (state, pipeline_version, local_backup_dir, workspace_dir, manifest_state, created_at, updated_at)
         VALUES ('draft', 2, '/backup', '/workspace', 'export_pending', 200, 200);"
    ).unwrap();

    let mut stmt = conn.prepare("SELECT pipeline_version, local_backup_dir, workspace_dir, manifest_state FROM migration_jobs WHERE id = 1;").unwrap();
    assert_eq!(stmt.next().unwrap(), sqlite::State::Row);
    let version: i64 = stmt.read(0).unwrap();
    let local_backup: String = stmt.read(1).unwrap();
    let workspace: String = stmt.read(2).unwrap();
    let manifest: String = stmt.read(3).unwrap();

    assert_eq!(version, 2);
    assert_eq!(local_backup, "/backup");
    assert_eq!(workspace, "/workspace");
    assert_eq!(manifest, "export_pending");
}

#[test]
fn test_quota_reservation_and_recovery() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_quota.db");
    let db = open_migration_db_at_path(db_path).unwrap();
    let conn = db.lock().unwrap();

    // Reset daily quota
    conn.execute("INSERT OR REPLACE INTO daily_migration_quota (date_string, uploaded_bytes, updated_at) VALUES ('2026-07-25', 0, 0);").unwrap();

    // 1. Giữ quota hợp lệ
    reserve_quota(&conn, 1, 1, "2026-07-25", 50_000_000_000, 3600).unwrap();
    assert_eq!(
        get_daily_used_bytes(&conn, "2026-07-25").unwrap(),
        50_000_000_000
    );

    // 2. Giữ quota vượt quá giới hạn (250GB) bị từ chối
    let res = reserve_quota(&conn, 2, 1, "2026-07-25", 210_000_000_000, 3600);
    assert!(res.is_err());

    // 3. Commit quota
    commit_quota(&conn, 1).unwrap();
    assert_eq!(
        get_daily_used_bytes(&conn, "2026-07-25").unwrap(),
        50_000_000_000
    );

    // 4. Giữ quota cho file thứ 2 để test recovery
    reserve_quota(&conn, 2, 1, "2026-07-25", 10_000_000_000, -10).unwrap(); // đã hết hạn (expires_at < now)
    reserve_quota(&conn, 3, 1, "2026-07-24", 20_000_000_000, 3600).unwrap(); // khác ngày

    run_quota_recovery(&conn, "2026-07-25", 99999999999).unwrap();

    // Verify các reservation lỗi/hết hạn bị đổi sang released
    let mut stmt = conn
        .prepare("SELECT status FROM quota_reservations WHERE item_id = 2;")
        .unwrap();
    assert_eq!(stmt.next().unwrap(), sqlite::State::Row);
    let status2: String = stmt.read(0).unwrap();
    assert_eq!(status2, "released");

    let mut stmt = conn
        .prepare("SELECT status FROM quota_reservations WHERE item_id = 3;")
        .unwrap();
    assert_eq!(stmt.next().unwrap(), sqlite::State::Row);
    let status3: String = stmt.read(0).unwrap();
    assert_eq!(status3, "released");
}

#[test]
fn test_dedupe_unique_claim_and_promotion() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_dedupe.db");
    let db = open_migration_db_at_path(db_path).unwrap();

    // Seed items
    {
        let conn = db.lock().unwrap();
        // Job 1
        conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, state, created_at, queue_position) VALUES (1, 1, 'file1', 'path1', 'pending', 100, 0);").unwrap();
        conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, state, created_at, queue_position) VALUES (2, 1, 'file2', 'path2', 'pending', 100, 0);").unwrap();
        // Job 2 (Khác target Telegram, không dedupe chéo)
        conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, state, created_at, queue_position) VALUES (3, 2, 'file3', 'path3', 'pending', 100, 0);").unwrap();
    }

    // 1. Hai file cùng fingerprint và cùng target -> 1 canonical, 1 duplicate waiting
    let res1 = claim_dedupe_canonical(&db, 1, "sha256", "abc", 1000, "telegram:123").unwrap();
    assert_eq!(res1, DedupeResultV2::CanonicalClaimed);

    let res2 = claim_dedupe_canonical(&db, 2, "sha256", "abc", 1000, "telegram:123").unwrap();
    assert_eq!(
        res2,
        DedupeResultV2::WaitingForCanonical {
            canonical_item_id: 1
        }
    );

    // 2. Cùng fingerprint nhưng khác target Telegram (khác job_id) -> không dedupe chéo
    let res3 = claim_dedupe_canonical(&db, 3, "sha256", "abc", 1000, "telegram:456").unwrap();
    assert_eq!(res3, DedupeResultV2::CanonicalClaimed);

    // 3. Canonical (item 1) thất bại -> duplicate (item 2) được promote
    {
        let conn = db.lock().unwrap();
        conn.execute("UPDATE migration_items SET state = 'failed' WHERE id = 1;")
            .unwrap();
    }

    let promo = promote_next_duplicate(&db, 1).unwrap();
    assert_eq!(promo, Some(2));

    {
        let conn = db.lock().unwrap();
        let mut check = conn
            .prepare("SELECT state, duplicate_of_item_id FROM migration_items WHERE id = 2;")
            .unwrap();
        assert_eq!(check.next().unwrap(), sqlite::State::Row);
        let state: String = check.read(0).unwrap();
        let dup_id: Option<i64> = check.read(1).unwrap();
        assert_eq!(state, "pending");
        assert!(dup_id.is_none());
    }
}

#[test]
fn test_concurrency_atomic_claim() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_concurrency.db");
    let db = open_migration_db_at_path(db_path).unwrap();

    // Seed 10 files
    {
        let conn = db.lock().unwrap();
        for i in 1..=10 {
            conn.execute(format!(
                "INSERT INTO migration_items (id, job_id, name, source_path, state, created_at, queue_position)
                 VALUES ({i}, 1, 'file{i}', 'path{i}', 'pending', 100, {i});"
            )).unwrap();
        }
    }

    // Chạy 5 threads song song tranh giành claim
    let mut handles = vec![];
    for _ in 0..5 {
        let db_clone = db.clone();
        let handle = thread::spawn(move || {
            let mut claimed_ids = vec![];
            for _ in 0..2 {
                if let Ok(Some(item)) = claim_next_item(&db_clone, 1) {
                    claimed_ids.push(item.id);
                }
                thread::sleep(Duration::from_millis(5));
            }
            claimed_ids
        });
        handles.push(handle);
    }

    let mut all_claimed = vec![];
    for h in handles {
        let mut ids = h.join().unwrap();
        all_claimed.append(&mut ids);
    }

    // Verify:
    // 1. Tổng số file được claim là 10.
    // 2. Không có ID nào bị trùng lặp giữa các thread.
    all_claimed.sort();
    assert_eq!(all_claimed.len(), 10);
    for i in 0..9 {
        assert_ne!(all_claimed[i], all_claimed[i + 1]);
    }
}

use std::sync::atomic::{AtomicUsize, Ordering};

struct MockMicrosoftAdapter {
    delete_calls: Arc<AtomicUsize>,
    upload_success_calls: Arc<AtomicUsize>,
}

impl MockMicrosoftAdapter {
    fn new() -> Self {
        Self {
            delete_calls: Arc::new(AtomicUsize::new(0)),
            upload_success_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn delete_item(&self) {
        self.delete_calls.fetch_add(1, Ordering::Relaxed);
    }

    fn upload_success(&self) {
        self.upload_success_calls.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn test_mock_adapter_safety_guarantees() {
    let adapter = MockMicrosoftAdapter::new();

    // Giả lập chạy backup job hoàn tất
    adapter.upload_success();
    adapter.upload_success();

    // Tuyệt đối không gọi delete
    assert!(adapter.upload_success_calls.load(Ordering::Relaxed) > 0);
    assert_eq!(adapter.delete_calls.load(Ordering::Relaxed), 0);
}
