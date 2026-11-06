use crate::commands::utils::resolve_peer;
use crate::commands::TelegramState;
use crate::vpn_optimizer::NetworkConfig;
use crate::workspace::AccountGuard;
use grammers_client::types::Media;
use serde::Serialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{Manager, State};
use tokio::io::AsyncWriteExt;

const MAX_ARCHIVE_ENTRIES: usize = 10_000;
const MAX_ARCHIVE_ENTRY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_TOTAL_UNCOMPRESSED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_COMPRESSION_RATIO: u64 = 1_000;

#[derive(Debug, Clone, Serialize)]
pub struct ArchiveEntry {
    pub filename: String,
    pub size: u64,
    pub compressed_size: u64,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtractedFile {
    pub temp_path: String,
    pub filename: String,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ArchiveType {
    Zip,
    Rar,
    SevenZ,
}

fn detect_archive_type(filename: &str) -> ArchiveType {
    let lower = filename.to_lowercase();
    if lower.ends_with(".rar") {
        ArchiveType::Rar
    } else if lower.ends_with(".7z") {
        ArchiveType::SevenZ
    } else {
        ArchiveType::Zip
    }
}

/// Private staging directory for downloaded archives and extracted entries.
fn staging_root() -> Result<std::path::PathBuf, String> {
    crate::temp_artifacts::staging_root()
        .map_err(|error| format!("Could not prepare private staging storage: {error}"))
}

fn generate_unique_temp_prefix(label: &str) -> String {
    format!(
        "archive_{}_{}_{}",
        label,
        std::process::id(),
        rand::random::<u64>()
    )
}

/// Download a zip, rar, or 7z file from Telegram and return its directory listing.
#[tauri::command]
pub async fn cmd_list_archive_contents(
    message_id: i32,
    folder_id: Option<i64>,
    app: tauri::AppHandle,
    state: State<'_, TelegramState>,
    net_config: State<'_, Arc<NetworkConfig>>,
) -> Result<Vec<ArchiveEntry>, String> {
    match archive_operation(message_id, folder_id, None, &app, &state, &net_config).await? {
        ArchiveOutput::Entries(entries) => Ok(entries),
        ArchiveOutput::Extracted(_) => Err("Unexpected extraction result".into()),
    }
}

/// Extract a single file from an archive and return its temp path for
/// subsequent upload.
#[tauri::command]
pub async fn cmd_extract_archive_entry(
    message_id: i32,
    folder_id: Option<i64>,
    entry_index: usize,
    app: tauri::AppHandle,
    state: State<'_, TelegramState>,
    net_config: State<'_, Arc<NetworkConfig>>,
) -> Result<ExtractedFile, String> {
    match archive_operation(
        message_id,
        folder_id,
        Some(entry_index),
        &app,
        &state,
        &net_config,
    )
    .await?
    {
        ArchiveOutput::Extracted(extracted) => Ok(extracted),
        ArchiveOutput::Entries(_) => Err("Unexpected listing result".into()),
    }
}

// ── Shared preparation ──────────────────────────────────────────────────

async fn prepare_archive_operation(
    message_id: i32,
    folder_id: Option<i64>,
    state: &TelegramState,
    net_config: &Arc<NetworkConfig>,
    account: &AccountGuard,
) -> Result<(grammers_client::Client, Media, String, u64), String> {
    let client_opt = { state.client.lock().await.clone() };
    let client = match client_opt {
        Some(c) => c,
        None => return Err("Telegram client is not connected".to_string()),
    };

    account.validate_client(&client).await?;
    let peer = resolve_peer(&client, folder_id, &state.peer_cache)
        .await
        .map_err(|e| format!("Failed to resolve peer: {}", e))?;

    let messages = client
        .get_messages_by_id(&peer, &[message_id])
        .await
        .map_err(|e| format!("Failed to fetch message: {}", e))?;

    account.validate()?;
    let msg = messages
        .into_iter()
        .flatten()
        .next()
        .ok_or("File not found")?;

    let media = msg.media().ok_or("Message has no media")?;

    let filename = match &media {
        Media::Document(d) => d.name().to_string(),
        _ => "unknown".to_string(),
    };

    let file_size = crate::commands::utils::media_size(&media);

    let max_bytes = net_config.archive_max_bytes();
    if max_bytes > 0 && file_size > max_bytes {
        return Err(format!(
            "Archive file ({} MiB) exceeds the {} MiB archive size limit",
            file_size / (1024 * 1024),
            max_bytes / (1024 * 1024),
        ));
    }

    Ok((client, media, filename, max_bytes))
}

#[derive(Serialize)]
#[serde(untagged)]
pub(crate) enum ArchiveOutput {
    Entries(Vec<ArchiveEntry>),
    Extracted(ExtractedFile),
}

// A blocking task owns the unpublished artifact. If its awaiting future is
// cancelled, dropping the task result removes it rather than leaking a path.
struct PendingExtraction {
    artifact: ExtractedArtifact,
    filename: String,
    size: u64,
}
enum PendingOutput {
    Entries(Vec<ArchiveEntry>),
    Extracted(PendingExtraction),
}
impl PendingOutput {
    fn publish(self, account: &Option<AccountGuard>) -> Result<ArchiveOutput, String> {
        validate_account(account)?;
        match self {
            Self::Entries(entries) => Ok(ArchiveOutput::Entries(entries)),
            Self::Extracted(pending) => pending
                .artifact
                .publish(pending.filename, pending.size, account)
                .map(ArchiveOutput::Extracted),
        }
    }
}

pub(crate) struct ArchiveDownload {
    pub root: PathBuf,
    pub max_bytes: u64,
    pub expected_bytes: Option<u64>,
    pub account: Option<AccountGuard>,
}

fn validate_account(account: &Option<AccountGuard>) -> Result<(), String> {
    if let Some(account) = account {
        account.validate()?;
    }
    Ok(())
}

pub(crate) struct ArchiveStaging {
    pub archive_path: PathBuf,
    extract_dir: PathBuf,
    archive_created: bool,
    extract_created: bool,
}
impl Drop for ArchiveStaging {
    fn drop(&mut self) {
        if self.archive_created {
            let _ = std::fs::remove_file(&self.archive_path);
        }
        if self.extract_created {
            let _ = std::fs::remove_dir_all(&self.extract_dir);
        }
    }
}
fn private_file(path: &Path) -> Result<std::fs::File, String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|error| format!("Could not create private archive file: {error}"))
}
impl ArchiveStaging {
    fn create(
        root: PathBuf,
        extension: &'static str,
        account: Option<AccountGuard>,
    ) -> Result<(Self, std::fs::File), String> {
        validate_account(&account)?;
        let name = generate_unique_temp_prefix("viewer");
        let mut staging = Self {
            archive_path: root.join(format!("{name}.{extension}")),
            extract_dir: root.join(format!("{name}_extract")),
            archive_created: false,
            extract_created: false,
        };
        let file = private_file(&staging.archive_path)?;
        staging.archive_created = true;
        let builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        let builder = {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = builder;
            builder.mode(0o700);
            builder
        };
        validate_account(&account)?;
        builder
            .create(&staging.extract_dir)
            .map_err(|error| error.to_string())?;
        staging.extract_created = true;
        Ok((staging, file))
    }
}
fn telegram_archive_chunks(
    client: &grammers_client::Client,
    media: &Media,
    traffic: crate::traffic::Traffic,
    mut reservation: crate::bandwidth::BandwidthReservation,
    expected: u64,
) -> impl futures::Stream<Item = Result<bytes::Bytes, String>> + use<> {
    let mut download = client.iter_download(media);
    async_stream::stream! {
        let mut count=0u64;
        while let Some(chunk)=download.next().await.transpose() {
            match chunk {
                Ok(bytes)=>{
                    count=count.saturating_add(bytes.len() as u64);
                    if count>expected {yield Err("Archive exceeded its declared size".into());return;}
                    if let Err(error)=traffic.wait(bytes.len()).await {yield Err(error);return;}
                    yield Ok(bytes::Bytes::from(bytes));
                }
                Err(error)=>{yield Err(error.to_string());return;}
            }
        }
        if count!=expected {yield Err("Archive ended before its declared size".into());return;}
        if let Err(error)=traffic.account.validate(){yield Err(error);return;}
        reservation.commit();
    }
}

pub(crate) async fn stage_archive_chunks<S>(
    download: S,
    setup: &ArchiveDownload,
    extension: &'static str,
) -> Result<ArchiveStaging, String>
where
    S: futures::Stream<Item = Result<bytes::Bytes, String>>,
{
    use futures::StreamExt;
    futures::pin_mut!(download);
    let root = setup.root.clone();
    let account = setup.account.clone();
    let (staging, file) =
        tokio::task::spawn_blocking(move || ArchiveStaging::create(root, extension, account))
            .await
            .map_err(|error| error.to_string())??;
    let mut file = tokio::fs::File::from_std(file);
    let mut total = 0u64;
    loop {
        validate_account(&setup.account)?;
        let Some(chunk) = download.next().await else {
            break;
        };
        let chunk = chunk.map_err(|error| format!("Archive download failed: {error}"))?;
        validate_account(&setup.account)?;
        total = total
            .checked_add(chunk.len() as u64)
            .ok_or("Archive size overflow")?;
        if (setup.max_bytes > 0 && total > setup.max_bytes)
            || setup.expected_bytes.is_some_and(|size| total > size)
        {
            return Err("Archive download exceeded the size limit".into());
        }
        file.write_all(&chunk)
            .await
            .map_err(|error| error.to_string())?;
    }
    if setup.expected_bytes.is_some_and(|size| total != size) {
        return Err("Archive download ended before the declared size".into());
    }
    validate_account(&setup.account)?;
    file.flush().await.map_err(|error| error.to_string())?;
    drop(file);
    Ok(staging)
}

struct ExtractedArtifact {
    path: PathBuf,
    keep: bool,
}
impl Drop for ExtractedArtifact {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
impl ExtractedArtifact {
    fn create(root: &Path, name: &str) -> Result<(Self, std::fs::File), String> {
        let path = root.join(format!(
            "{}_{}",
            generate_unique_temp_prefix("extract"),
            name
        ));
        let file = private_file(&path)?;
        Ok((Self { path, keep: false }, file))
    }
    fn publish(
        mut self,
        filename: String,
        size: u64,
        account: &Option<AccountGuard>,
    ) -> Result<ExtractedFile, String> {
        validate_account(account)?;
        crate::temp_artifacts::register(&self.path)?;
        self.keep = true;
        Ok(ExtractedFile {
            temp_path: self.path.to_string_lossy().into_owned(),
            filename,
            size,
        })
    }
}
struct VerifiedWriter<'a> {
    file: std::fs::File,
    account: &'a Option<AccountGuard>,
}
impl Write for VerifiedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        validate_account(self.account).map_err(std::io::Error::other)?;
        self.file.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        validate_account(self.account).map_err(std::io::Error::other)?;
        self.file.flush()
    }
}
fn extract_reader<R: Read>(
    reader: R,
    root: &Path,
    name: String,
    declared: u64,
    account: &Option<AccountGuard>,
) -> Result<PendingExtraction, String> {
    validate_account(account)?;
    let (artifact, file) = ExtractedArtifact::create(root, &name)?;
    let mut writer = VerifiedWriter { file, account };
    // Read through EOF to verify the decoder's CRC. Taking one extra byte
    // detects an understated length without allowing unbounded expansion.
    let actual = std::io::copy(&mut reader.take(MAX_ARCHIVE_ENTRY_BYTES + 1), &mut writer)
        .map_err(|error| format!("Archive extraction failed: {error}"))?;
    if actual > MAX_ARCHIVE_ENTRY_BYTES {
        return Err("Archive entry exceeds the extraction size limit".into());
    }
    if actual != declared {
        return Err("Archive entry length does not match its header".into());
    }
    writer.flush().map_err(|error| error.to_string())?;
    drop(writer);
    validate_account(account)?;
    Ok(PendingExtraction {
        artifact,
        filename: name,
        size: actual,
    })
}
fn zip_entries(archive: &mut zip::ZipArchive<std::fs::File>) -> Result<Vec<ArchiveEntry>, String> {
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(format!(
            "Archive contains more than {MAX_ARCHIVE_ENTRIES} entries"
        ));
    }
    let mut entries = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let file = archive.by_index(index).map_err(|error| error.to_string())?;
        entries.push(ArchiveEntry {
            filename: file
                .name()
                .map_err(|error| format!("Invalid ZIP entry name: {error}"))?
                .into_owned(),
            size: file.size(),
            compressed_size: file.compressed_size(),
            is_dir: file.is_dir(),
        });
    }
    validate_archive_entries(&entries)?;
    Ok(entries)
}
pub(crate) async fn zip_from_chunks<S>(
    download: S,
    setup: ArchiveDownload,
    entry: Option<usize>,
    filename: String,
) -> Result<ArchiveOutput, String>
where
    S: futures::Stream<Item = Result<bytes::Bytes, String>>,
{
    let staging = stage_archive_chunks(download, &setup, "zip").await?;
    let publish_account = setup.account.clone();
    let pending = tokio::task::spawn_blocking(move || {
        validate_account(&setup.account)?;
        let file = std::fs::File::open(&staging.archive_path).map_err(|error| error.to_string())?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|error| format!("Failed to parse ZIP file: {error}"))?;
        let entries = zip_entries(&mut archive)?;
        check_non_empty(&entries, &filename, "ZIP")?;
        if let Some(index) = entry {
            let file = archive.by_index(index).map_err(|error| error.to_string())?;
            if file.is_dir() {
                return Err("Cannot extract a directory entry".into());
            }
            let name = sanitise_entry_name(
                file.name()
                    .map_err(|error| format!("Invalid ZIP entry name: {error}"))?
                    .as_ref(),
                index,
            );
            let size = file.size();
            extract_reader(file, &setup.root, name, size, &setup.account)
                .map(PendingOutput::Extracted)
        } else {
            validate_account(&setup.account)?;
            Ok(PendingOutput::Entries(entries))
        }
    })
    .await
    .map_err(|error| format!("ZIP operation task failed: {error}"))??;
    pending.publish(&publish_account)
}

fn sevenz_operation(
    path: &Path,
    setup: &ArchiveDownload,
    index: Option<usize>,
    filename: &str,
) -> Result<PendingOutput, String> {
    validate_account(&setup.account)?;
    let archive = sevenz_rust2::Archive::open(path).map_err(|error| error.to_string())?;
    let entries = archive
        .files
        .iter()
        .map(|file| ArchiveEntry {
            filename: file.name().into(),
            size: file.size,
            compressed_size: file.compressed_size,
            is_dir: file.is_directory,
        })
        .collect::<Vec<_>>();
    validate_archive_entries(&entries)?;
    check_non_empty(&entries, filename, "7z")?;
    let Some(target) = index else {
        return Ok(PendingOutput::Entries(entries));
    };
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    let mut reader = sevenz_rust2::SevenZReader::new(file, length, [].as_slice().into())
        .map_err(|error| error.to_string())?;
    let mut current = 0usize;
    let mut extracted = None;
    reader
        .for_each_entries(|entry, input| {
            let index = current;
            current += 1;
            validate_account(&setup.account).map_err(sevenz_rust2::Error::other)?;
            if index != target {
                return Ok(true);
            }
            if entry.is_directory {
                return Err(sevenz_rust2::Error::other(
                    "Cannot extract a directory entry",
                ));
            }
            let name = sanitise_entry_name(entry.name(), index);
            extracted = Some(
                extract_reader(input, &setup.root, name, entry.size, &setup.account)
                    .map_err(sevenz_rust2::Error::other)?,
            );
            Ok(false)
        })
        .map_err(|error| error.to_string())?;
    extracted
        .map(PendingOutput::Extracted)
        .ok_or_else(|| "Archive entry index not found".into())
}

#[cfg(not(target_os = "android"))]
fn rar_operation(
    path: &Path,
    setup: &ArchiveDownload,
    index: Option<usize>,
    filename: &str,
) -> Result<PendingOutput, String> {
    validate_account(&setup.account)?;
    let listing = unrar::Archive::new(path)
        .open_for_listing()
        .map_err(|error| error.to_string())?;
    let mut entries = Vec::new();
    for header in listing {
        if entries.len() >= MAX_ARCHIVE_ENTRIES {
            return Err(format!(
                "Archive contains more than {MAX_ARCHIVE_ENTRIES} entries"
            ));
        }
        let header = header.map_err(|error| error.to_string())?;
        entries.push(ArchiveEntry {
            filename: header.filename.to_string_lossy().into_owned(),
            size: header.unpacked_size,
            compressed_size: header.unpacked_size,
            is_dir: header.is_directory(),
        });
    }
    validate_archive_entries(&entries)?;
    check_non_empty(&entries, filename, "RAR")?;
    let Some(target) = index else {
        return Ok(PendingOutput::Entries(entries));
    };
    let mut archive = unrar::Archive::new(path)
        .open_for_processing()
        .map_err(|error| error.to_string())?;
    let mut current = 0;
    while let Some(header) = archive.read_header().map_err(|error| error.to_string())? {
        validate_account(&setup.account)?;
        if current != target {
            current += 1;
            archive = header.skip().map_err(|error| error.to_string())?;
            continue;
        }
        if header.entry().is_directory() {
            return Err("Cannot extract a directory entry".into());
        }
        let size = header.entry().unpacked_size;
        let name = sanitise_entry_name(&header.entry().filename.to_string_lossy(), target);
        let (artifact, file) = ExtractedArtifact::create(&setup.root, &name)?;
        drop(file);
        // The native decoder writes to this explicit private filename, never
        // to an archive-supplied path, and verifies its CRC during extraction.
        header
            .extract_to(&artifact.path)
            .map_err(|error| error.to_string())?;
        let actual = std::fs::metadata(&artifact.path)
            .map_err(|error| error.to_string())?
            .len();
        if actual > MAX_ARCHIVE_ENTRY_BYTES || actual != size {
            return Err("Archive entry exceeds its declared size or extraction size limit".into());
        }
        validate_account(&setup.account)?;
        return Ok(PendingOutput::Extracted(PendingExtraction {
            artifact,
            filename: name,
            size: actual,
        }));
    }
    Err("Archive entry index not found".into())
}

async fn archive_operation(
    message: i32,
    folder: Option<i64>,
    entry: Option<usize>,
    app: &tauri::AppHandle,
    state: &TelegramState,
    config: &Arc<NetworkConfig>,
) -> Result<ArchiveOutput, String> {
    let account = AccountGuard::open(
        &app.path()
            .app_data_dir()
            .map_err(|error| error.to_string())?,
        None,
    )?;
    let (client, media, filename, max_bytes) =
        prepare_archive_operation(message, folder, state, config, &account).await?;
    let setup = ArchiveDownload {
        root: staging_root()?,
        max_bytes,
        expected_bytes: Some(crate::commands::utils::media_size(&media)).filter(|size| *size > 0),
        account: Some(account.clone()),
    };
    let kind = detect_archive_type(&filename);
    let expected = crate::commands::utils::media_size(&media);
    let reservation = crate::bandwidth::BandwidthReservation::download(
        app.state::<Arc<crate::bandwidth::BandwidthManager>>()
            .inner()
            .clone(),
        expected,
    )?;
    let download = telegram_archive_chunks(
        &client,
        &media,
        crate::traffic::Traffic {
            network: config.clone(),
            account,
            direction: crate::traffic::Direction::Download,
        },
        reservation,
        expected,
    );
    if kind == ArchiveType::Zip {
        return zip_from_chunks(download, setup, entry, filename).await;
    }
    #[cfg(target_os = "android")]
    if kind == ArchiveType::Rar {
        return Err("RAR archives are not supported on Android".into());
    }
    let extension = if kind == ArchiveType::Rar {
        "rar"
    } else {
        "7z"
    };
    let staging = stage_archive_chunks(download, &setup, extension).await?;
    let publish_account = setup.account.clone();
    let pending = tokio::task::spawn_blocking(move || {
        if kind == ArchiveType::SevenZ {
            sevenz_operation(&staging.archive_path, &setup, entry, &filename)
        } else {
            #[cfg(not(target_os = "android"))]
            {
                rar_operation(&staging.archive_path, &setup, entry, &filename)
            }
            #[cfg(target_os = "android")]
            {
                Err("RAR archives are not supported on Android".into())
            }
        }
    })
    .await
    .map_err(|error| format!("Archive operation task failed: {error}"))??;
    pending.publish(&publish_account)
}

// ── Shared utilities ────────────────────────────────────────────────────

fn sanitise_entry_name(entry_name: &str, entry_index: usize) -> String {
    let normalized = entry_name.replace('\\', "/");
    let name = Path::new(&normalized)
        .file_name()
        .map(|name| {
            name.to_string_lossy()
                .chars()
                .filter(|ch| !ch.is_control())
                .collect::<String>()
        })
        .unwrap_or_default();
    if name.is_empty() {
        format!("extracted_{entry_index}")
    } else {
        name
    }
}

fn check_non_empty(entries: &[ArchiveEntry], filename: &str, label: &str) -> Result<(), String> {
    if entries.is_empty() {
        return Err(format!(
            "The file \"{}\" does not appear to be a valid {} archive (no entries found)",
            filename, label,
        ));
    }
    Ok(())
}

fn validate_archive_entries(entries: &[ArchiveEntry]) -> Result<(), String> {
    if entries.len() > MAX_ARCHIVE_ENTRIES {
        return Err(format!(
            "Archive contains more than {MAX_ARCHIVE_ENTRIES} entries"
        ));
    }
    let mut total = 0_u64;
    for entry in entries.iter().filter(|entry| !entry.is_dir) {
        if entry.size > MAX_ARCHIVE_ENTRY_BYTES {
            return Err("Archive entry exceeds the extraction size limit".to_string());
        }
        total = total
            .checked_add(entry.size)
            .ok_or_else(|| "Archive uncompressed size overflowed".to_string())?;
        if total > MAX_ARCHIVE_TOTAL_UNCOMPRESSED_BYTES {
            return Err("Archive exceeds the total uncompressed size limit".to_string());
        }
        if entry.compressed_size > 0
            && entry.size / entry.compressed_size > MAX_ARCHIVE_COMPRESSION_RATIO
        {
            return Err("Archive entry exceeds the compression ratio limit".to_string());
        }
    }
    Ok(())
}
