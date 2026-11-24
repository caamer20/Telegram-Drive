import { invoke } from '@tauri-apps/api/core';
import { load } from '@tauri-apps/plugin-store';

// The SDK saves open stores at exit. Validate this shared file before opening
// a handle, including when a caller only wants a legacy sign-in field.
export async function loadAppStore(path: string) {
  if (path === 'settings.json') await invoke('cmd_reload_missing_settings');
  return load(path);
}
