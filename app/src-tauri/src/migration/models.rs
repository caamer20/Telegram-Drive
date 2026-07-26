use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsAccountInfo {
    pub account_name: String,
    pub account_email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OneDriveItem {
    pub id: String,
    pub name: String,
    pub item_type: String, // "folder" | "file"
    pub size: i64,
    pub path: Option<String>,
    pub child_count: Option<i64>,
    pub etag: Option<String>,
    pub quickxor_hash: Option<String>,
    pub sha1_hash: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationJob {
    pub id: i64,
    pub source_folder_id: String,
    pub source_folder_path: String,
    pub telegram_destination_id: Option<i64>,
    pub telegram_destination_name: String,
    pub local_backup_dir: String,
    pub workspace_dir: String,
    pub state: String,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub last_error: Option<String>,
    pub flood_wait_until: Option<i64>,
    pub total_folders: i64,
    pub total_files: i64,
    pub total_bytes: i64,
    pub processed_files: i64,
    pub processed_bytes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderQueueItem {
    pub id: i64,
    pub job_id: i64,
    pub folder_id: String,
    pub parent_id: Option<String>,
    pub folder_path: String,
    pub state: String, // pending, fetching, completed, failed
    pub next_page_token: Option<String>,
    pub has_more: bool,
    pub files_discovered: i64,
    pub files_completed: i64,
    pub folders_discovered: i64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationItem {
    pub id: i64,
    pub job_id: i64,
    pub folder_id: String,
    pub source_item_id: String,
    pub name: String,
    pub source_path: String,
    pub size_bytes: i64,
    pub item_type: String, // file, video, audio
    pub pipeline_stage: String, // pending, downloaded, processed, uploaded, completed, failed
    pub original_artifact_path: Option<String>,
    pub processed_artifact_path: Option<String>,
    pub original_sha256: Option<String>,
    pub processed_sha256: Option<String>,
    pub video_decision: Option<String>, // passthrough, remux, transcode
    pub telegram_random_id: Option<i64>,
    pub telegram_message_id: Option<i64>,
    pub retry_count: i64,
    pub last_error: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyQuota {
    pub date_string: String, // YYYY-MM-DD
    pub committed_bytes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacingState {
    pub id: i64,
    pub next_allowed_at: i64,
    pub flood_wait_until: i64,
    pub last_upload_success_at: i64,
}
