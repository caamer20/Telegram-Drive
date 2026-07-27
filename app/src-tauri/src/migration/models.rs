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
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub last_error: Option<String>,
    pub flood_wait_until: Option<i64>,
    pub discovered_folders: i64,
    pub completed_folders: i64,
    pub discovered_items: i64,
    pub completed_items: i64,
    pub failed_items: i64,
    pub waiting_items: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderQueueItem {
    pub id: i64,
    pub job_id: i64,
    pub folder_id: String,
    pub parent_id: Option<String>,
    pub folder_path: String,
    pub state: String,
    pub next_page_link: Option<String>,
    pub has_more: bool,
    pub discovered_files_count: i64,
    pub discovered_folders_count: i64,
    pub completed_files_count: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationItem {
    pub id: i64,
    pub job_id: i64,
    pub folder_id: String,
    pub source_item_id: String,
    pub name: String,
    pub path: String,
    pub size: i64,
    pub item_category: String,
    pub pipeline_stage: String,
    pub original_artifact_path: Option<String>,
    pub processed_artifact_path: Option<String>,
    pub original_sha256: Option<String>,
    pub processed_sha256: Option<String>,
    pub video_decision: Option<String>,
    pub artifact_size: Option<i64>,
    pub telegram_attempt_id: Option<String>,
    pub telegram_random_id: Option<i64>,
    pub telegram_message_id: Option<i64>,
    pub retry_count: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderSummary {
    pub source_path: String,
    pub name: String,
    pub file_count: i64,
    pub total_size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationStats {
    pub total_folders: i64,
    pub total_files: i64,
    pub total_bytes: i64,
    pub completed_telegram: i64,
    pub completed_local: i64,
    pub completed_bytes: i64,
    pub failed_files: i64,
    pub waiting_files: i64,
    pub pending_files: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationJobDetail {
    pub job: MigrationJob,
    pub stats: MigrationStats,
    pub folders: Vec<FolderSummary>,
    pub files: Vec<MigrationItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyQuota {
    pub date_string: String,
    pub used_bytes: i64,
    pub reset_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaReservation {
    pub id: i64,
    pub job_id: i64,
    pub item_id: String,
    pub reserved_bytes: i64,
    pub reserved_at: i64,
    pub expires_at: i64,
    pub status: String,
}

