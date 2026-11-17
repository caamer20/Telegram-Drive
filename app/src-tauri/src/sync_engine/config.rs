use super::policy::{StoredPairPolicy, SyncPreferences};
use crate::db::DbConnection;
use serde::{Deserialize, Serialize};
use sqlite::State;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSettings {
    pub enabled: bool,
    pub debounce_ms: u64,
    pub encryption: String,
    /// `full` re-hashes every local file and re-reads the Telegram folder on
    /// every cycle. `incremental` reuses recorded hashes for files whose size
    /// and modification time are unchanged and polls idle mappings less often.
    #[serde(default = "default_scanner")]
    pub scanner: String,
}

pub const SCANNER_FULL: &str = "full";
pub const SCANNER_INCREMENTAL: &str = "incremental";

fn default_scanner() -> String {
    SCANNER_FULL.to_string()
}

impl SyncSettings {
    pub fn uses_incremental_scanner(&self) -> bool {
        self.scanner == SCANNER_INCREMENTAL
    }
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            debounce_ms: 3_000,
            encryption: "inherit".to_string(),
            scanner: default_scanner(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPair {
    pub id: i64,
    pub local_path: String,
    pub channel_id: i64,
    pub folder_key: String,
    pub label: Option<String>,
    pub sync_direction: String,
    pub is_active: bool,
    pub created_at: i64,
    pub account_owner: Option<String>,
    pub preferences: SyncPreferences,
}

pub async fn load_settings(db: DbConnection) -> Result<SyncSettings, String> {
    crate::db::with_connection(db, |connection| {
        let mut settings = SyncSettings::default();
        let mut statement = connection
            .prepare("SELECT key, value FROM sync_settings")
            .map_err(|error| error.to_string())?;
        while statement.next().map_err(|error| error.to_string())? == State::Row {
            let key = statement
                .read::<String, _>(0)
                .map_err(|error| error.to_string())?;
            let value = statement
                .read::<String, _>(1)
                .map_err(|error| error.to_string())?;
            match key.as_str() {
                "sync_enabled" => settings.enabled = value == "true",
                "sync_debounce_ms" => {
                    settings.debounce_ms = value.parse().unwrap_or(3_000).clamp(250, 60_000)
                }
                "sync_encryption" => settings.encryption = value,
                // An unknown value keeps the conservative full scanner.
                "sync_scanner" if value == SCANNER_INCREMENTAL => settings.scanner = value,
                _ => {}
            }
        }
        Ok(settings)
    })
    .await
}

pub async fn load_pairs(db: DbConnection, active_only: bool) -> Result<Vec<SyncPair>, String> {
    crate::db::with_connection(db, move |connection| {
    let query = if active_only {
        "SELECT p.id, p.local_path, p.channel_id, p.folder_key, p.label, p.sync_direction, p.is_active, p.created_at, s.value FROM sync_pairs p LEFT JOIN sync_settings s ON s.key = 'sync_pair_policy:' || p.id WHERE p.is_active = 1 ORDER BY p.id"
    } else {
        "SELECT p.id, p.local_path, p.channel_id, p.folder_key, p.label, p.sync_direction, p.is_active, p.created_at, s.value FROM sync_pairs p LEFT JOIN sync_settings s ON s.key = 'sync_pair_policy:' || p.id ORDER BY p.id"
    };
    let mut statement = connection
        .prepare(query)
        .map_err(|error| error.to_string())?;
    let mut pairs = Vec::new();
    while statement.next().map_err(|error| error.to_string())? == State::Row {
        let policy = statement.read::<Option<String>, _>(8).ok().flatten()
            .and_then(|value| serde_json::from_str::<StoredPairPolicy>(&value).ok())
            .unwrap_or_default();
        let policy = match policy.preferences.clone().validated() {
            Ok(preferences) => StoredPairPolicy { preferences, ..policy },
            Err(_) => StoredPairPolicy::default(),
        };
        pairs.push(SyncPair {
            id: statement.read(0).map_err(|error| error.to_string())?,
            local_path: statement.read(1).map_err(|error| error.to_string())?,
            channel_id: statement.read(2).map_err(|error| error.to_string())?,
            folder_key: statement.read(3).map_err(|error| error.to_string())?,
            label: statement.read::<Option<String>, _>(4).ok().flatten(),
            sync_direction: statement.read(5).map_err(|error| error.to_string())?,
            is_active: statement.read::<i64, _>(6).unwrap_or(0) != 0,
            created_at: statement.read(7).map_err(|error| error.to_string())?,
            account_owner: policy.account_owner,
            preferences: policy.preferences,
        });
    }
    Ok(pairs)
    }).await
}

pub async fn save_pair_policy(
    db: DbConnection,
    pair_id: i64,
    policy: &StoredPairPolicy,
) -> Result<(), String> {
    let policy = policy.clone();
    crate::db::with_connection(db, move |connection| {
        write_pair_policy(connection, pair_id, &policy)
    })
    .await
}

pub(crate) fn write_pair_policy(
    connection: &sqlite::Connection,
    pair_id: i64,
    policy: &StoredPairPolicy,
) -> Result<(), String> {
    let mut statement = connection.prepare("INSERT INTO sync_settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value").map_err(|error| error.to_string())?;
    statement
        .bind((1, format!("sync_pair_policy:{pair_id}").as_str()))
        .map_err(|error| error.to_string())?;
    statement
        .bind((
            2,
            serde_json::to_string(policy)
                .map_err(|error| error.to_string())?
                .as_str(),
        ))
        .map_err(|error| error.to_string())?;
    statement.next().map_err(|error| error.to_string())?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingCleanup {
    pub relative_path: String,
    pub old_message_id: i32,
    pub new_message_id: i32,
    #[serde(default)]
    pub old_remote_hash: Option<String>,
}

pub async fn load_pending_cleanup(
    db: DbConnection,
    pair_id: i64,
) -> Result<Vec<PendingCleanup>, String> {
    crate::db::with_connection(db, move |connection| {
        let mut statement = connection
            .prepare("SELECT value FROM sync_settings WHERE key = ?")
            .map_err(|error| error.to_string())?;
        statement
            .bind((1, format!("sync_pair_cleanup:{pair_id}").as_str()))
            .map_err(|error| error.to_string())?;
        if statement.next().map_err(|error| error.to_string())? != State::Row {
            return Ok(Vec::new());
        }
        let value = statement
            .read::<String, _>(0)
            .map_err(|error| error.to_string())?;
        serde_json::from_str(&value)
            .map_err(|error| format!("Pending sync cleanup could not be read: {error}"))
    })
    .await
}

pub async fn save_pending_cleanup(
    db: DbConnection,
    pair_id: i64,
    pending: &[PendingCleanup],
) -> Result<(), String> {
    set_setting(
        db,
        format!("sync_pair_cleanup:{pair_id}"),
        serde_json::to_string(pending).map_err(|error| error.to_string())?,
    )
    .await
}

pub async fn set_setting(db: DbConnection, key: String, value: String) -> Result<(), String> {
    crate::db::with_connection(db, move |connection| {
    let mut statement = connection
        .prepare("INSERT INTO sync_settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .map_err(|error| error.to_string())?;
    statement
        .bind((1, key.as_str()))
        .map_err(|error| error.to_string())?;
    statement
        .bind((2, value.as_str()))
        .map_err(|error| error.to_string())?;
    statement.next().map_err(|error| error.to_string())?;
    Ok(())
    }).await
}

pub async fn log_sync(
    db: DbConnection,
    pair_id: Option<i64>,
    action: String,
    path: Option<String>,
    detail: Option<String>,
) {
    let _ = crate::db::with_connection(db, move |connection| {
    let mut statement = connection.prepare(
        "INSERT INTO sync_log (pair_id, action, relative_path, detail, created_at) VALUES (?, ?, ?, ?, ?)",
    ).map_err(|error| error.to_string())?;
    statement.bind::<(usize, Option<i64>)>((1, pair_id)).map_err(|error| error.to_string())?;
    statement.bind((2, action.as_str())).map_err(|error| error.to_string())?;
    statement.bind::<(usize, Option<&str>)>((3, path.as_deref())).map_err(|error| error.to_string())?;
    statement.bind::<(usize, Option<&str>)>((4, detail.as_deref())).map_err(|error| error.to_string())?;
    statement.bind((5, crate::app_time::unix_seconds())).map_err(|error| error.to_string())?;
    statement.next().map_err(|error| error.to_string())?;
    drop(statement);
    // Keep diagnostics useful without allowing an always-offline or otherwise
    // failing pair to grow the database forever.
    connection.execute(
        "DELETE FROM sync_log WHERE id < (SELECT COALESCE(MAX(id), 0) - 10000 FROM sync_log)",
    ).map_err(|error| error.to_string())?;
    Ok(())
    }).await;
}
