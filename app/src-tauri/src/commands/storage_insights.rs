use std::collections::HashMap;

use serde::Serialize;
use tauri::{Manager, State};

use crate::commands::TelegramState;
use crate::models::FileMetadata;
use crate::workspace::AccountGuard;

const DEFAULT_LARGE_FILE_BYTES: u64 = 100 * 1024 * 1024;
const DEFAULT_OLD_FILE_DAYS: i64 = 365;

#[derive(Debug, Serialize)]
pub struct StorageInsightResult {
    files: Vec<FileMetadata>,
    scanned_count: usize,
    duplicate_groups: usize,
    complete: bool,
}

#[derive(Clone)]
struct IndexedFile {
    metadata: FileMetadata,
    created_at_unix: i64,
}

fn duplicate_key(file: &FileMetadata) -> (String, u64) {
    (file.name.trim().to_lowercase(), file.size)
}

pub(crate) async fn from_source(
    account: AccountGuard,
    crypto: crate::crypto::state::CryptoState,
    source: &dyn crate::local_search::Source,
    view: String,
    large_threshold_bytes: Option<u64>,
    old_file_days: Option<i64>,
) -> Result<StorageInsightResult, String> {
    crate::local_search::inventory_summary(account, crypto, source, move |rows, complete| {
        let indexed = rows
            .iter()
            .map(|row| IndexedFile {
                metadata: row.file.clone(),
                created_at_unix: crate::app_time::parse_rfc3339_seconds(&row.file.created_at)
                    .unwrap_or(i64::MAX),
            })
            .collect::<Vec<_>>();
        summarize(
            indexed,
            complete,
            &view,
            large_threshold_bytes,
            old_file_days,
        )
    })
    .await
}

fn summarize(
    indexed: Vec<IndexedFile>,
    complete: bool,
    view: &str,
    large_threshold_bytes: Option<u64>,
    old_file_days: Option<i64>,
) -> Result<StorageInsightResult, String> {
    let scanned_count = indexed.len();
    let mut duplicate_groups = 0;

    let mut files = match view {
        "large" => {
            let threshold = large_threshold_bytes
                .unwrap_or(DEFAULT_LARGE_FILE_BYTES)
                .max(1);
            let mut matches: Vec<_> = indexed
                .into_iter()
                .filter(|file| file.metadata.size >= threshold)
                .map(|file| file.metadata)
                .collect();
            matches.sort_by_key(|file| std::cmp::Reverse(file.size));
            matches
        }
        "old" => {
            let days = old_file_days
                .unwrap_or(DEFAULT_OLD_FILE_DAYS)
                .clamp(1, 36500);
            let cutoff = crate::app_time::unix_seconds() - days * 86_400;
            let mut matches: Vec<_> = indexed
                .into_iter()
                .filter(|file| file.created_at_unix <= cutoff)
                .collect();
            matches.sort_by_key(|file| file.created_at_unix);
            matches.into_iter().map(|file| file.metadata).collect()
        }
        "duplicates" => {
            let mut groups: HashMap<(String, u64), Vec<FileMetadata>> = HashMap::new();
            for file in indexed {
                groups
                    .entry(duplicate_key(&file.metadata))
                    .or_default()
                    .push(file.metadata);
            }
            let mut matches = Vec::new();
            for mut group in groups.into_values().filter(|group| group.len() > 1) {
                duplicate_groups += 1;
                group.sort_by_key(|file| (file.folder_id, file.id));
                matches.extend(group);
            }
            matches.sort_by_cached_key(|file| file.name.to_lowercase());
            matches
        }
        _ => return Err("Unknown storage insight".to_string()),
    };

    files.truncate(1_000);
    Ok(StorageInsightResult {
        files,
        scanned_count,
        duplicate_groups,
        complete,
    })
}

#[tauri::command]
pub async fn cmd_get_storage_insight(
    app: tauri::AppHandle,
    owner_id: Option<String>,
    _state: State<'_, TelegramState>,
    crypto_state: State<'_, crate::crypto::state::CryptoState>,
    view: String,
    large_threshold_bytes: Option<u64>,
    old_file_days: Option<i64>,
) -> Result<StorageInsightResult, String> {
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let account = AccountGuard::open(&root, owner_id.as_deref())?;
    from_source(
        account,
        crypto_state.inner().clone(),
        &crate::commands::search::LiveSource(app),
        view,
        large_threshold_bytes,
        old_file_days,
    )
    .await
}
