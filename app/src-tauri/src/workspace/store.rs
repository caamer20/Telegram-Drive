use crate::models::FileMetadata;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sqlite::{Connection, State, Value};
use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, String>;

pub fn file_key(folder: Option<i64>, message: i64) -> String {
    format!(
        "{}:{message}",
        folder
            .map(|id| id.to_string())
            .unwrap_or_else(|| "saved".into())
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub color: String,
    pub icon: String,
    #[serde(default)]
    pub cover_key: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedSearch {
    pub id: String,
    pub name: String,
    pub query: String,
    pub filters: serde_json::Value,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub folder_key: Option<String>,
    #[serde(default)]
    pub collection_id: Option<String>,
    #[serde(default)]
    pub favorites_only: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFile {
    #[serde(flatten)]
    pub file: FileMetadata,
    pub key: String,
    pub folder_name: String,
    pub tags: Vec<String>,
    pub collection_ids: Vec<String>,
}
pub(crate) struct SearchOverlay {
    pub favorite: bool,
    pub pinned: bool,
    pub hidden: bool,
    pub tags: Vec<String>,
    pub collections: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub owner_id: String,
    pub files: Vec<WorkspaceFile>,
    pub collections: Vec<Collection>,
    pub searches: Vec<SavedSearch>,
    pub scans: Vec<serde_json::Value>,
}

struct IdleConnection {
    path: PathBuf,
    identity: Option<(u64, u64)>,
    connection: Connection,
}
static IDLE_CONNECTIONS: std::sync::LazyLock<std::sync::Mutex<Vec<IdleConnection>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));
const MAX_IDLE_CONNECTIONS: usize = 32;
const MAX_IDLE_PER_PATH: usize = 2;

#[cfg(unix)]
fn database_identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.dev(), metadata.ino()))
}
#[cfg(windows)]
fn database_identity(path: &Path) -> Option<(u64, u64)> {
    super::account::session_file_identity(path).ok()
}

/// An exclusive connection lease for the entire Store operation. Transaction
/// callbacks use this same connection without re-locking individual methods.
/// Registry locking never encloses SQLite work or an async suspension.
pub struct ConnectionLease {
    connection: Option<Connection>,
    path: PathBuf,
    identity: Option<(u64, u64)>,
}
impl std::ops::Deref for ConnectionLease {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.connection.as_ref().expect("Live connection lease")
    }
}
impl Drop for ConnectionLease {
    fn drop(&mut self) {
        let Some(connection) = self.connection.take() else {
            return;
        };
        // Never return a failed transaction or an abandoned pagination read to
        // another operation. ROLLBACK outside a transaction is harmless.
        let _ = connection.execute("ROLLBACK");
        if self.identity.is_none() || database_identity(&self.path) != self.identity {
            return;
        }
        let mut discarded = Vec::new();
        if let Ok(mut idle) = IDLE_CONNECTIONS.lock() {
            while idle.iter().filter(|entry| entry.path == self.path).count() >= MAX_IDLE_PER_PATH {
                let index = idle
                    .iter()
                    .position(|entry| entry.path == self.path)
                    .unwrap();
                discarded.push(idle.remove(index));
            }
            if idle.len() >= MAX_IDLE_CONNECTIONS {
                discarded.push(idle.remove(0));
            }
            idle.push(IdleConnection {
                path: self.path.clone(),
                identity: self.identity,
                connection,
            });
        }
        // Closing an evicted SQLite handle can checkpoint WAL: do it only
        // after releasing the registry lock.
        drop(discarded);
    }
}

/// A database per verified Telegram account. Additional feature modules use
/// typed records in the same transaction-capable store, not browser storage.
pub struct Store {
    pub db: ConnectionLease,
    pub root: PathBuf,
    pub owner: i64,
    metadata_transaction: std::sync::atomic::AtomicBool,
    metadata_fence: std::sync::Mutex<Option<crate::local_search::MetadataMutation>>,
}
impl Store {
    pub fn open(data: &Path, owner: i64) -> Result<Self> {
        if owner <= 0 {
            return Err("ACCOUNT_REQUIRED".into());
        }
        let root = data.join("workspace").join(owner.to_string());
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let root = root.canonicalize().map_err(|error| error.to_string())?;
        let path = root.join("workspace.db");
        let identity = database_identity(&path);
        let (cached, discarded) = {
            let mut idle = IDLE_CONNECTIONS
                .lock()
                .map_err(|_| "Workspace connection cache unavailable")?;
            let mut discarded = Vec::new();
            let mut index = 0;
            while index < idle.len() {
                if idle[index].path == path && idle[index].identity != identity {
                    discarded.push(idle.remove(index));
                } else {
                    index += 1;
                }
            }
            let cached = idle
                .iter()
                .rposition(|entry| entry.path == path && identity.is_some())
                .map(|index| idle.remove(index).connection);
            (cached, discarded)
        };
        drop(discarded);
        let connection = match cached {
            Some(connection) => connection,
            None => Self::open_connection(&path)?,
        };
        {
            let mut version = connection
                .prepare("PRAGMA user_version")
                .map_err(|error| error.to_string())?;
            version.next().map_err(|error| error.to_string())?;
            if version
                .read::<i64, _>(0)
                .map_err(|error| error.to_string())?
                > 1
            {
                return Err("Workspace database is newer than this application".into());
            }
        }
        let db = ConnectionLease {
            connection: Some(connection),
            identity: database_identity(&path),
            path,
        };
        Ok(Self {
            db,
            root,
            owner,
            metadata_transaction: std::sync::atomic::AtomicBool::new(false),
            metadata_fence: std::sync::Mutex::new(None),
        })
    }
    fn open_connection(path: &Path) -> Result<Connection> {
        let mut db = sqlite::open(path).map_err(|error| error.to_string())?;
        db.set_busy_timeout(5000)
            .map_err(|error| error.to_string())?;
        let version = {
            let mut query = db
                .prepare("PRAGMA user_version")
                .map_err(|error| error.to_string())?;
            query.next().map_err(|error| error.to_string())?;
            query.read::<i64, _>(0).map_err(|error| error.to_string())?
        };
        if version > 1 {
            return Err("Workspace database is newer than this application".into());
        }
        db.execute("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|error| error.to_string())?;
        if version == 0 {
            db.execute(
                "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS workspace_files (
              key TEXT PRIMARY KEY, folder TEXT NOT NULL, metadata TEXT NOT NULL,
              folder_name TEXT NOT NULL, scan TEXT NOT NULL DEFAULT '');
            CREATE TABLE IF NOT EXISTS workspace_records (
              kind TEXT NOT NULL, id TEXT NOT NULL, value TEXT NOT NULL,
              updated INTEGER NOT NULL, PRIMARY KEY(kind,id));
            CREATE TABLE IF NOT EXISTS workspace_membership (
              collection TEXT NOT NULL, file TEXT NOT NULL, PRIMARY KEY(collection,file));
            CREATE TABLE IF NOT EXISTS workspace_tags (
              file TEXT NOT NULL, tag TEXT NOT NULL COLLATE NOCASE, PRIMARY KEY(file,tag));
            PRAGMA user_version=1;",
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(db)
    }
    pub fn execute(&self, sql: &str, args: &[Value]) -> Result<()> {
        let relevant = crate::local_search::relevant_metadata(sql, args);
        let _mutation = if relevant {
            if self
                .metadata_transaction
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                self.metadata_fence
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get_or_insert_with(|| {
                        crate::local_search::MetadataMutation::begin(&self.root)
                    });
                None
            } else {
                Some(crate::local_search::MetadataMutation::begin(&self.root))
            }
        } else {
            None
        };
        let mut statement = self.db.prepare(sql).map_err(|e| e.to_string())?;
        statement.bind(args).map_err(|e| e.to_string())?;
        statement.next().map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn transaction<T>(&self, action: impl FnOnce() -> Result<T>) -> Result<T> {
        self.db
            .execute("BEGIN IMMEDIATE")
            .map_err(|e| e.to_string())?;
        self.metadata_transaction
            .store(true, std::sync::atomic::Ordering::SeqCst);
        struct Finish<'a>(&'a Store);
        impl Drop for Finish<'_> {
            fn drop(&mut self) {
                // Also rolls back panicking callbacks or failed COMMITs before releasing the fence.
                let _ = self.0.db.execute("ROLLBACK");
                self.0
                    .metadata_transaction
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                self.0
                    .metadata_fence
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take();
            }
        }
        let _finish = Finish(self);
        match action() {
            Ok(value) => self
                .db
                .execute("COMMIT")
                .map(|_| value)
                .map_err(|e| e.to_string()),
            Err(error) => {
                let _ = self.db.execute("ROLLBACK");
                Err(error)
            }
        }
    }
    pub fn record<T: DeserializeOwned>(&self, kind: &str, id: &str) -> Result<Option<T>> {
        let mut s = self
            .db
            .prepare("SELECT value FROM workspace_records WHERE kind=? AND id=?")
            .map_err(|e| e.to_string())?;
        s.bind(&[Value::String(kind.into()), Value::String(id.into())][..])
            .map_err(|e| e.to_string())?;
        if s.next().map_err(|e| e.to_string())? == State::Row {
            serde_json::from_str(&s.read::<String, _>(0).map_err(|e| e.to_string())?)
                .map(Some)
                .map_err(|e| e.to_string())
        } else {
            Ok(None)
        }
    }
    pub fn records<T: DeserializeOwned>(&self, kind: &str) -> Result<Vec<T>> {
        let mut s = self
            .db
            .prepare("SELECT value FROM workspace_records WHERE kind=? ORDER BY updated DESC,id")
            .map_err(|e| e.to_string())?;
        s.bind((1, kind)).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        while s.next().map_err(|e| e.to_string())? == State::Row {
            out.push(
                serde_json::from_str(&s.read::<String, _>(0).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?,
            );
        }
        Ok(out)
    }
    pub fn put_record<T: Serialize>(&self, kind: &str, id: &str, value: &T) -> Result<()> {
        if id.is_empty() || id.len() > 256 {
            return Err("Invalid record identifier".into());
        }
        self.execute("INSERT INTO workspace_records VALUES(?,?,?,?) ON CONFLICT(kind,id) DO UPDATE SET value=excluded.value,updated=excluded.updated",
            &[kind.into(), id.into(), serde_json::to_string(value).map_err(|e| e.to_string())?.into(), crate::app_time::unix_millis().into()])
    }
    pub fn remove_record(&self, kind: &str, id: &str) -> Result<()> {
        self.execute(
            "DELETE FROM workspace_records WHERE kind=? AND id=?",
            &[kind.into(), id.into()],
        )
    }
    pub fn remember_files(
        &self,
        files: &[FileMetadata],
        folder_name: &str,
        scan: &str,
    ) -> Result<()> {
        self.transaction(|| {
            for original in files {
                if original.id <= 0 { return Err("Invalid message identifier".into()); }
                let mut file = original.clone();
                // A metadata-protected filename must not become a persistent
                // plaintext tag/gallery index after the vault is locked.
                if file.encryption_state != "plain" {
                    file.name = "Protected file".into();
                    file.mime_type = Some("application/octet-stream".into());
                    file.file_ext = None;
                    file.encryption_state = "encrypted_locked".into();
                }
                self.execute("INSERT INTO workspace_files VALUES(?,?,?,?,?) ON CONFLICT(key) DO UPDATE SET metadata=excluded.metadata,folder_name=excluded.folder_name,scan=excluded.scan",
                    &[file_key(file.folder_id, file.id).into(), file.folder_id.map(|id| id.to_string()).unwrap_or_else(|| "saved".into()).into(), serde_json::to_string(&file).map_err(|e| e.to_string())?.into(), folder_name.into(), scan.into()])?;
            }
            Ok(())
        })
    }
    pub fn complete_scan(&self, folder: Option<i64>, scan: &str) -> Result<()> {
        self.execute(
            "DELETE FROM workspace_files WHERE folder=? AND scan<>?",
            &[
                folder
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "saved".into())
                    .into(),
                scan.into(),
            ],
        )
    }
    /// A user action may refresh metadata without taking the file out of an
    /// in-progress inventory generation. Protected names remain transient.
    pub fn remember_local_file(&self, original: &FileMetadata) -> Result<()> {
        if original.id <= 0 {
            return Err("Invalid message identifier".into());
        }
        let key = file_key(original.folder_id, original.id);
        let mut file = original.clone();
        if let Some(previous) = self.file(&key)? {
            file.is_favorite = previous.file.is_favorite;
            file.is_pinned = previous.file.is_pinned;
            if file.created_at.is_empty() {
                file.created_at = previous.file.created_at;
            }
        }
        if file.encryption_state != "plain" {
            file.name = "Protected file".into();
            file.mime_type = Some("application/octet-stream".into());
            file.file_ext = None;
            file.encryption_state = "encrypted_locked".into();
        }
        let folder = file
            .folder_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "saved".into());
        let folder_name = file
            .folder_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "Saved Messages".into());
        self.execute("INSERT INTO workspace_files VALUES(?,?,?,?,'activity') ON CONFLICT(key) DO UPDATE SET metadata=excluded.metadata",
            &[key.into(), folder.into(), serde_json::to_string(&file).map_err(|e| e.to_string())?.into(), folder_name.into()])
    }
    fn strings(&self, sql: &str, key: &str) -> Result<Vec<String>> {
        let mut s = self.db.prepare(sql).map_err(|e| e.to_string())?;
        s.bind((1, key)).map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        while s.next().map_err(|e| e.to_string())? == State::Row {
            result.push(s.read(0).map_err(|e| e.to_string())?);
        }
        Ok(result)
    }
    fn string_groups(&self, sql: &str) -> Result<std::collections::HashMap<String, Vec<String>>> {
        let mut statement = self.db.prepare(sql).map_err(|e| e.to_string())?;
        let mut groups = std::collections::HashMap::<String, Vec<String>>::new();
        while statement.next().map_err(|e| e.to_string())? == State::Row {
            groups
                .entry(statement.read(0).map_err(|e| e.to_string())?)
                .or_default()
                .push(statement.read(1).map_err(|e| e.to_string())?);
        }
        Ok(groups)
    }
    /// Point lookup keeps each thumbnail request independent of library size.
    pub fn file(&self, key: &str) -> Result<Option<WorkspaceFile>> {
        let mut statement = self
            .db
            .prepare("SELECT metadata,folder_name FROM workspace_files WHERE key=?")
            .map_err(|e| e.to_string())?;
        statement.bind((1, key)).map_err(|e| e.to_string())?;
        if statement.next().map_err(|e| e.to_string())? != State::Row {
            return Ok(None);
        }
        let mut file: FileMetadata =
            serde_json::from_str(&statement.read::<String, _>(0).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        file.is_favorite = self
            .record::<bool>("favorite", key)?
            .unwrap_or(file.is_favorite);
        file.is_pinned = self.record::<bool>("pin", key)?.unwrap_or(file.is_pinned);
        Ok(Some(WorkspaceFile {
            file,
            key: key.into(),
            folder_name: statement.read(1).map_err(|e| e.to_string())?,
            tags: self.strings(
                "SELECT tag FROM workspace_tags WHERE file=? ORDER BY tag",
                key,
            )?,
            collection_ids: self.strings(
                "SELECT collection FROM workspace_membership WHERE file=? ORDER BY collection",
                key,
            )?,
        }))
    }
    pub fn folder_files(&self, folder: Option<i64>) -> Result<Vec<FileMetadata>> {
        let key = folder
            .map(|id| id.to_string())
            .unwrap_or_else(|| "saved".into());
        let mut statement = self
            .db
            .prepare(
                "SELECT f.metadata,r.value,p.value FROM workspace_files f
             LEFT JOIN workspace_records r ON r.kind='favorite' AND r.id=f.key
             LEFT JOIN workspace_records p ON p.kind='pin' AND p.id=f.key
             WHERE f.folder=? ORDER BY json_extract(f.metadata,'$.id') DESC",
            )
            .map_err(|e| e.to_string())?;
        statement
            .bind((1, key.as_str()))
            .map_err(|e| e.to_string())?;
        let mut files = Vec::new();
        while statement.next().map_err(|e| e.to_string())? == State::Row {
            let mut file: FileMetadata =
                serde_json::from_str(&statement.read::<String, _>(0).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            if let Some(value) = statement
                .read::<Option<String>, _>(1)
                .map_err(|e| e.to_string())?
            {
                file.is_favorite = serde_json::from_str(&value).map_err(|e| e.to_string())?;
            }
            if let Some(value) = statement
                .read::<Option<String>, _>(2)
                .map_err(|e| e.to_string())?
            {
                file.is_pinned = serde_json::from_str(&value).map_err(|e| e.to_string())?;
            }
            files.push(file);
        }
        Ok(files)
    }
    pub fn files(&self) -> Result<Vec<WorkspaceFile>> {
        // Three queries per snapshot rather than three queries per file.
        let mut tags =
            self.string_groups("SELECT file,tag FROM workspace_tags ORDER BY file,tag")?;
        let mut memberships = self.string_groups(
            "SELECT file,collection FROM workspace_membership ORDER BY file,collection",
        )?;
        let mut s = self.db.prepare("SELECT f.key,f.metadata,f.folder_name,r.value,p.value FROM workspace_files f LEFT JOIN workspace_records r ON r.kind='favorite' AND r.id=f.key LEFT JOIN workspace_records p ON p.kind='pin' AND p.id=f.key ORDER BY json_extract(f.metadata,'$.created_at') DESC,f.key").map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        while s.next().map_err(|e| e.to_string())? == State::Row {
            let key = s.read::<String, _>(0).map_err(|e| e.to_string())?;
            let mut file: FileMetadata =
                serde_json::from_str(&s.read::<String, _>(1).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            if let Some(value) = s.read::<Option<String>, _>(3).map_err(|e| e.to_string())? {
                file.is_favorite = serde_json::from_str(&value).map_err(|e| e.to_string())?;
            }
            if let Some(value) = s.read::<Option<String>, _>(4).map_err(|e| e.to_string())? {
                file.is_pinned = serde_json::from_str(&value).map_err(|e| e.to_string())?;
            }
            result.push(WorkspaceFile {
                file,
                tags: tags.remove(&key).unwrap_or_default(),
                collection_ids: memberships.remove(&key).unwrap_or_default(),
                key,
                folder_name: s.read(2).map_err(|e| e.to_string())?,
            });
        }
        Ok(result)
    }
    pub(crate) fn search_overlays(
        &self,
        keys: &[String],
    ) -> Result<std::collections::HashMap<String, SearchOverlay>> {
        if keys.len() > 512 {
            return Err("SEARCH_INDEX_LIMIT".into());
        }
        if keys.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let values = vec!["(?)"; keys.len()].join(",");
        let sql=format!("WITH keys(key) AS (VALUES {values}) SELECT key,
            COALESCE((SELECT value FROM workspace_records WHERE kind='favorite' AND id=key),(SELECT CASE WHEN json_extract(metadata,'$.is_favorite') THEN 'true' ELSE 'false' END FROM workspace_files WHERE workspace_files.key=keys.key)),
            COALESCE((SELECT value FROM workspace_records WHERE kind='pin' AND id=key),(SELECT CASE WHEN json_extract(metadata,'$.is_pinned') THEN 'true' ELSE 'false' END FROM workspace_files WHERE workspace_files.key=keys.key)),
            EXISTS(SELECT 1 FROM workspace_records WHERE kind='removal' AND id=key AND json_extract(value,'$.status') IN ('pending','deleting','deleted')),
            (SELECT json_group_array(tag) FROM workspace_tags WHERE file=key),
            (SELECT json_group_array(collection) FROM workspace_membership WHERE file=key) FROM keys");
        let mut query = self.db.prepare(sql).map_err(|e| e.to_string())?;
        query
            .bind(
                keys.iter()
                    .cloned()
                    .map(Value::String)
                    .collect::<Vec<_>>()
                    .as_slice(),
            )
            .map_err(|e| e.to_string())?;
        let mut result = std::collections::HashMap::new();
        let mut bytes = 0usize;
        while query.next().map_err(|e| e.to_string())? == State::Row {
            let key = query.read::<String, _>(0).map_err(|e| e.to_string())?;
            let tags = query.read::<String, _>(4).map_err(|e| e.to_string())?;
            let collections = query.read::<String, _>(5).map_err(|e| e.to_string())?;
            bytes = bytes.saturating_add(key.len() + tags.len() + collections.len());
            if bytes > 32 * 1024 * 1024 {
                return Err("SEARCH_INDEX_LIMIT".into());
            }
            let flag = |index| -> Result<bool> {
                query
                    .read::<Option<String>, _>(index)
                    .map_err(|e| e.to_string())?
                    .map(|value| serde_json::from_str(&value).map_err(|e| e.to_string()))
                    .transpose()
                    .map(|value| value.unwrap_or(false))
            };
            result.insert(
                key,
                SearchOverlay {
                    favorite: flag(1)?,
                    pinned: flag(2)?,
                    hidden: query.read::<i64, _>(3).map_err(|e| e.to_string())? != 0,
                    tags: serde_json::from_str(&tags).map_err(|e| e.to_string())?,
                    collections: serde_json::from_str(&collections).map_err(|e| e.to_string())?,
                },
            );
        }
        Ok(result)
    }
    /// Search reads only verified folders and Saved Messages. Bound raw metadata
    /// before decoding it and return hidden keys for filtering fresh Telegram rows.
    pub(crate) fn search_rows(
        &self,
        folders: &[i64],
        folder: Option<&str>,
    ) -> Result<(Vec<WorkspaceFile>, std::collections::HashSet<String>)> {
        let mut args: Vec<Value> = folders
            .iter()
            .map(|id| Value::String(id.to_string()))
            .collect();
        let mut scope = format!(
            "(f.folder='saved' OR f.folder IN ({}))",
            vec!["?"; args.len()].join(",")
        );
        if let Some(folder) = folder {
            scope.push_str(" AND f.folder=?");
            args.push(folder.into());
        }
        let sql=format!("SELECT f.key,f.metadata,f.folder_name,r.value,p.value,
            (SELECT json_group_array(tag) FROM workspace_tags WHERE file=f.key),
            (SELECT json_group_array(collection) FROM workspace_membership WHERE file=f.key),
            EXISTS (SELECT 1 FROM workspace_records hidden WHERE hidden.kind='removal' AND hidden.id=f.key AND json_extract(hidden.value,'$.status') IN ('pending','deleting','deleted'))
            FROM workspace_files f LEFT JOIN workspace_records r ON r.kind='favorite' AND r.id=f.key LEFT JOIN workspace_records p ON p.kind='pin' AND p.id=f.key WHERE {scope} ORDER BY json_extract(f.metadata,'$.created_at') DESC,f.key LIMIT 100001");
        let mut query = self.db.prepare(sql).map_err(|e| e.to_string())?;
        query.bind(args.as_slice()).map_err(|e| e.to_string())?;
        let mut rows = Vec::new();
        let mut hidden = std::collections::HashSet::new();
        let mut bytes = 0usize;
        let mut count = 0usize;
        while query.next().map_err(|e| e.to_string())? == State::Row {
            count += 1;
            let cells: Vec<String> = (0..7)
                .map(|i| {
                    query
                        .read::<Option<String>, _>(i)
                        .map(|v| v.unwrap_or_default())
                        .map_err(|e| e.to_string())
                })
                .collect::<Result<_>>()?;
            bytes = bytes
                .saturating_add(cells.iter().map(String::len).sum::<usize>())
                .saturating_add(std::mem::size_of::<WorkspaceFile>());
            if count > 100_000 || bytes > 32 * 1024 * 1024 {
                return Err("SEARCH_INDEX_LIMIT: Narrow the search to a folder".into());
            }
            if query.read::<i64, _>(7).map_err(|e| e.to_string())? != 0 {
                hidden.insert(cells[0].clone());
                continue;
            }
            let mut file: FileMetadata =
                serde_json::from_str(&cells[1]).map_err(|e| e.to_string())?;
            if !cells[3].is_empty() {
                file.is_favorite = serde_json::from_str(&cells[3]).map_err(|e| e.to_string())?;
            }
            if !cells[4].is_empty() {
                file.is_pinned = serde_json::from_str(&cells[4]).map_err(|e| e.to_string())?;
            }
            rows.push(WorkspaceFile {
                key: cells[0].clone(),
                file,
                folder_name: cells[2].clone(),
                tags: serde_json::from_str(&cells[5]).map_err(|e| e.to_string())?,
                collection_ids: serde_json::from_str(&cells[6]).map_err(|e| e.to_string())?,
            });
        }
        Ok((rows, hidden))
    }
    /// A bounded page inside the caller's read transaction. Hidden removals are
    /// excluded before LIMIT; tag and membership reads cover only this page.
    pub fn snapshot_page(&self, offset: usize, limit: usize) -> Result<(Snapshot, usize)> {
        const VISIBLE: &str = "NOT EXISTS (SELECT 1 FROM workspace_records hidden WHERE hidden.kind='removal' AND hidden.id=f.key AND json_extract(hidden.value,'$.status') IN ('pending','deleting','deleted'))";
        let total = {
            let mut statement = self
                .db
                .prepare(format!(
                    "SELECT COUNT(*) FROM workspace_files f WHERE {VISIBLE}"
                ))
                .map_err(|error| error.to_string())?;
            statement.next().map_err(|error| error.to_string())?;
            usize::try_from(
                statement
                    .read::<i64, _>(0)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?
        };
        let mut statement = self.db.prepare(format!("SELECT f.key,f.metadata,f.folder_name,r.value,p.value FROM workspace_files f LEFT JOIN workspace_records r ON r.kind='favorite' AND r.id=f.key LEFT JOIN workspace_records p ON p.kind='pin' AND p.id=f.key WHERE {VISIBLE} ORDER BY json_extract(f.metadata,'$.created_at') DESC,f.key LIMIT ? OFFSET ?")).map_err(|error| error.to_string())?;
        statement
            .bind(&[Value::Integer(limit as i64), Value::Integer(offset as i64)][..])
            .map_err(|error| error.to_string())?;
        let mut files = Vec::new();
        while statement.next().map_err(|error| error.to_string())? == State::Row {
            let mut file: FileMetadata = serde_json::from_str(
                &statement
                    .read::<String, _>(1)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            if let Some(value) = statement
                .read::<Option<String>, _>(3)
                .map_err(|error| error.to_string())?
            {
                file.is_favorite =
                    serde_json::from_str(&value).map_err(|error| error.to_string())?;
            }
            if let Some(value) = statement
                .read::<Option<String>, _>(4)
                .map_err(|error| error.to_string())?
            {
                file.is_pinned = serde_json::from_str(&value).map_err(|error| error.to_string())?;
            }
            files.push(WorkspaceFile {
                file,
                key: statement.read(0).map_err(|error| error.to_string())?,
                folder_name: statement.read(2).map_err(|error| error.to_string())?,
                tags: Vec::new(),
                collection_ids: Vec::new(),
            });
        }
        drop(statement);
        if !files.is_empty() {
            let args: Vec<Value> = files.iter().map(|file| file.key.clone().into()).collect();
            let placeholders = vec!["?"; files.len()].join(",");
            for (table, column, membership) in [
                ("workspace_tags", "tag", false),
                ("workspace_membership", "collection", true),
            ] {
                let mut query = self.db.prepare(format!("SELECT file,{column} FROM {table} WHERE file IN ({placeholders}) ORDER BY file,{column}")).map_err(|error| error.to_string())?;
                query
                    .bind(args.as_slice())
                    .map_err(|error| error.to_string())?;
                let mut groups = std::collections::HashMap::<String, Vec<String>>::new();
                while query.next().map_err(|error| error.to_string())? == State::Row {
                    groups
                        .entry(query.read(0).map_err(|error| error.to_string())?)
                        .or_default()
                        .push(query.read(1).map_err(|error| error.to_string())?);
                }
                for file in &mut files {
                    let values = groups.remove(&file.key).unwrap_or_default();
                    if membership {
                        file.collection_ids = values;
                    } else {
                        file.tags = values;
                    }
                }
            }
        }
        Ok((
            Snapshot {
                owner_id: self.owner.to_string(),
                files,
                collections: if offset == 0 {
                    self.records("collection")?
                } else {
                    Vec::new()
                },
                searches: if offset == 0 {
                    self.records("search")?
                } else {
                    Vec::new()
                },
                scans: if offset == 0 {
                    self.records("scan")?
                } else {
                    Vec::new()
                },
            },
            total,
        ))
    }
    pub fn snapshot(&self) -> Result<Snapshot> {
        let removals = self.records::<serde_json::Value>("removal")?;
        let hidden: std::collections::HashSet<_> = removals
            .iter()
            .filter(|value| {
                matches!(
                    value.get("status").and_then(|v| v.as_str()),
                    Some("pending" | "deleting" | "deleted")
                )
            })
            .filter_map(|value| value.get("key").and_then(|v| v.as_str()))
            .collect();
        let files = self
            .files()?
            .into_iter()
            .filter(|file| !hidden.contains(file.key.as_str()))
            .collect();
        Ok(Snapshot {
            owner_id: self.owner.to_string(),
            files,
            collections: self.records("collection")?,
            searches: self.records("search")?,
            scans: self.records("scan")?,
        })
    }
    pub fn save_collection(&self, value: &Collection) -> Result<()> {
        if value.name.trim().is_empty() || value.name.len() > 120 {
            return Err("Collection name must contain 1–120 characters".into());
        }
        if !["blue", "green", "amber", "rose", "violet", "slate"].contains(&value.color.as_str())
            || !["folder", "heart", "plane", "briefcase", "film", "book"]
                .contains(&value.icon.as_str())
        {
            return Err("Choose a collection color and icon".into());
        }
        self.put_record("collection", &value.id, value)
    }
    pub fn remove_collection(&self, id: &str) -> Result<()> {
        self.transaction(|| {
            self.remove_record("collection", id)?;
            self.execute(
                "DELETE FROM workspace_membership WHERE collection=?",
                &[id.into()],
            )
        })
    }
    pub fn assign(&self, keys: &[String], collection: &str, add: bool) -> Result<()> {
        if self
            .record::<Collection>("collection", collection)?
            .is_none()
        {
            return Err("Collection no longer exists".into());
        }
        self.transaction(|| {
            for key in keys {
                self.execute(
                    if add {
                        "INSERT OR IGNORE INTO workspace_membership VALUES(?,?)"
                    } else {
                        "DELETE FROM workspace_membership WHERE collection=? AND file=?"
                    },
                    &[collection.into(), key.clone().into()],
                )?;
            }
            Ok(())
        })
    }
    pub fn tag(&self, keys: &[String], tag: &str, add: bool) -> Result<()> {
        let tag = tag.trim();
        if tag.is_empty() || tag.chars().count() > 40 {
            return Err("Tags must contain 1–40 characters".into());
        }
        self.transaction(|| {
            for key in keys {
                self.execute(
                    if add {
                        "INSERT OR IGNORE INTO workspace_tags VALUES(?,?)"
                    } else {
                        "DELETE FROM workspace_tags WHERE file=? AND tag=?"
                    },
                    &[key.clone().into(), tag.into()],
                )?;
            }
            Ok(())
        })
    }
    pub fn save_search(&self, value: &SavedSearch) -> Result<()> {
        if value.name.trim().is_empty() || value.name.len() > 120 || value.query.len() > 500 {
            return Err("Invalid saved search".into());
        }
        if value
            .folder_key
            .as_ref()
            .is_some_and(|key| key != "saved" && !key.parse::<i64>().is_ok_and(|id| id > 0))
        {
            return Err("Invalid saved-search folder".into());
        }
        if value.tags.len() > 100
            || value
                .tags
                .iter()
                .any(|tag| tag.trim().is_empty() || tag.chars().count() > 40)
        {
            return Err("Invalid saved-search tags".into());
        }
        if value
            .collection_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256)
        {
            return Err("Invalid saved-search collection".into());
        }
        for (field, options) in [
            ("scope", &["folder", "all"][..]),
            (
                "type",
                &[
                    "all", "image", "video", "audio", "document", "archive", "other",
                ][..],
            ),
            ("size", &["any", "small", "medium", "large"][..]),
            ("date", &["any", "7d", "30d", "1y"][..]),
        ] {
            if !value
                .filters
                .get(field)
                .and_then(|v| v.as_str())
                .is_some_and(|v| options.contains(&v))
            {
                return Err(format!("Invalid search {field}"));
            }
        }
        if value.filters.get("protection").is_some_and(|value| {
            !value.as_str().is_some_and(|value| {
                ["any", "plain", "protected", "locked", "unlocked"].contains(&value)
            })
        }) {
            return Err("Invalid search protection".into());
        }
        self.put_record("search", &value.id, value)
    }
}
