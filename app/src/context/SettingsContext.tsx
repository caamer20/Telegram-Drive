import { createContext, useContext, useState, useEffect, ReactNode, useCallback } from 'react';
import { load } from '@tauri-apps/plugin-store';

export interface Settings {
    viewMode: 'grid' | 'list';
    autoUpdate: boolean;
    maxConcurrentUploads: number;
    maxConcurrentDownloads: number;
    zipFolders: boolean;
    deleteAfterUpload: boolean;
    uploadVideoAsMedia: boolean;

    // ── Proxy ──────────────────────────────────────────────
    proxyEnabled: boolean;
    proxyType: 'socks5' | 'mtproto';
    proxyHost: string;
    proxyPort: number;
    proxyUsername: string;
    proxyPassword: string;   // SOCKS5
    proxySecret: string;     // MTProto

    // ── VPN Optimizer (master toggle) ─────────────────────
    vpnMode: boolean;

    // Individual controls (active only when vpnMode = true)
    timeoutMultiplier: number;       // 1–5
    retryAttempts: number;           // 0–5
    retryBaseBackoffSec: number;     // 0.5–5
    retryMaxBackoffSec: number;      // 8–60
    adaptivePolling: boolean;
    pollingMinSec: number;           // 10–30
    pollingMaxSec: number;           // 45–120
    preferredDC: 'auto' | 'dc1' | 'dc2' | 'dc3' | 'dc4' | 'dc5';
    dcFallbackAttempts: number;      // 1–4
    floodWaitRespect: boolean;
    peerCacheSize: number;           // 100–2000
    bandwidthLimitUpKBs: number;     // 0 = unlimited, KB/s
    bandwidthLimitDownKBs: number;   // 0 = unlimited, KB/s
    chunkSizeKb: number;             // 128, 256, 512
    keepAliveIntervalSec: number;    // 0 = disabled, 30–120
    autoDetectVpn: boolean;
}

const defaultSettings: Settings = {
    viewMode: 'grid',
    autoUpdate: true,
    maxConcurrentUploads: 6,
    maxConcurrentDownloads: 6,
    zipFolders: true,
    deleteAfterUpload: false,
    uploadVideoAsMedia: true,

    // Proxy — off by default
    proxyEnabled: false,
    proxyType: 'socks5',
    proxyHost: '',
    proxyPort: 1080,
    proxyUsername: '',
    proxyPassword: '',
    proxySecret: '',

    // VPN Optimizer — off by default (preserves existing behaviour)
    vpnMode: false,
    timeoutMultiplier: 3,
    retryAttempts: 3,
    retryBaseBackoffSec: 1,
    retryMaxBackoffSec: 30,
    adaptivePolling: true,
    pollingMinSec: 15,
    pollingMaxSec: 60,
    preferredDC: 'auto',
    dcFallbackAttempts: 2,
    floodWaitRespect: true,
    peerCacheSize: 500,
    bandwidthLimitUpKBs: 0,
    bandwidthLimitDownKBs: 0,
    chunkSizeKb: 512,
    keepAliveIntervalSec: 0,
    autoDetectVpn: false,
};

interface SettingsContextType {
    settings: Settings;
    updateSetting: <K extends keyof Settings>(key: K, value: Settings[K]) => void;
    resetSettings: () => void;
    isLoaded: boolean;
}

const SettingsContext = createContext<SettingsContextType | undefined>(undefined);

export function SettingsProvider({ children }: { children: ReactNode }) {
    const [settings, setSettings] = useState<Settings>(defaultSettings);
    const [isLoaded, setIsLoaded] = useState(false);

    // Load settings from Tauri store on mount
    useEffect(() => {
        const loadSettings = async () => {
            try {
                const store = await load('settings.json');
                const saved = await store.get<Settings>('settings');
                if (saved) {
                    // Merge with defaults so new keys are always present
                    setSettings({ ...defaultSettings, ...saved });
                }
            } catch {
                // Store not available or first run — use defaults
            } finally {
                setIsLoaded(true);
            }
        };
        loadSettings();
    }, []);

    const persistSettings = useCallback(async (next: Settings) => {
        try {
            const store = await load('settings.json');
            await store.set('settings', next);
            await store.save();
        } catch {
            // best-effort persistence
        }
    }, []);

    const updateSetting = useCallback(<K extends keyof Settings>(key: K, value: Settings[K]) => {
        setSettings(prev => {
            const next = { ...prev, [key]: value };
            persistSettings(next);
            return next;
        });
    }, [persistSettings]);

    const resetSettings = useCallback(() => {
        setSettings(defaultSettings);
        persistSettings(defaultSettings);
    }, [persistSettings]);

    return (
        <SettingsContext.Provider value={{ settings, updateSetting, resetSettings, isLoaded }}>
            {children}
        </SettingsContext.Provider>
    );
}

export const useSettings = () => {
    const context = useContext(SettingsContext);
    if (!context) throw new Error('useSettings must be used within a SettingsProvider');
    return context;
};
