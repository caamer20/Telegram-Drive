use grammers_session::{
    storages::SqliteSession,
    types::{PeerId, PeerInfo},
    Session,
};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex, Weak},
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

#[derive(Default)]
struct WorkspaceLifecycle {
    signing_out: AtomicBool,
    generation: AtomicU64,
}

impl WorkspaceLifecycle {
    fn suspend(&self) {
        self.signing_out.store(true, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
}

static LIFECYCLE: WorkspaceLifecycle = WorkspaceLifecycle {
    signing_out: AtomicBool::new(false),
    generation: AtomicU64::new(0),
};

#[cfg(not(any(target_os = "android", target_os = "ios")))]
struct RegisteredSession {
    session: Weak<SqliteSession>,
    identity: Option<(u64, u64)>,
    generation: u64,
    ready: bool,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
static LIVE_SESSIONS: LazyLock<Mutex<HashMap<PathBuf, RegisteredSession>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Synthetic logins replace the whole session file. Serialize that fixture-only
/// swap with fallback identity readers, whose SQLite handles deny replacement
/// on Windows. This does not suspend accounts or close retained live sessions.
#[cfg(all(
    feature = "native-e2e",
    not(any(target_os = "android", target_os = "ios"))
))]
pub(crate) fn with_fixture_session_file_replacement<T>(replace: impl FnOnce() -> T) -> T {
    let _sessions = LIVE_SESSIONS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    replace()
}

/// Opening the library's storage can migrate its schema. Serialize that work
/// against pre-registration readers too; no runner writes until registration.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn open_session(root: &Path) -> sqlite::Result<Arc<SqliteSession>> {
    let mut sessions = LIVE_SESSIONS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let generation = LIFECYCLE.generation.load(Ordering::SeqCst);
    let path = root.join("telegram.session");
    let before = session_file_identity(&path).ok();
    if let Some(session) = sessions.get(root).and_then(|registered| {
        (registered.generation == generation && registered.identity == before && before.is_some())
            .then(|| registered.session.upgrade())
            .flatten()
    }) {
        // Reconnect may still have an older client winding down. Keep its
        // shared SQLite mutex rather than opening another competing writer.
        return Ok(session);
    }
    let session = Arc::new(SqliteSession::open(&path)?);
    let identity = session_file_identity(&path)
        .ok()
        .filter(|after| before.is_none_or(|before| before == *after));
    sessions.retain(|_, registered| registered.session.strong_count() > 0);
    sessions.insert(
        root.into(),
        RegisteredSession {
            session: Arc::downgrade(&session),
            identity,
            generation,
            ready: false,
        },
    );
    Ok(session)
}

#[cfg(all(unix, not(any(target_os = "android", target_os = "ios"))))]
fn session_file_identity(path: &Path) -> std::io::Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path)?;
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(target_os = "windows")]
pub(crate) fn session_file_identity(path: &Path) -> std::io::Result<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let file = std::fs::File::open(path)?;
    let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // The handle stays open for the call, and Windows initializes the output on success.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let information = unsafe { information.assume_init() };
    Ok((
        information.dwVolumeSerialNumber.into(),
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow),
    ))
}

/// Register before starting the runner so ownership reads share the session's
/// mutex with Telegram's writes instead of opening a competing connection.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn register_session(root: &Path, session: &Arc<SqliteSession>) -> Result<(), String> {
    let mut sessions = LIVE_SESSIONS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let path = root.join("telegram.session");
    let registered = sessions
        .get_mut(root)
        .ok_or("ACCOUNT_UNAVAILABLE: The saved account is temporarily unavailable")?;
    let same_session = Weak::ptr_eq(&registered.session, &Arc::downgrade(session));
    let same_file = || {
        registered.identity.is_some() && session_file_identity(&path).ok() == registered.identity
    };
    if !same_session
        || registered.generation != LIFECYCLE.generation.load(Ordering::SeqCst)
        || !same_file()
    {
        return Err("ACCOUNT_CHANGED: The saved session has been replaced".into());
    }
    if registered.ready {
        return Ok(());
    }
    // Before any runner writes, verify the supported layout, including sessions
    // created for a new login that do not have a self peer yet.
    read_session_identity(&path)?;
    if !same_file() || registered.generation != LIFECYCLE.generation.load(Ordering::SeqCst) {
        return Err("ACCOUNT_CHANGED: The saved session has been replaced".into());
    }
    registered.ready = true;
    Ok(())
}

/// A login reply may only activate the exact session that sent its request.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) struct AuthenticationSession {
    root: PathBuf,
    session: Arc<SqliteSession>,
    generation: u64,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
impl AuthenticationSession {
    fn capture_in(
        sessions: &HashMap<PathBuf, RegisteredSession>,
        lifecycle: &WorkspaceLifecycle,
        session: &Arc<SqliteSession>,
    ) -> Result<Self, String> {
        let root = sessions
            .iter()
            .find_map(|(root, registered)| {
                Weak::ptr_eq(&registered.session, &Arc::downgrade(session)).then(|| root.clone())
            })
            .ok_or("ACCOUNT_CHANGED: Restart sign-in for the current session")?;
        let captured = Self {
            root,
            session: session.clone(),
            generation: lifecycle.generation.load(Ordering::SeqCst),
        };
        captured.validate_in(sessions, lifecycle)?;
        Ok(captured)
    }

    pub(crate) fn capture(session: &Arc<SqliteSession>) -> Result<Self, String> {
        let sessions = LIVE_SESSIONS
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        Self::capture_in(&sessions, &LIFECYCLE, session)
    }

    fn validate_in(
        &self,
        sessions: &HashMap<PathBuf, RegisteredSession>,
        lifecycle: &WorkspaceLifecycle,
    ) -> Result<(), String> {
        let valid = sessions.get(&self.root).is_some_and(|registered| {
            registered.ready
                && registered.generation == self.generation
                && lifecycle.generation.load(Ordering::SeqCst) == self.generation
                && Weak::ptr_eq(&registered.session, &Arc::downgrade(&self.session))
                && registered.identity.is_some()
                && session_file_identity(&self.root.join("telegram.session")).ok()
                    == registered.identity
        });
        if valid {
            Ok(())
        } else {
            Err("ACCOUNT_CHANGED: Restart sign-in for the current session".into())
        }
    }

    fn complete_in(
        &self,
        sessions: &HashMap<PathBuf, RegisteredSession>,
        lifecycle: &WorkspaceLifecycle,
        peer: &PeerInfo,
        updates: Option<grammers_session::types::UpdatesState>,
    ) -> Result<(), String> {
        self.validate_in(sessions, lifecycle)?;
        if !matches!(peer, PeerInfo::User { id, is_self: Some(true), .. } if (1..=0xffffffffff).contains(id))
        {
            return Err("ACCOUNT_UNAVAILABLE: Telegram did not confirm a valid account".into());
        }
        // The peer comes from a verified authorization/get_me response, not a
        // browser-supplied owner ID. Persist it before making workspace reads available.
        self.session.cache_peer(peer);
        if let Some(updates) = updates {
            self.session
                .set_update_state(grammers_session::types::UpdateState::All(updates));
        }
        self.validate_in(sessions, lifecycle)?;
        lifecycle.signing_out.store(false, Ordering::SeqCst);
        Ok(())
    }

    pub(crate) fn complete_verified(
        &self,
        peer: &PeerInfo,
        updates: Option<grammers_session::types::UpdatesState>,
    ) -> Result<(), String> {
        let sessions = LIVE_SESSIONS
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.complete_in(&sessions, &LIFECYCLE, peer, updates)
    }
}

tokio::task_local! {
    static OPERATION_ACCOUNT: AccountGuard;
}

pub(crate) fn operation_account() -> Result<Option<AccountGuard>, String> {
    let account = OPERATION_ACCOUNT.try_with(Clone::clone).ok();
    if let Some(account) = &account {
        account.validate()?;
    }
    Ok(account)
}

/// Preserve the original owner and epoch across each awaited sync operation.
pub(crate) async fn with_operation_account<T>(
    account: &AccountGuard,
    operation: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    account.validate()?;
    let result = OPERATION_ACCOUNT.scope(account.clone(), operation).await;
    if result.is_ok() {
        account.validate()?;
    }
    result
}

pub fn suspend() {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let _sessions = LIVE_SESSIONS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    LIFECYCLE.suspend();
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    drop(_sessions);
    crate::local_search::invalidate();
}

pub fn resume() {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let _sessions = LIVE_SESSIONS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    LIFECYCLE.signing_out.store(false, Ordering::SeqCst);
}

/// The saved session's authenticated self peer is available offline. Never
/// infer ownership from a folder number, API ID, or an unscoped legacy cache.
pub fn current_owner(root: &Path) -> Result<i64, String> {
    if LIFECYCLE.signing_out.load(Ordering::SeqCst) {
        return Err("ACCOUNT_CHANGED: Sign in again to open this workspace".into());
    }
    let path = root.join("telegram.session");
    if !path.is_file() {
        return Err("ACCOUNT_REQUIRED: Sign in to open your workspace".into());
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let generation = LIFECYCLE.generation.load(Ordering::SeqCst);
        // Keep fallback reads serialized with registration. Once a runner is
        // active, all checks use its own connection and SQLite mutex.
        let sessions = LIVE_SESSIONS
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let owner = if let Some((registered, session)) = sessions.get(root).and_then(|registered| {
            registered
                .session
                .upgrade()
                .map(|session| (registered, session))
        }) {
            let same_file = || {
                registered.identity.is_some()
                    && session_file_identity(&path).ok() == registered.identity
            };
            if registered.generation != generation || !same_file() {
                return Err("ACCOUNT_CHANGED: The saved session has been replaced".into());
            }
            if !registered.ready {
                return Err(
                    "ACCOUNT_UNAVAILABLE: The saved account is temporarily unavailable".into(),
                );
            }
            let peer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                session.peer(PeerId::self_user())
            }))
            .map_err(|_| "ACCOUNT_UNAVAILABLE: The saved account is temporarily unavailable")?;
            if !same_file() {
                return Err("ACCOUNT_CHANGED: The saved session has been replaced".into());
            }
            match peer {
                Some(PeerInfo::User {
                    id,
                    is_self: Some(true),
                    ..
                }) if (1..=0xffffffffff).contains(&id) => Some(id),
                _ => None,
            }
        } else {
            read_session_identity(&path)?
        };
        if LIFECYCLE.signing_out.load(Ordering::SeqCst)
            || generation != LIFECYCLE.generation.load(Ordering::SeqCst)
        {
            return Err("ACCOUNT_CHANGED: Sign in again to open your workspace".into());
        }
        owner.ok_or_else(|| "ACCOUNT_REQUIRED: Sign in to open your workspace".into())
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let session = SqliteSession::open(path)
            .map_err(|_| "ACCOUNT_UNAVAILABLE: The saved account is temporarily unavailable")?;
        match session.peer(PeerId::self_user()) {
            Some(PeerInfo::User {
                id,
                is_self: Some(true),
                ..
            }) if id > 0 => Ok(id),
            _ => Err("ACCOUNT_REQUIRED: Sign in to open your workspace".into()),
        }
    }
}

#[cfg(feature = "native-e2e")]
static FALLBACK_READS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "native-e2e")]
static ACCOUNT_VALIDATIONS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "native-e2e")]
pub(crate) fn test_account_validation_counts() -> (u64, u64) {
    (
        FALLBACK_READS.load(Ordering::SeqCst),
        ACCOUNT_VALIDATIONS.load(Ordering::SeqCst),
    )
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn read_session_identity(path: &Path) -> Result<Option<i64>, String> {
    #[cfg(feature = "native-e2e")]
    FALLBACK_READS.fetch_add(1, Ordering::SeqCst);
    const UNAVAILABLE: &str = "ACCOUNT_UNAVAILABLE: The saved account is temporarily unavailable";
    let unsupported = || sqlite::Error {
        code: None,
        message: Some("Unsupported session identity layout".into()),
    };
    let read = || -> sqlite::Result<Option<i64>> {
        let mut connection =
            sqlite::Connection::open_with_flags(path, sqlite::OpenFlags::new().with_read_only())?;
        // Telegram updates its peer cache on a separate connection. A short
        // writer lock must not become a missing account or a grammers panic.
        connection.set_busy_timeout(250)?;
        connection.execute("BEGIN")?;
        let mut version = connection.prepare("PRAGMA user_version")?;
        if version.next()? != sqlite::State::Row || version.read::<i64, _>(0)? != 1 {
            return Err(unsupported());
        }
        drop(version);

        // Read only identity metadata from the pinned grammers d07f96f v1
        // schema. Its self-user subtype is 1 (or 3 for a bot), and user peer IDs
        // are positive Bot API IDs in 1..=0xffffffffff. No auth key/hash is read.
        // Reject a future layout, invalid ID, or ambiguous self rows instead of
        // guessing an owner. Fixtures below are written by SqliteSession itself.
        let mut peer = connection
            .prepare("SELECT peer_id, subtype FROM peer_info WHERE subtype & 1 != 0 LIMIT 2")?;
        if peer.next()? != sqlite::State::Row {
            return Ok(None);
        }
        let owner = peer.read::<i64, _>(0)?;
        let subtype = peer.read::<i64, _>(1)?;
        if !(1..=0xffffffffff).contains(&owner)
            || !matches!(subtype, 1 | 3)
            || peer.next()? != sqlite::State::Done
        {
            return Err(unsupported());
        }
        Ok(Some(owner))
    };
    read().map_err(|_| UNAVAILABLE.to_string())
}

#[derive(Clone)]
pub struct AccountGuard {
    pub root: PathBuf,
    pub owner: i64,
    generation: u64,
}

impl AccountGuard {
    /// Compare captured sessions without opening SQLite under another lock.
    pub(crate) fn same_session(&self, other: &Self) -> bool {
        self.root == other.root && self.owner == other.owner && self.generation == other.generation
    }

    pub fn open(root: &Path, expected: Option<&str>) -> Result<Self, String> {
        let generation = LIFECYCLE.generation.load(Ordering::SeqCst);
        let owner = current_owner(root)?;
        if generation != LIFECYCLE.generation.load(Ordering::SeqCst)
            || expected.is_some_and(|id| id != owner.to_string())
        {
            return Err("ACCOUNT_CHANGED: Reopen your workspace for the current account".into());
        }
        Ok(Self {
            root: root.into(),
            owner,
            generation,
        })
    }
    pub fn validate(&self) -> Result<(), String> {
        #[cfg(feature = "native-e2e")]
        ACCOUNT_VALIDATIONS.fetch_add(1, Ordering::SeqCst);
        if self.generation != LIFECYCLE.generation.load(Ordering::SeqCst)
            || current_owner(&self.root)? != self.owner
        {
            Err("ACCOUNT_CHANGED: This operation belongs to a different session".into())
        } else {
            Ok(())
        }
    }

    /// The persisted session and in-memory client can change at different awaits.
    /// Verify the actual connected client once at the start of a remote read.
    pub async fn validate_client(&self, client: &grammers_client::Client) -> Result<(), String> {
        self.validate()?;
        let user = client.get_me().await.map_err(|e| e.to_string())?;
        self.validate_client_owner(user.bare_id())
    }

    fn validate_client_owner(&self, owner: i64) -> Result<(), String> {
        if owner != self.owner {
            return Err("ACCOUNT_CHANGED: Connected client belongs to another account".into());
        }
        self.validate()
    }
}
