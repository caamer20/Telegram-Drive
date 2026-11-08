export interface SyncSettings {
  enabled: boolean;
  debounceMs: number;
  encryption: 'inherit' | 'always_vault' | string;
  /** `incremental` reuses recorded hashes for unchanged files; `full` is the default. */
  scanner?: 'full' | 'incremental';
}

export interface SyncPair {
  id: number;
  localPath: string;
  channelId: number;
  folderKey: string;
  label: string | null;
  syncDirection: 'bidirectional' | 'upload_only' | 'download_only';
  isActive: boolean;
  createdAt: number;
  accountOwner: string | null;
  preferences: SyncPreferences;
}

export interface SyncStatus {
  enabled: boolean;
  running: boolean;
  activePairs: number;
  pendingOps: number;
  conflicts: number;
  lastError: string | null;
  pairs: SyncPairStatus[];
}

export type SyncDirection = SyncPair['syncDirection'];

export interface SyncPreferences {
  ignorePatterns: string[];
  propagateDeletions: boolean;
  pauseOnConflicts: boolean;
  /** Treat same-path, same-size files as already in sync when a mapping is first reviewed. */
  adoptMatchingFiles?: boolean;
}

export interface SyncPairStatus {
  pairId: number;
  phase: 'waiting' | 'scanning' | 'syncing' | 'paused' | 'ready';
  waitReason?: 'telegram' | null;
  pendingOps: number;
  conflicts: number;
  lastError: string | null;
  lastCheckedAt: number | null;
}

export interface SyncPreviewRequest {
  pairId: number | null;
  localPath: string;
  channelId: number;
  syncDirection: SyncDirection;
  preferences: SyncPreferences;
}

export type SyncPreviewAction = 'upload' | 'download' | 'delete_local' | 'delete_remote' | 'conflict' | 'skip';

export interface SyncPreviewOperation {
  action: SyncPreviewAction;
  relativePath: string;
  detail: string;
}

export interface SyncPreview {
  request: SyncPreviewRequest;
  generatedAt: number;
  reviewToken: string;
  accountOwner: string;
  localFiles: number;
  remoteFiles: number;
  counts: { uploads: number; downloads: number; deleteLocal: number; deleteRemote: number; conflicts: number; skipped: number };
  operations: SyncPreviewOperation[];
  pauseReasons: string[];
  warnings: string[];
}

export interface SyncPairSaveOptions {
  syncDirection: SyncDirection;
  preferences: SyncPreferences;
  previewToken: string;
  isActive: boolean;
}

export interface SyncLogEntry {
  id: number;
  pairId: number | null;
  action: string;
  relativePath: string | null;
  detail: string | null;
  createdAt: number;
}

export interface SyncConflict {
  pairId: number;
  relativePath: string;
  localPath: string;
  label: string | null;
}

export type ConflictResolution = 'keep_local' | 'keep_remote' | 'keep_both';
