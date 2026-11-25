import { createContext, useContext, useState, useEffect, ReactNode, useCallback, useRef } from 'react';
import { clearProxyPassword } from '../services/networkSettings';
import { DEFAULT_SETTINGS } from '../config/defaultSettings';
import { configureCrashTelemetry } from '../services/crashTelemetry';
import {
    readPersistedSettings,
    resolveLegacyProxyPassword,
    writePersistedSettings,
} from '../services/settingsPersistence';
import type { Settings } from '../types/settings';

export type { Settings } from '../types/settings';

interface SettingsContextType {
    settings: Settings;
    updateSetting: <K extends keyof Settings>(key: K, value: Settings[K]) => void;
    updateSettings: (updates: Partial<Settings>) => void;
    setCrashReportingConsent: (allow: boolean) => void;
    resetSettings: () => void;
    isLoaded: boolean;
    hasStoredSettings: boolean;
    persistenceStatus: 'loading' | 'saved' | 'saving' | 'error';
    retryPersistence: () => Promise<void>;
}

const SettingsContext = createContext<SettingsContextType | undefined>(undefined);

export function SettingsProvider({ children }: { children: ReactNode }) {
    const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
    const [isLoaded, setIsLoaded] = useState(false);
    const [hasStoredSettings, setHasStoredSettings] = useState(false);
    // Before the first successful read, defaults are only a display fallback.
    const unmergedEdits = useRef<Partial<Settings> | null>({});
    const latestSettingsRef = useRef<Settings>(DEFAULT_SETTINGS);
    const acceptStoredSettings = useCallback((stored: Settings) => {
        unmergedEdits.current = null;
        setHasStoredSettings(true);
        setSettings(stored);
        latestSettingsRef.current = stored;
        configureCrashTelemetry(stored.crashReportingConsentSeen === true && stored.crashReportingEnabled === true, stored.crashReportingConsentId);
    }, []);
    const [persistenceStatus, setPersistenceStatus] = useState<'loading' | 'saved' | 'saving' | 'error'>('loading');
    const persistenceQueueRef = useRef<Promise<void>>(Promise.resolve());
    const persistenceRevisionRef = useRef(0);
    const retryOperationRef = useRef<'load' | 'save'>('load');
    const loadingPromiseRef = useRef<Promise<void> | null>(null);

    const loadSettings = useCallback((): Promise<void> => {
        if (loadingPromiseRef.current) return loadingPromiseRef.current;
        const revision = ++persistenceRevisionRef.current;
        retryOperationRef.current = 'load';
        setPersistenceStatus('loading');
        const operation = (async () => {
            let loadFailed = false;
            const loaded = await readPersistedSettings(DEFAULT_SETTINGS).catch(() => {
                loadFailed = true;
                return DEFAULT_SETTINGS;
            });
            const readFailed = loadFailed;
            if (revision !== persistenceRevisionRef.current) {
                setIsLoaded(true);
                return;
            }
            if (!readFailed) {
                try {
                    await resolveLegacyProxyPassword('import');
                } catch (error) {
                    // Proxy settings show the specific Clear action for conflicting credentials.
                    // Other storage failures keep the existing application-level load retry.
                    loadFailed = error !== 'proxy_legacy_password_requires_clear';
                }
            }
            setIsLoaded(true);
            if (revision !== persistenceRevisionRef.current) return;
            if (!readFailed) acceptStoredSettings(loaded);
            setPersistenceStatus(loadFailed ? 'error' : 'saved');
            if (loadFailed) throw new Error('Settings loading failed');
            retryOperationRef.current = 'save';
        })();
        loadingPromiseRef.current = operation;
        void operation.then(
            () => { loadingPromiseRef.current = null; },
            () => { loadingPromiseRef.current = null; },
        );
        return operation;
    }, [acceptStoredSettings]);

    useEffect(() => {
        void loadSettings().catch(() => undefined);
    }, [loadSettings]);

    const persistSettings = useCallback((next: Settings, edits?: Partial<Settings>): Promise<void> => {
        latestSettingsRef.current = next;
        const revision = ++persistenceRevisionRef.current;
        // An explicit edit/reset is now the state to retry, even after a failed read.
        retryOperationRef.current = 'save';
        setPersistenceStatus('saving');
        const pending = unmergedEdits.current && { ...unmergedEdits.current, ...edits };
        unmergedEdits.current = pending;
        const operation = persistenceQueueRef.current.then(async () => {
            // Retry the read before an unrelated edit can overwrite saved settings.
            const saved = pending && next !== DEFAULT_SETTINGS
                ? { ...await readPersistedSettings(DEFAULT_SETTINGS), ...pending }
                : next;
            await writePersistedSettings(saved);
            if (pending && revision === persistenceRevisionRef.current) acceptStoredSettings(saved);
        });
        persistenceQueueRef.current = operation.catch(() => undefined);
        return operation.then(
            () => {
                if (revision === persistenceRevisionRef.current) setPersistenceStatus('saved');
            },
            () => {
                if (revision === persistenceRevisionRef.current) setPersistenceStatus('error');
                throw new Error('Settings persistence failed');
            },
        );
    }, [acceptStoredSettings]);

    const updateSetting = useCallback(<K extends keyof Settings>(key: K, value: Settings[K]) => {
        setSettings(prev => {
            const next = { ...prev, [key]: value };
            void persistSettings(next, { [key]: value } as Partial<Settings>).catch(() => undefined);
            return next;
        });
    }, [persistSettings]);

    const updateSettings = useCallback((updates: Partial<Settings>) => {
        setSettings(prev => {
            const next = { ...prev, ...updates };
            void persistSettings(next, updates).catch(() => undefined);
            return next;
        });
    }, [persistSettings]);

    const setCrashReportingConsent = useCallback((allow: boolean) => {
        const grant = allow ? crypto.getRandomValues(new Uint32Array(4)).join('-') : undefined;
        configureCrashTelemetry(allow, grant);
        updateSettings({ crashReportingEnabled: allow, crashReportingConsentSeen: true, crashReportingConsentId: grant });
    }, [updateSettings]);

    const resetSettings = useCallback(() => {
        configureCrashTelemetry(false);
        setSettings(DEFAULT_SETTINGS);
        latestSettingsRef.current = DEFAULT_SETTINGS;
        void persistSettings(DEFAULT_SETTINGS, DEFAULT_SETTINGS).catch(() => undefined);
        void clearProxyPassword().catch(() => {
            console.error('[Settings] Unable to remove the saved proxy credential.');
        });
    }, [persistSettings]);

    const retryPersistence = useCallback(
        () => retryOperationRef.current === 'load' ? loadSettings() : persistSettings(latestSettingsRef.current),
        [loadSettings, persistSettings],
    );

    return (
        <SettingsContext.Provider value={{ settings, updateSetting, updateSettings, setCrashReportingConsent, resetSettings, isLoaded, hasStoredSettings, persistenceStatus, retryPersistence }}>
            {children}
        </SettingsContext.Provider>
    );
}

export const useSettings = () => {
    const context = useContext(SettingsContext);
    if (!context) throw new Error('useSettings must be used within a SettingsProvider');
    return context;
};
