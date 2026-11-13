import type { LanguagePreference } from '../i18n/languages';

export type VideoUploadMode = 'file' | 'media';
export type UploadProtectionPreference = 'ask' | 'standard' | 'encrypted';

export interface BandwidthWindow {
  days: number[];
  start_minute: number;
  end_minute: number;
  up_kbs: number;
  down_kbs: number;
  pause: boolean;
}

export interface Settings {
  bandwidth_schedule: boolean;
  bandwidthWindows: BandwidthWindow[];
  viewMode: 'grid' | 'list';
  fileSortField: 'name' | 'size' | 'date';
  fileSortDirection: 'asc' | 'desc';
  autoUpdate: boolean;
  maxConcurrentUploads: number;
  maxConcurrentDownloads: number;
  zipFolders: boolean;
  videoUploadMode: VideoUploadMode;
  videoFilenameCaption: boolean;
  language: LanguagePreference;
  crashReportingEnabled: boolean;
  crashReportingConsentSeen: boolean;
  telegramSettingsSyncEnabled: boolean;
  supporterMode: boolean;
  supporterPromptLastShownAt: number;
  driveTourSeen: boolean;
  vaultRecoveryDrillCompleted: boolean;
  /** Vault the completed drill was run against; empty for drills recorded before 4.0. */
  vaultRecoveryDrillVaultId: string;
  webdavPermissionExplained: boolean;
  restPermissionExplained: boolean;
  downloadWebdavTipSeen: boolean;
  proxyEnabled: boolean;
  proxyType: 'socks5' | 'http' | 'https';
  proxyHost: string;
  proxyPort: number;
  proxyUsername: string;
  proxyPassword: string;
  proxyLiveStateEnabled: boolean;
  sidebarCollapsed: boolean;
  hideGroups: boolean;
  vpnMode: boolean;
  timeoutMultiplier: number;
  retryAttempts: number;
  retryBaseBackoffSec: number;
  retryMaxBackoffSec: number;
  adaptivePolling: boolean;
  pollingMinSec: number;
  pollingMaxSec: number;
  preferredDC: 'auto' | 'dc1' | 'dc2' | 'dc3' | 'dc4' | 'dc5';
  dcFallbackAttempts: number;
  floodWaitRespect: boolean;
  peerCacheSize: number;
  bandwidthLimitUpKBs: number;
  bandwidthLimitDownKBs: number;
  chunkSizeKb: number;
  keepAliveIntervalSec: number;
  autoDetectVpn: boolean;
  archiveMaxBytes: number;
  performanceMode: boolean;
  linuxRenderingFix: boolean;
  transcodeCacheMaxGb: number;
  uploadProtectionPreference: UploadProtectionPreference;
  encryptionDefaultMode: 'standard' | 'vault' | 'passphrase' | 'vault_and_passphrase';
  encryptionProtectMetadata: boolean;
  encryptionAutoLockMinutes: number;
  encryptionLockOnSleep: boolean;
  encryptionTempPolicy: 'balanced' | 'strict';
  androidWifiOnlyTransfers: boolean;
  androidAllowRoaming: boolean;
  androidRequireCharging: boolean;
  androidPauseOnLowBattery: boolean;
  androidMinimumFreeStorageGb: number;
  androidBiometricLock: boolean;
  androidPrivacyScreen: boolean;
  androidPrivateMediaMetadata: boolean;
  androidLockAfterBackgroundMinutes: number;
  androidMediaOrientation: 'auto' | 'landscape' | 'portrait';
  androidSubtitleScale: number;
  androidPlaybackSpeed: number;
  androidMediaCacheMaxGb: number;
}

export type Theme = 'light' | 'dark';
export type ThemePreference = Theme | 'system' | 'default';
