//! REST listings share the account/session folder inventory with desktop and DAV.
//! Cold reads walk newest history once; warm reads catch up above a high-water ID.
//! Snapshots can be reused for 30 seconds. External edits use a bounded rotating
//! audit, and folders over 50,000 files report partial results without errors.
use crate::commands::utils::media_size;
use crate::commands::TelegramState;
use crate::models::FolderMetadata;
use crate::workspace::AccountGuard;
use grammers_client::types::{Media, Message, Peer};
use jiff::{fmt::temporal::DateTimePrinter, Timestamp};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

/// Longest history walked for one folder; the application's own listing uses
/// the same ceiling. A folder that reaches it is reported as incomplete.
pub const MAX_FOLDER_FILES: usize = 50_000;
/// How long a walk is reused. Changes made through the API discard it at once;
/// changes made elsewhere appear after at most this long.
const CATALOG_TTL: Duration = Duration::from_secs(240);

#[derive(Serialize, Clone, Debug)]
pub struct ApiFile {
    pub id: i64,
    pub folder_id: Option<i64>,
    pub name: String,
    pub size: u64,
    pub mime_type: Option<String>,
    /// RFC 3339 in UTC, for example `2026-06-05T10:00:00Z`.
    pub created_at: String,
    /// Stored as an encrypted envelope. Routes that would expose plaintext
    /// refuse these files.
    pub encrypted: bool,
    #[serde(skip)]
    pub timestamp: i64,
    /// The uploaded document's own name, which a rename does not change.
    #[serde(skip)]
    pub document_name: String,
    /// A document or photo, as opposed to another kind of message media.
    #[serde(skip)]
    pub is_document: bool,
}

pub fn rfc3339(seconds: i64) -> String {
    let timestamp = Timestamp::from_second(seconds)
        .expect("Telegram message timestamps fit the supported calendar");
    DateTimePrinter::new()
        .precision(Some(0))
        .timestamp_to_string(&timestamp)
}

/// Accepts RFC 3339, a plain date, Unix seconds, and the
/// `2026-06-05 10:00:00 UTC` form returned by releases before 4.0.
pub fn parse_time(value: &str) -> Option<i64> {
    let value = value.trim();
    if let Some(seconds) = crate::app_time::parse_rfc3339_seconds(value) {
        return Some(seconds);
    }
    if let Some(seconds) = crate::app_time::parse_legacy_calendar_seconds(value) {
        return Some(seconds);
    }
    value.parse::<i64>().ok().filter(|seconds| *seconds >= 0)
}

pub fn file_from_message(message: &Message, folder_id: Option<i64>) -> Option<ApiFile> {
    let media = message.media()?;
    let size = media_size(&media);
    let caption = message.text();
    let (name, mime_type, document_name, is_document) = match &media {
        Media::Document(document) => {
            let document_name = document.name().to_string();
            // A rename is stored as the message caption.
            let name = if caption.is_empty() {
                document_name.clone()
            } else {
                caption.to_string()
            };
            (
                name,
                document.mime_type().map(str::to_string),
                document_name,
                true,
            )
        }
        Media::Photo(_) => (
            "Photo.jpg".to_string(),
            Some("image/jpeg".to_string()),
            "Photo.jpg".to_string(),
            true,
        ),
        _ => ("Unknown".to_string(), None, "Unknown".to_string(), false),
    };
    Some(ApiFile {
        id: i64::from(message.id()),
        folder_id,
        encrypted: is_document
            && crate::workspace::envelope_cache::suspected_envelope(&document_name, caption),
        name,
        size,
        mime_type,
        created_at: rfc3339(message.date().timestamp()),
        timestamp: message.date().timestamp(),
        document_name,
        is_document,
    })
}

#[derive(Clone, Debug)]
pub struct Folder {
    /// `None` is Saved Messages.
    pub id: Option<i64>,
    pub name: String,
}

#[derive(Clone)]
pub struct Listing {
    pub files: Arc<Vec<ApiFile>>,
    /// False when the folder holds more than [`MAX_FOLDER_FILES`] files.
    pub complete: bool,
}

#[derive(Debug)]
pub enum CatalogError {
    NotConnected,
    Account(String),
    Folder(String),
    Remote(String),
}

struct Cached<T> {
    scope: Option<AccountGuard>,
    value: T,
    expires: Instant,
}

#[derive(Default)]
struct Catalog {
    folders: HashMap<i64, Cached<Arc<Vec<Folder>>>>,
    listings: HashMap<(i64, Option<i64>), Cached<Listing>>,
}

static CATALOG: LazyLock<Mutex<Catalog>> = LazyLock::new(|| Mutex::new(Catalog::default()));

fn catalog() -> std::sync::MutexGuard<'static, Catalog> {
    CATALOG.lock().unwrap_or_else(|error| error.into_inner())
}

/// Forget every walk kept for `owner`. Call after any change made through the
/// API so the next request sees it.
pub fn invalidate(owner: i64) {
    crate::file_inventory::invalidate(owner);
    invalidate_cached(owner);
}
/// Clear REST transport snapshots without dirtying unrelated file inventories.
pub fn invalidate_cached(owner: i64) {
    let mut catalog = catalog();
    catalog.folders.remove(&owner);
    catalog.listings.retain(|(account, _), _| *account != owner);
}

fn cached_folders(account: &AccountGuard) -> Option<Arc<Vec<Folder>>> {
    let mut catalog = catalog();
    let now = Instant::now();
    catalog.folders.retain(|_, cached| cached.expires > now);
    catalog
        .folders
        .get(&account.owner)
        .filter(|cached| {
            cached
                .scope
                .as_ref()
                .is_none_or(|scope| scope.same_session(account))
        })
        .map(|cached| cached.value.clone())
}

#[cfg(feature = "native-e2e")]
fn cached_listing(owner: i64, folder: Option<i64>) -> Option<Listing> {
    let mut catalog = catalog();
    let now = Instant::now();
    catalog.listings.retain(|_, cached| cached.expires > now);
    catalog
        .listings
        .get(&(owner, folder))
        .map(|cached| cached.value.clone())
}

/// The signed-in account's connection, verified to still be that account.
pub async fn client(
    account: &AccountGuard,
    state: &TelegramState,
) -> Result<grammers_client::Client, CatalogError> {
    let client = state
        .client
        .lock()
        .await
        .clone()
        .ok_or(CatalogError::NotConnected)?;
    account
        .validate_client(&client)
        .await
        .map_err(CatalogError::Account)?;
    Ok(client)
}

pub fn folder_display_name(title: &str) -> String {
    title
        .replace(" [TD]", "")
        .replace(" [td]", "")
        .replace("[TD]", "")
        .replace("[td]", "")
        .trim()
        .to_string()
}

/// Walk the account's dialogs for Telegram Drive folders. A failed walk is an
/// error rather than a shorter list.
pub async fn discover_folders(
    account: &AccountGuard,
    client: &grammers_client::Client,
    state: &TelegramState,
) -> Result<Vec<FolderMetadata>, CatalogError> {
    account.validate().map_err(CatalogError::Account)?;
    let mut folders = Vec::new();
    let mut discovered = HashMap::new();
    let mut dialogs = client.iter_dialogs();
    while let Some(dialog) = dialogs
        .next()
        .await
        .map_err(|error| CatalogError::Remote(error.to_string()))?
    {
        if let Peer::Channel(ref channel) = dialog.peer {
            let id = channel.raw.id;
            discovered.insert(id, dialog.peer.clone());
            if channel.raw.title.to_lowercase().contains("[td]") {
                let username = channel.raw.username.clone();
                folders.push(FolderMetadata {
                    id,
                    name: folder_display_name(&channel.raw.title),
                    parent_id: None,
                    is_public: username.is_some(),
                    username,
                    group_id: None,
                    display_order: 0,
                });
            }
        }
    }
    // Legacy folders are admitted only when their ID is among the current
    // account's dialogs. Names always come from Telegram, never unowned rows.
    let guard = account.clone();
    let legacy = tokio::task::spawn_blocking(move || {
        guard.validate()?;
        let marker = std::fs::read_to_string(guard.root.join("folder-layout.owner"))
            .ok()
            .and_then(|value| value.trim().parse::<i64>().ok());
        if marker != Some(guard.owner) {
            return Ok::<_, String>(Vec::new());
        }
        let path = guard.root.join("shares.db");
        if !path.is_file() {
            return Ok(Vec::new());
        }
        let mut db =
            sqlite::Connection::open_with_flags(path, sqlite::OpenFlags::new().with_read_only())
                .map_err(|error| error.to_string())?;
        db.set_busy_timeout(1000)
            .map_err(|error| error.to_string())?;
        let mut query = db
            .prepare("SELECT channel_id FROM folder_metadata")
            .map_err(|error| error.to_string())?;
        let mut ids = Vec::new();
        while query.next().map_err(|error| error.to_string())? == sqlite::State::Row {
            ids.push(query.read::<i64, _>(0).map_err(|error| error.to_string())?);
        }
        guard.validate()?;
        Ok(ids)
    })
    .await
    .map_err(|error| CatalogError::Remote(error.to_string()))?
    .map_err(CatalogError::Account)?;
    for id in legacy {
        if folders.iter().any(|folder| folder.id == id) {
            continue;
        }
        if let Some(Peer::Channel(channel)) = discovered.get(&id) {
            folders.push(FolderMetadata {
                id,
                name: folder_display_name(channel.title()),
                parent_id: None,
                is_public: channel.raw.username.is_some(),
                username: channel.raw.username.clone(),
                group_id: None,
                display_order: 0,
            });
        }
    }
    account.validate().map_err(CatalogError::Account)?;
    let mut peers = state.peer_cache.write().await;
    account.validate().map_err(CatalogError::Account)?;
    peers.extend(discovered);
    Ok(folders)
}

/// Saved Messages followed by every Telegram Drive folder.
pub async fn folders(
    account: &AccountGuard,
    state: &TelegramState,
) -> Result<Arc<Vec<Folder>>, CatalogError> {
    account.validate().map_err(CatalogError::Account)?;
    if let Some(cached) = cached_folders(account) {
        return Ok(cached);
    }
    let client = client(account, state).await?;
    let mut folders = vec![Folder {
        id: None,
        name: "Saved Messages".to_string(),
    }];
    folders.extend(
        discover_folders(account, &client, state)
            .await?
            .into_iter()
            .map(|folder| Folder {
                id: Some(folder.id),
                name: folder.name,
            }),
    );
    account.validate().map_err(CatalogError::Account)?;
    let folders = Arc::new(folders);
    catalog().folders.insert(
        account.owner,
        Cached {
            scope: Some(account.clone()),
            value: folders.clone(),
            expires: Instant::now() + CATALOG_TTL,
        },
    );
    Ok(folders)
}

/// Every file in one folder.
pub async fn listing(
    account: &AccountGuard,
    state: &TelegramState,
    folder: Option<i64>,
) -> Result<Listing, CatalogError> {
    account.validate().map_err(CatalogError::Account)?;
    #[cfg(feature = "native-e2e")]
    if let Some(cached) = cached_listing(account.owner, folder) {
        return Ok(cached);
    }
    if state.client.lock().await.is_none() {
        return Err(CatalogError::NotConnected);
    }
    let messages = crate::file_inventory::messages(account, state, folder)
        .await
        .map_err(|error| {
            if error.contains("ACCOUNT_") {
                CatalogError::Account(error)
            } else {
                CatalogError::Remote(error)
            }
        })?;
    let files = messages
        .rows
        .iter()
        .filter_map(|message| file_from_message(message, folder))
        .collect();
    messages
        .ticket
        .validate(account)
        .map_err(CatalogError::Account)?;
    let complete = messages.complete;
    let listing = Listing {
        files: Arc::new(files),
        complete,
    };
    account.validate().map_err(CatalogError::Account)?;
    Ok(listing)
}

/// Files of one folder, or of every folder when `scope` is `None`, with
/// whether every walk was complete.
pub async fn files(
    account: &AccountGuard,
    state: &TelegramState,
    scope: Option<Option<i64>>,
) -> Result<(Vec<ApiFile>, bool), CatalogError> {
    match scope {
        Some(folder) => {
            let listing = listing(account, state, folder).await?;
            Ok((listing.files.as_ref().clone(), listing.complete))
        }
        None => {
            let mut all = Vec::new();
            let mut complete = true;
            for folder in folders(account, state).await?.iter() {
                let listing = listing(account, state, folder.id).await?;
                all.extend(listing.files.iter().cloned());
                complete &= listing.complete;
            }
            Ok((all, complete))
        }
    }
}

#[cfg(feature = "native-e2e")]
static FIXTURE_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "native-e2e")]
pub(crate) fn fixture_revision() -> u64 {
    FIXTURE_REVISION.load(std::sync::atomic::Ordering::SeqCst)
}

/// Stand-in for the Telegram walk in native journeys: the folders and files an
/// account's walk would have produced. Everything after the walk — account
/// scoping, filtering, ordering, paging and the HTTP contract — is production
/// code.
#[cfg(feature = "native-e2e")]
pub fn seed(owner: i64, folders: Vec<Folder>, files: Vec<ApiFile>, complete: bool) {
    FIXTURE_REVISION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let expires = Instant::now() + Duration::from_secs(3600);
    let mut catalog = catalog();
    for folder in &folders {
        let listing = Listing {
            files: Arc::new(
                files
                    .iter()
                    .filter(|file| file.folder_id == folder.id)
                    .cloned()
                    .collect(),
            ),
            complete,
        };
        catalog.listings.insert(
            (owner, folder.id),
            Cached {
                scope: None,
                value: listing,
                expires,
            },
        );
    }
    catalog.folders.insert(
        owner,
        Cached {
            scope: None,
            value: Arc::new(folders),
            expires,
        },
    );
}
