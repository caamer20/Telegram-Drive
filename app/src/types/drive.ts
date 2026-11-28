/**
 * Drive-wide analytics and soft-delete ("Trash") types.
 *
 * These mirror the Rust `models::DriveStats` / `models::TrashStats` payloads
 * returned by `cmd_get_drive_stats` and `cmd_get_trash_stats`.
 */

export interface GlobalStats {
  total_size: number;
  image_count: number;
  image_size: number;
  video_count: number;
  video_size: number;
  music_count: number;
  music_size: number;
  document_count: number;
  document_size: number;
  app_count: number;
  app_size: number;
  exe_count: number;
  exe_size: number;
  other_count: number;
  other_size: number;
  total_files: number;
}

export interface FolderStatItem {
  total_size: number;
  total_files: number;
}

export interface DriveStats {
  global: GlobalStats;
  /** Keyed by folder id as a string; `"null"` is Saved Messages. */
  folders: Record<string, FolderStatItem>;
}

export interface TrashStats {
  count: number;
  total_size: number;
}

export interface TrashItem {
  trash_message_id: number;
  trash_folder_id: number;
  original_folder_id: number;
  original_message_id: number;
  file_name: string;
  file_size: number;
  created_at: string;
  expires_at: string;
}

export interface SharedFolderInfo {
  folderId: number;
  folderName: string;
  inviteLink: string;
  createdAt: number;
  fileCount: number;
  totalSize: number;
}

/** A single entry in the animated background slideshow. */
export interface BgItem {
  src: string;
  type: 'image' | 'video';
  duration: number;
  messageId?: number;
  localPath?: string;
}
