//! Account-bound full-text search. Both main and temporary SQLite storage are memory-only.
use crate::{
    crypto::state::{CryptoState, UnlockSessionId},
    workspace::{
        store::{SavedSearch, WorkspaceFile},
        AccountGuard,
    },
};
use serde::{Deserialize, Serialize};
use sqlite::{Connection, State};
use std::collections::HashSet;
const ROW_LIMIT: usize = 100_000;
const TEXT_LIMIT: usize = 32 * 1024 * 1024;
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Query {
    pub query: String,
    pub index_id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub size: String,
    pub date: String,
    pub folder_key: Option<String>,
    pub tags: Vec<String>,
    pub collection_id: Option<String>,
    pub favorites_only: bool,
    pub protection: String,
    pub offset: usize,
    pub limit: Option<usize>,
}
impl Query {
    pub(crate) fn saved(value: &SavedSearch) -> Result<Self, String> {
        let query = Self {
            query: value.query.clone(),
            kind: value
                .filters
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("all")
                .into(),
            size: value
                .filters
                .get("size")
                .and_then(|v| v.as_str())
                .unwrap_or("any")
                .into(),
            date: value
                .filters
                .get("date")
                .and_then(|v| v.as_str())
                .unwrap_or("any")
                .into(),
            protection: value
                .filters
                .get("protection")
                .and_then(|v| v.as_str())
                .unwrap_or("any")
                .into(),
            folder_key: value.folder_key.clone(),
            tags: value.tags.clone(),
            collection_id: value.collection_id.clone(),
            favorites_only: value.favorites_only,
            ..Default::default()
        };
        query.validate()?;
        Ok(query)
    }
    fn validate(&self) -> Result<(), String> {
        if self.query.len() > 500
            || self.query.split_whitespace().count() > 32
            || self.query.chars().any(char::is_control)
            || self.tags.len() > 100
            || self
                .tags
                .iter()
                .any(|t| t.is_empty() || t.chars().count() > 40)
            || self
                .collection_id
                .as_ref()
                .is_some_and(|v| v.is_empty() || v.len() > 256)
            || self
                .folder_key
                .as_ref()
                .is_some_and(|v| v != "saved" && !v.parse::<i64>().is_ok_and(|n| n > 0))
            || self.limit.unwrap_or(100) == 0
            || self.limit.unwrap_or(100) > 512
            || self.offset > ROW_LIMIT
        {
            return Err("INVALID_SEARCH: Query exceeds its bounds".into());
        }
        for (value, allowed) in [
            (
                &self.kind,
                &[
                    "", "all", "image", "video", "audio", "document", "archive", "other",
                ][..],
            ),
            (&self.size, &["", "any", "small", "medium", "large"][..]),
            (&self.date, &["", "any", "7d", "30d", "1y"][..]),
            (
                &self.protection,
                &["", "any", "plain", "protected", "locked", "unlocked"][..],
            ),
        ] {
            if !allowed.contains(&value.as_str()) {
                return Err("INVALID_SEARCH: Unknown filter".into());
            }
        }
        Ok(())
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Reply {
    pub index_id: String,
    pub files: Vec<WorkspaceFile>,
    pub total: usize,
    pub next_offset: Option<usize>,
    pub indexed: bool,
    pub complete: bool,
    pub offline: bool,
}
pub(crate) struct Index {
    account: AccountGuard,
    connection: Connection,
    rows: Vec<WorkspaceFile>,
    credential: Option<UnlockSessionId>,
    date_anchor: i64,
    truncated: bool,
}
impl Index {
    #[cfg(feature = "native-e2e")]
    pub(crate) fn build(
        account: AccountGuard,
        rows: Vec<WorkspaceFile>,
        folders: &[i64],
        credential: Option<UnlockSessionId>,
    ) -> Result<Self, String> {
        Self::build_checked(account, rows, folders, credential, || Ok(()))
    }
    fn build_checked(
        account: AccountGuard,
        rows: Vec<WorkspaceFile>,
        folders: &[i64],
        credential: Option<UnlockSessionId>,
        check: impl Fn() -> Result<(), String>,
    ) -> Result<Self, String> {
        account.validate()?;
        check()?;
        #[cfg(feature = "native-e2e")]
        if let Some((started, release)) =
            BUILD_GATE.lock().unwrap_or_else(|e| e.into_inner()).take()
        {
            std::fs::write(started, b"building").map_err(|e| e.to_string())?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !release.is_file() {
                if std::time::Instant::now() > deadline {
                    return Err("Fixture build gate timed out".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            check()?;
        }
        let allowed: HashSet<_> = folders.iter().copied().collect();
        let mut retained = Vec::new();
        let mut bytes = 0usize;
        let mut truncated = false;
        for row in rows
            .into_iter()
            .filter(|r| r.file.folder_id.is_none_or(|id| allowed.contains(&id)))
        {
            let next = bytes.saturating_add(row_bytes(&row));
            if retained.len() == ROW_LIMIT || next > TEXT_LIMIT {
                truncated = true;
                break;
            }
            bytes = next;
            retained.push(row);
        }
        let rows = retained;
        if rows
            .iter()
            .any(|r| r.file.encryption_state == "encrypted_unlocked")
            && credential.is_none()
        {
            return Err("VAULT_LOCKED: Search requires the current credential".into());
        }
        let connection = sqlite::open(":memory:").map_err(|e| e.to_string())?;
        connection
            .execute(concat!(
                "PRAGMA temp_store=MEMORY; ",
                "CREATE VIRTUAL TABLE search_names USING ",
                "fts5(name,folder,tags,tokenize='unicode61 remove_diacritics 2'); BEGIN",
            ))
            .map_err(|e| format!("SEARCH_INDEX_UNAVAILABLE: {e}"))?;
        for (index, row) in rows.iter().enumerate() {
            if index % 128 == 0 {
                account.validate()?;
                check()?;
            }
            let mut insert = connection
                .prepare("INSERT INTO search_names(rowid,name,folder,tags) VALUES(?,?,?,?)")
                .map_err(|e| e.to_string())?;
            insert
                .bind(
                    &[
                        sqlite::Value::Integer(index as i64 + 1),
                        row.file.name.clone().into(),
                        row.folder_name.clone().into(),
                        row.tags.join(" ").into(),
                    ][..],
                )
                .map_err(|e| e.to_string())?;
            insert.next().map_err(|e| e.to_string())?;
        }
        check()?;
        connection.execute("COMMIT").map_err(|e| e.to_string())?;
        account.validate()?;
        Ok(Self {
            account,
            connection,
            rows,
            credential,
            date_anchor: crate::app_time::unix_seconds(),
            truncated,
        })
    }
    pub(crate) fn search(
        &self,
        query: &Query,
        crypto: &CryptoState,
        complete: bool,
        offline: bool,
    ) -> Result<Reply, String> {
        self.account.validate()?;
        query.validate()?;
        if let Some(credential) = self.credential {
            crypto
                .with_current_session(credential, || ())
                .map_err(|_| "VAULT_LOCKED: Reload the search index".to_string())?;
        }
        let result = self.search_inner(query, complete && !self.truncated, offline)?;
        let result = if let Some(credential) = self.credential {
            crypto
                .with_current_session(credential, || result)
                .map_err(|_| "VAULT_LOCKED: Reload the search index".to_string())?
        } else {
            result
        };
        self.account.validate()?;
        Ok(result)
    }
    fn search_inner(&self, query: &Query, complete: bool, offline: bool) -> Result<Reply, String> {
        let text = query.query.trim();
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();
        if text.is_empty() {
            candidates.extend(0..self.rows.len());
        } else {
            let expression = text
                .split_whitespace()
                .map(|word| format!("\"{}\"*", word.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(" AND ");
            let mut matched = self
                .connection
                .prepare(
                    "SELECT rowid FROM search_names WHERE search_names MATCH ? ORDER BY rank,rowid",
                )
                .map_err(|e| e.to_string())?;
            matched
                .bind((1, expression.as_str()))
                .map_err(|e| e.to_string())?;
            while matched.next().map_err(|e| format!("INVALID_SEARCH: {e}"))? == State::Row {
                let index = matched.read::<i64, _>(0).map_err(|e| e.to_string())? as usize - 1;
                seen.insert(index);
                candidates.push(index);
            }
            // Literal substrings cover short words, punctuation and mid-word CJK text.
            let literal = text.chars().count() < 3 || text.chars().any(|c| {
                matches!(c,'\u{3040}'..='\u{30ff}'|'\u{3400}'..='\u{9fff}'|'\u{ac00}'..='\u{d7af}')
                    || (!c.is_alphanumeric() && !c.is_whitespace())
            });
            if literal {
                let needle = text.to_lowercase();
                for (index, row) in self.rows.iter().enumerate() {
                    if row.file.name.to_lowercase().contains(&needle) && seen.insert(index) {
                        candidates.push(index);
                    }
                }
            }
        }
        let filtered: Vec<_> = candidates
            .into_iter()
            .filter(|index| matches(&self.rows[*index], query, self.date_anchor))
            .collect();
        let total = filtered.len();
        let limit = query.limit.unwrap_or(100);
        let files = filtered
            .into_iter()
            .skip(query.offset)
            .take(limit)
            .map(|i| self.rows[i].clone())
            .collect();
        let next_offset =
            (query.offset.saturating_add(limit) < total).then_some(query.offset + limit);
        Ok(Reply {
            index_id: String::new(),
            files,
            total,
            next_offset,
            indexed: true,
            complete,
            offline,
        })
    }
}
pub(crate) fn row_bytes(row: &WorkspaceFile) -> usize {
    serde_json::to_vec(row).map_or(usize::MAX, |v| {
        v.len().saturating_add(std::mem::size_of::<WorkspaceFile>())
    })
}
fn matches(row: &WorkspaceFile, query: &Query, date_anchor: i64) -> bool {
    let file = &row.file;
    if query.folder_key.as_ref().is_some_and(|key| {
        key != &file
            .folder_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "saved".into())
    }) || query
        .collection_id
        .as_ref()
        .is_some_and(|id| !row.collection_ids.contains(id))
        || query.favorites_only && !file.is_favorite
        || !query.tags.iter().all(|tag| {
            row.tags
                .iter()
                .any(|value| value.to_lowercase() == tag.to_lowercase())
        })
    {
        return false;
    }
    let size = file.size;
    if match query.size.as_str() {
        "small" => size >= 10 * 1024 * 1024,
        "medium" => !(10 * 1024 * 1024..100 * 1024 * 1024).contains(&size),
        "large" => size < 100 * 1024 * 1024,
        _ => false,
    } {
        return false;
    }
    if let Some(days) = match query.date.as_str() {
        "7d" => Some(7),
        "30d" => Some(30),
        "1y" => Some(365),
        _ => None,
    } {
        let Some(date) = crate::api_catalog::parse_time(&file.created_at) else {
            return false;
        };
        if date < date_anchor - days * 86400 {
            return false;
        }
    }
    let protected = file.encryption_state != "plain";
    if match query.protection.as_str() {
        "plain" => protected,
        "protected" => !protected,
        "unlocked" => file.encryption_state != "encrypted_unlocked",
        "locked" => !protected || file.encryption_state == "encrypted_unlocked",
        _ => false,
    } {
        return false;
    }
    if query.kind.is_empty() || query.kind == "all" {
        return true;
    }
    let extension = file
        .file_ext
        .as_deref()
        .unwrap_or_else(|| file.name.rsplit('.').next().unwrap_or(""))
        .to_ascii_lowercase();
    let kind = if ["jpg", "jpeg", "png", "gif", "webp", "heic", "avif", "svg"]
        .contains(&extension.as_str())
    {
        "image"
    } else if ["mp4", "mov", "mkv", "webm", "avi", "m4v"].contains(&extension.as_str()) {
        "video"
    } else if ["mp3", "m4a", "wav", "flac", "aac", "ogg", "opus"].contains(&extension.as_str()) {
        "audio"
    } else if [
        "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "rtf", "csv",
    ]
    .contains(&extension.as_str())
    {
        "document"
    } else if ["zip", "rar", "7z", "tar", "gz", "bz2", "xz"].contains(&extension.as_str()) {
        "archive"
    } else {
        "other"
    };
    kind == query.kind
}

#[derive(Default)]
struct MetadataEpoch {
    revision: std::sync::atomic::AtomicU64,
    pending: std::sync::atomic::AtomicUsize,
}
type MetadataEpochs = std::collections::HashMap<std::path::PathBuf, std::sync::Arc<MetadataEpoch>>;
static METADATA_EPOCHS: std::sync::OnceLock<std::sync::Mutex<MetadataEpochs>> =
    std::sync::OnceLock::new();
fn epoch(root: &std::path::Path) -> std::sync::Arc<MetadataEpoch> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    METADATA_EPOCHS
        .get_or_init(|| std::sync::Mutex::new(MetadataEpochs::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(root)
        .or_default()
        .clone()
}
pub(crate) fn metadata_revision(root: &std::path::Path) -> Result<u64, String> {
    let epoch = epoch(root);
    if epoch.pending.load(std::sync::atomic::Ordering::SeqCst) > 0 {
        return Err("SEARCH_CHANGED: Wait for the current metadata update".into());
    }
    Ok(epoch.revision.load(std::sync::atomic::Ordering::SeqCst))
}
pub(crate) struct MetadataMutation(std::sync::Arc<MetadataEpoch>);
impl MetadataMutation {
    pub(crate) fn begin(root: &std::path::Path) -> Self {
        let epoch = epoch(root);
        epoch
            .pending
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        epoch
            .revision
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self(epoch)
    }
}
impl Drop for MetadataMutation {
    fn drop(&mut self) {
        self.0
            .revision
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.0
            .pending
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}
pub(crate) fn relevant_metadata(sql: &str, args: &[sqlite::Value]) -> bool {
    if ["workspace_files", "workspace_tags", "workspace_membership"]
        .iter()
        .any(|table| sql.contains(table))
    {
        return true;
    }
    if !sql.contains("workspace_records") {
        return false;
    }
    args.first().is_some_and(|value| {
        let sqlite::Value::String(kind) = value else {
            return false;
        };
        ["favorite", "pin", "collection", "search", "removal", "scan"].contains(&kind.as_str())
    })
}

pub(crate) struct Snapshot {
    pub rows: Vec<WorkspaceFile>,
    pub folders: Vec<i64>,
    pub credential: Option<UnlockSessionId>,
    pub complete: bool,
    pub offline: bool,
    pub inventory: Vec<InventoryStamp>,
}
pub(crate) trait Source: Send + Sync {
    fn snapshot<'a>(
        &'a self,
        account: &'a AccountGuard,
        folder: Option<&'a str>,
    ) -> futures::future::BoxFuture<'a, Result<Snapshot, String>>;
}
#[derive(Clone, PartialEq, Eq, Hash)]
struct Scope {
    root: std::path::PathBuf,
    owner: i64,
    folder: Option<String>,
}
struct Ready {
    index: Index,
    revision: u64,
    credential: Option<UnlockSessionId>,
    created: std::time::Instant,
    complete: bool,
    offline: bool,
    id: String,
    ticket: crate::file_inventory::RevisionTicket,
    inventory: Vec<InventoryStamp>,
}
type ReadyIndex = std::sync::Arc<std::sync::Mutex<Ready>>;
type ReadyIndexes = std::collections::HashMap<Scope, ReadyIndex>;
static INDEXES: std::sync::OnceLock<std::sync::Mutex<ReadyIndexes>> = std::sync::OnceLock::new();
type BuildLocks = std::collections::HashMap<Scope, std::sync::Weak<tokio::sync::Mutex<()>>>;
static BUILD_LOCKS: std::sync::OnceLock<std::sync::Mutex<BuildLocks>> = std::sync::OnceLock::new();
static VAULT_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static BUILD_CAPACITY: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
    std::sync::OnceLock::new();
#[cfg(feature = "native-e2e")]
static BUILD_GATE: std::sync::Mutex<Option<(std::path::PathBuf, std::path::PathBuf)>> =
    std::sync::Mutex::new(None);
#[cfg(feature = "native-e2e")]
pub(crate) fn install_build_gate(started: std::path::PathBuf, release: std::path::PathBuf) {
    *BUILD_GATE.lock().unwrap_or_else(|e| e.into_inner()) = Some((started, release));
}
#[cfg(feature = "native-e2e")]
pub(crate) fn available_builds() -> usize {
    BUILD_CAPACITY
        .get()
        .map_or(3, |capacity| capacity.available_permits())
}
fn indexes() -> std::sync::MutexGuard<'static, ReadyIndexes> {
    INDEXES
        .get_or_init(|| std::sync::Mutex::new(ReadyIndexes::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}
type InventoryGenerations = std::collections::HashMap<
    (std::path::PathBuf, i64, Option<i64>),
    std::sync::Weak<std::sync::atomic::AtomicU64>,
>;
static INVENTORY_GENERATIONS: std::sync::OnceLock<std::sync::Mutex<InventoryGenerations>> =
    std::sync::OnceLock::new();
pub(crate) struct InventoryPin(std::sync::Arc<std::sync::atomic::AtomicUsize>);
impl InventoryPin {
    pub(crate) fn acquire(
        count: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) -> std::sync::Arc<Self> {
        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::sync::Arc::new(Self(count.clone()))
    }
}
impl Drop for InventoryPin {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}
#[derive(Clone)]
pub(crate) struct InventoryStamp {
    generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    value: u64,
    pin: Option<std::sync::Arc<InventoryPin>>,
    lease: Option<(
        std::sync::Arc<std::sync::Mutex<std::time::Instant>>,
        std::time::Duration,
    )>,
}
impl InventoryStamp {
    pub(crate) fn capture(generation: &std::sync::Arc<std::sync::atomic::AtomicU64>) -> Self {
        Self {
            generation: generation.clone(),
            value: generation.load(std::sync::atomic::Ordering::SeqCst),
            lease: None,
            pin: None,
        }
    }
    pub(crate) fn monitored(
        generation: &std::sync::Arc<std::sync::atomic::AtomicU64>,
        touched: std::sync::Arc<std::sync::Mutex<std::time::Instant>>,
        retention: std::time::Duration,
        pins: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) -> Self {
        let mut stamp = Self::capture(generation);
        stamp.lease = Some((touched, retention));
        stamp.pin = Some(InventoryPin::acquire(pins));
        stamp
    }
    fn touch(&self) {
        if let Some((touched, _)) = &self.lease {
            *touched.lock().unwrap_or_else(|e| e.into_inner()) = std::time::Instant::now();
        }
    }
    fn valid(&self) -> bool {
        self.lease.as_ref().is_none_or(|(touched, retention)| {
            touched.lock().unwrap_or_else(|e| e.into_inner()).elapsed() < *retention
        }) && self.generation.load(std::sync::atomic::Ordering::SeqCst) == self.value
    }
}
pub(crate) fn inventory_generation(
    account: &AccountGuard,
    folder: Option<i64>,
) -> std::sync::Arc<std::sync::atomic::AtomicU64> {
    let key = (
        account
            .root
            .canonicalize()
            .unwrap_or_else(|_| account.root.clone()),
        account.owner,
        folder,
    );
    let mut map = INVENTORY_GENERATIONS
        .get_or_init(|| std::sync::Mutex::new(InventoryGenerations::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    map.retain(|_, value| value.strong_count() > 0);
    if let Some(value) = map.get(&key).and_then(|value| value.upgrade()) {
        value
    } else {
        let value = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        map.insert(key, std::sync::Arc::downgrade(&value));
        value
    }
}
pub(crate) fn inventory_changed(
    account: &AccountGuard,
    folder: Option<i64>,
    generation: &std::sync::Arc<std::sync::atomic::AtomicU64>,
) {
    generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let root = account
        .root
        .canonicalize()
        .unwrap_or_else(|_| account.root.clone());
    let key = folder
        .map(|id| id.to_string())
        .unwrap_or_else(|| "saved".into());
    indexes().retain(|scope, _| {
        scope.root != root
            || scope.owner != account.owner
            || scope.folder.as_ref().is_some_and(|value| value != &key)
    });
}
pub(crate) fn invalidate() {
    VAULT_EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    indexes().clear();
}
pub(crate) async fn search(
    account: AccountGuard,
    crypto: CryptoState,
    query: Query,
    source: &dyn Source,
) -> Result<Reply, String> {
    query.validate()?;
    account.validate()?;
    let scope = Scope {
        root: account.root.canonicalize().map_err(|e| e.to_string())?,
        owner: account.owner,
        folder: query.folder_key.clone(),
    };
    let build_lock = {
        let mut locks = BUILD_LOCKS
            .get_or_init(|| std::sync::Mutex::new(BuildLocks::new()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&scope).and_then(|lock| lock.upgrade()) {
            lock
        } else {
            let lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
            locks.insert(scope.clone(), std::sync::Arc::downgrade(&lock));
            lock
        }
    };
    // Remove old owners before any network wait; account and vault changes also clear on lifecycle hooks.
    indexes().retain(|key, _| key.root == scope.root && key.owner == scope.owner);
    let build = std::sync::Arc::new(build_lock.lock_owned().await);
    let store_root = scope.root.join("workspace").join(account.owner.to_string());
    let revision = metadata_revision(&store_root)?;
    let credential = crypto.current_credential().ok().map(|(id, _)| id);
    let vault_epoch = VAULT_EPOCH.load(std::sync::atomic::Ordering::SeqCst);
    let existing = indexes().get(&scope).cloned();
    let ready = if let Some(ready) = existing.filter(|ready| {
        let ready = ready.lock().unwrap_or_else(|e| e.into_inner());
        ready.index.account.same_session(&account)
            && ready.revision == revision
            && ready.credential == credential
            // Offline snapshots have no monitored remote generations. Retry
            // discovery after a short lease so reconnection becomes visible.
            && (!ready.offline || ready.created.elapsed() < std::time::Duration::from_secs(30))
            && ready.ticket.validate(&account).is_ok()
            && ready.inventory.iter().all(InventoryStamp::valid)
    }) {
        ready
    } else {
        indexes().clear();
        let capacity = BUILD_CAPACITY
            .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(3)))
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| "Search service stopped")?;
        let ticket = crate::file_inventory::RevisionTicket::capture(&account);
        let snapshot = tokio::time::timeout(
            std::time::Duration::from_secs(12 * 60),
            source.snapshot(&account, query.folder_key.as_deref()),
        )
        .await
        .map_err(|_| "SEARCH_INDEX_BUILDING: Search reconciliation timed out")??;
        account.validate()?;
        ticket.validate(&account)?;
        if query.folder_key.as_ref().is_some_and(|key| {
            key != "saved"
                && key
                    .parse::<i64>()
                    .is_ok_and(|id| !snapshot.folders.contains(&id))
        }) {
            return Err("NOT_DRIVE_FOLDER: Search is limited to Telegram Drive folders".into());
        }
        if snapshot.credential != credential
            || VAULT_EPOCH.load(std::sync::atomic::Ordering::SeqCst) != vault_epoch
        {
            return Err("VAULT_LOCKED: Search credential changed".into());
        }
        if !snapshot.inventory.iter().all(InventoryStamp::valid) {
            return Err("SEARCH_CHANGED: Inventory changed during reconciliation".into());
        }
        if metadata_revision(&store_root)? != revision {
            return Err("SEARCH_CHANGED: Local metadata changed".into());
        }
        struct Cancel(std::sync::Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Cancel {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let cancelled = Cancel(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            false,
        )));
        let flag = cancelled.0.clone();
        let owner = account.clone();
        let retained_build = build.clone();
        let build_root = store_root.clone();
        let inventory = snapshot.inventory.clone();
        let index = tokio::task::spawn_blocking(move || {
            let _leases = (retained_build, capacity);
            Index::build_checked(owner, snapshot.rows, &snapshot.folders, credential, || {
                if flag.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err("SEARCH_CANCELLED".into());
                }
                if VAULT_EPOCH.load(std::sync::atomic::Ordering::SeqCst) != vault_epoch {
                    return Err("VAULT_LOCKED: Search credential changed".into());
                }
                if !inventory.iter().all(InventoryStamp::valid) {
                    return Err("SEARCH_CHANGED: Inventory changed".into());
                }
                if metadata_revision(&build_root)? != revision {
                    return Err("SEARCH_CHANGED: Local metadata changed".into());
                }
                Ok(())
            })
        })
        .await
        .map_err(|e| e.to_string())??;
        let ready = std::sync::Arc::new(std::sync::Mutex::new(Ready {
            index,
            revision,
            credential,
            created: std::time::Instant::now(),
            complete: snapshot.complete,
            offline: snapshot.offline,
            id: uuid::Uuid::new_v4().to_string(),
            ticket,
            inventory: snapshot.inventory.clone(),
        }));
        account.validate()?;
        if metadata_revision(&store_root)? != revision {
            return Err("SEARCH_CHANGED: Local metadata changed".into());
        }
        let mut cache = indexes();
        if VAULT_EPOCH.load(std::sync::atomic::Ordering::SeqCst) != vault_epoch {
            return Err("VAULT_LOCKED: Search credential changed".into());
        }
        if cache.len() >= 4 {
            cache.clear();
        }
        cache.retain(|key, _| key.root == scope.root && key.owner == scope.owner);
        if !snapshot.inventory.iter().all(InventoryStamp::valid) {
            return Err("SEARCH_CHANGED: Inventory changed".into());
        }
        cache.insert(scope, ready.clone());
        ready
    };
    let owner = account.clone();
    tokio::task::spawn_blocking(move || {
        let ready = ready.lock().unwrap_or_else(|e| e.into_inner());
        owner.validate()?;
        ready.ticket.validate(&owner)?;
        if !ready.inventory.iter().all(InventoryStamp::valid) {
            return Err("SEARCH_CHANGED: Inventory changed".into());
        }
        for stamp in &ready.inventory {
            stamp.touch();
        }
        if metadata_revision(&store_root)? != ready.revision {
            return Err("SEARCH_CHANGED: Local metadata changed".into());
        }
        if query.offset > 0 && query.index_id.is_none() {
            return Err("SEARCH_CHANGED: Page requires an index generation".into());
        }
        if query.index_id.as_ref().is_some_and(|id| id != &ready.id) {
            return Err("SEARCH_CHANGED: Page belongs to an expired index".into());
        }
        let mut reply = ready
            .index
            .search(&query, &crypto, ready.complete, ready.offline)?;
        reply.index_id = ready.id.clone();
        ready.ticket.validate(&owner)?;
        if metadata_revision(&store_root)? != ready.revision
            || VAULT_EPOCH.load(std::sync::atomic::Ordering::SeqCst) != vault_epoch
            || !ready.inventory.iter().all(InventoryStamp::valid)
        {
            return Err("SEARCH_CHANGED: Search scope changed".into());
        }
        Ok(reply)
    })
    .await
    .map_err(|e| e.to_string())?
}

pub(crate) async fn inventory_summary<T: Send + 'static>(
    account: AccountGuard,
    crypto: CryptoState,
    source: &dyn Source,
    summarize: impl FnOnce(&[WorkspaceFile], bool) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let reply = search(
        account.clone(),
        crypto.clone(),
        Query {
            limit: Some(1),
            ..Default::default()
        },
        source,
    )
    .await?;
    let scope = Scope {
        root: account.root.canonicalize().map_err(|e| e.to_string())?,
        owner: account.owner,
        folder: None,
    };
    let ready = indexes()
        .get(&scope)
        .cloned()
        .ok_or("SEARCH_CHANGED: Index expired")?;
    let epoch = VAULT_EPOCH.load(std::sync::atomic::Ordering::SeqCst);
    let credential = crypto.current_credential().ok().map(|(id, _)| id);
    tokio::task::spawn_blocking(move || {
        let ready = ready.lock().unwrap_or_else(|e| e.into_inner());
        let check = || -> Result<(), String> {
            account.validate()?;
            ready.ticket.validate(&account)?;
            if ready.id != reply.index_id
                || ready.credential != credential
                || crypto.current_credential().ok().map(|(id, _)| id) != credential
                || VAULT_EPOCH.load(std::sync::atomic::Ordering::SeqCst) != epoch
                || !ready.inventory.iter().all(InventoryStamp::valid)
                || metadata_revision(&scope.root.join("workspace").join(account.owner.to_string()))?
                    != ready.revision
            {
                return Err("SEARCH_CHANGED: Insight inventory changed".into());
            }
            Ok(())
        };
        check()?;
        for stamp in &ready.inventory {
            stamp.touch();
        }
        let result = summarize(&ready.index.rows, ready.complete && !ready.offline)?;
        check()?;
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}
