import { loadAppStore } from './appStore';
import { invoke } from '@tauri-apps/api/core';
import type { Settings } from '../types/settings';

const SETTINGS_FILE = 'settings.json';
const SETTINGS_KEY = 'settings';

export interface SettingsStore {
  get<T>(key: string): Promise<T | null | undefined>;
  set(key: string, value: unknown): Promise<void>;
  save(): Promise<void>;
}

type StoreLoader = () => Promise<SettingsStore>;

const loadSettingsStore: StoreLoader = () => loadAppStore(SETTINGS_FILE);

// A credential read from disk may only be imported or explicitly discarded.
// It must never become an editable password or a normal proxy Set request.
let legacyPassword = '';
let storeQueue: Promise<unknown> = Promise.resolve();

function useStore<T>(work: () => Promise<T>): Promise<T> {
  const operation = storeQueue.then(work);
  storeQueue = operation.catch(() => undefined);
  return operation;
}

export async function resolveLegacyProxyPassword(action: 'import' | 'clear'): Promise<void> {
  if (!legacyPassword) return;
  await useStore(async () => {
    if (!legacyPassword) return;
    if (action === 'import') {
      await invoke('cmd_migrate_proxy_secret', { password: legacyPassword });
    }
    const store = await loadSettingsStore();
    const stored = await store.get<Partial<Settings>>(SETTINGS_KEY);
    if (stored) {
      await store.set(SETTINGS_KEY, { ...stored, proxyPassword: '' });
      await store.save();
    }
    legacyPassword = '';
  });
}

export function mergeStoredSettings(defaults: Settings, stored?: Partial<Settings> | null): Settings {
  const merged = { ...defaults, ...stored };
  merged.strictUploadOrder = stored?.strictUploadOrder === true;
  if ((merged.proxyType as string) === 'mtproto') merged.proxyType = 'socks5';
  if (!['ask', 'standard', 'encrypted'].includes(merged.uploadProtectionPreference)) {
    merged.uploadProtectionPreference = 'ask';
  }
  return merged;
}

export async function readPersistedSettings(
  defaults: Settings,
  loadStore: StoreLoader = loadSettingsStore,
): Promise<Settings> {
  return useStore(async () => {
    const store = await loadStore();
    const stored = await store.get<Partial<Settings>>(SETTINGS_KEY);
    // A failed save may already have blanked the plugin's in-memory copy.
    // Retain the pending value until its scrub has been flushed successfully.
    if (stored?.proxyPassword) legacyPassword = stored.proxyPassword;
    return mergeStoredSettings(defaults, { ...stored, proxyPassword: '' });
  });
}

export async function writePersistedSettings(
  settings: Settings,
  loadStore: StoreLoader = loadSettingsStore,
): Promise<void> {
  await useStore(async () => {
    const store = await loadStore();
    await store.set(SETTINGS_KEY, { ...settings, proxyPassword: legacyPassword });
    await store.save();
  });
}
