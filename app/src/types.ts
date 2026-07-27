export interface TelegramFile {
    id: number;
    name: string;
    size: number;
    sizeStr: string; // Formatted size
    created_at?: string;
    type?: 'folder' | 'file'; // implied icon_type
    folder_id?: number | null;
    // Add other fields if backend sends them
}

export interface TelegramFolder {
    id: number;
    name: string;
    parent_id?: number;
    username?: string;
    /** Whether the channel has a public username set */
    is_public?: boolean;
    group_id?: number | null;
    display_order?: number;
}

export interface FolderGroup {
    id: number;
    name: string;
    color_hex: string;
    display_order: number;
}

export interface FolderInviteInfo {
    link: string;
    is_public: boolean;
    username?: string;
}

export interface QueueItem {
    id: string;
    path: string;
    url?: string;
    folderId: number | null;
    status: 'pending' | 'downloading' | 'uploading' | 'success' | 'error' | 'cancelled';
    error?: string;
    progress?: number; // 0-100
    uploadedBytes?: number;
    totalBytes?: number;
    speedBytesPerSec?: number;
    tempZipPath?: string; // Set when the upload originated from a zipped folder
}

export interface BandwidthStats {
    up_bytes: number;
    down_bytes: number;
}

export interface DownloadItem {
    id: string;
    messageId: number;
    filename: string;
    folderId: number | null;
    status: 'pending' | 'downloading' | 'success' | 'error' | 'cancelled';
    error?: string;
    progress?: number; // 0-100
    downloadedBytes?: number;
    totalBytes?: number;
    speedBytesPerSec?: number;
    savePath?: string;
}
export interface ShareInfo {
    id: string;
    folder_id: number | null;
    message_id: number;
    file_name: string;
    file_size: number;
    created_at: number;
    expires_at: number | null;
    revoked: boolean;
    has_password: boolean;
    link: string;
}

// ── Adaptive streaming types ─────────────────────────────────────────

export type StreamingQuality = '360p' | '480p' | '720p' | '1080p' | 'original';

export interface StreamingSettings {
    quality: StreamingQuality;
    adaptiveMode: boolean;
}

export interface VideoTrackInfo {
    id: number;
    type: 'video' | 'audio';
    width?: number;
    height?: number;
    bitrate?: number;
    codec?: string;
    duration?: number;
}

/** Bandwidth cap in kilobits per second for each quality preset. 0 = unlimited. */
export const QUALITY_THROTTLE_MAP: Record<StreamingQuality, number> = {
    '360p': 500,
    '480p': 1000,
    '720p': 2500,
    '1080p': 5000,
    'original': 0,
};

/** Thresholds for adaptive quality switching (check from highest to lowest). */
export const ADAPTIVE_THRESHOLDS: { minKbps: number; quality: StreamingQuality }[] = [
    { minKbps: 4000, quality: '1080p' },
    { minKbps: 2000, quality: '720p' },
    { minKbps: 800, quality: '480p' },
    { minKbps: 0, quality: '360p' },
];

export const QUALITY_LABELS: Record<StreamingQuality, string> = {
    '360p': '360p',
    '480p': '480p',
    '720p': '720p',
    '1080p': '1080p',
    'original': 'Original',
};

export const HLS_QUALITIES: StreamingQuality[] = ['360p', '480p', '720p', '1080p'];

// ── Transcode types (HLS backend) ────────────────────────────────────

export interface TranscodeCapabilities {
    available: boolean;
    variants: QualityVariant[];
    mode: 'hls' | 'original';
}

export interface QualityVariant {
    label: string;
    height: number;
    available: boolean;
}

export interface TranscodePrepareResult {
    job_id: string;
    status: 'started' | 'pending' | 'caching' | 'transcoding' | 'ready' | 'error' | 'cancelled';
    progress: number;
    playlist_url: string | null;
}

export interface TranscodeStatusResult {
    job_id: string;
    status: 'pending' | 'caching' | 'transcoding' | 'ready' | 'error' | 'cancelled';
    progress: number;
    error: string | null;
    playlist_url: string | null;
}

export interface MasterPlaylistInfo {
    file_key: string;
    variants: MasterVariant[];
    master_playlist_url: string | null;
}

export interface MasterVariant {
    bandwidth: number;
    resolution: string;
    quality: string;
    playlist_path: string;
}

export interface CacheEntry {
    file_key: string;
    quality: string;
    size_bytes: number;
    playlist_exists: boolean;
}

export interface DetailedCacheInfo {
    entries: CacheEntry[];
    total_bytes: number;
    max_bytes: number;
}

export type TranscodeJobPhase = 'idle' | 'preparing' | 'caching' | 'transcoding' | 'ready' | 'failed';

// ── Rust command return types ────────────────────────────────────────

export interface ArchiveEntry {
    filename: string;
    size: number;
    compressed_size: number;
    is_dir: boolean;
}

export interface VideoMetadata {
    duration_secs: number | null;
    video_codec: string | null;
    has_audio: boolean;
    track_count: number;
    width: number | null;
    height: number | null;
}

// ── OneDrive Migration Types ─────────────────────────────────────────

export interface MsAccountInfo {
    account_name: string;
    account_email: string;
}

export interface ProcessingLogEntry {
    id: string;
    timestamp: number;
    category: 'scan' | 'download' | 'processing' | 'upload' | 'job' | 'system';
    level: 'info' | 'success' | 'warning' | 'error';
    message_key: string;
    params?: Record<string, string | number>;
}

export interface DailyMigrationQuota {
    date_string: string;
    uploaded_bytes: number;
    limit_bytes: number;
    remaining_bytes: number;
    resets_at: number;
}

export interface MigrationActivity {
    id: number;
    job_id: number;
    item_id?: number | null;
    item_name?: string | null;
    phase: 'scan' | 'downloading' | 'processing' | 'uploading' | 'completed' | 'failed' | 'quota';
    status: string;
    attempt: number;
    revision: number;
    message?: string | null;
    created_at: number;
}


export interface OneDriveItem {
    id: string;
    name: string;
    item_type: 'folder' | 'file';
    size: number;
    path?: string | null;
    child_count?: number | null;
    etag?: string | null;
    quickxor_hash?: string | null;
    sha1_hash?: string | null;
    last_modified?: string | null;
}

export interface OneDriveFolder {
    id: string;
    name: string;
    source_path: string;
    file_count: number;
    total_size: number;
}

export type JobState = 'running' | 'completed' | 'completed_with_errors' | 'stopped' | 'waiting_for_quota' | 'failed';
export type ItemState = 'discovered' | 'queued_download' | 'downloading' | 'downloaded' | 'queued_processing' | 'processing' | 'processed' | 'queued_upload' | 'uploading' | 'waiting_for_quota' | 'saving_local' | 'completed_telegram' | 'completed_local' | 'reconciliation_required' | 'failed';

export interface MigrationJob {
    id: number;
    source_folder_id: string;
    source_folder_path: string;
    telegram_destination_id: number | null;
    telegram_destination_name: string;
    local_backup_dir: string;
    workspace_dir: string;
    state: string;
    started_at: number;
    completed_at?: number | null;
    last_error?: string | null;
    flood_wait_until?: number | null;
    discovered_folders: number;
    completed_folders: number;
    discovered_items: number;
    completed_items: number;
    failed_items: number;
    waiting_items: number;
    created_at: number;
    updated_at: number;
}

export interface MigrationJobSummary {
    id: number;
    state: string;
    source_folder_path: string;
    total_files: number;
    completed_files: number;
    created_at: number;
}

export interface MigrationStats {
    total_folders: number;
    total_files: number;
    total_bytes: number;
    completed_telegram: number;
    completed_local: number;
    completed_bytes: number;
    failed_files: number;
    waiting_files: number;
    pending_files: number;
}

export interface FolderSummary {
    source_path: string;
    name: string;
    file_count: number;
    total_size: number;
}

export interface MigrationItem {
    id: number;
    job_id: number;
    folder_id: string;
    source_item_id: string;
    name: string;
    path: string;
    size: number;
    item_category: string;
    pipeline_stage: string;
    original_artifact_path?: string | null;
    processed_artifact_path?: string | null;
    original_sha256?: string | null;
    processed_sha256?: string | null;
    video_decision?: string | null;
    artifact_size?: number | null;
    telegram_attempt_id?: string | null;
    telegram_random_id?: number | null;
    telegram_message_id?: number | null;
    retry_count: number;
    last_error?: string | null;
    created_at: number;
    updated_at: number;
    completed_at?: number | null;
}

export interface MigrationJobDetail {
    job: MigrationJob;
    stats: MigrationStats;
    folders: FolderSummary[];
    files: MigrationItem[];
}

export interface JobStatePayload {
    job_id: number;
    state: JobState;
    previous_state: JobState;
}

export interface ItemProgressPayload {
    job_id: number;
    item_id: number;
    item_name: string;
    phase: 'downloading' | 'analyzing' | 'processing' | 'uploading';
    percent: number;
    bytes_done: number;
    bytes_total: number;
    speed_bytes_per_sec: number;
    event_id?: string;
    attempt?: number;
    revision?: number;
    timestamp: number;
}

export interface ItemCompletePayload {
    job_id: number;
    item_id: number;
    item_name: string;
    status: ItemState;
    error_type?: string;
    error_message?: string;
}

export interface StatsPayload {
    job_id: number;
    stats: MigrationStats;
}

export interface CooldownPayload {
    job_id: number;
    cooldown_until: number | null;
    seconds_remaining: number;
}
