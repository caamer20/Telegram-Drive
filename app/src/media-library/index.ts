import { invoke } from '@tauri-apps/api/core';

export interface MediaLibraryResult {
  exitReason: 'back' | 'closed' | 'error' | string;
  accountId?: number | null;
  error?: string | null;
}

export interface MediaLibraryState {
  status: 'closed' | 'opening' | 'open' | 'offline' | string;
  isOpen: boolean;
  accountId?: number | null;
  online: boolean;
  syncRunning: boolean;
}

export function openMediaLibrary(): Promise<MediaLibraryResult> {
  return invoke<MediaLibraryResult>('plugin:media-library|open_media_library');
}

export async function closeMediaLibrary(): Promise<void> {
  await invoke('plugin:media-library|close_media_library');
}

export function getMediaLibraryState(): Promise<MediaLibraryState> {
  return invoke<MediaLibraryState>('plugin:media-library|get_media_library_state');
}

export async function clearMediaLibraryData(): Promise<void> {
  await invoke('plugin:media-library|clear_media_library_data');
}
