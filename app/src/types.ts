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

export interface ScanProgressPayload {
    phase: 'starting' | 'enumerating' | 'building_snapshot' | 'stopping' | 'stopped' | 'completed' | 'failed';
    pages_scanned: number;
    discovered_files: number;
    discovered_folders: number;
    elapsed_ms: number;
}

export interface ProcessingLogEntry {
    id: string;
    timestamp: number;
    category: 'scan' | 'download' | 'upload' | 'job' | 'system';
    level: 'info' | 'success' | 'warning' | 'error';
    message_key: string;
    params?: Record<string, string | number>;
}

export interface AutoMigrationProfile {
    id: number;
    account_id: string;
    enabled: boolean;
    default_telegram_dest_id?: number | null;
    default_telegram_dest_name?: string | null;
    local_temp_dir?: string | null;
    last_auto_scan_at?: number | null;
    created_at: number;
    updated_at: number;
    active_job_id?: number | null;
    pause_reason?: string | null;
}

export interface AutoMigrationStatus {
    profile: AutoMigrationProfile | null;
    account: MsAccountInfo | null;
    active_job: MigrationJobDetail | null;
    scan_progress: ScanProgressPayload | null;
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
    phase: 'scan' | 'downloading' | 'uploading' | 'completed' | 'failed' | 'quota';
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

export type JobState = 'draft' | 'ready' | 'running' | 'paused' | 'completed' | 'cancelled' | 'failed';
export type ItemState = 'pending' | 'downloading' | 'uploading' | 'completed' | 'skipped_duplicate' | 'failed';

export interface MigrationJob {
    id: number;
    state: JobState;
    onedrive_folder_id?: string | null;
    onedrive_folder_path?: string | null;
    telegram_destination_id?: number | null;
    telegram_destination_name?: string | null;
    local_dir?: string | null;
    cooldown_until?: number | null;
    created_at: number;
    started_at?: number | null;
    completed_at?: number | null;
    updated_at: number;
    job_origin: 'manual' | 'auto';
    pause_reason?: string | null;
}

export interface MigrationJobSummary {
    id: number;
    state: JobState;
    onedrive_folder_path?: string | null;
    total_files: number;
    completed_files: number;
    created_at: number;
}

export interface MigrationStats {
    total_folders: number;
    total_files: number;
    total_bytes: number;
    completed_files: number;
    completed_bytes: number;
    failed_files: number;
    skipped_duplicates: number;
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
    item_type: 'file' | 'folder';
    name: string;
    source_path: string;
    source_item_id?: string | null;
    size_bytes: number;
    source_etag?: string | null;
    source_last_modified?: string | null;
    source_fingerprint_type?: string | null;
    source_fingerprint_value?: string | null;
    state: ItemState;
    last_error_code?: string | null;
    last_error_message?: string | null;
    attempt_count: number;
    computed_sha256?: string | null;
    telegram_message_id?: number | null;
    created_at: number;
    completed_at?: number | null;
    queue_position: number;
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
    phase: 'downloading' | 'uploading';
    percent: number;
    bytes_done: number;
    bytes_total: number;
    speed_bytes_per_sec: number;
    event_id: string;
    attempt: number;
    revision: number;
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
