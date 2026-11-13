//! A single account/session-scoped folder inventory. History is walked only for
//! bootstrap; subsequent reads catch up above an independent high-water ID and
//! verify known IDs in batches. No Telegram update/session cursor is consumed.
use crate::{
    commands::{utils::resolve_peer, TelegramState},
    workspace::{store::Store, AccountGuard},
};
use futures::{future::BoxFuture, FutureExt};
use grammers_client::{
    types::{Message, Peer},
    Client,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

pub const MAX_FILES: usize = 50_000;
pub const POLL_INTERVAL: Duration = Duration::from_secs(30);
pub const AUDIT_INTERVAL: Duration = Duration::from_secs(240);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const RECONCILIATION_DEADLINE: Duration = Duration::from_secs(9 * 60);
// Bound both retained rows and cross-peer lookup rounding. The latter keeps
// a monitored sweep below an hour at the shared 25-batch/minute ceiling.
const MAX_RETAINED_ROWS: usize = 100_000;
const MAX_AUDIT_BATCHES: usize = 1_000;
const MAX_ENTRIES: usize = 1_024;
const AUDIT_BATCHES_PER_MINUTE: usize = 25;

struct AuditBudget {
    used: Mutex<std::collections::VecDeque<Instant>>,
    window: Duration,
    #[cfg(feature = "native-e2e")]
    clock_offset_ms: std::sync::atomic::AtomicU64,
    #[cfg(feature = "native-e2e")]
    observed: Mutex<std::collections::HashMap<(Option<i64>, i32), AuditObservation>>,
    #[cfg(feature = "native-e2e")]
    clock_enabled: std::sync::atomic::AtomicBool,
    #[cfg(feature = "native-e2e")]
    max_detection_gap: Mutex<Duration>,
    #[cfg(feature = "native-e2e")]
    repeated: std::sync::atomic::AtomicUsize,
}
#[cfg(feature = "native-e2e")]
struct AuditObservation {
    admitted: Instant,
    last: Option<Instant>,
    count: usize,
}
impl Default for AuditBudget {
    fn default() -> Self {
        Self::new(Duration::from_secs(60))
    }
}
impl AuditBudget {
    fn new(window: Duration) -> Self {
        Self {
            used: Mutex::new(Default::default()),
            window,
            #[cfg(feature = "native-e2e")]
            clock_offset_ms: std::sync::atomic::AtomicU64::new(0),
            #[cfg(feature = "native-e2e")]
            observed: Mutex::new(Default::default()),
            #[cfg(feature = "native-e2e")]
            clock_enabled: std::sync::atomic::AtomicBool::new(false),
            #[cfg(feature = "native-e2e")]
            max_detection_gap: Mutex::new(Duration::ZERO),
            #[cfg(feature = "native-e2e")]
            repeated: std::sync::atomic::AtomicUsize::new(0),
        }
    }
    fn now(&self) -> Instant {
        let now = Instant::now();
        #[cfg(feature = "native-e2e")]
        let now = now
            + Duration::from_millis(
                self.clock_offset_ms
                    .load(std::sync::atomic::Ordering::SeqCst),
            );
        now
    }
    fn age(&self, at: Instant) -> Duration {
        self.now().saturating_duration_since(at)
    }
    #[cfg(feature = "native-e2e")]
    fn admit(&self, folder: Option<i64>, ids: impl Iterator<Item = i32>) {
        if !self.clock_enabled.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let now = self.now();
        let mut seen = self.observed.lock().unwrap_or_else(|e| e.into_inner());
        for id in ids {
            seen.entry((folder, id)).or_insert(AuditObservation {
                admitted: now,
                last: None,
                count: 0,
            });
        }
    }
    #[cfg(feature = "native-e2e")]
    fn observed(&self, folder: Option<i64>, ids: &[i32]) {
        if !self.clock_enabled.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let now = self.now();
        let mut seen = self.observed.lock().unwrap_or_else(|e| e.into_inner());
        let mut max_gap = self
            .max_detection_gap
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for id in ids {
            let observation = seen.entry((folder, *id)).or_insert(AuditObservation {
                admitted: now,
                last: None,
                count: 0,
            });
            *max_gap = (*max_gap).max(
                now.saturating_duration_since(observation.last.unwrap_or(observation.admitted)),
            );
            if observation.last.is_some() {
                self.repeated
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            observation.last = Some(now);
            observation.count += 1;
        }
    }
    fn available(&self) -> usize {
        let mut used = self.used.lock().unwrap_or_else(|e| e.into_inner());
        while used.front().is_some_and(|at| self.age(*at) >= self.window) {
            used.pop_front();
        }
        AUDIT_BATCHES_PER_MINUTE.saturating_sub(used.len())
    }
    fn reserve(&self) -> bool {
        let mut used = self.used.lock().unwrap_or_else(|e| e.into_inner());
        while used.front().is_some_and(|at| self.age(*at) >= self.window) {
            used.pop_front();
        }
        if used.len() == AUDIT_BATCHES_PER_MINUTE {
            return false;
        }
        used.push_back(self.now());
        true
    }
}
const RETAIN_FOR: Duration = Duration::from_secs(600);

pub trait Row: Clone + Send + Sync {
    fn id(&self) -> i32;
    fn same(&self, other: &Self) -> bool;
}

impl Row for Message {
    fn id(&self) -> i32 {
        self.id()
    }
    fn same(&self, other: &Self) -> bool {
        same_listing_message(&self.raw, &other.raw)
    }
}
pub(crate) fn same_listing_message(
    first: &grammers_tl_types::enums::Message,
    second: &grammers_tl_types::enums::Message,
) -> bool {
    use grammers_tl_types::enums::{Document, Message as RawMessage, MessageMedia};
    let (RawMessage::Message(first), RawMessage::Message(second)) = (first, second) else {
        return first == second;
    };
    if first.id != second.id
        || first.date != second.date
        || first.message != second.message
        || first.noforwards != second.noforwards
    {
        return false;
    }
    match (&first.media, &second.media) {
        (Some(MessageMedia::Document(first)), Some(MessageMedia::Document(second))) => {
            match (&first.document, &second.document) {
                (Some(Document::Document(first)), Some(Document::Document(second))) => {
                    first.id == second.id
                        && first.size == second.size
                        && first.mime_type == second.mime_type
                        && first.attributes == second.attributes
                }
                (first, second) => first == second,
            }
        }
        (Some(MessageMedia::Photo(first)), Some(MessageMedia::Photo(second))) => {
            first.photo.as_ref().map(|photo| photo.id())
                == second.photo.as_ref().map(|photo| photo.id())
                && grammers_client::types::Photo::from_raw_media(first.clone()).size()
                    == grammers_client::types::Photo::from_raw_media(second.clone()).size()
        }
        (None, None) => true,
        (Some(first), Some(second)) => {
            std::mem::discriminant(first) == std::mem::discriminant(second)
        }
        _ => false,
    }
}

/// History includes non-file messages so deletion of the newest file never
/// resets the cursor. `None` means an observed message is not a supported file.
pub trait Source<R: Row>: Sync {
    fn history(&self, after: i32) -> BoxFuture<'_, Result<(i32, Vec<R>), String>>;
    fn lookup<'a>(&'a self, ids: &'a [i32]) -> BoxFuture<'a, Result<Vec<Option<R>>, String>>;
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Cursor {
    version: u8,
    highwater: i32,
    ids: Vec<i32>,
}

#[derive(Clone, Copy)]
pub struct Timing {
    pub poll: Duration,
    pub audit: Duration,
    pub reconciliation: Duration,
    pub retention: Duration,
    pub audit_window: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            poll: POLL_INTERVAL,
            audit: AUDIT_INTERVAL,
            reconciliation: RECONCILIATION_DEADLINE,
            retention: RETAIN_FOR,
            audit_window: Duration::from_secs(60),
        }
    }
}

pub struct Inventory<R: Row> {
    account: AccountGuard,
    folder: Option<i64>,
    rows: BTreeMap<i32, Arc<R>>,
    snapshot: Arc<Vec<Arc<R>>>,
    complete: bool,
    audit_after: i32,
    budget: Arc<AuditBudget>,
    quota: usize,
    dirty: u64,
    cursor: Option<Cursor>,
    last_poll: Option<Instant>,
    last_audit: Option<Instant>,
    revision: u64,
    serial: u64,
    generation: Arc<std::sync::atomic::AtomicU64>,
    timing: Timing,
}

impl<R: Row> Inventory<R> {
    pub fn new(account: AccountGuard, folder: Option<i64>) -> Self {
        let generation = crate::local_search::inventory_generation(&account, folder);
        Self {
            account,
            folder,
            generation,
            rows: BTreeMap::new(),
            snapshot: Arc::new(Vec::new()),
            complete: true,
            audit_after: 0,
            budget: Arc::new(AuditBudget::default()),
            quota: 1,
            dirty: 0,
            cursor: None,
            last_poll: None,
            last_audit: None,
            revision: 0,
            serial: 0,
            timing: Timing::default(),
        }
    }
    pub fn same_session(&self, account: &AccountGuard) -> bool {
        self.account.same_session(account)
    }
    pub fn expire_poll(&mut self) {
        self.last_poll = None;
    }
    pub fn snapshot(&self) -> Arc<Vec<Arc<R>>> {
        self.snapshot.clone()
    }
    pub async fn read(
        &mut self,
        source: &impl Source<R>,
        force_audit: bool,
    ) -> Result<Arc<Vec<Arc<R>>>, String> {
        self.account.validate()?;
        let observed_revision = revision(self.account.owner);
        DIRTY
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(dirty_key(&self.account, self.folder))
            .or_default();
        let mutation = dirty_state(&self.account, self.folder);
        let dirty = self.dirty != mutation.0;
        let audit = force_audit
            || (self.quota > 0
                && self
                    .last_audit
                    .is_some_and(|at| self.budget.age(at) >= self.timing.audit));
        if !audit
            && !dirty
            && self
                .last_poll
                .is_some_and(|at| self.budget.age(at) < self.timing.poll)
        {
            self.revision = observed_revision;
            return Ok(self.snapshot());
        }
        let started = self.budget.now();
        let cold = self.cursor.is_none();
        if cold {
            let account = self.account.clone();
            tokio::task::spawn_blocking(move || {
                account.validate()?;
                Store::open(&account.root, account.owner)?;
                account.validate()
            })
            .await
            .map_err(|e| e.to_string())??;
        }
        let after = self.cursor.as_ref().map_or(0, |cursor| cursor.highwater);
        let (observed, additions) =
            tokio::time::timeout(self.timing.reconciliation, source.history(after))
                .await
                .map_err(|_| "INVENTORY_STALE: Folder reconciliation timed out")?
                .map_err(|e| format!("INVENTORY_STALE: {e}"))?;
        if observed < after {
            return Err("INVENTORY_INVALID: Regressed remote cursor".into());
        }
        let mut rows = self.rows.clone(); // Arc references, never deep Message copies.
        let mut complete = if cold { true } else { self.complete };
        if additions.len() > MAX_FILES {
            complete = false;
        }
        for row in additions {
            rows.insert(row.id(), Arc::new(row));
        }
        let mut audit_after = self.audit_after;
        let mut sweep_finished = false;
        if !cold && dirty && !mutation.1.is_empty() {
            let verified = self.verify(source, &mutation.1).await?;
            for id in &mutation.1 {
                rows.remove(id);
            }
            rows.extend(verified);
        }
        if !cold && audit {
            let ids: Vec<_> = rows
                .range((
                    std::ops::Bound::Excluded(audit_after),
                    std::ops::Bound::Unbounded,
                ))
                .map(|(id, _)| *id)
                .collect();
            for (processed, batch) in ids.chunks(100).enumerate() {
                if !force_audit && (processed >= self.quota || !self.budget.reserve()) {
                    break;
                }
                let verified = self.verify(source, batch).await?;
                for id in batch {
                    rows.remove(id);
                }
                rows.extend(verified);
                audit_after = *batch.last().unwrap();
            }
            sweep_finished = ids.last().is_none_or(|last| audit_after >= *last);
            if sweep_finished {
                audit_after = 0;
            }
        }
        if rows.len() > MAX_FILES {
            complete = false;
            while rows.len() > MAX_FILES {
                rows.pop_first();
            }
        }
        self.account.validate()?;
        if revision(self.account.owner) != observed_revision
            || dirty_state(&self.account, self.folder).0 != mutation.0
        {
            return Err("INVENTORY_CHANGED: Retry the folder listing".into());
        }
        let cursor = Cursor {
            version: 1,
            highwater: observed,
            ids: rows.keys().copied().collect(),
        };
        let ticket = RevisionTicket::capture(&self.account);
        let account = self.account.clone();
        let folder = self.folder;
        let saved = cursor.clone();
        tokio::task::spawn_blocking(move || {
            ticket.with(&account, || {
                let store = Store::open(&account.root, account.owner)?;
                let key = folder
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "saved".into());
                if store.record::<Cursor>("file-inventory-v1", &key)?.as_ref() != Some(&saved) {
                    store.put_record("file-inventory-v1", &key, &saved)?;
                }
                account.validate()
            })
        })
        .await
        .map_err(|e| e.to_string())??;
        self.account.validate()?;
        if revision(self.account.owner) != observed_revision {
            return Err("INVENTORY_CHANGED: Retry the folder listing".into());
        }
        if self.complete != complete
            || self.rows.len() != rows.len()
            || self
                .rows
                .iter()
                .any(|(id, row)| rows.get(id).is_none_or(|other| !row.same(other)))
        {
            self.serial = self.serial.wrapping_add(1);
            crate::local_search::inventory_changed(&self.account, self.folder, &self.generation);
        }
        self.rows = rows;
        #[cfg(feature = "native-e2e")]
        self.budget.admit(self.folder, self.rows.keys().copied());
        // Refresh transport access references even when visible fields match.
        self.snapshot = Arc::new(self.rows.values().rev().cloned().collect());
        self.cursor = Some(cursor);
        self.complete = complete;
        self.audit_after = audit_after;
        self.revision = observed_revision;
        self.dirty = mutation.0;
        if let Some(state) = DIRTY
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&dirty_key(&self.account, self.folder))
        {
            if state.generation == mutation.0 {
                state.ids.clear();
            }
        }
        self.last_poll = Some(started);
        if cold || sweep_finished {
            self.last_audit = Some(started);
        }
        Ok(self.snapshot())
    }
    async fn verify(
        &self,
        source: &impl Source<R>,
        ids: &[i32],
    ) -> Result<BTreeMap<i32, Arc<R>>, String> {
        let mut rows = BTreeMap::new();
        for batch in ids.chunks(100) {
            self.account.validate()?;
            let values = source.lookup(batch).await?;
            self.account.validate()?;
            if values.len() != batch.len() {
                return Err("INVENTORY_INVALID: Incomplete verification batch".into());
            }
            for (&id, value) in batch.iter().zip(values) {
                if let Some(row) = value {
                    if row.id() != id {
                        return Err("INVENTORY_INVALID: Wrong message in verification".into());
                    }
                    rows.insert(id, Arc::new(row));
                }
            }
            #[cfg(feature = "native-e2e")]
            self.budget.observed(self.folder, batch);
        }
        Ok(rows)
    }
}
static REVISIONS: LazyLock<Mutex<HashMap<i64, Arc<Mutex<u64>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
fn revision_scope(owner: i64) -> Arc<Mutex<u64>> {
    REVISIONS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .entry(owner)
        .or_insert_with(|| Arc::new(Mutex::new(0)))
        .clone()
}

fn revision(owner: i64) -> u64 {
    *revision_scope(owner)
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

#[derive(Default, Clone)]
struct Dirty {
    generation: u64,
    ids: Vec<i32>,
}
static DIRTY: LazyLock<Mutex<HashMap<Key, Dirty>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
fn dirty_key(account: &AccountGuard, folder: Option<i64>) -> (PathBuf, i64, Option<i64>) {
    (
        account
            .root
            .canonicalize()
            .unwrap_or_else(|_| account.root.clone()),
        account.owner,
        folder,
    )
}
fn dirty_state(account: &AccountGuard, folder: Option<i64>) -> (u64, Vec<i32>) {
    let map = DIRTY.lock().unwrap_or_else(|e| e.into_inner());
    map.get(&dirty_key(account, folder))
        .map_or((0, Vec::new()), |dirty| {
            (dirty.generation, dirty.ids.clone())
        })
}
/// Called only after a verified upload/mutation. New messages need catch-up;
/// existing IDs need targeted verification, never an unrelated folder audit.
pub fn changed(account: &AccountGuard, folder: Option<i64>, ids: &[i32]) -> Result<(), String> {
    account.validate()?;
    {
        let revision = revision_scope(account.owner);
        let mut value = revision.lock().unwrap_or_else(|e| e.into_inner());
        *value = value.wrapping_add(1);
    }
    let mut map = DIRTY.lock().unwrap_or_else(|e| e.into_inner());
    let dirty = map.entry(dirty_key(account, folder)).or_default();
    dirty.generation = dirty.generation.wrapping_add(1);
    dirty.ids.extend_from_slice(ids);
    dirty.ids.sort_unstable();
    dirty.ids.dedup();
    if dirty.ids.len() > MAX_FILES {
        dirty.ids.drain(..dirty.ids.len() - MAX_FILES);
    }
    drop(map);
    let generation = crate::local_search::inventory_generation(account, folder);
    crate::local_search::inventory_changed(account, folder, &generation);
    Ok(())
}

pub fn invalidate(owner: i64) {
    let scope = revision_scope(owner);
    let mut value = scope.lock().unwrap_or_else(|error| error.into_inner());
    *value = value.wrapping_add(1);
    drop(value);
    let mut dirty = DIRTY.lock().unwrap_or_else(|e| e.into_inner());
    for ((_, account_owner, _), state) in dirty.iter_mut() {
        if *account_owner == owner {
            state.generation = state.generation.wrapping_add(1);
        }
    }
}
#[derive(Clone)]
pub struct RevisionTicket {
    owner: i64,
    value: u64,
    scope: Arc<Mutex<u64>>,
}

impl RevisionTicket {
    pub fn capture(account: &AccountGuard) -> Self {
        Self {
            owner: account.owner,
            value: revision(account.owner),
            scope: revision_scope(account.owner),
        }
    }
    pub fn with<T>(
        &self,
        account: &AccountGuard,
        operation: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let revision = self
            .scope
            .lock()
            .map_err(|_| "Inventory revision unavailable")?;
        if account.owner != self.owner || *revision != self.value {
            return Err("INVENTORY_CHANGED: Retry the folder listing".into());
        }
        account.validate()?;
        operation()
    }
    pub fn validate(&self, account: &AccountGuard) -> Result<(), String> {
        self.with(account, || Ok(()))
    }
}
/// Serialize local metadata publication with generation invalidation. The
/// account revision lock is always acquired before either SQLite transaction.
pub fn mutate<T>(
    account: &AccountGuard,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let scope = revision_scope(account.owner);
    let mut revision = scope.lock().map_err(|_| "Inventory revision unavailable")?;
    account.validate()?;
    *revision = revision.wrapping_add(1);
    let result = operation();
    *revision = revision.wrapping_add(1);
    result
}

/// Captured listing scope used by persistence and its later UI publication.
#[derive(Clone)]
pub struct Publication {
    pub account: AccountGuard,
    pub ticket: RevisionTicket,
    pub credential: Option<crate::crypto::state::UnlockSessionId>,
}

impl Publication {
    pub fn persist<T>(&self, operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        self.ticket.with(&self.account, operation)
    }
    pub fn output<T>(
        &self,
        crypto: &crate::crypto::state::CryptoState,
        files: &[crate::models::FileMetadata],
        operation: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.ticket.with(&self.account, || {
            if files
                .iter()
                .any(|file| file.encryption_state == "encrypted_unlocked")
            {
                let credential = self
                    .credential
                    .ok_or("VAULT_LOCKED: Reload the protected listing")?;
                crypto
                    .with_current_session(credential, operation)
                    .map_err(|_| "VAULT_LOCKED: Reload the protected listing".to_string())?
            } else {
                operation()
            }
        })
    }
}
#[derive(Clone)]
pub struct GuardedListing<R: Row> {
    pub rows: Arc<Vec<Arc<R>>>,
    pub complete: bool,
    pub cached: bool,
    pub ticket: RevisionTicket,
    pub serial: u64,
    pub(crate) generation: crate::local_search::InventoryStamp,
}

#[derive(Clone)]
struct TelegramSource {
    account: AccountGuard,
    client: Client,
    peer: Peer,
}

fn supported(message: &Message) -> bool {
    matches!(
        message.media(),
        Some(grammers_client::types::Media::Document(_) | grammers_client::types::Media::Photo(_))
    )
}

impl Source<Message> for TelegramSource {
    fn history(&self, after: i32) -> BoxFuture<'_, Result<(i32, Vec<Message>), String>> {
        Box::pin(async move {
            let mut messages = self.client.iter_messages(&self.peer);
            let mut rows = Vec::new();
            let mut highwater = after;
            let mut previous = None;
            let mut seen = 0usize;
            while let Some(message) = messages.next().await.map_err(|error| error.to_string())? {
                self.account.validate()?;
                let id = message.id();
                if id <= after {
                    break;
                }
                if previous.is_some_and(|previous| id >= previous) {
                    return Err("INVENTORY_INCOMPLETE: History did not advance".into());
                }
                previous = Some(id);
                highwater = highwater.max(id);
                seen += 1;
                if seen > 1_000_000 {
                    return Err("INVENTORY_INCOMPLETE: History safety limit reached".into());
                }
                if supported(&message) {
                    rows.push(message);
                    if rows.len() > MAX_FILES {
                        // One extra row signals truncation without walking older history.
                        break;
                    }
                }
            }
            Ok((highwater, rows))
        })
    }
    fn lookup<'a>(&'a self, ids: &'a [i32]) -> BoxFuture<'a, Result<Vec<Option<Message>>, String>> {
        Box::pin(async move {
            self.account.validate()?;
            let values = self
                .client
                .get_messages_by_id(&self.peer, ids)
                .await
                .map_err(|error| error.to_string())?;
            self.account.validate()?;
            Ok(values
                .into_iter()
                .map(|value| value.filter(supported))
                .collect())
        })
    }
}

struct Job<R: Row> {
    forced_poll: bool,
    result: Mutex<Option<Result<GuardedListing<R>, String>>>,
    notify: tokio::sync::Notify,
}

impl<R: Row> Job<R> {
    fn complete(&self, result: Result<GuardedListing<R>, String>) {
        self.result
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_or_insert(result);
        self.notify.notify_waiters();
    }
    async fn wait(&self) -> Result<GuardedListing<R>, String> {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let result = self
                .result
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            if let Some(result) = result {
                return result;
            }
            notified.await;
        }
    }
}

struct Completion<R: Row>(Arc<Job<R>>);
impl<R: Row> Drop for Completion<R> {
    fn drop(&mut self) {
        self.0
            .complete(Err("INVENTORY_STALE: Reconciliation was canceled".into()));
    }
}

struct Entry<R: Row> {
    account: AccountGuard,
    folder: Option<i64>,
    inventory: tokio::sync::Mutex<Inventory<R>>,
    touched: Arc<Mutex<Instant>>,
    pending: Mutex<Option<Arc<Job<R>>>>,
    notified: std::sync::atomic::AtomicU64,
    row_count: std::sync::atomic::AtomicUsize,
    audit_quota: std::sync::atomic::AtomicUsize,
    audited: Mutex<Option<Instant>>,
    failed_until: Mutex<Option<Instant>>,
    pins: Arc<std::sync::atomic::AtomicUsize>,
    stale: std::sync::atomic::AtomicBool,
}
type Key = (PathBuf, i64, Option<i64>);
pub struct InventoryCache<R: Row> {
    entries: Mutex<HashMap<Key, Arc<Entry<R>>>>,
    timing: Timing,
    budget: Arc<AuditBudget>,
    audit_round: Mutex<Instant>,
    rotation: std::sync::atomic::AtomicUsize,
    capacity: Arc<tokio::sync::Semaphore>,
}

impl<R: Row> Default for InventoryCache<R> {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            budget: Arc::new(AuditBudget::default()),
            audit_round: Mutex::new(Instant::now()),
            rotation: std::sync::atomic::AtomicUsize::new(0),
            capacity: Arc::new(tokio::sync::Semaphore::new(2)),
            timing: Timing::default(),
        }
    }
}

impl<R: Row + 'static> InventoryCache<R> {
    #[cfg(feature = "native-e2e")]
    pub fn with_timing(timing: Timing) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            budget: Arc::new(AuditBudget::new(timing.audit_window)),
            audit_round: Mutex::new(Instant::now()),
            rotation: std::sync::atomic::AtomicUsize::new(0),
            capacity: Arc::new(tokio::sync::Semaphore::new(2)),
            timing,
        }
    }
    #[cfg(feature = "native-e2e")]
    pub fn advance_test_clock(&self, duration: Duration) {
        self.budget
            .clock_enabled
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.budget.clock_offset_ms.fetch_add(
            duration.as_millis() as u64,
            std::sync::atomic::Ordering::SeqCst,
        );
    }
    #[cfg(feature = "native-e2e")]
    pub fn test_detection_stats(
        &self,
        original_folder_max: i64,
    ) -> (usize, usize, Duration, usize, usize) {
        let seen = self
            .budget
            .observed
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let now = self.budget.now();
        let max_gap = seen.values().fold(
            *self
                .budget
                .max_detection_gap
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
            |gap, row| gap.max(now.saturating_duration_since(row.last.unwrap_or(row.admitted))),
        );
        (
            seen.len(),
            self.budget
                .repeated
                .load(std::sync::atomic::Ordering::Relaxed),
            max_gap,
            seen.values().map(|row| row.count).min().unwrap_or(0),
            seen.iter()
                .filter(|((folder, _), _)| folder.is_some_and(|id| id <= original_folder_max))
                .map(|(_, row)| row.count)
                .min()
                .unwrap_or(0),
        )
    }
    fn key(account: &AccountGuard, folder: Option<i64>) -> Key {
        (
            account
                .root
                .canonicalize()
                .unwrap_or_else(|_| account.root.clone()),
            account.owner,
            folder,
        )
    }
    fn entry(
        &self,
        account: &AccountGuard,
        folder: Option<i64>,
        consumer: bool,
    ) -> Result<Arc<Entry<R>>, String> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        entries.retain(|_, entry| {
            Arc::strong_count(entry) > 1
                || entry
                    .touched
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .elapsed()
                    < self.timing.retention
        });
        let key = Self::key(account, folder);
        if let Some(entry) = entries
            .get(&key)
            .filter(|entry| entry.account.same_session(account))
        {
            if consumer {
                *entry
                    .touched
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Instant::now();
            }
            return Ok(entry.clone());
        }
        if entries.len() >= MAX_ENTRIES {
            let oldest = entries
                .iter()
                .filter(|(_, entry)| {
                    Arc::strong_count(entry) == 1
                        && entry.pins.load(std::sync::atomic::Ordering::SeqCst) == 0
                })
                .min_by_key(|(_, entry)| {
                    *entry
                        .touched
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                })
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                entries.remove(&oldest);
            } else {
                return Err("INVENTORY_BUSY: Too many concurrent folder listings".into());
            }
        }
        let mut inventory = Inventory::new(account.clone(), folder);
        inventory.timing = self.timing;
        inventory.budget = self.budget.clone();
        let entry = Arc::new(Entry {
            account: account.clone(),
            folder,
            inventory: tokio::sync::Mutex::new(inventory),
            touched: Arc::new(Mutex::new(Instant::now())),
            pending: Mutex::new(None),
            notified: std::sync::atomic::AtomicU64::new(u64::MAX),
            row_count: std::sync::atomic::AtomicUsize::new(0),
            audit_quota: std::sync::atomic::AtomicUsize::new(1),
            audited: Mutex::new(None),
            failed_until: Mutex::new(None),
            pins: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            stale: std::sync::atomic::AtomicBool::new(false),
        });
        entries.insert(key, entry.clone());
        Ok(entry)
    }
    fn recent_accounts(&self) -> Vec<AccountGuard> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let mut accounts = HashMap::new();
        for entry in entries.values() {
            if entry.account.validate().is_ok()
                && entry
                    .touched
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .elapsed()
                    < self.timing.retention
            {
                accounts
                    .entry((entry.account.root.clone(), entry.account.owner))
                    .or_insert_with(|| entry.account.clone());
            }
        }
        accounts.into_values().collect()
    }
    pub fn pending(&self, account: &AccountGuard, folder: Option<i64>) -> Result<bool, String> {
        account.validate()?;
        let entry = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&Self::key(account, folder))
            .filter(|entry| entry.account.same_session(account))
            .cloned();
        Ok(entry.is_some_and(|entry| {
            entry
                .pending
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .is_some_and(|job| {
                    job.result
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .is_none()
                })
        }))
    }
    #[cfg(feature = "native-e2e")]
    pub fn test_last_consumer_touch(
        &self,
        account: &AccountGuard,
        folder: Option<i64>,
    ) -> Option<Instant> {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&Self::key(account, folder))
            .filter(|entry| entry.account.same_session(account))
            .map(|entry| {
                *entry
                    .touched
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
            })
    }
    pub fn touch(&self, account: &AccountGuard, folder: Option<i64>) -> Result<bool, String> {
        account.validate()?;
        let entry = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&Self::key(account, folder))
            .filter(|entry| entry.account.same_session(account))
            .cloned();
        if let Some(entry) = entry {
            *entry
                .touched
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Instant::now();
            account.validate()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    async fn read_scoped<S>(
        &self,
        account: &AccountGuard,
        folder: Option<i64>,
        source: &S,
        audit: bool,
        poll: bool,
        consumer: bool,
    ) -> Result<GuardedListing<R>, String>
    where
        S: Source<R> + Clone + Send + 'static,
    {
        account.validate()?;
        let entry = self.entry(account, folder, consumer)?;
        loop {
            let job = {
                let mut pending = entry
                    .pending
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if let Some(job) = pending.as_ref() {
                    job.clone()
                } else {
                    let job = Arc::new(Job {
                        forced_poll: poll,
                        result: Mutex::new(None),
                        notify: tokio::sync::Notify::new(),
                    });
                    *pending = Some(job.clone());
                    let target = entry.clone();
                    let source = source.clone();
                    let account = account.clone();
                    let task = job.clone();
                    let deadline = self.timing.reconciliation;
                    let capacity = self.capacity.clone();
                    tokio::spawn(async move {
                        let completion = Completion(task.clone());
                        let work = async {
                            let _capacity = capacity
                                .acquire()
                                .await
                                .map_err(|_| "INVENTORY_STALE: Inventory stopped")?;
                            let mut inventory = target.inventory.lock().await;
                            account.validate()?;
                            if !inventory.same_session(&account) {
                                return Err("ACCOUNT_CHANGED".into());
                            }
                            if poll {
                                inventory.expire_poll();
                            }
                            inventory.quota = if consumer {
                                0
                            } else {
                                target
                                    .audit_quota
                                    .load(std::sync::atomic::Ordering::Relaxed)
                            };
                            let previous_audit = inventory.last_audit;
                            let rows = inventory.read(&source, audit).await?;
                            if inventory.last_audit != previous_audit
                                && target
                                    .stale
                                    .swap(false, std::sync::atomic::Ordering::SeqCst)
                            {
                                inventory.serial = inventory.serial.wrapping_add(1);
                                crate::local_search::inventory_changed(
                                    &account,
                                    folder,
                                    &inventory.generation,
                                );
                            }
                            *target.audited.lock().unwrap_or_else(|e| e.into_inner()) =
                                inventory.last_audit;
                            account.validate()?;
                            let ticket = RevisionTicket {
                                owner: account.owner,
                                value: inventory.revision,
                                scope: revision_scope(account.owner),
                            };
                            ticket.validate(&account)?;
                            Ok(GuardedListing {
                                complete: inventory.complete
                                    && !target.stale.load(std::sync::atomic::Ordering::SeqCst),
                                cached: true,
                                rows,
                                ticket,
                                serial: inventory.serial,
                                generation: crate::local_search::InventoryStamp::monitored(
                                    &inventory.generation,
                                    target.touched.clone(),
                                    inventory.timing.retention,
                                    &target.pins,
                                ),
                            })
                        };
                        let canceled = async {
                            loop {
                                if account.validate().is_err() {
                                    break;
                                }
                                tokio::time::sleep(Duration::from_millis(250)).await;
                            }
                        };
                        let outcome=std::panic::AssertUnwindSafe(async {tokio::select! {
                        result=tokio::time::timeout(deadline,work)=>result.unwrap_or_else(|_|Err("INVENTORY_STALE: Reconciliation timed out".into())),
                        _=canceled=>Err("ACCOUNT_CHANGED: Reconciliation canceled".into()),
                    }}).catch_unwind().await.unwrap_or_else(|_|Err("INVENTORY_STALE: Reconciliation failed".into()));
                        task.complete(outcome);
                        drop(completion);
                    });
                    job
                }
            };
            let mut result = job.wait().await.and_then(|listing| {
                account.validate()?;
                listing.ticket.validate(account)?;
                if consumer {
                    // Establish only the initial observed baseline. Later REST/DAV
                    // reads cannot consume notifications for other consumers.
                    let _ = entry.notified.compare_exchange(
                        u64::MAX,
                        listing.serial,
                        std::sync::atomic::Ordering::Relaxed,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                }
                Ok(listing)
            });
            {
                let mut pending = entry
                    .pending
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if pending
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &job))
                {
                    pending.take();
                }
            }
            if poll && !job.forced_poll {
                continue;
            }
            if let Ok(listing) = &mut result {
                entry
                    .row_count
                    .store(listing.rows.len(), std::sync::atomic::Ordering::Relaxed);
                listing.cached = self.trim(&entry);
            }
            return result;
        }
    }
    fn trim(&self, current: &Arc<Entry<R>>) -> bool {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            let rows: usize = entries
                .values()
                .map(|entry| entry.row_count.load(std::sync::atomic::Ordering::Relaxed))
                .sum();
            let batches: usize = entries
                .values()
                .map(|entry| {
                    entry
                        .row_count
                        .load(std::sync::atomic::Ordering::Relaxed)
                        .div_ceil(100)
                })
                .sum();
            if rows <= MAX_RETAINED_ROWS && batches <= MAX_AUDIT_BATCHES {
                break;
            }
            let oldest = entries
                .iter()
                .filter(|(_, entry)| {
                    !Arc::ptr_eq(entry, current)
                        && entry.pins.load(std::sync::atomic::Ordering::SeqCst) == 0
                })
                .min_by_key(|(_, entry)| *entry.touched.lock().unwrap_or_else(|e| e.into_inner()))
                .map(|(key, _)| key.clone());
            if let Some(key) = oldest {
                entries.remove(&key);
            } else {
                entries.retain(|_, entry| !Arc::ptr_eq(entry, current));
                return false;
            }
        }
        true
    }
    pub async fn poll_recent<S, F, Fut>(
        &self,
        source: F,
    ) -> Vec<(AccountGuard, Option<i64>, GuardedListing<R>)>
    where
        S: Source<R> + Clone + Send + 'static,
        F: Fn(AccountGuard, Option<i64>) -> Fut + Sync,
        Fut: std::future::Future<Output = Result<S, String>>,
    {
        let entries = {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            entries.retain(|_, entry| {
                Arc::strong_count(entry) > 1
                    || entry
                        .touched
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .elapsed()
                        < self.timing.retention
            });
            entries
                .values()
                .filter(|entry| {
                    entry
                        .touched
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .elapsed()
                        < self.timing.retention
                })
                .cloned()
                .collect::<Vec<_>>()
        };
        let mut entries = entries;
        let round = {
            let mut round = self.audit_round.lock().unwrap_or_else(|e| e.into_inner());
            if self.budget.age(*round) >= self.timing.audit
                && entries.iter().all(|entry| {
                    entry
                        .audited
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .is_some_and(|at| at >= *round)
                        || entry
                            .failed_until
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .is_some()
                })
            {
                *round = self.budget.now();
            }
            *round
        };
        entries.sort_by_key(|entry| entry.folder);
        if !entries.is_empty() && self.budget.available() > 0 {
            let offset = self.rotation.fetch_add(
                AUDIT_BATCHES_PER_MINUTE,
                std::sync::atomic::Ordering::Relaxed,
            ) % entries.len();
            entries.rotate_left(offset);
        }
        let due = |entry: &Arc<Entry<R>>| {
            entry
                .audited
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_none_or(|at| at < round)
        };
        let mut remaining = entries.iter().filter(|entry| due(entry)).count();
        let mut changes = Vec::new();
        for entry in entries {
            if entry
                .failed_until
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some_and(|at| at > self.budget.now())
            {
                continue;
            }
            if entry.account.validate().is_err() {
                continue;
            }
            let quota = if due(&entry) {
                let quota = self.budget.available().div_ceil(remaining.max(1));
                remaining = remaining.saturating_sub(1);
                quota
            } else {
                0
            };
            entry
                .audit_quota
                .store(quota, std::sync::atomic::Ordering::Relaxed);
            let result = tokio::time::timeout(REQUEST_TIMEOUT, async {
                let source = source(entry.account.clone(), entry.folder).await?;
                self.read_scoped(&entry.account, entry.folder, &source, false, true, false)
                    .await
            })
            .await;
            entry
                .audit_quota
                .store(1, std::sync::atomic::Ordering::Relaxed);
            if let Ok(Ok(listing)) = result {
                *entry.failed_until.lock().unwrap_or_else(|e| e.into_inner()) = None;
                if entry
                    .notified
                    .swap(listing.serial, std::sync::atomic::Ordering::Relaxed)
                    != listing.serial
                {
                    changes.push((entry.account.clone(), entry.folder, listing));
                }
            } else {
                if !entry.stale.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    let generation =
                        crate::local_search::inventory_generation(&entry.account, entry.folder);
                    crate::local_search::inventory_changed(
                        &entry.account,
                        entry.folder,
                        &generation,
                    );
                }
                *entry.failed_until.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(self.budget.now() + self.timing.audit);
            }
        }
        changes
    }

    pub async fn read_with_ticket<S>(
        &self,
        account: &AccountGuard,
        folder: Option<i64>,
        source: &S,
        audit: bool,
        poll: bool,
    ) -> Result<GuardedListing<R>, String>
    where
        S: Source<R> + Clone + Send + 'static,
    {
        self.read_scoped(account, folder, source, audit, poll, true)
            .await
    }
    pub async fn read_background<S>(
        &self,
        account: &AccountGuard,
        folder: Option<i64>,
        source: &S,
    ) -> Result<GuardedListing<R>, String>
    where
        S: Source<R> + Clone + Send + 'static,
    {
        self.read_scoped(account, folder, source, false, false, false)
            .await
    }
    pub async fn read<S>(
        &self,
        account: &AccountGuard,
        folder: Option<i64>,
        source: &S,
        audit: bool,
        poll: bool,
    ) -> Result<Vec<R>, String>
    where
        S: Source<R> + Clone + Send + 'static,
    {
        self.read_with_ticket(account, folder, source, audit, poll)
            .await
            .map(|listing| {
                listing
                    .rows
                    .iter()
                    .map(|row| row.as_ref().clone())
                    .collect()
            })
    }
}
impl<R: Row> Drop for Entry<R> {
    fn drop(&mut self) {
        let generation = crate::local_search::inventory_generation(&self.account, self.folder);
        crate::local_search::inventory_changed(&self.account, self.folder, &generation);
    }
}

static INVENTORIES: LazyLock<InventoryCache<Message>> = LazyLock::new(InventoryCache::default);
async fn telegram_source(
    account: &AccountGuard,
    state: &TelegramState,
    folder: Option<i64>,
) -> Result<TelegramSource, String> {
    account.validate()?;
    let client = state
        .client
        .lock()
        .await
        .clone()
        .ok_or("Client not connected")?;
    account.validate_client(&client).await?;
    let peer = resolve_peer(&client, folder, &state.peer_cache).await?;
    Ok(TelegramSource {
        account: account.clone(),
        client,
        peer,
    })
}

async fn messages_inner(
    account: &AccountGuard,
    state: &TelegramState,
    folder: Option<i64>,
    consumer: bool,
    poll: bool,
) -> Result<GuardedListing<Message>, String> {
    let source = telegram_source(account, state, folder).await?;
    INVENTORIES
        .read_scoped(account, folder, &source, false, poll, consumer)
        .await
}
#[tauri::command]
pub async fn cmd_touch_file_inventory(
    app: tauri::AppHandle,
    owner_id: String,
    folder_id: Option<i64>,
) -> Result<bool, String> {
    use tauri::Manager;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    tokio::task::spawn_blocking(move || {
        let account = AccountGuard::open(&root, Some(&owner_id))?;
        INVENTORIES.touch(&account, folder_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

pub async fn messages(
    account: &AccountGuard,
    state: &TelegramState,
    folder: Option<i64>,
) -> Result<GuardedListing<Message>, String> {
    messages_refresh(account, state, folder, false).await
}

pub async fn messages_refresh(
    account: &AccountGuard,
    state: &TelegramState,
    folder: Option<i64>,
    poll: bool,
) -> Result<GuardedListing<Message>, String> {
    tokio::time::timeout(
        REQUEST_TIMEOUT,
        messages_inner(account, state, folder, true, poll),
    )
    .await
    .map_err(|_| match INVENTORIES.pending(account, folder) {
        Ok(true) => "INVENTORY_BUILDING: Folder reconciliation continues".into(),
        Ok(false) => "INVENTORY_STALE: Folder reconciliation timed out".into(),
        Err(error) => error,
    })?
}
/// Reconcile only folders used recently by a listing consumer. No peer/global
/// lock is held across Telegram I/O. Idle folders reconcile on their next read.
pub fn start_background(app: tauri::AppHandle) {
    use tauri::{Emitter, Manager};
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        let mut catalog_polled = Instant::now();
        let mut catalog_task: Option<tokio::task::JoinHandle<()>> = None;
        loop {
            interval.tick().await;
            if catalog_polled.elapsed() >= AUDIT_INTERVAL
                && catalog_task.as_ref().is_none_or(|task| task.is_finished())
            {
                let catalog_app = app.clone();
                let accounts = INVENTORIES.recent_accounts();
                catalog_polled = Instant::now();
                catalog_task = Some(tokio::spawn(async move {
                    let refresh = async {
                        for account in accounts {
                            let state = catalog_app.state::<TelegramState>().inner().clone();
                            let result = async {
                                let client = crate::api_catalog::client(&account, &state).await?;
                                crate::api_catalog::discover_folders(&account, &client, &state)
                                    .await
                            }
                            .await;
                            match result {
                                Ok(folders) => {
                                    let _ = crate::commands::search::verified_catalog(
                                        &account, &folders,
                                    );
                                }
                                Err(_) => {
                                    let generation = crate::local_search::inventory_generation(
                                        &account,
                                        Some(i64::MIN),
                                    );
                                    crate::local_search::inventory_changed(
                                        &account,
                                        Some(i64::MIN),
                                        &generation,
                                    );
                                }
                            }
                        }
                    };
                    let _ = tokio::time::timeout(RECONCILIATION_DEADLINE, refresh).await;
                }));
            }
            let changes = INVENTORIES
                .poll_recent(|account, folder| {
                    let state = app.state::<TelegramState>().inner().clone();
                    async move { telegram_source(&account, &state, folder).await }
                })
                .await;
            for (account, folder, _) in changes {
                let _ = app.emit(
                    "file-inventory-changed",
                    serde_json::json!({"ownerId":account.owner.to_string(),"folderId":folder}),
                );
            }
        }
    });
}
