import { load } from '@tauri-apps/plugin-store';
import type { Settings } from '../types/settings';

const SETTINGS_FILE = 'settings.json';
const SETTINGS_KEY = 'settings';

export interface SettingsStore {
  get<T>(key: string): Promise<T | null | undefined>;
  set(key: string, value: unknown): Promise<void>;
  save(): Promise<void>;
}

type StoreLoader = () => Promise<SettingsStore>;

const loadSettingsStore: StoreLoader = () => load(SETTINGS_FILE);

// A legacy password is retained only until the Rust backend confirms that it
// was migrated into secure credential storage. New passwords are never written
// to the plugin-store JSON file.
let unmigratedLegacyProxyPassword: string | null = null;

export function markProxySecretMigrated(): void {
  unmigratedLegacyProxyPassword = null;
}

export function settingsForPersistence(settings: Settings): Settings {
  const passwordIsUnchangedLegacy = unmigratedLegacyProxyPassword !== null
    && settings.proxyPassword === unmigratedLegacyProxyPassword;
  if (!passwordIsUnchangedLegacy) {
    // Once the user replaces or clears a legacy value, never write the old
    // credential back over the newly secured value.
    unmigratedLegacyProxyPassword = null;
  }
  return {
    ...settings,
    proxyPassword: passwordIsUnchangedLegacy ? settings.proxyPassword : '',
  };
}

export function mergeStoredSettings(defaults: Settings, stored?: Partial<Settings> | null): Settings {
  const merged = { ...defaults, ...stored };
  if ((merged.proxyType as string) === 'mtproto') merged.proxyType = 'socks5';
  if (!['ask', 'standard', 'encrypted'].includes(merged.uploadProtectionPreference)) {
    merged.uploadProtectionPreference = 'ask';
  }
  return merged;
}

export async function readPersistedSettings(
  defaults: Settings,
  loadStore: StoreLoader = loadSettingsStore,
  onError?: (error: unknown) => void,
): Promise<Settings> {
  try {
    const store = await loadStore();
    const stored = await store.get<Partial<Settings>>(SETTINGS_KEY);
    unmigratedLegacyProxyPassword = stored?.proxyPassword || null;
    return mergeStoredSettings(defaults, stored);
  } catch (error) {
    onError?.(error);
    return defaults;
  }
}

export async function writePersistedSettings(
  settings: Settings,
  loadStore: StoreLoader = loadSettingsStore,
): Promise<void> {
  const store = await loadStore();
  await store.set(SETTINGS_KEY, settingsForPersistence(settings));
  await store.save();
}
