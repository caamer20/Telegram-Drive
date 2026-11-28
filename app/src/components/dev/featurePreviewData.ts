/**
 * Mock Tauri IPC for the browser-only feature preview (`?feature-preview`).
 *
 * The ported screens call Rust commands through `@tauri-apps/api`. In a plain
 * browser there is no backend, so we stub `window.__TAURI_INTERNALS__` with
 * representative sample data. This file is imported only for the preview route
 * and never affects the packaged app.
 */
import type { DriveStats, TrashStats, TrashItem, SharedFolderInfo, TelegramFolder } from '../../types';

export const PREVIEW_FOLDERS: TelegramFolder[] = [
  { id: 101, name: 'Movies', is_public: true, username: 'my_movies' },
  { id: 102, name: 'Photos 2026', is_public: false },
  { id: 103, name: 'Work Docs', is_public: false },
  { id: 104, name: 'Music', is_public: true },
  { id: 105, name: 'Backups', is_public: false },
  { id: 106, name: 'Design Assets', is_public: false },
];

export const PREVIEW_DRIVE_STATS: DriveStats = {
  global: {
    total_size: 84_213_776_384,
    image_count: 1284, image_size: 6_442_450_944,
    video_count: 96, video_size: 52_428_800_000,
    music_count: 412, music_size: 4_294_967_296,
    document_count: 318, document_size: 1_073_741_824,
    app_count: 22, app_size: 2_147_483_648,
    exe_count: 14, exe_size: 6_871_947_264,
    other_count: 51, other_size: 10_954_286_408,
    total_files: 2197,
  },
  folders: {
    null: { total_size: 12_884_901_888, total_files: 320 },
    '101': { total_size: 31_138_250_752, total_files: 180 },
    '102': { total_size: 8_589_934_592, total_files: 1044 },
    '103': { total_size: 644_245_094, total_files: 260 },
    '104': { total_size: 4_294_967_296, total_files: 412 },
    '105': { total_size: 22_548_578_304, total_files: 40 },
    '106': { total_size: 4_112_896_000, total_files: 118 },
  },
};

export const PREVIEW_TRASH_STATS: TrashStats = { count: 3, total_size: 754_974_720 };

export const PREVIEW_TRASH: TrashItem[] = [
  { trash_message_id: 9001, trash_folder_id: 777, original_folder_id: 102, original_message_id: 501, file_name: 'IMG_2026_beach.jpg', file_size: 4_194_304, created_at: '1764200000', expires_at: `${Math.floor(Date.now() / 1000) + 12 * 86400}` },
  { trash_message_id: 9002, trash_folder_id: 777, original_folder_id: 101, original_message_id: 88, file_name: 'old_trailer.mp4', file_size: 734_003_200, created_at: '1764200000', expires_at: `${Math.floor(Date.now() / 1000) + 5 * 86400}` },
  { trash_message_id: 9003, trash_folder_id: 777, original_folder_id: 0, original_message_id: 12, file_name: 'notes.txt', file_size: 16_777_216, created_at: '1764200000', expires_at: `${Math.floor(Date.now() / 1000) + 18 * 86400}` },
];

export const PREVIEW_SHARED: SharedFolderInfo[] = [
  { folderId: 101, folderName: 'Movies', inviteLink: 'https://t.me/+abc', createdAt: 1764200000, fileCount: 180, totalSize: 31_138_250_752 },
  { folderId: 104, folderName: 'Music', inviteLink: 'https://t.me/+def', createdAt: 1764200000, fileCount: 412, totalSize: 4_294_967_296 },
];

export const PREVIEW_GROUPS = [
  { id: 1, name: 'Media', color_hex: '#3B82F6', display_order: 0 },
  { id: 2, name: 'Work', color_hex: '#8B5CF6', display_order: 1 },
];

export const PREVIEW_FILES = [
  { id: 2001, name: 'Interstellar_2014_4K.mkv', size: 16_106_127_360, sizeStr: '15 GB', type: 'file' as const, folder_id: 101, file_ext: 'mkv', mime_type: 'video/x-matroska', created_at: '2026-11-02 21:10', is_pinned: true },
  { id: 2002, name: 'Dune_Part_Two.mp4', size: 9_126_805_504, sizeStr: '8.5 GB', type: 'file' as const, folder_id: 101, file_ext: 'mp4', mime_type: 'video/mp4', created_at: '2026-11-01 19:04' },
  { id: 2003, name: 'IMG_4421.jpg', size: 6_291_456, sizeStr: '6 MB', type: 'file' as const, folder_id: 102, file_ext: 'jpg', mime_type: 'image/jpeg', created_at: '2026-10-28 12:33' },
  { id: 2004, name: 'Team_Budget_2026.xlsx', size: 482_344, sizeStr: '471 KB', type: 'file' as const, folder_id: 103, file_ext: 'xlsx', mime_type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', created_at: '2026-10-30 09:15', is_favorite: true },
  { id: 2005, name: 'Product_Roadmap.pdf', size: 2_411_724, sizeStr: '2.3 MB', type: 'file' as const, folder_id: 103, file_ext: 'pdf', mime_type: 'application/pdf', created_at: '2026-10-29 16:40' },
  { id: 2006, name: 'Nightcall.mp3', size: 9_384_921, sizeStr: '8.9 MB', type: 'file' as const, folder_id: 104, file_ext: 'mp3', mime_type: 'audio/mpeg', created_at: '2026-10-25 22:12' },
  { id: 2007, name: 'app-release.apk', size: 58_720_256, sizeStr: '56 MB', type: 'file' as const, folder_id: 106, file_ext: 'apk', mime_type: 'application/vnd.android.package-archive', created_at: '2026-10-20 08:02' },
  { id: 2008, name: 'backup_archive.zip', size: 1_610_612_736, sizeStr: '1.5 GB', type: 'file' as const, folder_id: 105, file_ext: 'zip', mime_type: 'application/zip', created_at: '2026-10-18 03:44' },
];

const SUPPORTER_STATUS = {
  state: 'inactive',
  ad_free: true,
  message: '',
  terms_version: '1',
  terms_url: null,
  expires_at: null,
  offline_until: null,
  recovery_code_saved: true,
  checkout_pending: false,
};

const RESPONSES: Record<string, unknown> = {
  cmd_get_supporter_status: SUPPORTER_STATUS,
  cmd_refresh_supporter: SUPPORTER_STATUS,
  cmd_poll_supporter_checkout: { status: 'idle' },
  cmd_get_drive_stats: PREVIEW_DRIVE_STATS,
  cmd_workspace_account: 'preview-owner',
  cmd_check_connection: true,
  cmd_get_enriched_folders: PREVIEW_FOLDERS,
  cmd_scan_folders: PREVIEW_FOLDERS,
  cmd_get_groups: PREVIEW_GROUPS,
  cmd_get_trash_stats: PREVIEW_TRASH_STATS,
  cmd_list_shared_folders: PREVIEW_SHARED,
  cmd_list_trash: PREVIEW_TRASH,
  cmd_get_trash_count: PREVIEW_TRASH.length,
  cmd_restore_from_trash: true,
  cmd_purge_expired_trash: true,
  cmd_load_backgrounds: [],
  cmd_save_backgrounds: null,
};

let installed = false;

export function installFeaturePreviewMock(): void {
  if (installed || typeof window === 'undefined') return;
  installed = true;

  const w = window as unknown as {
    __TAURI_INTERNALS__?: Record<string, any>;
    __TAURI_EVENT_PLUGIN_INTERNALS__?: Record<string, unknown>;
  };

  if (!w.__TAURI_EVENT_PLUGIN_INTERNALS__) {
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: () => undefined,
      emit: async () => undefined,
      emitTo: async () => undefined,
    };
  }

  const callbacks: Record<number, { cb: (data: unknown) => void; once: boolean }> = {};
  let nextId = 1;

  w.__TAURI_INTERNALS__ = {
    callbacks,
    transformCallback(cb: (data: unknown) => void, once = false) { const id = nextId++; callbacks[id] = { cb, once }; return id; },
    unregisterCallback(id: number) { delete callbacks[id]; },
    runCallback(id: number, data: unknown) { const e = callbacks[id]; if (e) { if (e.once) delete callbacks[id]; e.cb(data); } },
    convertFileSrc: (path: string) => path,
    metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
    plugins: {},
    invoke: async (cmd: string, args?: { folderId?: number | null; requestId?: string; ownerId?: string }) => {
      if (cmd === 'cmd_get_cached_files') {
        const folderId = args?.folderId ?? null;
        return folderId == null ? PREVIEW_FILES : PREVIEW_FILES.filter(f => f.folder_id === folderId);
      }
      if (cmd === 'cmd_get_files') {
        const folderId = args?.folderId ?? null;
        const files = folderId == null ? PREVIEW_FILES : PREVIEW_FILES.filter(f => f.folder_id === folderId);
        return { ownerId: 'preview-owner', folderId, requestId: args?.requestId ?? 'preview', complete: true, files };
      }
      if (cmd in RESPONSES) return RESPONSES[cmd];
      if (cmd === 'cmd_upload_background_file' || cmd === 'cmd_download_background_file') {
        throw new Error('Preview: background upload/download disabled');
      }
      return null;
    },
  };
}
