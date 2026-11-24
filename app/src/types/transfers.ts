import type { UploadProtectionIntent } from './encryption';
import type { VideoUploadMode } from './settings';

export type DownloadCollisionPolicy = 'keep_both' | 'skip' | 'replace';
export type DownloadOutcome = 'saved' | 'skipped';

export interface QueueItem {
    ownerId?: string;
  id: string;
  /** Original queue creation time in milliseconds; absent on legacy entries. */
  createdAt?: number;
  path: string;
  url?: string;
  folderId: number | null;
  status: 'pending' | 'paused' | 'waiting_for_network' | 'downloading' | 'uploading' | 'success' | 'error' | 'cancelled' | 'waiting_for_unlock' | 'encrypting' | 'decrypting' | 'verifying';
  error?: string;
  errorCategory?: import('../services/transferActivity').TransferErrorCategory | null;
  progress?: number;
  uploadedBytes?: number;
  totalBytes?: number;
  speedBytesPerSec?: number;
  tempZipPath?: string;
  /** Android-private staged copy that survives activity/process recreation. */
  androidStaged?: boolean;
  protection?: UploadProtectionIntent;
  /** Snapshot of the video preference used when this upload was queued. */
  videoUploadMode?: VideoUploadMode;
  /** Snapshot of whether plaintext video media should use a literal filename caption. */
  videoFilenameCaption?: boolean;
}

export type DroppedPathRejectionReason = 'directory' | 'missing' | 'unreadable' | 'unsupported';

export interface DroppedPathRejection {
  path: string;
  reason: DroppedPathRejectionReason;
}

export interface DroppedPathValidation {
  accepted: string[];
  rejected: DroppedPathRejection[];
}

export interface DropUploadResult {
  queued: number;
  rejected: DroppedPathRejection[];
  cancelled?: boolean;
}

export interface DownloadItem {
    ownerId?: string;
  id: string;
  /** Original queue creation time in milliseconds; absent on legacy entries. */
  createdAt?: number;
  messageId: number;
  filename: string;
  folderId: number | null;
  status: 'pending' | 'paused' | 'waiting_for_network' | 'cooldown' | 'downloading' | 'success' | 'error' | 'cancelled' | 'waiting_for_unlock' | 'decrypting' | 'verifying';
  error?: string;
  errorCategory?: import('../services/transferActivity').TransferErrorCategory | null;
  progress?: number;
  downloadedBytes?: number;
  totalBytes?: number;
  speedBytesPerSec?: number;
  savePath?: string;
  collisionPolicy?: DownloadCollisionPolicy;
  downloadOutcome?: DownloadOutcome;
  protectionMode?: 'vault' | 'passphrase' | 'vault_and_passphrase';
  /** In-memory only. Never persist this single-use credential handle. */
  promptToken?: number;
}
