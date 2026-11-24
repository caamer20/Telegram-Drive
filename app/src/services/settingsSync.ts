import { invoke } from '@tauri-apps/api/core';
import type { Settings } from '../types/settings';

export const SYNCABLE_SETTING_KEYS = [
  'viewMode',
  'fileSortField',
  'fileSortDirection',
  'autoUpdate',
  'maxConcurrentUploads',
  'strictUploadOrder',
  'maxConcurrentDownloads',
  'zipFolders',
  'videoUploadMode',
  'videoFilenameCaption',
  'language',
  'sidebarCollapsed',
  'hideGroups',
  'vpnMode',
  'timeoutMultiplier',
  'retryAttempts',
  'retryBaseBackoffSec',
  'retryMaxBackoffSec',
  'adaptivePolling',
  'pollingMinSec',
  'pollingMaxSec',
  'preferredDC',
  'dcFallbackAttempts',
  'floodWaitRespect',
  'peerCacheSize',
  'bandwidthLimitUpKBs',
  'bandwidthLimitDownKBs',
  'chunkSizeKb',
  'keepAliveIntervalSec',
  'autoDetectVpn',
  'archiveMaxBytes',
  'performanceMode',
  'linuxRenderingFix',
  'transcodeCacheMaxGb',
  'encryptionDefaultMode',
  'encryptionProtectMetadata',
  'encryptionAutoLockMinutes',
  'encryptionLockOnSleep',
  'encryptionTempPolicy',
] as const satisfies readonly (keyof Settings)[];

export type SyncableSettings = Pick<Settings, (typeof SYNCABLE_SETTING_KEYS)[number]>;

export interface SettingsSyncStatus {
  available: boolean;
  updated_at: number | null;
  device_id: string | null;
  current_device: boolean;
}

export interface SettingsSyncDownload {
  settings: Partial<SyncableSettings>;
  updated_at: number;
  device_id: string;
}

export function pickSyncableSettings(settings: Settings): SyncableSettings {
  return Object.fromEntries(
    SYNCABLE_SETTING_KEYS.map(key => [key, settings[key]]),
  ) as SyncableSettings;
}

export function getSettingsSyncStatus(): Promise<SettingsSyncStatus> {
  return invoke('cmd_get_settings_sync_status');
}

export function uploadSettingsSync(
  settings: Settings,
  passphrase: string,
): Promise<SettingsSyncStatus> {
  return invoke('cmd_upload_settings_sync', {
    settings: pickSyncableSettings(settings),
    passphrase,
  });
}

export function downloadSettingsSync(passphrase: string): Promise<SettingsSyncDownload> {
  return invoke('cmd_download_settings_sync', { passphrase });
}
