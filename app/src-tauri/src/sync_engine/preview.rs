use super::{
    config::{self, SyncPair},
    planner::{self, FileTree, SyncOperation, SyncedTree},
    policy::SyncPreferences,
    SyncEngine,
};
use crate::{db::DbConnection, workspace::AccountGuard};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tauri::Manager;

const REVIEW_LIFETIME: Duration = Duration::from_secs(5 * 60);

fn backup_direction() -> String {
    "upload_only".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPreviewRequest {
    pub pair_id: Option<i64>,
    pub local_path: String,
    pub channel_id: i64,
    #[serde(default = "backup_direction")]
    pub sync_direction: String,
    #[serde(default)]
    pub preferences: SyncPreferences,
}

impl SyncPreviewRequest {
    pub fn normalized(mut self) -> Result<Self, String> {
        if !matches!(
            self.sync_direction.as_str(),
            "upload_only" | "download_only" | "bidirectional"
        ) {
            return Err("Choose backup uploads, download only, or two-way sync".into());
        }
        if self.channel_id <= 0 {
            return Err("Choose a Telegram folder".into());
        }
        let canonical = Path::new(&self.local_path)
            .canonicalize()
            .map_err(|error| format!("Local folder is unavailable: {error}"))?;
        if !canonical.is_dir() {
            return Err("Select an existing local folder".into());
        }
        self.local_path = canonical
            .to_str()
            .ok_or("The local folder path must be valid UTF-8")?
            .to_string();
        self.preferences = self.preferences.validated()?;
        Ok(self)
    }

    fn review_key(&self, owner: i64) -> Result<String, String> {
        let encoded = serde_json::to_vec(&(owner, self)).map_err(|error| error.to_string())?;
        Ok(format!("{:x}", Sha256::digest(encoded)))
    }
}

pub(crate) struct PreviewReceipt {
    key: String,
    expires: Instant,
}

impl PreviewReceipt {
    fn accepts(&self, key: &str, now: Instant) -> bool {
        self.key == key && now < self.expires
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewCounts {
    pub uploads: usize,
    pub downloads: usize,
    pub delete_local: usize,
    pub delete_remote: usize,
    pub conflicts: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewOperation {
    pub action: String,
    pub relative_path: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPreview {
    pub request: SyncPreviewRequest,
    pub generated_at: i64,
    pub review_token: String,
    pub account_owner: String,
    pub local_files: usize,
    pub remote_files: usize,
    pub counts: PreviewCounts,
    pub operations: Vec<PreviewOperation>,
    pub pause_reasons: Vec<String>,
    pub warnings: Vec<String>,
}

fn summarize_operations(
    operations: &[SyncOperation],
    local: &FileTree,
    remote: &FileTree,
    direction: &str,
) -> (PreviewCounts, Vec<PreviewOperation>) {
    let mut counts = PreviewCounts::default();
    let entries = operations.iter().map(|operation| {
        let path = operation.path();
        let (action, detail) = match operation {
            SyncOperation::Upload { .. } => {
                counts.uploads += 1;
                ("upload", if remote.contains_key(path) { "Upload the local file and replace the previous Telegram copy" } else { "Upload a new file to Telegram" })
            }
            SyncOperation::Download { keep_both, .. } => {
                counts.downloads += 1;
                ("download", if *keep_both { "Save a separate remote-conflict copy beside the local file" } else if local.contains_key(path) { "Download Telegram's file and replace the local copy" } else { "Download a new local file" })
            }
            SyncOperation::DeleteLocal { .. } => { counts.delete_local += 1; ("delete_local", "Delete the local copy because its Telegram copy was deleted") }
            SyncOperation::DeleteRemote { .. } => { counts.delete_remote += 1; ("delete_remote", "Delete the Telegram copy because its local copy was deleted") }
            SyncOperation::Conflict { .. } => {
                counts.conflicts += 1;
                ("conflict", match (local.contains_key(path), remote.contains_key(path)) {
                    (true, false) => "The local file was edited while its Telegram copy was deleted; preserve it for review",
                    (false, true) => "The Telegram file changed while its local copy was deleted; preserve it for review",
                    _ => "The copies differ or a prior conflict is unresolved; choose which copy to keep",
                })
            }
            SyncOperation::Skip { .. } => {
                counts.skipped += 1;
                ("skip", match (local.contains_key(path), remote.contains_key(path), direction) {
                    (false, true, "upload_only") => "Keep this Telegram-only file; it is outside the upload source",
                    (true, false, "download_only") => "Keep this local-only file; it is outside the download source",
                    (true, false, _) | (false, true, _) => "Preserve the remaining copy; deletion propagation is off",
                    _ => "No transfer is needed under the selected direction",
                })
            }
        };
        PreviewOperation { action: action.into(), relative_path: path.into(), detail: detail.into() }
    }).collect();
    (counts, entries)
}

pub async fn preview_pair(
    app: &tauri::AppHandle,
    db: &DbConnection,
    request: SyncPreviewRequest,
    expected_owner: &str,
) -> Result<SyncPreview, String> {
    let request = request.normalized()?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let account = AccountGuard::open(&root, Some(expected_owner))?;
    let engine = app.state::<SyncEngine>();
    let _operation = engine.operation_lock.lock().await;
    let existing = if let Some(pair_id) = request.pair_id {
        Some(
            config::load_pairs(db.clone(), false)
                .await?
                .into_iter()
                .find(|pair| pair.id == pair_id)
                .ok_or("Sync mapping was not found")?,
        )
    } else {
        None
    };
    if let Some(pair) = &existing {
        if pair.local_path != request.local_path || pair.channel_id != request.channel_id {
            return Err("A mapping's source and destination cannot be changed; create a new mapping instead".into());
        }
        if pair
            .account_owner
            .as_deref()
            .is_some_and(|owner| owner != account.owner.to_string())
        {
            return Err("This mapping belongs to a different Telegram account. Sign into that account to review it".into());
        }
        if !config::load_pending_cleanup(db.clone(), pair.id)
            .await?
            .is_empty()
        {
            return Err("A previous replacement is waiting for cleanup. Resume this reviewed mapping to retry cleanup before requesting another preview".into());
        }
    }
    let pair = SyncPair {
        id: request.pair_id.unwrap_or(0),
        local_path: request.local_path.clone(),
        channel_id: request.channel_id,
        folder_key: request.channel_id.to_string(),
        label: existing.as_ref().and_then(|pair| pair.label.clone()),
        sync_direction: request.sync_direction.clone(),
        is_active: false,
        created_at: 0,
        account_owner: Some(account.owner.to_string()),
        preferences: request.preferences.clone(),
    };
    // A preview always reads every file: it is what the user reviews before
    // any change is allowed, so it never relies on recorded hashes.
    let mut local = super::scan_local(
        &pair.local_path,
        &pair.preferences,
        super::LocalScanMode::HashEveryFile,
        None,
    )
    .await?
    .tree;
    let mut synced = if let Some(pair_id) = request.pair_id {
        super::load_synced_tree(db, pair_id).await?
    } else {
        SyncedTree::new()
    };
    // A preview is read-only: it neither resolves conflicts nor retries remote
    // cleanup. Its cancellation channel is independent of automatic sync.
    let (_preview_shutdown, shutdown) = tokio::sync::watch::channel(false);
    let mut remote = super::scan_remote(app, &pair, &synced, shutdown, &account).await?;
    account.validate()?;
    super::retain_tree_paths(&mut local, &mut remote, &mut synced, &pair.preferences);
    // Mirror what execution will do, without persisting anything.
    planner::resume_interrupted_uploads(&local, &remote, &mut synced);
    if pair.preferences.adopt_matching_files {
        planner::adopt_matching_files(&local, &remote, &mut synced);
    }
    let operations = planner::operations_for_policy(
        &local,
        &remote,
        &synced,
        &pair.sync_direction,
        pair.preferences.propagate_deletions,
    );
    let (counts, entries) =
        summarize_operations(&operations, &local, &remote, &pair.sync_direction);
    let mut pause_reasons = Vec::new();
    if let Err(error) = planner::enforce_mass_deletion(operations.clone(), &synced) {
        pause_reasons.push(error.to_string());
    }
    if counts.conflicts > 0 && pair.preferences.pause_on_conflicts {
        pause_reasons.push("This mapping will pause until its conflicts are resolved".into());
    }
    let settings = config::load_settings(db.clone()).await?;
    if counts.uploads > 0 {
        if let Err(reason) = super::executor::upload_protection_mode(app, &settings) {
            pause_reasons.push(reason);
        }
    }
    let mut warnings = Vec::new();
    if existing
        .as_ref()
        .is_some_and(|pair| pair.account_owner.is_none())
    {
        warnings.push("This older mapping has no verified account owner. Saving this reviewed mapping explicitly associates it with the account currently signed in".into());
    }
    if synced.values().any(|entry| {
        entry
            .remote_hash
            .as_ref()
            .is_some_and(|hash| !hash.starts_with("v2:") && !hash.is_empty())
    }) {
        warnings.push("Older comparison data needs a one-time reconciliation. The plan includes those changes; simultaneous local changes remain conflicts".into());
    }
    if operations.iter().any(|operation| matches!(operation, SyncOperation::Upload { local, .. } if local.file_size > super::executor::TELEGRAM_MAX_FILE_BYTES)) {
        warnings.push("Files over the 2 GB sync upload limit will be skipped and reported in the mapping status".into());
    }
    let review_token = uuid::Uuid::new_v4().to_string();
    let now = Instant::now();
    let mut receipts = engine
        .preview_receipts
        .lock()
        .map_err(|_| "Sync preview state is unavailable")?;
    receipts.retain(|_, receipt| receipt.expires > now);
    if receipts.len() >= 64 {
        receipts.clear();
    }
    receipts.insert(
        review_token.clone(),
        PreviewReceipt {
            key: request.review_key(account.owner)?,
            expires: now + REVIEW_LIFETIME,
        },
    );
    Ok(SyncPreview {
        request,
        generated_at: crate::app_time::unix_seconds(),
        review_token,
        account_owner: account.owner.to_string(),
        local_files: local.len(),
        remote_files: remote.len(),
        counts,
        operations: entries,
        pause_reasons,
        warnings,
    })
}

pub fn consume_review(
    app: &tauri::AppHandle,
    request: &SyncPreviewRequest,
    token: Option<&str>,
    expected_owner: &str,
) -> Result<AccountGuard, String> {
    let token = token.ok_or("Preview this mapping before saving or activating it")?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let account = AccountGuard::open(&root, Some(expected_owner))?;
    let engine = app.state::<SyncEngine>();
    let receipt = engine
        .preview_receipts
        .lock()
        .map_err(|_| "Sync preview state is unavailable")?
        .remove(token)
        .ok_or("The sync preview expired; preview the mapping again")?;
    if !receipt.accepts(&request.review_key(account.owner)?, Instant::now()) {
        return Err(
            "The mapping changed or its preview expired; preview it again before saving".into(),
        );
    }
    account.validate()?;
    Ok(account)
}
