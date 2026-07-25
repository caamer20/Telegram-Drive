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
pub struct ScanProgressPayload {
    pub phase: String,
    pub pages_scanned: usize,
    pub discovered_files: usize,
    pub discovered_folders: usize,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OneDriveFolder {
    pub id: String,
    pub name: String,
    pub source_path: String,
    pub file_count: i64,
    pub total_size: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Draft,
    Ready,
    Running,
    Paused,
    Completed,
    Cancelled,
    Failed,
}

impl JobState {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobState::Draft => "draft",
            JobState::Ready => "ready",
            JobState::Running => "running",
            JobState::Paused => "paused",
            JobState::Completed => "completed",
            JobState::Cancelled => "cancelled",
            JobState::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "ready" => JobState::Ready,
            "running" => JobState::Running,
            "paused" => JobState::Paused,
            "completed" => JobState::Completed,
            "cancelled" => JobState::Cancelled,
            "failed" => JobState::Failed,
            _ => JobState::Draft,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemState {
    Pending,
    Downloading,
    Uploading,
    Completed,
    SkippedDuplicate,
    Failed,
}

impl ItemState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ItemState::Pending => "pending",
            ItemState::Downloading => "downloading",
            ItemState::Uploading => "uploading",
            ItemState::Completed => "completed",
            ItemState::SkippedDuplicate => "skipped_duplicate",
            ItemState::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "downloading" => ItemState::Downloading,
            "uploading" => ItemState::Uploading,
            "completed" => ItemState::Completed,
            "skipped_duplicate" => ItemState::SkippedDuplicate,
            "failed" => ItemState::Failed,
            _ => ItemState::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationJob {
    pub id: i64,
    pub state: String,
    pub onedrive_folder_id: Option<String>,
    pub onedrive_folder_path: Option<String>,
    pub telegram_destination_id: Option<i64>,
    pub telegram_destination_name: Option<String>,
    pub local_dir: Option<String>,
    pub cooldown_until: Option<i64>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub updated_at: i64,
    pub job_origin: String,
    pub pause_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationJobSummary {
    pub id: i64,
    pub state: String,
    pub onedrive_folder_path: Option<String>,
    pub total_files: i64,
    pub completed_files: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationStats {
    pub total_folders: i64,
    pub total_files: i64,
    pub total_bytes: i64,
    pub completed_files: i64,
    pub completed_bytes: i64,
    pub failed_files: i64,
    pub skipped_duplicates: i64,
    pub pending_files: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderSummary {
    pub source_path: String,
    pub name: String,
    pub file_count: i64,
    pub total_size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationItem {
    pub id: i64,
    pub job_id: i64,
    pub item_type: String, // "file" | "folder"
    pub name: String,
    pub source_path: String,
    pub source_item_id: Option<String>,
    pub size_bytes: i64,
    pub source_etag: Option<String>,
    pub source_last_modified: Option<String>,
    pub source_fingerprint_type: Option<String>,
    pub source_fingerprint_value: Option<String>,
    pub state: String,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    pub attempt_count: i64,
    pub computed_sha256: Option<String>,
    pub telegram_message_id: Option<i64>,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub queue_position: i64,
    pub action_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationJobDetail {
    pub job: MigrationJob,
    pub stats: MigrationStats,
    pub folders: Vec<FolderSummary>,
    pub files: Vec<MigrationItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoMigrationProfile {
    pub id: i64,
    pub account_id: String,
    pub enabled: bool,
    pub default_telegram_dest_id: Option<i64>,
    pub default_telegram_dest_name: Option<String>,
    pub local_temp_dir: Option<String>,
    pub last_auto_scan_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub active_job_id: Option<i64>,
    pub pause_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoMigrationStatus {
    pub profile: Option<AutoMigrationProfile>,
    pub account: Option<MsAccountInfo>,
    pub active_job: Option<MigrationJobDetail>,
    pub scan_progress: Option<ScanProgressPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyMigrationQuota {
    pub date_string: String,
    pub uploaded_bytes: u64,
    pub limit_bytes: u64,
    pub remaining_bytes: u64,
    pub resets_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationActivity {
    pub id: i64,
    pub job_id: i64,
    pub item_id: Option<i64>,
    pub item_name: Option<String>,
    pub phase: String,
    pub status: String,
    pub attempt: i64,
    pub revision: i64,
    pub message: Option<String>,
    pub created_at: i64,
}
