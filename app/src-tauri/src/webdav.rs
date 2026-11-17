#![cfg(not(any(target_os = "android", target_os = "ios")))]

use actix_web::{web, Either, HttpResponse};
use bytes::{Buf, Bytes};
use dav_server::actix::{DavRequest, DavResponse};
use dav_server::davpath::DavPath;
use dav_server::fs::{
    DavDirEntry, DavFile, DavFileSystem, DavMetaData, FsError, FsFuture, FsResult, FsStream,
    OpenOptions, ReadDirMeta,
};
use dav_server::memls::MemLs;
use dav_server::DavHandler;
use futures::stream;
use grammers_client::types::{Media, Peer};
use grammers_client::InputMessage;
use grammers_tl_types as tl;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use crate::bandwidth::{BandwidthManager, BandwidthReservation};
use crate::commands::utils::{media_size, resolve_peer};
use crate::commands::{
    create_folder_inner, delete_folder_inner, rename_folder_inner, TelegramState,
};
use crate::vpn_optimizer::NetworkConfig;
use crate::workspace::AccountGuard;

fn inventory_error(error: String) -> FsError {
    if error.contains("ACCOUNT_")
        || [
            "CHANNEL_PRIVATE",
            "CHAT_WRITE_FORBIDDEN",
            "AUTH_KEY_UNREGISTERED",
            "USER_BANNED_IN_CHANNEL",
            "CHAT_ADMIN_REQUIRED",
            "AUTH_KEY_INVALID",
            "SESSION_REVOKED",
        ]
        .iter()
        .any(|code| error.contains(code))
    {
        FsError::Forbidden
    } else if [
        "MESSAGE_ID_INVALID",
        "CHANNEL_INVALID",
        "PEER_ID_INVALID",
        "Folder not found",
        "Message not found",
    ]
    .iter()
    .any(|code| error.contains(code))
    {
        FsError::NotFound
    } else {
        FsError::IsRemote
    }
}
fn io_error(error: std::io::Error) -> FsError {
    match error.kind() {
        std::io::ErrorKind::NotFound => FsError::NotFound,
        std::io::ErrorKind::PermissionDenied => FsError::Forbidden,
        _ => FsError::GeneralFailure,
    }
}
fn telegram_error(error: grammers_mtsender::InvocationError) -> FsError {
    inventory_error(error.to_string())
}
pub(crate) async fn retry_upload<T, F, Fut>(
    attempts: u32,
    mut upload: F,
    validate: impl Fn() -> FsResult<()>,
    base_ms: u64,
    max_ms: u64,
) -> FsResult<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = FsResult<T>>,
{
    for attempt in 0..=attempts {
        validate()?;
        match upload().await {
            Ok(value) => {
                validate()?;
                return Ok(value);
            }
            Err(
                error @ (FsError::Forbidden
                | FsError::NotFound
                | FsError::TooLarge
                | FsError::InsufficientStorage),
            ) => return Err(error),
            Err(error) => {
                if attempt == attempts {
                    return Err(error);
                }
                tokio::time::sleep(Duration::from_millis(crate::vpn_optimizer::backoff_ms(
                    attempt, base_ms, max_ms,
                )))
                .await;
            }
        }
    }
    Err(FsError::IsRemote)
}

struct DavRequestAccess {
    account: AccountGuard,
    client: tokio::sync::OnceCell<grammers_client::Client>,
}

tokio::task_local! {
    static DAV_REQUEST: Arc<DavRequestAccess>;
}

async fn scoped_dav_request<T>(
    account: &AccountGuard,
    operation: impl std::future::Future<Output = FsResult<T>>,
) -> FsResult<T> {
    account.validate().map_err(|_| FsError::Forbidden)?;
    let access = Arc::new(DavRequestAccess {
        account: account.clone(),
        client: tokio::sync::OnceCell::new(),
    });
    let result = DAV_REQUEST.scope(access, operation).await;
    account.validate().map_err(|_| FsError::Forbidden)?;
    result
}

async fn protected_media(
    account: &AccountGuard,
    client: &grammers_client::Client,
    folder: Option<i64>,
    message: i32,
    media: &Media,
    caption: &str,
) -> FsResult<bool> {
    crate::commands::fs::resolve_remote_envelope(account, client, folder, message, media, caption)
        .await
        .map(|record| record.is_some())
        .map_err(|_| FsError::Forbidden)
}

const INDEX_TTL: Duration = crate::file_inventory::POLL_INTERVAL;
const MAX_LISTED_FILES: usize = 50_000;
type DavFolderFile = (i32, String, u64, SystemTime, bool);
const READ_CHUNK_SIZE: i32 = 524_288;
const CDN_ALIGNMENT: u64 = 524_288;

#[derive(Clone)]
pub struct WebDavAuth {
    token_hash: String,
}

#[derive(Clone, Debug)]
enum DavNode {
    Root,
    Folder {
        folder_id: Option<i64>,
        modified: SystemTime,
    },
    File {
        folder_id: Option<i64>,
        message_id: i32,
        size: u64,
        modified: SystemTime,
        encrypted: bool,
    },
}

impl DavNode {
    fn metadata(&self) -> DavMetadata {
        match self {
            Self::Root => DavMetadata::directory(UNIX_EPOCH),
            Self::Folder { modified, .. } => DavMetadata::directory(*modified),
            Self::File {
                message_id,
                size,
                modified,
                ..
            } => DavMetadata {
                len: *size,
                modified: *modified,
                is_dir: false,
                etag: Some(format!("td-{}-{}", message_id, size)),
            },
        }
    }
}

#[derive(Default)]
struct DavIndex {
    account: Option<AccountGuard>,
    nodes: HashMap<String, DavNode>,
    children: HashMap<String, Vec<String>>,
    refreshed: HashMap<String, Instant>,
    #[cfg(feature = "native-e2e")]
    fixture_revision: u64,
}

impl DavIndex {
    fn bind(&mut self, account: &AccountGuard) -> FsResult<()> {
        account.validate().map_err(|_| FsError::Forbidden)?;
        if self.account.as_ref().is_none_or(|previous| {
            previous.owner != account.owner
                || previous.root != account.root
                || previous.validate().is_err()
        }) {
            *self = Self {
                account: Some(account.clone()),
                ..Self::default()
            };
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct TelegramDavFs {
    state: Arc<TelegramState>,
    bandwidth: Arc<BandwidthManager>,
    network: Arc<NetworkConfig>,
    write_enabled: bool,
    staging_dir: PathBuf,
    account_root: PathBuf,
    index: Arc<tokio::sync::RwLock<DavIndex>>,
    #[cfg(feature = "native-e2e")]
    remote_error: Option<String>,
    #[cfg(feature = "native-e2e")]
    fixture_catalog: bool,
}

impl TelegramDavFs {
    pub fn new(
        state: Arc<TelegramState>,
        bandwidth: Arc<BandwidthManager>,
        network: Arc<NetworkConfig>,
        write_enabled: bool,
        staging_dir: PathBuf,
        account_root: PathBuf,
    ) -> Self {
        Self {
            state,
            bandwidth,
            network,
            write_enabled,
            staging_dir,
            account_root,
            index: Arc::new(tokio::sync::RwLock::new(DavIndex::default())),
            #[cfg(feature = "native-e2e")]
            remote_error: None,
            #[cfg(feature = "native-e2e")]
            fixture_catalog: false,
        }
    }

    #[cfg(feature = "native-e2e")]
    pub(crate) fn with_remote_error(mut self, error: Option<String>) -> Self {
        self.remote_error = error;
        self
    }

    #[cfg(feature = "native-e2e")]
    pub(crate) fn with_catalog_fixture(mut self, enabled: bool) -> Self {
        self.fixture_catalog = enabled;
        self
    }

    #[cfg(feature = "native-e2e")]
    pub(crate) async fn fixture_expire_root_index(&self) -> bool {
        let mut index = self.index.write().await;
        match (
            index.refreshed.get_mut("/"),
            Instant::now().checked_sub(INDEX_TTL),
        ) {
            (Some(refreshed), Some(expired)) => {
                *refreshed = expired;
                true
            }
            _ => false,
        }
    }

    #[cfg(feature = "native-e2e")]
    async fn fixture_wait_to_publish(
        &self,
        account: &AccountGuard,
        folder_id: Option<i64>,
    ) -> FsResult<()> {
        let identity = folder_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "saved".into());
        let hold = self.account_root.join("webdav-publish-hold");
        if std::fs::read_to_string(&hold).ok().as_deref() != Some(identity.as_str()) {
            return Ok(());
        }
        std::fs::write(self.account_root.join("webdav-publish-entered"), &identity)
            .map_err(io_error)?;
        let deadline = Instant::now() + Duration::from_secs(120);
        while std::fs::read_to_string(&hold).ok().as_deref() == Some(identity.as_str()) {
            account.validate().map_err(|_| FsError::Forbidden)?;
            if Instant::now() >= deadline {
                return Err(FsError::IsRemote);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        account.validate().map_err(|_| FsError::Forbidden)
    }

    fn account(&self) -> FsResult<AccountGuard> {
        let account = DAV_REQUEST
            .try_with(|access| access.account.clone())
            .map_err(|_| FsError::Forbidden)?;
        account.validate().map_err(|_| FsError::Forbidden)?;
        Ok(account)
    }

    async fn with_account<T>(
        &self,
        operation: impl std::future::Future<Output = FsResult<T>>,
    ) -> FsResult<T> {
        let account =
            AccountGuard::open(&self.account_root, None).map_err(|_| FsError::Forbidden)?;
        self.with_existing_account(&account, operation).await
    }

    async fn with_existing_account<T>(
        &self,
        account: &AccountGuard,
        operation: impl std::future::Future<Output = FsResult<T>>,
    ) -> FsResult<T> {
        self.index.write().await.bind(account)?;
        scoped_dav_request(account, operation).await
    }

    async fn client(&self) -> FsResult<grammers_client::Client> {
        #[cfg(feature = "native-e2e")]
        if let Some(error) = &self.remote_error {
            return Err(inventory_error(error.clone()));
        }
        let access = DAV_REQUEST
            .try_with(Arc::clone)
            .map_err(|_| FsError::Forbidden)?;
        access.account.validate().map_err(|_| FsError::Forbidden)?;
        let client = access
            .client
            .get_or_try_init(|| async {
                let client = self
                    .state
                    .client
                    .lock()
                    .await
                    .clone()
                    .ok_or(FsError::GeneralFailure)?;
                access
                    .account
                    .validate_client(&client)
                    .await
                    .map_err(|_| FsError::Forbidden)?;
                Ok::<_, FsError>(client)
            })
            .await?;
        access.account.validate().map_err(|_| FsError::Forbidden)?;
        Ok(client.clone())
    }

    async fn should_refresh(&self, path: &str) -> bool {
        #[cfg(feature = "native-e2e")]
        if self.fixture_catalog && path != "/" {
            return self.index.read().await.fixture_revision
                != crate::api_catalog::fixture_revision();
        }
        self.index
            .read()
            .await
            .refreshed
            .get(path)
            .map(|when| when.elapsed() >= INDEX_TTL)
            .unwrap_or(true)
    }

    async fn refresh_root(&self) -> FsResult<()> {
        if !self.should_refresh("/").await {
            return Ok(());
        }
        let account = self.account()?;
        let folders = match crate::api_catalog::folders(&account, &self.state).await {
            Ok(folders) => folders.as_ref().clone(),
            // Preserve the offline Saved Messages root, without inventing a
            // completed listing or using unowned legacy folder rows.
            Err(crate::api_catalog::CatalogError::NotConnected) => {
                vec![crate::api_catalog::Folder {
                    id: None,
                    name: "Saved Messages".into(),
                }]
            }
            Err(crate::api_catalog::CatalogError::Account(_)) => return Err(FsError::Forbidden),
            Err(_) => return Err(FsError::IsRemote),
        };
        let mut raw_folders: Vec<_> = folders
            .into_iter()
            .map(|folder| (folder.id, folder.name, UNIX_EPOCH))
            .collect();

        raw_folders.sort_by_cached_key(|folder| folder.1.to_lowercase());
        let aliases = unique_aliases(
            raw_folders
                .iter()
                .map(|(id, name, _)| (id.unwrap_or(0), name.as_str())),
        );

        let mut index = self.index.write().await;
        index.bind(&self.account()?)?;
        let incoming = raw_folders
            .into_iter()
            .zip(aliases)
            .map(|((folder_id, _name, modified), alias)| (format!("/{alias}"), folder_id, modified))
            .collect::<Vec<_>>();
        let retained = incoming
            .iter()
            .filter(|(path, folder_id, _)| {
                matches!(index.nodes.get(path), Some(DavNode::Folder { folder_id: previous, .. }) if previous == folder_id)
            })
            .map(|(path, folder_id, _)| (path.clone(), *folder_id))
            .collect::<HashMap<_, _>>();

        // Root discovery must not discard a retained folder's partial history or
        // reset its own refresh clock. Alias reuse is safe only for the same ID.
        // Remove leaf nodes as well as directory indexes for changed/removed
        // folders, so their former file URLs cannot survive a path reassignment.
        index.nodes.retain(|path, node| match node {
            DavNode::Root => path == "/",
            DavNode::Folder { folder_id, .. } => retained.get(path) == Some(folder_id),
            DavNode::File { folder_id, .. } => path
                .rsplit_once('/')
                .is_some_and(|(parent, _)| retained.get(parent) == Some(folder_id)),
        });
        index.children.retain(|path, _| retained.contains_key(path));
        index
            .refreshed
            .retain(|path, _| retained.contains_key(path));
        index.nodes.insert("/".to_string(), DavNode::Root);
        let mut children = Vec::new();
        for (path, folder_id, modified) in incoming {
            index.nodes.insert(
                path.clone(),
                DavNode::Folder {
                    folder_id,
                    modified,
                },
            );
            children.push(path);
        }
        index.children.insert("/".to_string(), children);
        index.refreshed.insert("/".to_string(), Instant::now());
        Ok(())
    }

    async fn refresh_folder(&self, folder_path: &str) -> FsResult<()> {
        if !self.should_refresh(folder_path).await {
            return Ok(());
        }
        self.refresh_root().await?;
        let folder = self
            .index
            .read()
            .await
            .nodes
            .get(folder_path)
            .cloned()
            .ok_or(FsError::NotFound)?;
        let DavNode::Folder { folder_id, .. } = folder else {
            return Err(FsError::Forbidden);
        };

        let account = self.account()?;
        #[cfg(feature = "native-e2e")]
        if self.fixture_catalog {
            let listing = crate::api_catalog::listing(&account, &self.state, folder_id)
                .await
                .map_err(|_| FsError::IsRemote)?;
            let mut files = listing.files.iter().collect::<Vec<_>>();
            files.sort_by_key(|file| std::cmp::Reverse(file.id));
            let complete = listing.complete && files.len() <= MAX_LISTED_FILES;
            let raw = files
                .into_iter()
                .take(MAX_LISTED_FILES)
                .map(|file| {
                    (
                        file.id as i32,
                        file.name.clone(),
                        file.size,
                        UNIX_EPOCH + Duration::from_secs(file.timestamp.max(0) as u64),
                        file.encrypted,
                    )
                })
                .collect();
            self.fixture_wait_to_publish(&account, folder_id).await?;
            return self
                .publish_folder_rows(
                    &account,
                    crate::file_inventory::RevisionTicket::capture(&account),
                    folder_id,
                    folder_path,
                    raw,
                    complete,
                )
                .await;
        }
        let client = self.client().await?;
        let listing = crate::file_inventory::messages(&account, &self.state, folder_id)
            .await
            .map_err(inventory_error)?;
        let mut messages = listing.rows.iter();
        let mut raw_files = Vec::new();
        while raw_files.len() < MAX_LISTED_FILES {
            let Some(message) = messages.next() else {
                break;
            };
            let Some(media) = message.media() else {
                continue;
            };
            let encrypted = protected_media(
                &account,
                &client,
                folder_id,
                message.id(),
                &media,
                message.text(),
            )
            .await?;
            let size = media_size(&media);
            let remote_name = match media {
                Media::Document(document) => document.name().to_string(),
                Media::Photo(_) => "Photo.jpg".to_string(),
                _ => continue,
            };
            let caption = message.text();
            let name = if caption.is_empty() {
                remote_name
            } else {
                caption.to_string()
            };
            let timestamp = message.date().timestamp().max(0) as u64;
            raw_files.push((
                message.id(),
                name,
                size,
                UNIX_EPOCH + Duration::from_secs(timestamp),
                encrypted,
            ));
        }
        self.publish_folder_rows(
            &account,
            listing.ticket,
            folder_id,
            folder_path,
            raw_files,
            listing.complete,
        )
        .await
    }

    async fn publish_folder_rows(
        &self,
        account: &AccountGuard,
        ticket: crate::file_inventory::RevisionTicket,
        folder_id: Option<i64>,
        folder_path: &str,
        mut raw_files: Vec<DavFolderFile>,
        complete: bool,
    ) -> FsResult<()> {
        raw_files.sort_by_key(|entry| entry.0);
        let aliases = unique_aliases(
            raw_files
                .iter()
                .map(|(id, name, ..)| (i64::from(*id), name.as_str())),
        );

        let mut index = self.index.write().await;
        ticket
            .with(account, || {
                index.bind(account).map_err(|error| error.to_string())?;
                if !matches!(index.nodes.get(folder_path), Some(DavNode::Folder { folder_id: current, .. }) if *current == folder_id) {
                    return Err("Folder not found".into());
                }
                let previous = index.children.remove(folder_path).unwrap_or_default();
                let fresh_ids = raw_files.iter().map(|file| file.0).collect::<HashSet<_>>();
                let mut children = Vec::new();
                for path in previous {
                    let refreshed = matches!(index.nodes.get(&path), Some(DavNode::File { message_id, .. }) if fresh_ids.contains(message_id));
                    if complete || refreshed { index.nodes.remove(&path); }
                    else { children.push(path); }
                }
                let mut occupied = children.iter().map(|path| path.to_lowercase()).collect::<HashSet<_>>();
                for ((message_id, _name, size, modified, encrypted), alias) in
                    raw_files.into_iter().zip(aliases)
                {
                    let mut path = format!("{}/{}", folder_path, alias);
                    let mut counter = 1;
                    while !occupied.insert(path.to_lowercase()) {
                        let suffix = if counter == 1 { message_id.to_string() } else { format!("{message_id}-{counter}") };
                        path = format!("{}/{}", folder_path, disambiguate_name(&alias, suffix));
                        counter += 1;
                    }
                    index.nodes.insert(
                        path.clone(),
                        DavNode::File {
                            folder_id,
                            message_id,
                            size,
                            modified,
                            encrypted,
                        },
                    );
                    children.push(path);
                }
                index.children.insert(folder_path.to_string(), children);
                #[cfg(feature = "native-e2e")]
                {
                    index.fixture_revision = crate::api_catalog::fixture_revision();
                }
                index
                    .refreshed
                    .insert(folder_path.to_string(), Instant::now());
                Ok(())
            })
            .map_err(inventory_error)
    }

    async fn refresh_for_path(&self, path: &str) -> FsResult<()> {
        self.refresh_root().await?;
        let segments = path_segments(path);
        if segments.len() >= 2 {
            self.refresh_folder(&format!("/{}", segments[0])).await?;
        }
        Ok(())
    }

    async fn node(&self, path: &str) -> FsResult<DavNode> {
        self.refresh_for_path(path).await?;
        self.index
            .read()
            .await
            .nodes
            .get(path)
            .cloned()
            .ok_or(FsError::NotFound)
    }

    async fn invalidate(&self, folder_path: &str) {
        if let Ok(account) = self.account() {
            let folder = self.index.read().await.nodes.get(folder_path).cloned();
            if let Some(DavNode::Folder { folder_id, .. }) = folder {
                let _ = crate::file_inventory::changed(&account, folder_id, &[]);
            } else if folder_path == "/" {
                let _ = crate::file_inventory::changed(&account, Some(i64::MIN), &[]);
            }
            crate::api_catalog::invalidate_cached(account.owner);
        }
        let mut index = self.index.write().await;
        index.refreshed.remove(folder_path);
        if folder_path == "/" {
            index.refreshed.clear();
        }
    }

    async fn folder_for_path(&self, path: &str) -> FsResult<(String, Option<i64>)> {
        let segments = path_segments(path);
        let Some(folder_name) = segments.first() else {
            return Err(FsError::Forbidden);
        };
        let folder_path = format!("/{folder_name}");
        let node = self.node(&folder_path).await?;
        match node {
            DavNode::Folder { folder_id, .. } => Ok((folder_path, folder_id)),
            _ => Err(FsError::Forbidden),
        }
    }

    async fn fetch_media(
        &self,
        folder_id: Option<i64>,
        message_id: i32,
    ) -> FsResult<(grammers_client::Client, Media, DavMetadata)> {
        let client = self.client().await?;
        let peer = resolve_peer(&client, folder_id, &self.state.peer_cache)
            .await
            .map_err(inventory_error)?;
        let messages = client
            .get_messages_by_id(&peer, &[message_id])
            .await
            .map_err(telegram_error)?;
        let message = messages
            .into_iter()
            .flatten()
            .next()
            .ok_or(FsError::NotFound)?;
        let media = message.media().ok_or(FsError::NotFound)?;
        let account = self.account()?;
        if protected_media(
            &account,
            &client,
            folder_id,
            message_id,
            &media,
            message.text(),
        )
        .await?
        {
            return Err(FsError::Forbidden);
        }
        let size = media_size(&media);
        let metadata = DavMetadata {
            len: size,
            modified: UNIX_EPOCH + Duration::from_secs(message.date().timestamp().max(0) as u64),
            is_dir: false,
            etag: Some(format!("td-{message_id}-{size}")),
        };
        Ok((client, media, metadata))
    }

    async fn upload_temp_file(&self, target_path: &str, temp_path: &Path) -> FsResult<DavMetadata> {
        if !self.write_enabled {
            return Err(FsError::Forbidden);
        }
        let segments = path_segments(target_path);
        if segments.len() != 2 {
            return Err(FsError::Forbidden);
        }
        let filename = validate_portable_name(segments[1])?;
        if is_system_metadata(&filename) {
            return Err(FsError::Forbidden);
        }
        let (folder_path, folder_id) = self.folder_for_path(target_path).await?;
        let existing = self.node(target_path).await.ok();
        if matches!(
            existing,
            Some(DavNode::File {
                encrypted: true,
                ..
            })
        ) {
            return Err(FsError::Forbidden);
        }
        if let Some(DavNode::File {
            folder_id,
            message_id,
            ..
        }) = &existing
        {
            self.fetch_media(*folder_id, *message_id).await?;
        }

        let size = tokio::fs::metadata(temp_path)
            .await
            .map_err(|_| FsError::GeneralFailure)?
            .len();
        let mut reservation = BandwidthReservation::upload(self.bandwidth.clone(), size)
            .map_err(|_| FsError::InsufficientStorage)?;
        let client = self.client().await?;
        let peer = resolve_peer(&client, folder_id, &self.state.peer_cache)
            .await
            .map_err(inventory_error)?;
        let uploaded = retry_upload(
            self.network.retry_attempts(),
            || async {
                self.account()?;
                let file = tokio::fs::File::open(temp_path).await.map_err(io_error)?;
                let mut file = crate::traffic::Reader::new(
                    file,
                    crate::traffic::Traffic {
                        network: self.network.clone(),
                        account: self.account()?,
                        direction: crate::traffic::Direction::Upload,
                    },
                );
                client
                    .upload_stream(&mut file, size as usize, filename.clone())
                    .await
                    .map_err(|error| match io_error(error) {
                        FsError::GeneralFailure => FsError::IsRemote,
                        error => error,
                    })
            },
            || self.account().map(|_| ()),
            self.network.retry_base_backoff_ms(),
            self.network.retry_max_backoff_ms(),
        )
        .await?;
        self.account()?;
        // send_message generates a new random_id on every invocation. A lost
        // reply cannot safely be retried here: invalidate and let a fresh listing
        // reconcile the remote result before a user retries the PUT.
        let sent = match client
            .send_message(&peer, InputMessage::new().text("").file(uploaded))
            .await
        {
            Ok(sent) => sent,
            Err(error) => {
                self.invalidate(&folder_path).await;
                return Err(telegram_error(error));
            }
        };
        crate::file_inventory::changed(&self.account()?, folder_id, &[])
            .map_err(inventory_error)?;
        reservation.commit();

        if let Some(DavNode::File {
            folder_id: old_folder,
            message_id: old_message,
            ..
        }) = existing
        {
            if let Ok(old_peer) = resolve_peer(&client, old_folder, &self.state.peer_cache).await {
                self.account()?;
                if let Err(error) = client.delete_messages(&old_peer, &[old_message]).await {
                    log::error!(
                        "WebDAV replacement uploaded as message {} but old message {} could not be deleted: {}",
                        sent.id(),
                        old_message,
                        error
                    );
                    return Err(telegram_error(error));
                }
                crate::workspace::remote_changes::record(
                    &self.account()?,
                    vec![crate::workspace::remote_changes::Change::Delete {
                        folder: old_folder,
                        message: old_message,
                    }],
                )
                .await
                .map_err(|_| FsError::GeneralFailure)?;
            }
        }
        self.invalidate(&folder_path).await;
        Ok(DavMetadata {
            len: size,
            modified: SystemTime::now(),
            is_dir: false,
            etag: Some(format!("td-{}-{}", sent.id(), size)),
        })
    }

    async fn delete_file_node(&self, node: &DavNode) -> FsResult<()> {
        let DavNode::File {
            folder_id,
            message_id,
            encrypted,
            ..
        } = node
        else {
            return Err(FsError::Forbidden);
        };
        if *encrypted {
            return Err(FsError::Forbidden);
        }
        self.fetch_media(*folder_id, *message_id).await?;
        let client = self.client().await?;
        let peer = resolve_peer(&client, *folder_id, &self.state.peer_cache)
            .await
            .map_err(inventory_error)?;
        self.account()?;
        client
            .delete_messages(&peer, &[*message_id])
            .await
            .map_err(telegram_error)?;
        crate::workspace::remote_changes::record(
            &self.account()?,
            vec![crate::workspace::remote_changes::Change::Delete {
                folder: *folder_id,
                message: *message_id,
            }],
        )
        .await
        .map_err(|_| FsError::GeneralFailure)?;
        Ok(())
    }

    async fn edit_message_name(
        &self,
        client: &grammers_client::Client,
        peer: &Peer,
        message_id: i32,
        new_name: String,
    ) -> FsResult<()> {
        let input_peer = peer_to_input_peer(peer)?;
        self.account()?;
        client
            .invoke(&tl::functions::messages::EditMessage {
                peer: input_peer,
                id: message_id,
                no_webpage: false,
                invert_media: false,
                message: Some(new_name),
                media: None,
                reply_markup: None,
                entities: None,
                schedule_date: None,
                quick_reply_shortcut_id: None,
                schedule_repeat_period: None,
            })
            .await
            .map_err(telegram_error)?;
        Ok(())
    }

    async fn copy_or_move_file(&self, from: &str, to: &str, move_file: bool) -> FsResult<()> {
        if !self.write_enabled {
            return Err(FsError::Forbidden);
        }
        let source = self.node(from).await?;
        let DavNode::File {
            folder_id: source_folder_id,
            message_id,
            encrypted,
            ..
        } = source.clone()
        else {
            return Err(FsError::Forbidden);
        };
        if encrypted {
            return Err(FsError::Forbidden);
        }
        self.fetch_media(source_folder_id, message_id).await?;
        let to_segments = path_segments(to);
        if to_segments.len() != 2 {
            return Err(FsError::Forbidden);
        }
        let new_name = validate_portable_name(to_segments[1])?;
        let (source_folder_path, _) = self.folder_for_path(from).await?;
        let (target_folder_path, target_folder_id) = self.folder_for_path(to).await?;
        let client = self.client().await?;
        let source_peer = resolve_peer(&client, source_folder_id, &self.state.peer_cache)
            .await
            .map_err(inventory_error)?;

        if move_file && source_folder_id == target_folder_id {
            self.edit_message_name(&client, &source_peer, message_id, new_name.clone())
                .await?;
            crate::workspace::remote_changes::record(
                &self.account()?,
                vec![crate::workspace::remote_changes::Change::Rename {
                    folder: source_folder_id,
                    message: message_id,
                    name: new_name,
                }],
            )
            .await
            .map_err(|_| FsError::GeneralFailure)?;
        } else {
            let target_peer = resolve_peer(&client, target_folder_id, &self.state.peer_cache)
                .await
                .map_err(inventory_error)?;
            self.account()?;
            let forwarded = client
                .forward_messages(&target_peer, &[message_id], &source_peer)
                .await
                .map_err(telegram_error)?;
            let new_message_id = forwarded
                .into_iter()
                .flatten()
                .next()
                .map(|message| message.id())
                .ok_or(FsError::GeneralFailure)?;
            self.edit_message_name(&client, &target_peer, new_message_id, new_name.clone())
                .await?;
            if move_file {
                self.account()?;
                client
                    .delete_messages(&source_peer, &[message_id])
                    .await
                    .map_err(telegram_error)?;
                crate::workspace::remote_changes::record(
                    &self.account()?,
                    vec![
                        crate::workspace::remote_changes::Change::Move {
                            source: source_folder_id,
                            message: message_id,
                            target: target_folder_id,
                            new_message: new_message_id,
                        },
                        crate::workspace::remote_changes::Change::Rename {
                            folder: target_folder_id,
                            message: new_message_id,
                            name: new_name,
                        },
                    ],
                )
                .await
                .map_err(|_| FsError::GeneralFailure)?;
            }
        }
        self.invalidate(&source_folder_path).await;
        self.invalidate(&target_folder_path).await;
        Ok(())
    }
}

impl DavFileSystem for TelegramDavFs {
    fn open<'a>(
        &'a self,
        path: &'a DavPath,
        options: OpenOptions,
    ) -> FsFuture<'a, Box<dyn DavFile>> {
        Box::pin(self.with_account(async move {
            let path = dav_path_string(path)?;
            if options.write {
                if !self.write_enabled || !supports_staged_write(&options) {
                    return Err(FsError::Forbidden);
                }
                if options.create_new && self.node(&path).await.is_ok() {
                    return Err(FsError::Exists);
                }
                tokio::fs::create_dir_all(&self.staging_dir)
                    .await
                    .map_err(|_| FsError::GeneralFailure)?;
                let temp_path = self.staging_dir.join(format!(
                    "webdav-{}-{}",
                    std::process::id(),
                    rand::random::<u64>()
                ));
                let mut file_options = tokio::fs::OpenOptions::new();
                file_options.create_new(true).write(true);
                #[cfg(unix)]
                {
                    file_options.mode(0o600);
                }
                let file = file_options
                    .open(&temp_path)
                    .await
                    .map_err(|_| FsError::GeneralFailure)?;
                let metadata = DavMetadata {
                    len: options.size.unwrap_or(0),
                    modified: SystemTime::now(),
                    is_dir: false,
                    etag: None,
                };
                return Ok(Box::new(TelegramDavFile::Write {
                    account: self.account()?,
                    fs: self.clone(),
                    target_path: path,
                    temp_path,
                    file,
                    metadata,
                    committed: false,
                }) as Box<dyn DavFile>);
            }

            let node = self.node(&path).await?;
            let DavNode::File {
                folder_id,
                message_id,
                ..
            } = node.clone()
            else {
                return Err(FsError::Forbidden);
            };
            let (client, media, metadata) = self.fetch_media(folder_id, message_id).await?;
            Ok(Box::new(TelegramDavFile::Read {
                account: self.account()?,
                client,
                media,
                position: 0,
                metadata,
                bandwidth: self.bandwidth.clone(),
                network: self.network.clone(),
            }) as Box<dyn DavFile>)
        }))
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a DavPath,
        _meta: ReadDirMeta,
    ) -> FsFuture<'a, FsStream<Box<dyn DavDirEntry>>> {
        Box::pin(self.with_account(async move {
            let path = dav_path_string(path)?;
            let node = self.node(&path).await?;
            if !matches!(node, DavNode::Root | DavNode::Folder { .. }) {
                return Err(FsError::Forbidden);
            }
            if matches!(node, DavNode::Folder { .. }) {
                self.refresh_folder(&path).await?;
            }
            let index = self.index.read().await;
            let account = self.account()?;
            let entries: Vec<FsResult<Box<dyn DavDirEntry>>> = index
                .children
                .get(&path)
                .into_iter()
                .flatten()
                .filter_map(|child_path| {
                    let child = index.nodes.get(child_path)?.clone();
                    let name = child_path.rsplit('/').next()?.as_bytes().to_vec();
                    Some(Ok(Box::new(TelegramDavDirEntry {
                        account: account.clone(),
                        name,
                        metadata: child.metadata(),
                    }) as Box<dyn DavDirEntry>))
                })
                .collect();
            Ok(Box::pin(stream::iter(entries)) as FsStream<Box<dyn DavDirEntry>>)
        }))
    }

    fn metadata<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, Box<dyn DavMetaData>> {
        Box::pin(self.with_account(async move {
            let path = dav_path_string(path)?;
            let node = self.node(&path).await?;
            Ok(Box::new(node.metadata()) as Box<dyn DavMetaData>)
        }))
    }

    fn create_dir<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, ()> {
        Box::pin(self.with_account(async move {
            if !self.write_enabled {
                return Err(FsError::Forbidden);
            }
            let path = dav_path_string(path)?;
            let segments = path_segments(&path);
            if segments.len() != 1 || segments[0].eq_ignore_ascii_case("Saved Messages") {
                return Err(FsError::Forbidden);
            }
            if self.node(&path).await.is_ok() {
                return Err(FsError::Exists);
            }
            let name = validate_portable_name(segments[0])?;
            let client = self.client().await?;
            create_folder_inner(&name, &client, &self.state.peer_cache)
                .await
                .map_err(inventory_error)?;
            self.invalidate("/").await;
            Ok(())
        }))
    }

    fn remove_dir<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, ()> {
        Box::pin(self.with_account(async move {
            if !self.write_enabled {
                return Err(FsError::Forbidden);
            }
            let path = dav_path_string(path)?;
            let node = self.node(&path).await?;
            let DavNode::Folder { folder_id, .. } = node else {
                return Err(FsError::Forbidden);
            };
            let Some(folder_id) = folder_id else {
                return Err(FsError::Forbidden);
            };
            self.refresh_folder(&path).await?;
            if self
                .index
                .read()
                .await
                .children
                .get(&path)
                .is_some_and(|children| !children.is_empty())
            {
                return Err(FsError::Exists);
            }
            let client = self.client().await?;
            delete_folder_inner(folder_id, &client, &self.state.peer_cache)
                .await
                .map_err(inventory_error)?;
            self.invalidate("/").await;
            Ok(())
        }))
    }

    fn remove_file<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, ()> {
        Box::pin(self.with_account(async move {
            if !self.write_enabled {
                return Err(FsError::Forbidden);
            }
            let path = dav_path_string(path)?;
            let node = self.node(&path).await?;
            let (folder_path, _) = self.folder_for_path(&path).await?;
            self.delete_file_node(&node).await?;
            self.invalidate(&folder_path).await;
            Ok(())
        }))
    }

    fn rename<'a>(&'a self, from: &'a DavPath, to: &'a DavPath) -> FsFuture<'a, ()> {
        Box::pin(self.with_account(async move {
            if !self.write_enabled {
                return Err(FsError::Forbidden);
            }
            let from = dav_path_string(from)?;
            let to = dav_path_string(to)?;
            let source = self.node(&from).await?;
            if matches!(source, DavNode::File { .. }) {
                return self.copy_or_move_file(&from, &to, true).await;
            }
            let DavNode::Folder { folder_id, .. } = source else {
                return Err(FsError::Forbidden);
            };
            let Some(folder_id) = folder_id else {
                return Err(FsError::Forbidden);
            };
            let segments = path_segments(&to);
            if segments.len() != 1 || self.node(&to).await.is_ok() {
                return Err(FsError::Exists);
            }
            let new_name = validate_portable_name(segments[0])?;
            let client = self.client().await?;
            rename_folder_inner(folder_id, &new_name, &client, &self.state.peer_cache)
                .await
                .map_err(inventory_error)?;
            self.invalidate("/").await;
            Ok(())
        }))
    }

    fn copy<'a>(&'a self, from: &'a DavPath, to: &'a DavPath) -> FsFuture<'a, ()> {
        Box::pin(self.with_account(async move {
            let from = dav_path_string(from)?;
            let to = dav_path_string(to)?;
            if !matches!(self.node(&from).await?, DavNode::File { .. }) {
                return Err(FsError::Forbidden);
            }
            self.copy_or_move_file(&from, &to, false).await
        }))
    }
}

// Both variants own substantial async state. Boxing the media would add indirection to every
// read, while the enum itself is already heap-owned behind DavFile trait objects.
#[allow(clippy::large_enum_variant)]
enum TelegramDavFile {
    Read {
        account: AccountGuard,
        client: grammers_client::Client,
        media: Media,
        position: u64,
        metadata: DavMetadata,
        bandwidth: Arc<BandwidthManager>,
        network: Arc<NetworkConfig>,
    },
    Write {
        account: AccountGuard,
        fs: TelegramDavFs,
        target_path: String,
        temp_path: PathBuf,
        file: tokio::fs::File,
        metadata: DavMetadata,
        committed: bool,
    },
}

impl TelegramDavFile {
    fn account(&self) -> &AccountGuard {
        match self {
            Self::Read { account, .. } | Self::Write { account, .. } => account,
        }
    }
}

impl fmt::Debug for TelegramDavFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { position, .. } => formatter
                .debug_struct("TelegramDavFile::Read")
                .field("position", position)
                .finish(),
            Self::Write {
                target_path,
                committed,
                ..
            } => formatter
                .debug_struct("TelegramDavFile::Write")
                .field("target_path", target_path)
                .field("committed", committed)
                .finish(),
        }
    }
}

impl Drop for TelegramDavFile {
    fn drop(&mut self) {
        if let Self::Write { temp_path, .. } = self {
            let path = temp_path.clone();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = tokio::fs::remove_file(path).await;
                });
            } else {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

impl DavFile for TelegramDavFile {
    fn metadata(&mut self) -> FsFuture<'_, Box<dyn DavMetaData>> {
        Box::pin(async move {
            self.account().validate().map_err(|_| FsError::Forbidden)?;
            let metadata = match self {
                Self::Read { metadata, .. } | Self::Write { metadata, .. } => metadata.clone(),
            };
            Ok(Box::new(metadata) as Box<dyn DavMetaData>)
        })
    }

    fn write_buf(&mut self, mut buffer: Box<dyn Buf + Send>) -> FsFuture<'_, ()> {
        let bytes = buffer.copy_to_bytes(buffer.remaining());
        self.write_bytes(bytes)
    }

    fn write_bytes(&mut self, bytes: Bytes) -> FsFuture<'_, ()> {
        Box::pin(async move {
            self.account().validate().map_err(|_| FsError::Forbidden)?;
            let Self::Write { file, metadata, .. } = self else {
                return Err(FsError::Forbidden);
            };
            file.write_all(&bytes)
                .await
                .map_err(|_| FsError::GeneralFailure)?;
            metadata.len = metadata.len.max(
                file.stream_position()
                    .await
                    .map_err(|_| FsError::GeneralFailure)?,
            );
            Ok(())
        })
    }

    fn read_bytes(&mut self, count: usize) -> FsFuture<'_, Bytes> {
        Box::pin(async move {
            let Self::Read {
                account,
                client,
                media,
                position,
                metadata,
                bandwidth,
                network,
            } = self
            else {
                return Err(FsError::Forbidden);
            };
            account.validate().map_err(|_| FsError::Forbidden)?;
            if *position >= metadata.len || count == 0 {
                return Ok(Bytes::new());
            }
            let remaining = metadata.len.saturating_sub(*position);
            let wanted = remaining.min(count as u64) as usize;
            let mut reservation = BandwidthReservation::download(bandwidth.clone(), wanted as u64)
                .map_err(|_| FsError::InsufficientStorage)?;
            let bytes = read_media_range(client, media, *position, wanted).await?;
            network
                .pacer
                .wait(
                    network,
                    crate::traffic::Direction::Download,
                    bytes.len(),
                    || account.validate(),
                )
                .await
                .map_err(|_| FsError::Forbidden)?;
            reservation
                .resize(bytes.len() as u64)
                .map_err(|_| FsError::InsufficientStorage)?;
            account.validate().map_err(|_| FsError::Forbidden)?;
            *position += bytes.len() as u64;
            reservation.commit();
            Ok(bytes)
        })
    }

    fn seek(&mut self, seek: SeekFrom) -> FsFuture<'_, u64> {
        Box::pin(async move {
            self.account().validate().map_err(|_| FsError::Forbidden)?;
            match self {
                Self::Read {
                    position, metadata, ..
                } => {
                    *position = checked_seek(*position, metadata.len, seek)?;
                    Ok(*position)
                }
                // Telegram uploads are replaced atomically after a complete PUT. Reject
                // ranged/partial writes so a client cannot accidentally replace a remote
                // file with a sparse staging file.
                Self::Write { .. } => Err(FsError::Forbidden),
            }
        })
    }

    fn flush(&mut self) -> FsFuture<'_, ()> {
        Box::pin(async move {
            let Self::Write {
                account,
                fs,
                target_path,
                temp_path,
                file,
                metadata,
                committed,
            } = self
            else {
                return Ok(());
            };
            account.validate().map_err(|_| FsError::Forbidden)?;
            if *committed {
                return Ok(());
            }
            file.flush().await.map_err(|_| FsError::GeneralFailure)?;
            *metadata = fs
                .with_existing_account(account, fs.upload_temp_file(target_path, temp_path))
                .await?;
            *committed = true;
            Ok(())
        })
    }
}

#[derive(Clone, Debug)]
struct DavMetadata {
    len: u64,
    modified: SystemTime,
    is_dir: bool,
    etag: Option<String>,
}

impl DavMetadata {
    fn directory(modified: SystemTime) -> Self {
        Self {
            len: 0,
            modified,
            is_dir: true,
            etag: None,
        }
    }
}

impl DavMetaData for DavMetadata {
    fn len(&self) -> u64 {
        self.len
    }

    fn modified(&self) -> FsResult<SystemTime> {
        Ok(self.modified)
    }

    fn is_dir(&self) -> bool {
        self.is_dir
    }

    fn etag(&self) -> Option<String> {
        self.etag.clone().or_else(|| {
            self.modified
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|duration| format!("td-dir-{}", duration.as_secs()))
        })
    }
}

struct TelegramDavDirEntry {
    account: AccountGuard,
    name: Vec<u8>,
    metadata: DavMetadata,
}

impl DavDirEntry for TelegramDavDirEntry {
    fn name(&self) -> Vec<u8> {
        if self.account.validate().is_ok() {
            self.name.clone()
        } else {
            Vec::new()
        }
    }

    fn metadata(&self) -> FsFuture<'_, Box<dyn DavMetaData>> {
        let metadata = self.metadata.clone();
        Box::pin(async move {
            self.account.validate().map_err(|_| FsError::Forbidden)?;
            Ok(Box::new(metadata) as Box<dyn DavMetaData>)
        })
    }
}

pub fn build_handler(fs: TelegramDavFs, token_hash: String) -> (DavHandler, WebDavAuth) {
    let handler = DavHandler::builder()
        .filesystem(Box::new(fs))
        .locksystem(MemLs::new())
        .principal("telegram-drive-webdav")
        .read_buf_size(READ_CHUNK_SIZE as usize)
        .build_handler();
    (handler, WebDavAuth { token_hash })
}

/// Serve WebDAV on an already bound loopback listener.
pub fn serve(
    listener: std::net::TcpListener,
    filesystem: TelegramDavFs,
    token_hash: String,
) -> std::io::Result<actix_web::dev::Server> {
    let (handler, auth) = build_handler(filesystem, token_hash);
    let handler = web::Data::new(handler);
    let auth = web::Data::new(auth);
    Ok(actix_web::HttpServer::new(move || {
        actix_web::App::new()
            .app_data(handler.clone())
            .app_data(auth.clone())
            .service(web::resource("/{tail:.*}").to(webdav_handler))
    })
    .listen(listener)?
    .run())
}

pub async fn webdav_handler(
    request: DavRequest,
    handler: web::Data<DavHandler>,
    auth: web::Data<WebDavAuth>,
) -> Either<DavResponse, HttpResponse> {
    let original_path = request.request.uri().path().to_string();
    let Some((token, _relative_path)) = split_authenticated_path(&original_path) else {
        return Either::Right(HttpResponse::NotFound().finish());
    };
    if !crate::commands::webdav_settings::verify_token(token, &auth.token_hash) {
        return Either::Right(HttpResponse::NotFound().finish());
    }

    if let Some(destination) = request.request.headers().get("destination") {
        if let Ok(destination) = destination.to_str() {
            if let Some(path_start) = destination_path(destination) {
                let valid_destination =
                    split_authenticated_path(path_start).is_some_and(|(destination_token, _)| {
                        crate::commands::webdav_settings::verify_token(
                            destination_token,
                            &auth.token_hash,
                        )
                    });
                if !valid_destination {
                    return Either::Right(HttpResponse::NotFound().finish());
                }
            } else {
                return Either::Right(HttpResponse::BadRequest().finish());
            }
        } else {
            return Either::Right(HttpResponse::BadRequest().finish());
        }
    }

    // Keep the authenticated prefix in the request and let dav-server strip it.
    // This makes every href in PROPFIND/LOCK responses include /dav/<token>/,
    // which Finder and Explorer require when following child resources.
    let prefix = format!("/dav/{token}");
    let config = DavHandler::builder().strip_prefix(prefix);
    Either::Left(handler.handle_with(config, request.request).await.into())
}

fn destination_path(destination: &str) -> Option<&str> {
    if destination.starts_with('/') {
        return Some(destination);
    }
    let scheme = destination.find("://")?;
    let after_host = &destination[scheme + 3..];
    let slash = after_host.find('/')?;
    Some(&after_host[slash..])
}

fn split_authenticated_path(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("/dav/")?;
    let (token, tail) = rest.split_once('/').unwrap_or((rest, ""));
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let relative = if tail.is_empty() {
        "/"
    } else {
        &path[path.len() - tail.len() - 1..]
    };
    Some((token, relative))
}

fn dav_path_string(path: &DavPath) -> FsResult<String> {
    String::from_utf8(path.as_bytes().to_vec())
        .map(|value| normalize_index_path(&value))
        .map_err(|_| FsError::Forbidden)
}

fn normalize_index_path(path: &str) -> String {
    if path == "/" {
        "/".to_string()
    } else {
        format!("/{}", path.trim_matches('/'))
    }
}

fn path_segments(path: &str) -> Vec<&str> {
    path.trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty())
        .collect()
}

fn sanitize_name(name: &str) -> String {
    let mut sanitized: String = name
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            character if character.is_control() => '_',
            character => character,
        })
        .collect();
    while sanitized.ends_with(['.', ' ']) {
        sanitized.pop();
    }
    if sanitized.is_empty() {
        sanitized.push_str("Untitled");
    }
    if is_windows_reserved_name(&sanitized) {
        sanitized.insert(0, '_');
    }
    if sanitized.chars().count() > 240 {
        sanitized = sanitized.chars().take(240).collect();
    }
    sanitized
}

fn validate_portable_name(name: &str) -> FsResult<String> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name
            .chars()
            .any(|character| character == '/' || character == '\0')
    {
        return Err(FsError::Forbidden);
    }
    let sanitized = sanitize_name(name);
    if sanitized != name {
        return Err(FsError::Forbidden);
    }
    Ok(sanitized)
}

fn is_windows_reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .and_then(|number| number.parse::<u8>().ok())
            .is_some_and(|number| (1..=9).contains(&number))
}

fn is_system_metadata(name: &str) -> bool {
    name.eq_ignore_ascii_case(".DS_Store")
        || name.starts_with("._")
        || name.eq_ignore_ascii_case("desktop.ini")
        || name.eq_ignore_ascii_case("Thumbs.db")
}

fn unique_aliases<'a>(items: impl Iterator<Item = (i64, &'a str)>) -> Vec<String> {
    let items: Vec<(i64, String)> = items.map(|(id, name)| (id, sanitize_name(name))).collect();
    let mut totals = HashMap::<String, usize>::new();
    for (_, name) in &items {
        *totals.entry(name.to_lowercase()).or_default() += 1;
    }
    let mut used = HashSet::new();
    items
        .into_iter()
        .map(|(id, name)| {
            let key = name.to_lowercase();
            let candidate = if totals.get(&key).copied().unwrap_or(0) > 1 {
                disambiguate_name(&name, id)
            } else {
                name
            };
            let mut unique = candidate.clone();
            let mut counter = 2;
            while !used.insert(unique.to_lowercase()) {
                unique = disambiguate_name(&candidate, counter);
                counter += 1;
            }
            unique
        })
        .collect()
}

fn disambiguate_name(name: &str, id: impl fmt::Display) -> String {
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(name);
    match path.extension().and_then(|value| value.to_str()) {
        Some(extension) if !extension.is_empty() => format!("{stem} ({id}).{extension}"),
        _ => format!("{name} ({id})"),
    }
}

fn peer_to_input_peer(peer: &Peer) -> FsResult<tl::enums::InputPeer> {
    match peer {
        Peer::User(user) => {
            let (id, access_hash) = match &user.raw {
                tl::enums::User::User(raw) => (raw.id, raw.access_hash.unwrap_or(0)),
                tl::enums::User::Empty(raw) => (raw.id, 0),
            };
            Ok(tl::enums::InputPeer::User(tl::types::InputPeerUser {
                user_id: id,
                access_hash,
            }))
        }
        Peer::Channel(channel) => Ok(tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
            channel_id: channel.raw.id,
            access_hash: channel.raw.access_hash.ok_or(FsError::GeneralFailure)?,
        })),
        _ => Err(FsError::Forbidden),
    }
}

async fn read_media_range(
    client: &grammers_client::Client,
    media: &Media,
    start: u64,
    count: usize,
) -> FsResult<Bytes> {
    let aligned_start = (start / CDN_ALIGNMENT) * CDN_ALIGNMENT;
    let skip_chunks = (aligned_start / READ_CHUNK_SIZE as u64) as i32;
    let mut skip_bytes = (start - aligned_start) as usize;
    let mut downloader = client.iter_download(media).chunk_size(READ_CHUNK_SIZE);
    if skip_chunks > 0 {
        downloader = downloader.skip_chunks(skip_chunks);
    }
    let mut output = Vec::with_capacity(count);
    while output.len() < count {
        let Some(chunk) = downloader
            .next()
            .await
            .map_err(|error| inventory_error(error.to_string()))?
        else {
            break;
        };
        let data = if skip_bytes >= chunk.len() {
            skip_bytes -= chunk.len();
            continue;
        } else if skip_bytes > 0 {
            let data = &chunk[skip_bytes..];
            skip_bytes = 0;
            data
        } else {
            &chunk
        };
        let take = (count - output.len()).min(data.len());
        output.extend_from_slice(&data[..take]);
    }
    Ok(Bytes::from(output))
}

fn checked_seek(current: u64, len: u64, seek: SeekFrom) -> FsResult<u64> {
    let value = match seek {
        SeekFrom::Start(value) => i128::from(value),
        SeekFrom::Current(delta) => i128::from(current) + i128::from(delta),
        SeekFrom::End(delta) => i128::from(len) + i128::from(delta),
    };
    if value < 0 || value > i128::from(u64::MAX) {
        return Err(FsError::Forbidden);
    }
    Ok(value as u64)
}

fn supports_staged_write(options: &OpenOptions) -> bool {
    if options.append {
        return false;
    }

    // A normal PUT is truncated. dav-server also opens a non-existent resource
    // without truncation while creating a WebDAV lock-null resource; accepting that
    // open is required by Finder and Explorer before they send the eventual PUT.
    options.truncate || (options.create && options.size.is_none() && options.checksum.is_none())
}
