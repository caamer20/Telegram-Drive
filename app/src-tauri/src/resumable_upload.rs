//! Uploads that continue where they stopped.
//!
//! Telegram stores a large file as numbered parts under an identifier chosen
//! by the client, and keeps the parts of an unfinished upload for a while. An
//! upload records which parts Telegram has confirmed; a later attempt for the
//! same unchanged source sends only the rest. The record lives in the
//! account's own store, so it never crosses accounts.
//!
//! Nothing is taken on trust when resuming. The bytes that would have been
//! sent for the confirmed parts are produced again and compared with a digest
//! taken when they were sent. If they differ, or if Telegram no longer has the
//! parts, the upload starts again from the beginning.
use crate::workspace::{store::Store, AccountGuard};
use futures::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::Path,
    time::{Duration, Instant},
};
use tokio::io::{AsyncRead, AsyncReadExt};

/// Telegram's largest part, and the size of every part but the last.
pub const PART_BYTES: usize = 512 * 1024;
/// Smaller files finish in seconds and use Telegram's single-request form,
/// which needs a checksum of the whole file. They are simply sent again.
pub const RESUMABLE_FROM_BYTES: u64 = 10 * 1024 * 1024;
/// Parts sent at once, as the Telegram client library does.
const WORKERS: usize = 4;
/// Telegram discards the parts of an unfinished upload after some hours. An
/// older record is not worth verifying; if a younger one has expired on
/// Telegram's side anyway, publishing fails and the upload starts over.
const SESSION_MAX_AGE_SECS: i64 = 12 * 60 * 60;
/// Confirmed progress is written to disk at most this often while parts are
/// flowing, and always when an attempt ends.
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(1);
/// A source modified this close to when its upload began may have been
/// modified again without its timestamp changing. It is never resumed.
const RACY_SOURCE_WINDOW_SECS: i64 = 2;
const RECORD_KIND: &str = "upload-session-v1";

/// Where confirmed parts go. Telegram in the application; a recording stand-in
/// in native journeys, so interruption and resumption can be exercised.
pub trait PartSink: Sync {
    fn save_part(
        &self,
        file_id: i64,
        part: i32,
        total_parts: i32,
        bytes: Vec<u8>,
    ) -> impl Future<Output = Result<(), String>> + Send;
}

pub struct PacedSink<S> {
    pub sink: S,
    pub traffic: crate::traffic::Traffic,
}
impl<S: PartSink> PartSink for PacedSink<S> {
    async fn save_part(
        &self,
        file_id: i64,
        part: i32,
        total_parts: i32,
        bytes: Vec<u8>,
    ) -> Result<(), String> {
        self.traffic.wait(bytes.len()).await?;
        self.sink.save_part(file_id, part, total_parts, bytes).await
    }
}

pub struct TelegramSink(pub grammers_client::Client);

impl PartSink for TelegramSink {
    async fn save_part(
        &self,
        file_id: i64,
        part: i32,
        total_parts: i32,
        bytes: Vec<u8>,
    ) -> Result<(), String> {
        let stored = self
            .0
            .invoke(&grammers_tl_types::functions::upload::SaveBigFilePart {
                file_id,
                file_part: part,
                file_total_parts: total_parts,
                bytes,
            })
            .await
            .map_err(crate::commands::utils::map_error)?;
        if stored {
            Ok(())
        } else {
            Err("Telegram did not store an uploaded file part".to_string())
        }
    }
}

/// Identifies one version of a source file without reading it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFingerprint {
    pub bytes: u64,
    /// Nanoseconds since the Unix epoch, where the filesystem reports it.
    pub modified_ns: Option<i64>,
}

impl SourceFingerprint {
    pub async fn of(path: &Path) -> Result<Self, String> {
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|error| error.to_string())?;
        Ok(Self {
            bytes: metadata.len(),
            modified_ns: metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .and_then(|elapsed| i64::try_from(elapsed.as_nanos()).ok()),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadSession {
    pub file_id: i64,
    /// Bytes sent to Telegram: the envelope's length for a protected upload.
    pub total_bytes: u64,
    pub total_parts: i32,
    /// Parts `0..completed_parts` are confirmed by Telegram.
    pub completed_parts: i32,
    /// SHA-256 of those parts' bytes, in order, as they were sent.
    pub prefix_sha256: String,
    pub source: SourceFingerprint,
    pub remote_name: String,
    /// Header of a protected upload's envelope, base64. Resuming must continue
    /// the same envelope; the header carries its keys only in wrapped form,
    /// exactly as it is stored on Telegram.
    #[serde(default)]
    pub envelope_header: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl UploadSession {
    pub fn envelope_header_bytes(&self) -> Option<Vec<u8>> {
        use base64::Engine as _;
        self.envelope_header.as_deref().and_then(|header| {
            base64::engine::general_purpose::STANDARD
                .decode(header)
                .ok()
        })
    }
}

#[derive(Debug)]
pub enum UploadError {
    /// The recorded progress does not belong to what is being uploaded now.
    /// It has been discarded; start again, with new keys if protected.
    Stale(String),
    Failed(String),
}

/// Parts Telegram confirmed for one file, ready to be attached to a message.
#[derive(Debug, Clone)]
pub struct UploadedParts {
    pub file_id: i64,
    pub total_parts: i32,
    pub name: String,
    /// Parts that an earlier attempt had already sent.
    pub resumed_parts: i32,
}

impl UploadedParts {
    pub fn into_uploaded(self) -> grammers_client::types::media::Uploaded {
        grammers_client::types::media::Uploaded::from_raw(
            grammers_tl_types::types::InputFileBig {
                id: self.file_id,
                parts: self.total_parts,
                name: self.name,
            }
            .into(),
        )
    }
}

/// Telegram refused to build a file from the recorded parts: they expired or
/// were never complete. The record is useless and the upload must start over.
pub fn parts_unusable(error: &str) -> bool {
    let error = error.to_ascii_uppercase();
    error.contains("FILE_PART") || error.contains("FILE_ID_INVALID")
}

/// The record key for one destination, source and form of upload. `variant`
/// separates uploads of the same file that send different bytes.
pub fn session_key(folder: Option<i64>, path: &Path, variant: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(RECORD_KIND.as_bytes());
    digest.update([0]);
    digest.update(folder.map(|id| id.to_string()).unwrap_or_default());
    digest.update([0]);
    digest.update(path.to_string_lossy().as_bytes());
    digest.update([0]);
    digest.update(variant.as_bytes());
    format!("{:x}", digest.finalize())
}

fn hex(digest: &Sha256) -> String {
    format!("{:x}", digest.clone().finalize())
}

fn part_length(part: i32, total_parts: i32, total_bytes: u64) -> usize {
    if part + 1 < total_parts {
        PART_BYTES
    } else {
        (total_bytes - (total_parts as u64 - 1) * PART_BYTES as u64) as usize
    }
}

async fn read_part<R: AsyncRead + Unpin>(
    stream: &mut R,
    part: i32,
    total_parts: i32,
    total_bytes: u64,
) -> Result<Vec<u8>, String> {
    let mut bytes = vec![0; part_length(part, total_parts, total_bytes)];
    stream.read_exact(&mut bytes).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            "The source ended before the upload was complete".to_string()
        } else {
            error.to_string()
        }
    })?;
    Ok(bytes)
}

pub struct ResumableUpload {
    account: AccountGuard,
    key: String,
    source: SourceFingerprint,
}

impl ResumableUpload {
    pub fn new(account: &AccountGuard, key: String, source: SourceFingerprint) -> Self {
        Self {
            account: account.clone(),
            key,
            source,
        }
    }

    async fn with_store<T: Send + 'static>(
        &self,
        action: impl FnOnce(&Store, &str) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let account = self.account.clone();
        let key = self.key.clone();
        tokio::task::spawn_blocking(move || {
            account.validate()?;
            let store = Store::open(&account.root, account.owner)?;
            let value = action(&store, &key)?;
            account.validate()?;
            Ok(value)
        })
        .await
        .map_err(|error| error.to_string())?
    }

    /// Progress recorded for this exact source that is still worth continuing.
    /// Anything else recorded under the same key is removed.
    pub async fn session(&self) -> Result<Option<UploadSession>, String> {
        let source = self.source.clone();
        let now = crate::app_time::unix_seconds();
        self.with_store(move |store, key| {
            // An unreadable record is not an error: the upload starts over.
            let Some(session) = store
                .record::<UploadSession>(RECORD_KIND, key)
                .ok()
                .flatten()
            else {
                return Ok(None);
            };
            let settled = session.source.modified_ns.is_some_and(|modified_ns| {
                session.created_at - modified_ns / 1_000_000_000 >= RACY_SOURCE_WINDOW_SECS
            });
            let usable = session.source == source
                && settled
                && (0..=SESSION_MAX_AGE_SECS).contains(&(now - session.created_at))
                && session.total_parts > 0
                && (0..=session.total_parts).contains(&session.completed_parts);
            if usable {
                Ok(Some(session))
            } else {
                store.remove_record(RECORD_KIND, key)?;
                Ok(None)
            }
        })
        .await
    }

    /// Forget recorded progress: after the file is published, or when the
    /// progress turned out to be unusable.
    pub async fn discard(&self) -> Result<(), String> {
        self.with_store(|store, key| store.remove_record(RECORD_KIND, key))
            .await
    }

    async fn save(&self, session: &UploadSession) -> Result<(), String> {
        let mut session = session.clone();
        session.updated_at = crate::app_time::unix_seconds();
        self.with_store(move |store, key| store.put_record(RECORD_KIND, key, &session))
            .await
    }

    /// Send `stream`, which must start at the first byte of the upload, as the
    /// parts of one file. Parts an earlier attempt already delivered are read
    /// and checked but not sent again.
    pub async fn upload<S, R>(
        &self,
        sink: &S,
        stream: &mut R,
        total_bytes: u64,
        remote_name: &str,
        envelope_header: Option<&[u8]>,
    ) -> Result<UploadedParts, UploadError>
    where
        S: PartSink,
        R: AsyncRead + Unpin,
    {
        use base64::Engine as _;
        if total_bytes == 0 {
            return Err(UploadError::Failed("An empty file has no parts".into()));
        }
        let total_parts = i32::try_from(total_bytes.div_ceil(PART_BYTES as u64))
            .map_err(|_| UploadError::Failed("The file has too many parts".into()))?;
        let envelope_header =
            envelope_header.map(|header| base64::engine::general_purpose::STANDARD.encode(header));
        let recorded = self.session().await.map_err(UploadError::Failed)?;
        let now = crate::app_time::unix_seconds();
        let mut session = match recorded {
            Some(session)
                if session.total_bytes == total_bytes
                    && session.total_parts == total_parts
                    && session.remote_name == remote_name
                    && session.envelope_header == envelope_header =>
            {
                session
            }
            other => {
                if other.is_some() {
                    self.discard().await.map_err(UploadError::Failed)?;
                }
                UploadSession {
                    file_id: rand::random(),
                    total_bytes,
                    total_parts,
                    completed_parts: 0,
                    prefix_sha256: hex(&Sha256::new()),
                    source: self.source.clone(),
                    remote_name: remote_name.to_string(),
                    envelope_header,
                    created_at: now,
                    updated_at: now,
                }
            }
        };
        let resumed_parts = session.completed_parts;

        // What was sent before must be exactly what would be sent now.
        let mut sent = Sha256::new();
        for part in 0..resumed_parts {
            let bytes = read_part(stream, part, total_parts, total_bytes)
                .await
                .map_err(UploadError::Failed)?;
            sent.update(&bytes);
        }
        if resumed_parts > 0 && hex(&sent) != session.prefix_sha256 {
            self.discard().await.map_err(UploadError::Failed)?;
            return Err(UploadError::Stale(
                "The source no longer matches the part of it that was already uploaded".into(),
            ));
        }
        // Recorded before the first part is sent, so an interruption at any
        // point finds the identifier the parts were stored under.
        self.save(&session).await.map_err(UploadError::Failed)?;

        let file_id = session.file_id;
        let mut in_flight = FuturesUnordered::new();
        let mut next_part = resumed_parts;
        // Digest of the stream up to and including each part that has been
        // read but is not yet part of the confirmed prefix.
        let mut digests = BTreeMap::new();
        let mut confirmed = BTreeSet::new();
        let mut last_checkpoint = Instant::now();
        let mut failure: Option<String> = None;
        loop {
            while failure.is_none() && in_flight.len() < WORKERS && next_part < total_parts {
                match read_part(stream, next_part, total_parts, total_bytes).await {
                    Ok(bytes) => {
                        sent.update(&bytes);
                        digests.insert(next_part, hex(&sent));
                        let part = next_part;
                        in_flight.push(async move {
                            sink.save_part(file_id, part, total_parts, bytes)
                                .await
                                .map(|()| part)
                        });
                        next_part += 1;
                    }
                    Err(error) => failure = Some(error),
                }
            }
            // After a failure the parts already on their way are still
            // awaited: whatever Telegram confirms does not need sending again.
            let Some(outcome) = in_flight.next().await else {
                break;
            };
            match outcome {
                Ok(part) => {
                    confirmed.insert(part);
                    while confirmed.remove(&session.completed_parts) {
                        if let Some(digest) = digests.remove(&session.completed_parts) {
                            session.prefix_sha256 = digest;
                        }
                        session.completed_parts += 1;
                    }
                    if last_checkpoint.elapsed() >= CHECKPOINT_INTERVAL {
                        self.save(&session).await.map_err(UploadError::Failed)?;
                        last_checkpoint = Instant::now();
                    }
                }
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        // The final state is kept even after the last part: if publishing the
        // message fails, the next attempt has nothing left to send.
        self.save(&session).await.map_err(UploadError::Failed)?;
        if let Some(error) = failure {
            return Err(UploadError::Failed(error));
        }
        Ok(UploadedParts {
            file_id,
            total_parts,
            name: if remote_name.is_empty() {
                "a".to_string()
            } else {
                remote_name.to_string()
            },
            resumed_parts,
        })
    }
}
