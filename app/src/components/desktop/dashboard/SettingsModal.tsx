import { useState, useEffect, useCallback, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { X, RotateCcw, Download, Upload, Trash2, HardDrive, Globe, Key, Copy, Check, RefreshCw, FolderArchive, Shield, Zap, Activity, Gauge, Wifi, ChevronDown, Link, Sparkles, Info, Monitor, Loader2, Languages, Play, Palette, Tag, Search, Bug, Database, FolderSync } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { check, Update } from '@tauri-apps/plugin-updater';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useSettings } from '../../../context/SettingsContext';
import { useConfirm } from '../../../context/ConfirmContext';
import { useTranslation } from 'react-i18next';
import { LANGUAGES, type LanguagePreference } from '../../../i18n/languages';
import { ShareInfo, CacheEntry, DetailedCacheInfo, OfflineCacheStatus, TranscodeCapabilities } from '../../../types';
import { version as appVersion } from '../../../../package.json';
import { clearImageMemoryCaches } from '../../../services/imagePreviewCache';
import { useModalFocus } from '../../../hooks/useModalFocus';
import { AccessTransparencyDialog, type LocalAccessService } from '../../shared/AccessTransparencyDialog';
import { installVerifiedUpdate } from '../../../services/updateReliability';
import { getDetailedTranscodeCache, transcodeCacheErrorMessage } from '../../../services/transcodeCacheClient';
import { formatBytes } from '../../../utils';
import { SettingsRow, SettingsStepper, SettingsToggle } from './settings/SettingsControls';
import { AboutSettingsTab, AdvancedSettingsTab, EncryptionSettingsTab, GeneralSettingsTab, PrivacySettingsTab, ProxySettingsTab, SharingSettingsTab, ThemeSettingsTab, VpnSettingsTab, WebDavSettingsTab } from './settings/SettingsTabs';
import { FfmpegInstallNotice } from '../../shared/FfmpegInstallNotice';
import { SyncSettingsPanel } from '../sync/SyncSettingsPanel';
import { DesktopBehaviorSettings } from './settings/DesktopBehaviorSettings';
import { userFacingError } from '../../../services/userFacingError';
import { getInstallationInfo, RELEASES_URL, type InstallationInfo } from '../../../services/installationInfo';
import { useActionScope } from '../../../hooks/useActionScope';

interface SettingsModalProps {
    ownerId: string | null;
    isOpen: boolean;
    onClose: () => void;
    initialTab?: SettingsTab;
}

interface ApiSettings {
    enabled: boolean;
    port: number;
    key_set: boolean;
    running: boolean;
    last_error: string | null;
}

interface WebDavSettings {
    supported: boolean;
    enabled: boolean;
    port: number;
    write_enabled: boolean;
    token_set: boolean;
    running: boolean;
    last_error: string | null;
}

interface WebDavTokenResponse {
    token: string;
    url: string;
}

export type SettingsTab = 'general' | 'privacy' | 'advanced' | 'webdav' | 'themes' | 'proxy' | 'vpn' | 'encryption' | 'sharing' | 'sync' | 'about';

export function SettingsModal(props: SettingsModalProps) {
    return <OwnedSettingsModal key={props.ownerId ?? 'signed-out'} {...props} />;
}

function OwnedSettingsModal({ ownerId, isOpen, onClose, initialTab = 'general' }: SettingsModalProps) {
    const { settings, updateSetting, updateSettings, resetSettings } = useSettings();
    const { confirm } = useConfirm();
    const { t } = useTranslation();
    const [clearing, setClearing] = useState(false);

    // Transcode cache state
    const [transcodeCache, setTranscodeCache] = useState<DetailedCacheInfo | null>(null);
    const [cacheLoading, setCacheLoading] = useState(false);
    const [cacheError, setCacheError] = useState<string | null>(null);
    const [transcodeCapabilities, setTranscodeCapabilities] = useState<TranscodeCapabilities | null>(null);
    const cacheRequestId = useRef(0);
    const [offlineCache, setOfflineCache] = useState<OfflineCacheStatus | null>(null);
    const [offlineCacheLoading, setOfflineCacheLoading] = useState(false);
    const [offlineCacheError, setOfflineCacheError] = useState<string | null>(null);
    const [clearingVariant, setClearingVariant] = useState<string | null>(null); // file_key:quality being cleared
    const [activeTab, setActiveTab] = useState<SettingsTab>('general');
    const [settingsSearch, setSettingsSearch] = useState('');
    const [showGeneralAdvanced, setShowGeneralAdvanced] = useState(false);
    const [accessTransparency, setAccessTransparency] = useState<LocalAccessService | null>(null);
    const modalRef = useRef<HTMLDivElement>(null);
    useModalFocus(modalRef, onClose, isOpen && !accessTransparency);
    useEffect(() => {
        if (isOpen) {
            setActiveTab(initialTab);
        } else {
            setAccessTransparency(null);
        }
    }, [initialTab, isOpen]);
    const [latencyMs, setLatencyMs] = useState<number | null>(null);
    const [vpnDetected, setVpnDetected] = useState<boolean | null>(null);
    const [proxyStatus, setProxyStatus] = useState<{ reachable: boolean; latency_ms: number } | null>(null);
    const [isTestingProxy, setIsTestingProxy] = useState(false);

    // Update check state
    const [updateChecking, setUpdateChecking] = useState(false);
    const [updateAvailable, setUpdateAvailable] = useState<Update | null>(null);
    const [updateVersion, setUpdateVersion] = useState<string | null>(null);
    const [updateDownloading, setUpdateDownloading] = useState(false);
    const [updateProgress, setUpdateProgress] = useState(0);
    const [installationInfo, setInstallationInfo] = useState<InstallationInfo | null>(null);

    // Reconnect state
    const [reconnecting, setReconnecting] = useState(false);

    // Diagnostics state
    const [diagLoading, setDiagLoading] = useState(false);

    const handleCheckForUpdates = useCallback(async () => {
        setUpdateChecking(true);
        try {
            const installation = await getInstallationInfo();
            setInstallationInfo(installation);
            if (installation.packageManager === 'microsoft-store') {
                setUpdateAvailable(null);
                setUpdateVersion(null);
                await invoke('cmd_open_microsoft_store_updates');
                return;
            }
            const updateInfo = await check();
            if (updateInfo) {
                setUpdateAvailable(updateInfo);
                setUpdateVersion(updateInfo.version);
                toast.success(t('settings.update_available_toast', { version: updateInfo.version }));
            } else {
                setUpdateAvailable(null);
                setUpdateVersion(null);
                toast.success(t('settings.latest_version_toast'));
            }
        } catch (err) {
            const msg = err instanceof Error ? err.message : String(err);
            if (msg.includes('dev') || msg.includes('no current version')) {
                toast.info(t('settings.update_prod_only_toast'));
            } else {
                toast.error(t('settings.update_check_failed_toast', { error: msg }));
            }
        } finally {
            setUpdateChecking(false);
        }
    }, [t]);

    const handleInstallUpdate = useCallback(async () => {
        if (!updateAvailable) return;
        if (installationInfo?.managedByPackageManager) {
            try {
                await openUrl(RELEASES_URL);
            } catch (err) {
                toast.error(t('settings.update_failed_toast', { error: userFacingError(err, t) }));
            }
            return;
        }
        setUpdateDownloading(true);
        setUpdateProgress(0);
        try {
            await installVerifiedUpdate(updateAvailable, setUpdateProgress, (phase) => {
                if (phase === 'verifying') toast.info('Verifying the signed update before installation…');
            });
        } catch (err) {
            const msg = err instanceof Error ? err.message : String(err);
            toast.error(t('settings.update_failed_toast', { error: msg }));
            setUpdateDownloading(false);
        }
    }, [installationInfo?.managedByPackageManager, updateAvailable, t]);

    // Sharing settings state
    const [shares, setShares] = useState<ShareInfo[]>([]);
    const [refreshing, setRefreshing] = useState(false);
    const [copiedId, setCopiedId] = useState<string | null>(null);
    const [globalDomain, setGlobalDomain] = useState('');
    const captureShareScope = useActionScope(isOpen ? ownerId : null);
    const shareRequest = useRef(0);

    const fetchShares = useCallback(async () => {
        const isCurrent = captureShareScope();
        if (!ownerId || !isCurrent()) return;
        const request = ++shareRequest.current;
        setRefreshing(true);
        try {
            const list = await invoke<ShareInfo[]>('cmd_list_shares', { ownerId });
            if (isCurrent() && request === shareRequest.current) setShares(list.filter(share => share.owner_id === ownerId));
        } catch (e) {
            if (isCurrent() && request === shareRequest.current) toast.error(t('settings.load_shares_failed', { error: e }));
        } finally {
            if (isCurrent() && request === shareRequest.current) setRefreshing(false);
        }
    }, [captureShareScope, ownerId, t]);

    useEffect(() => {
        if (isOpen && activeTab === 'sharing') {
            fetchShares();
        }
    }, [isOpen, activeTab, fetchShares]);

    const handleRevokeShare = async (id: string) => {
        const isCurrent = captureShareScope();
        if (!ownerId || !isCurrent() || !shares.some(share => share.id === id && share.owner_id === ownerId)) return;
        const ok = await confirm({
            title: t('settings.revoke_link_title'),
            message: t('settings.revoke_link_desc'),
            confirmText: t('settings.revoke'),
            variant: 'danger',
        });
        if (!ok || !isCurrent()) return;

        try {
            await invoke('cmd_revoke_share', { id, ownerId });
            if (!isCurrent()) return;
            toast.success(t('settings.link_revoked'));
            fetchShares();
        } catch (e) {
            if (isCurrent()) toast.error(t('settings.link_revoke_failed', { error: e }));
        }
    };

    const handleCopyShare = (id: string) => {
        const isCurrent = captureShareScope();
        const share = shares.find(s => s.id === id && s.owner_id === ownerId);
        if (!share || !isCurrent()) return;
        
        let link = `http://127.0.0.1:14201/d/${share.id}`;
        if (globalDomain.trim()) {
            link = `http://${globalDomain.trim()}/d/${share.id}`;
        }
        
        navigator.clipboard.writeText(link);
        setCopiedId(share.id);
        setTimeout(() => { if (isCurrent()) setCopiedId(null); }, 2000);
    };

    // API settings state
    const [apiSettings, setApiSettings] = useState<ApiSettings>({ enabled: false, port: 8550, key_set: false, running: false, last_error: null });
    const [apiPort, setApiPort] = useState('8550');
    const [apiLoading, setApiLoading] = useState(false);
    const [generatedKey, setGeneratedKey] = useState<string | null>(null);
    const [keyCopied, setKeyCopied] = useState(false);

    // WebDAV settings state
    const [webDavSettings, setWebDavSettings] = useState<WebDavSettings>({
        supported: true,
        enabled: false,
        port: 8551,
        write_enabled: false,
        token_set: false,
        running: false,
        last_error: null,
    });
    const [webDavPort, setWebDavPort] = useState('8551');
    const [webDavLoading, setWebDavLoading] = useState(false);
    const [webDavGenerating, setWebDavGenerating] = useState(false);
    const [generatedWebDavUrl, setGeneratedWebDavUrl] = useState<string | null>(null);
    const [webDavUrlCopied, setWebDavUrlCopied] = useState(false);

    const fetchApiSettings = useCallback(async () => {
        try {
            const apiStatus = await invoke<ApiSettings>('cmd_get_api_settings');
            setApiSettings(apiStatus);
            setApiPort(apiStatus.port.toString());
        } catch {
            // API settings not available
        }
    }, []);

    const fetchWebDavSettings = useCallback(async (syncPort = true) => {
        try {
            const webDavStatus = await invoke<WebDavSettings>('cmd_get_webdav_settings');
            setWebDavSettings(webDavStatus);
            if (syncPort) setWebDavPort(webDavStatus.port.toString());
        } catch {
            setWebDavSettings(previous => ({ ...previous, supported: false }));
        }
    }, []);

    // Load API settings when modal opens
    useEffect(() => {
        if (isOpen) {
            fetchApiSettings();
            fetchWebDavSettings();
            setGeneratedKey(null);
            setKeyCopied(false);
            setGeneratedWebDavUrl(null);
            setWebDavUrlCopied(false);
        }
    }, [isOpen, fetchApiSettings, fetchWebDavSettings]);

    // Fetch transcode cache info
    const fetchTranscodeCache = useCallback(async (refresh = false) => {
        const requestId = ++cacheRequestId.current;
        setCacheLoading(true);
        setCacheError(null);
        try {
            const cacheDetails = await getDetailedTranscodeCache(refresh);
            if (cacheRequestId.current === requestId) {
                setTranscodeCache(cacheDetails);
                setCacheError(cacheDetails.last_error && cacheDetails.entries.length === 0
                    ? cacheDetails.last_error
                    : null);
            }
        } catch (error) {
            if (cacheRequestId.current === requestId) {
                setTranscodeCache(null);
                setCacheError(transcodeCacheErrorMessage(error));
            }
        } finally {
            if (cacheRequestId.current === requestId) {
                setCacheLoading(false);
            }
        }
    }, []);

    const fetchTranscodeCapabilities = useCallback(async () => {
        try {
            setTranscodeCapabilities(await invoke<TranscodeCapabilities>('cmd_get_transcode_capabilities'));
        } catch {
            setTranscodeCapabilities(null);
        }
    }, []);

    const fetchOfflineCache = useCallback(async () => {
        setOfflineCacheLoading(true);
        setOfflineCacheError(null);
        try {
            const cacheStatus = await invoke<OfflineCacheStatus>('cmd_get_offline_cache_status');
            setOfflineCache(cacheStatus);
        } catch (error) {
            setOfflineCache(null);
            setOfflineCacheError(error instanceof Error ? error.message : String(error));
        } finally {
            setOfflineCacheLoading(false);
        }
    }, []);

    // Load transcode cache when on general tab
    useEffect(() => {
        if (isOpen && activeTab === 'general') {
            fetchTranscodeCache();
            fetchTranscodeCapabilities();
            fetchOfflineCache();
        }
    }, [isOpen, activeTab, fetchOfflineCache, fetchTranscodeCache, fetchTranscodeCapabilities]);

    useEffect(() => {
        if (!isOpen || activeTab !== 'general' || !transcodeCache?.scan_in_progress) return;
        const interval = window.setInterval(() => void fetchTranscodeCache(false), 1_000);
        return () => window.clearInterval(interval);
    }, [activeTab, fetchTranscodeCache, isOpen, transcodeCache?.scan_in_progress]);

    // Poll API status while modal is open and API is enabled
    useEffect(() => {
        if (!isOpen || !apiSettings.enabled) return;
        const interval = setInterval(fetchApiSettings, 3000);
        return () => clearInterval(interval);
    }, [isOpen, apiSettings.enabled, fetchApiSettings]);

    // Poll WebDAV runtime state while its tab is visible.
    useEffect(() => {
        if (!isOpen || activeTab !== 'webdav') return;
        const interval = setInterval(() => void fetchWebDavSettings(false), 3000);
        return () => clearInterval(interval);
    }, [isOpen, activeTab, fetchWebDavSettings]);

    // Sync proxy settings to backend whenever they change
    useEffect(() => {
        const applyProxy = async () => {
            try {
                await invoke('cmd_apply_proxy_settings', {
                    enabled: settings.proxyEnabled,
                    proxyType: settings.proxyType,
                    host: settings.proxyHost,
                    port: settings.proxyPort,
                    username: settings.proxyUsername,
                    password: settings.proxyPassword,
                });
            } catch {
                // best-effort sync
            }
        };
        applyProxy();
    }, [
        settings.proxyEnabled, settings.proxyType, settings.proxyHost,
        settings.proxyPort, settings.proxyUsername, settings.proxyPassword,
    ]);

    // Sync VPN optimizer settings to backend whenever they change
    useEffect(() => {
        const applyVpn = async () => {
            try {
                await invoke('cmd_apply_vpn_settings', {
                    enabled: settings.vpnMode,
                    timeoutMultiplier: settings.timeoutMultiplier,
                    retryAttempts: settings.retryAttempts,
                    retryBaseBackoffMs: Math.round(settings.retryBaseBackoffSec * 1000),
                    retryMaxBackoffMs: Math.round(settings.retryMaxBackoffSec * 1000),
                    adaptivePolling: settings.adaptivePolling,
                    pollingMinSec: settings.pollingMinSec,
                    pollingMaxSec: settings.pollingMaxSec,
                    preferredDc: settings.preferredDC,
                    dcFallbackAttempts: settings.dcFallbackAttempts,
                    floodWaitRespect: settings.floodWaitRespect,
                    peerCacheSize: settings.peerCacheSize,
                    bandwidthLimitUpKbs: settings.bandwidthLimitUpKBs,
                    bandwidthLimitDownKbs: settings.bandwidthLimitDownKBs,
                    chunkSizeKb: settings.chunkSizeKb,
                    keepAliveIntervalSec: settings.keepAliveIntervalSec,
                    autoDetectVpn: settings.autoDetectVpn,
                    archiveMaxBytes: settings.archiveMaxBytes * 1024 * 1024,
                });
            } catch {
                // best-effort sync
            }
        };
        applyVpn();
    }, [
        settings.vpnMode, settings.timeoutMultiplier, settings.retryAttempts,
        settings.retryBaseBackoffSec, settings.retryMaxBackoffSec, settings.adaptivePolling,
        settings.pollingMinSec, settings.pollingMaxSec, settings.preferredDC,
        settings.dcFallbackAttempts, settings.floodWaitRespect, settings.peerCacheSize,
        settings.bandwidthLimitUpKBs, settings.bandwidthLimitDownKBs, settings.chunkSizeKb,
        settings.keepAliveIntervalSec, settings.autoDetectVpn, settings.archiveMaxBytes,
    ]);

    // Poll latency when VPN tab is active
    useEffect(() => {
        if (!isOpen || activeTab !== 'vpn') return;
        const check = async () => {
            try {
                const ms = await invoke<number>('cmd_check_latency');
                setLatencyMs(ms);
            } catch { setLatencyMs(null); }
        };
        check();
        const interval = setInterval(check, 5000);
        return () => clearInterval(interval);
    }, [isOpen, activeTab]);

    // Poll proxy status when Proxy tab is active
    useEffect(() => {
        if (!isOpen || activeTab !== 'proxy') return;
        const checkProxy = async () => {
            if (!settings.proxyEnabled || !settings.proxyLiveStateEnabled) {
                setProxyStatus(null);
                return;
            }
            try {
                const status = await invoke<{ reachable: boolean; latency_ms: number }>('cmd_get_proxy_status');
                setProxyStatus(status);
            } catch {
                setProxyStatus({ reachable: false, latency_ms: -1 });
            }
        };
        checkProxy();
        const interval = setInterval(checkProxy, 5000);
        return () => clearInterval(interval);
    }, [isOpen, activeTab, settings.proxyEnabled, settings.proxyLiveStateEnabled]);

    // Detect VPN interfaces when VPN tab opens
    useEffect(() => {
        if (!isOpen || activeTab !== 'vpn') return;
        const detect = async () => {
            try {
                const found = await invoke<boolean>('cmd_detect_vpn');
                setVpnDetected(found);
            } catch { setVpnDetected(null); }
        };
        detect();
    }, [isOpen, activeTab]);

    const handleApiToggle = async () => {
        if (!apiSettings.enabled && !settings.restPermissionExplained) {
            const understood = await confirm({
                title: 'Before enabling REST access',
                message: 'The REST server opens a local automation endpoint on the port shown below. Anyone who can reach that address still needs the generated API key. Keep the key private, bind only to networks you trust, and disable the server when it is no longer needed.',
                confirmText: 'I understand — enable',
                variant: 'info',
            });
            if (!understood) return;
            updateSetting('restPermissionExplained', true);
        }
        setApiLoading(true);
        try {
            const port = parseInt(apiPort, 10);
            if (isNaN(port) || port < 1024 || port > 65535) {
                toast.error(t('settings.port_range_error'));
                setApiLoading(false);
                return;
            }
            const updatedSettings = await invoke<ApiSettings>('cmd_update_api_settings', {
                enabled: !apiSettings.enabled,
                port,
            });
            setApiSettings(updatedSettings);
            if (updatedSettings.last_error) {
                toast.error(updatedSettings.last_error);
            } else {
                toast.success(updatedSettings.enabled ? t('settings.api_server_started') : t('settings.api_server_stopped'));
            }
        } catch (e) {
            toast.error(t('settings.api_update_failed', { error: e }));
        } finally {
            setApiLoading(false);
        }
    };

    const handlePortApply = async () => {
        const port = parseInt(apiPort, 10);
        if (isNaN(port) || port < 1024 || port > 65535) {
            toast.error(t('settings.port_range_error'));
            return;
        }
        if (port === apiSettings.port) return;
        setApiLoading(true);
        try {
            const updatedSettings = await invoke<ApiSettings>('cmd_update_api_settings', {
                enabled: apiSettings.enabled,
                port,
            });
            setApiSettings(updatedSettings);
            if (updatedSettings.last_error) {
                toast.error(updatedSettings.last_error);
            } else {
                toast.success(t('settings.api_port_updated', { port }));
            }
        } catch (e) {
            toast.error(t('settings.api_port_update_failed', { error: e }));
        } finally {
            setApiLoading(false);
        }
    };

    const handleGenerateKey = async () => {
        const ok = await confirm({
            title: t('settings.generate_api_key_title'),
            message: apiSettings.key_set
                ? t('settings.regenerate_api_key_desc')
                : t('settings.generate_api_key_desc'),
            confirmText: apiSettings.key_set ? t('settings.regenerate') : t('settings.generate'),
            variant: apiSettings.key_set ? 'danger' : 'info',
        });
        if (!ok) return;
        try {
            const key = await invoke<string>('cmd_regenerate_api_key');
            setGeneratedKey(key);
            setKeyCopied(false);
            setApiSettings(prev => ({ ...prev, key_set: true }));
            toast.success(t('settings.api_key_generated'));
        } catch (e) {
            toast.error(t('settings.api_key_generate_failed', { error: e }));
        }
    };

    const handleCopyKey = async () => {
        if (!generatedKey) return;
        try {
            await navigator.clipboard.writeText(generatedKey);
            setKeyCopied(true);
            setTimeout(() => setKeyCopied(false), 2000);
        } catch {
            toast.error(t('settings.copy_clipboard_failed'));
        }
    };

    const parseWebDavPort = () => {
        const port = Number.parseInt(webDavPort, 10);
        if (!Number.isInteger(port) || port < 1024 || port > 65535) {
            toast.error(t('settings.port_range_error'));
            return null;
        }
        return port;
    };

    const generateWebDavLink = async (askBeforeReplacing = true) => {
        if (askBeforeReplacing && webDavSettings.token_set) {
            const ok = await confirm({
                title: t('settings.webdav_regenerate_title'),
                message: t('settings.webdav_regenerate_desc'),
                confirmText: t('settings.regenerate'),
                variant: 'danger',
            });
            if (!ok) return null;
        }
        setWebDavGenerating(true);
        try {
            const generatedLink = await invoke<WebDavTokenResponse>('cmd_regenerate_webdav_token');
            setGeneratedWebDavUrl(generatedLink.url);
            setWebDavUrlCopied(false);
            setWebDavSettings(previous => ({ ...previous, token_set: true }));
            toast.success(t('settings.webdav_link_generated'));
            return generatedLink;
        } catch (error) {
            toast.error(t('settings.webdav_generate_failed', { error }));
            return null;
        } finally {
            setWebDavGenerating(false);
        }
    };

    const handleWebDavToggle = async () => {
        const port = parseWebDavPort();
        if (port === null || !webDavSettings.supported) return;
        if (!webDavSettings.enabled && !settings.webdavPermissionExplained) {
            const understood = await confirm({
                title: 'Before enabling WebDAV access',
                message: 'WebDAV lets Finder and file managers browse your Telegram Drive through the complete token URL. The token is the credential: Guest or anonymous login has no access. Read-only mode is safest; enabling changes also permits uploads, moves, renames, and deletes.',
                confirmText: 'I understand — enable',
                variant: 'info',
            });
            if (!understood) return;
            updateSetting('webdavPermissionExplained', true);
        }
        setWebDavLoading(true);
        try {
            if (!webDavSettings.enabled && !webDavSettings.token_set) {
                const generated = await generateWebDavLink(false);
                if (!generated) return;
            }
            const updatedSettings = await invoke<WebDavSettings>('cmd_update_webdav_settings', {
                enabled: !webDavSettings.enabled,
                port,
                writeEnabled: webDavSettings.write_enabled,
            });
            setWebDavSettings(updatedSettings);
            if (updatedSettings.last_error) {
                toast.error(updatedSettings.last_error);
            } else {
                toast.success(updatedSettings.enabled ? t('settings.webdav_started') : t('settings.webdav_stopped_toast'));
            }
        } catch (error) {
            toast.error(t('settings.webdav_update_failed', { error }));
        } finally {
            setWebDavLoading(false);
        }
    };

    const handleWebDavPortApply = async () => {
        const port = parseWebDavPort();
        if (port === null || port === webDavSettings.port || !webDavSettings.supported) return;
        setWebDavLoading(true);
        try {
            const updatedSettings = await invoke<WebDavSettings>('cmd_update_webdav_settings', {
                enabled: webDavSettings.enabled,
                port,
                writeEnabled: webDavSettings.write_enabled,
            });
            setWebDavSettings(updatedSettings);
            setGeneratedWebDavUrl(previous => {
                if (!previous) return previous;
                try {
                    const updated = new URL(previous);
                    updated.port = port.toString();
                    return updated.toString();
                } catch {
                    return previous;
                }
            });
            if (updatedSettings.last_error) {
                toast.error(updatedSettings.last_error);
            } else {
                toast.success(t('settings.webdav_port_updated', { port }));
            }
        } catch (error) {
            setWebDavPort(webDavSettings.port.toString());
            toast.error(t('settings.webdav_update_failed', { error }));
        } finally {
            setWebDavLoading(false);
        }
    };

    const handleWebDavWriteToggle = async () => {
        if (!webDavSettings.supported) return;
        const port = parseWebDavPort();
        if (port === null) return;
        const nextWriteEnabled = !webDavSettings.write_enabled;
        if (nextWriteEnabled) {
            const ok = await confirm({
                title: t('settings.webdav_enable_changes_title'),
                message: t('settings.webdav_enable_changes_confirm'),
                confirmText: t('common.confirm'),
                variant: 'danger',
            });
            if (!ok) return;
        }
        setWebDavLoading(true);
        try {
            const updatedSettings = await invoke<WebDavSettings>('cmd_update_webdav_settings', {
                enabled: webDavSettings.enabled,
                port,
                writeEnabled: nextWriteEnabled,
            });
            setWebDavSettings(updatedSettings);
            if (updatedSettings.last_error) {
                toast.error(updatedSettings.last_error);
            }
        } catch (error) {
            toast.error(t('settings.webdav_update_failed', { error }));
        } finally {
            setWebDavLoading(false);
        }
    };

    const handleCopyWebDavUrl = async () => {
        if (!generatedWebDavUrl) return;
        try {
            await navigator.clipboard.writeText(generatedWebDavUrl);
            setWebDavUrlCopied(true);
            setTimeout(() => setWebDavUrlCopied(false), 2000);
        } catch {
            toast.error(t('settings.copy_clipboard_failed'));
        }
    };

    return (
        <AnimatePresence>
            {isOpen && (
                <motion.div
                    initial={{ opacity: 0 }}
                    animate={{ opacity: 1 }}
                    exit={{ opacity: 0 }}
                    className="fixed inset-0 z-[100] flex items-center justify-center bg-app-overlay p-6 backdrop-blur-sm"
                    onClick={onClose}
                >
                    <motion.div
                        ref={modalRef}
                        role="dialog"
                        aria-modal="true"
                        aria-labelledby="settings-dialog-title"
                        tabIndex={-1}
                        layout
                        initial={{ opacity: 0, scale: 0.95, y: 10 }}
                        animate={{ opacity: 1, scale: 1, y: 0 }}
                        exit={{ opacity: 0, scale: 0.95, y: 10 }}
                        transition={{ duration: 0.18, ease: [0.2, 0.8, 0.2, 1] }}
                        className="quiet-raised flex h-[min(760px,calc(100vh-3rem))] w-[min(920px,calc(100vw-3rem))] flex-col overflow-hidden"
                        onClick={e => e.stopPropagation()}
                    >
                        {/* Header */}
                        <div className="flex min-h-16 items-center justify-between border-b border-app-border-subtle px-6 py-4">
                            <div>
                                <h2 id="settings-dialog-title" className="text-base font-semibold text-app-text">{t('settings.title')}</h2>
                                <p className="mt-0.5 text-xs text-app-text-tertiary">{activeTab === 'privacy' ? 'Privacy, crash reporting & supporter license' : activeTab === 'advanced' ? 'Advanced connections and integrations' : t(`settings.tab_${activeTab}`)}</p>
                            </div>
                            <button
                                onClick={onClose}
                                aria-label={t('common.close')}
                                className="quiet-control p-2 text-app-text-secondary hover:text-app-text"
                            >
                                <X className="w-4 h-4" />
                            </button>
                        </div>

                        <div className="flex min-h-0 flex-1">
                        {/* Settings navigation */}
                        <aside className="w-56 shrink-0 overflow-y-auto border-e border-app-border-subtle bg-app-sidebar p-3">
                            <div className="relative mb-3">
                                <Search className="pointer-events-none absolute start-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-app-text-tertiary" />
                                <input value={settingsSearch} onChange={(event) => setSettingsSearch(event.target.value)} placeholder="Search settings" className="quiet-control h-9 w-full border border-app-border bg-app-surface-sunken ps-8 pe-2 text-sm text-app-text outline-none focus:border-app-accent" />
                            </div>
                            {([
                                ['Essentials', [['general', Globe, 'General transfers language updates'], ['themes', Palette, 'Appearance colors themes']] as const],
                                ['Security & Privacy', [['privacy', Bug, 'Privacy telemetry crash reports consent supporter lifetime ad-free ads $5 PayPal recovery'], ['encryption', Shield, 'Encryption vault security auto lock']] as const],
                                ['Connections', [['sync', FolderSync, 'Folder sync local directories Telegram channels'], ['sharing', Link, 'Sharing links local server']] as const],
                                ['Advanced', [['advanced', Gauge, 'REST API proxy VPN WebDAV network integration Finder token port']] as const],
                                ['Support', [['about', Info, 'About diagnostics version updates']] as const],
                            ] as const).map(([group, items]) => {
                                const visibleItems = items.filter(([key, , keywords]) => `${key} ${keywords}`.toLowerCase().includes(settingsSearch.trim().toLowerCase()));
                                if (visibleItems.length === 0) return null;
                                return (
                                    <div key={group} className="mb-3">
                                        <div className="mb-1 px-2 text-[10px] font-semibold uppercase tracking-wider text-app-text-tertiary">{group}</div>
                                        {visibleItems.map(([key, Icon]) => (
                                            <button key={key} onClick={() => setActiveTab(key)} className={`quiet-control mb-0.5 flex w-full items-center gap-2.5 px-3 py-2 text-start text-sm font-medium ${activeTab === key ? 'bg-app-selected text-app-text' : 'text-app-text-secondary hover:text-app-text'}`}>
                                                <Icon className={`h-4 w-4 ${activeTab === key ? 'text-app-accent' : ''}`} />
                                                {key === 'privacy' ? 'Privacy' : key === 'advanced' ? 'Advanced' : t(`settings.tab_${key}`)}
                                            </button>
                                        ))}
                                    </div>
                                );
                            })}
                        </aside>

                        {/* Body */}
                        <div className="relative min-w-0 flex-1 overflow-y-auto overflow-x-hidden px-7 py-6">
                            <AnimatePresence mode="wait" initial={false}>

                                {activeTab === 'general' && (
                                    <GeneralSettingsTab>

                            {/* Transfers Section */}
                            <section className="space-y-3">
                                <h3 className="text-xs font-semibold text-telegram-subtext uppercase tracking-wider flex items-center gap-2">
                                    <Upload className="w-3.5 h-3.5" />
                                    {t('settings.transfers')}
                                </h3>

                                <SettingsRow
                                    icon={<Upload className="h-4 w-4 text-telegram-subtext" />}
                                    title={t('settings.concurrent_uploads')}
                                    description={t('settings.max_uploads_desc')}
                                    control={<SettingsStepper value={settings.maxConcurrentUploads} minimum={1} maximum={10} label={t('settings.concurrent_uploads')} onChange={value => updateSetting('maxConcurrentUploads', value)} />}
                                />

                                <SettingsRow
                                    icon={<Download className="h-4 w-4 text-telegram-subtext" />}
                                    title={t('settings.concurrent_downloads')}
                                    description={t('settings.max_downloads_desc')}
                                    control={<SettingsStepper value={settings.maxConcurrentDownloads} minimum={1} maximum={10} label={t('settings.concurrent_downloads')} onChange={value => updateSetting('maxConcurrentDownloads', value)} />}
                                />

                                <SettingsRow icon={<FolderArchive className="h-4 w-4 text-telegram-subtext" />} title={t('settings.zip_before_upload')} description={t('settings.zip_folders_desc')} control={<SettingsToggle checked={settings.zipFolders} label={t('settings.zip_before_upload')} onChange={() => updateSetting('zipFolders', !settings.zipFolders)} />} />

                                <SettingsRow
                                    icon={<Play className="h-4 w-4 text-telegram-subtext" />}
                                    title={t('settings.video_upload_default')}
                                    description={t('settings.video_upload_desc')}
                                    control={(
                                        <select
                                            value={settings.videoUploadMode}
                                            onChange={event => updateSetting('videoUploadMode', event.target.value as 'file' | 'media')}
                                            aria-label={t('settings.video_upload_default')}
                                            className="rounded-md border border-telegram-border bg-telegram-bg px-2.5 py-1.5 text-sm text-telegram-text focus:border-telegram-primary/50 focus:outline-none"
                                        >
                                            <option value="file">{t('settings.video_upload_file')}</option>
                                            <option value="media">{t('settings.video_upload_media')}</option>
                                        </select>
                                    )}
                                />

                                <SettingsRow icon={<Tag className="h-4 w-4 text-telegram-subtext" />} title={t('common.hide_groups')} description={t('common.hide_groups_desc')} control={<SettingsToggle checked={settings.hideGroups} label={t('common.hide_groups')} onChange={() => updateSetting('hideGroups', !settings.hideGroups)} />} />

                                <SettingsRow icon={<Zap className="h-4 w-4 text-telegram-subtext" />} title={t('settings.performance_mode')} description={t('settings.performance_mode_desc')} control={<SettingsToggle checked={settings.performanceMode} label={t('settings.performance_mode')} onChange={() => updateSetting('performanceMode', !settings.performanceMode)} />} />

                                <SettingsRow icon={<Monitor className="h-4 w-4 text-telegram-subtext" />} title={t('settings.linux_rendering_fix')} description={t('settings.linux_rendering_desc')} control={<SettingsToggle checked={settings.linuxRenderingFix} label={t('settings.linux_rendering_fix')} onChange={() => { updateSetting('linuxRenderingFix', !settings.linuxRenderingFix); toast.info(t('settings.restart_app_toast'), { duration: 5000 }); }} />} />
                            </section>

                            {/* Language & Region Section */}
                            <DesktopBehaviorSettings />

                            {/* Language & Region Section */}
                            <section className="space-y-3">
                                <h3 className="text-xs font-semibold text-telegram-subtext uppercase tracking-wider flex items-center gap-2">
                                    <Languages className="w-3.5 h-3.5" />
                                    {t('settings.language_region')}
                                </h3>

                                <div className="flex items-center justify-between p-3 rounded-lg bg-telegram-hover/50">
                                    <div className="flex items-center gap-2">
                                        <Globe className="w-4 h-4 text-telegram-subtext" />
                                        <div>
                                            <p className="text-sm text-telegram-text font-medium">{t('settings.app_language')}</p>
                                            <p className="text-xs text-telegram-subtext">{t('settings.choose_language')}</p>
                                        </div>
                                    </div>
                                    <div className="relative">
                                        <select
                                            value={settings.language}
                                            onChange={event => updateSetting('language', event.target.value as LanguagePreference)}
                                            aria-label={t('settings.app_language')}
                                            className="appearance-none bg-telegram-bg border border-telegram-border rounded-md pl-3 pr-8 py-1.5 text-sm text-telegram-text focus:outline-none focus:border-telegram-primary/50 transition cursor-pointer"
                                        >
                                            {LANGUAGES.map(lang => (
                                                <option key={lang.code} value={lang.code}>
                                                    {lang.nativeLabel}
                                                </option>
                                            ))}
                                        </select>
                                        <ChevronDown className="w-4 h-4 text-telegram-subtext absolute right-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
                                    </div>
                                </div>
                            </section>

                            {!showGeneralAdvanced && (
                                <button type="button" onClick={() => setActiveTab('advanced')} className="quiet-surface flex w-full items-center justify-between p-4 text-start hover:border-app-accent/30 hover:bg-app-hover"><span><strong className="block text-sm text-app-text">Advanced settings</strong><span className="mt-1 block text-xs text-app-text-secondary">REST API, WebDAV, proxy, VPN, and network tuning</span></span><Gauge className="h-5 w-5 text-app-accent" /></button>
                            )}

                            {/* REST API Section */}
                            <section className={`${showGeneralAdvanced ? '' : 'hidden'} space-y-3`}>
                                <h3 className="text-xs font-semibold text-telegram-subtext uppercase tracking-wider flex items-center gap-2">
                                    <Globe className="w-3.5 h-3.5" />
                                    {t('settings.rest_api')}
                                </h3>

                                {/* Enable Toggle */}
                                <div className="flex items-center justify-between p-3 rounded-lg bg-telegram-hover/50">
                                    <div className="flex items-center gap-2">
                                        <div className={`w-2 h-2 rounded-full ${apiSettings.running ? 'bg-green-400 shadow-[0_0_6px_rgba(74,222,128,0.5)]' : 'bg-gray-500'}`} />
                                        <div>
                                            <p className="text-sm text-telegram-text font-medium">{t('settings.enable_api_server')}</p>
                                            <p className="text-xs text-telegram-subtext">
                                                {apiSettings.running ? t('settings.api_running', { port: apiSettings.port }) : t('settings.api_stopped')}
                                            </p>
                                        </div>
                                    </div>
                                    <button
                                        type="button" role="switch" aria-checked={apiSettings.enabled} aria-label={t('settings.enable_api_server')}
                                        onClick={handleApiToggle}
                                        disabled={apiLoading}
                                        className={`relative w-11 h-6 rounded-full transition-colors duration-200 ${apiSettings.enabled ? 'bg-telegram-primary' : 'bg-telegram-border'} disabled:opacity-50`}
                                    >
                                        <span className={`absolute top-0.5 start-0.5 w-5 h-5 rounded-full bg-white shadow transition-transform duration-200 ${apiSettings.enabled ? 'translate-x-5 rtl:-translate-x-5' : 'translate-x-0'}`} />
                                    </button>
                                </div>

                                {apiSettings.last_error && (
                                    <div className="rounded-lg border border-red-500/20 bg-red-500/5 p-3">
                                        <p className="text-xs font-medium text-red-300">{t('settings.webdav_last_error')}</p>
                                        <p className="mt-1 break-words font-mono text-[11px] text-red-300/80">{apiSettings.last_error}</p>
                                    </div>
                                )}

                                {/* Port */}
                                <div className="flex items-center justify-between p-3 rounded-lg bg-telegram-hover/50">
                                    <div>
                                        <p className="text-sm text-telegram-text font-medium">{t('common.port')}</p>
                                        <p className="text-xs text-telegram-subtext">1024 - 65535</p>
                                    </div>
                                    <div className="flex items-center gap-2">
                                        <input
                                            type="number"
                                            min="1024"
                                            max="65535"
                                            value={apiPort}
                                            onChange={e => setApiPort(e.target.value)}
                                            onBlur={handlePortApply}
                                            onKeyDown={e => { if (e.key === 'Enter') handlePortApply(); }}
                                            className="w-20 bg-telegram-bg border border-telegram-border rounded-md px-2 py-1 text-sm text-telegram-text text-center focus:outline-none focus:border-telegram-primary/50 transition"
                                        />
                                    </div>
                                </div>

                                {/* API Key */}
                                <div className="p-3 rounded-lg bg-telegram-hover/50 space-y-2.5">
                                    <div className="flex items-center justify-between">
                                        <div className="flex items-center gap-2">
                                            <Key className="w-4 h-4 text-telegram-subtext" />
                                            <div>
                                                <p className="text-sm text-telegram-text font-medium">{t('settings.api_key')}</p>
                                                <p className="text-xs text-telegram-subtext">
                                                    {apiSettings.key_set ? t('settings.api_key_configured') : t('settings.api_key_unset')}
                                                </p>
                                            </div>
                                        </div>
                                        <button
                                            onClick={handleGenerateKey}
                                            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium bg-telegram-primary/10 text-telegram-primary hover:bg-telegram-primary/20 transition"
                                        >
                                            <RefreshCw className="w-3 h-3" />
                                            {apiSettings.key_set ? t('settings.regenerate') : t('settings.generate')}
                                        </button>
                                    </div>

                                    {/* One-time key reveal */}
                                    {generatedKey && (
                                        <div className="mt-2 p-2.5 bg-telegram-bg rounded-lg border border-yellow-500/20">
                                            <p className="text-[10px] text-yellow-400/80 uppercase tracking-wider font-semibold mb-1.5">
                                                {t('settings.api_copy_alert')}
                                            </p>
                                            <div className="flex items-center gap-2">
                                                <code className="flex-1 text-xs text-telegram-text font-mono bg-telegram-hover rounded px-2 py-1.5 overflow-x-auto select-all">
                                                    {generatedKey}
                                                </code>
                                                <button
                                                    onClick={handleCopyKey}
                                                    className="p-1.5 rounded-md hover:bg-telegram-hover text-telegram-subtext hover:text-telegram-text transition flex-shrink-0"
                                                    title="Copy to clipboard"
                                                >
                                                    {keyCopied ? <Check className="w-4 h-4 text-green-400" /> : <Copy className="w-4 h-4" />}
                                                </button>
                                            </div>
                                        </div>
                                    )}
                                </div>
                                <button type="button" onClick={() => setAccessTransparency('rest')} className="quiet-control flex w-full items-center justify-center gap-2 border border-app-border-subtle px-3 py-2 text-xs font-medium text-app-accent"><Info className="h-3.5 w-3.5" aria-hidden="true" />Understand REST permissions</button>
                            </section>

                            {/* Storage Section */}
                            <section className="space-y-3">
                                <h3 className="text-xs font-semibold text-telegram-subtext uppercase tracking-wider flex items-center gap-2">
                                    <HardDrive className="w-3.5 h-3.5" />
                                    {t('settings.storage')}
                                </h3>

                                {/* Transcode Cache Size */}
                                <div className="p-3 rounded-lg bg-telegram-hover/50 space-y-2">
                                    <div className="flex items-center justify-between">
                                        <div className="flex items-center gap-2">
                                            <HardDrive className="w-4 h-4 text-telegram-subtext" />
                                            <div>
                                                <p className="text-sm text-telegram-text font-medium">{t('settings.transcode_cache_limit')}</p>
                                                <p className="text-xs text-telegram-subtext">{t('settings.transcode_cache_desc')}</p>
                                            </div>
                                        </div>
                                        <span className="text-sm text-telegram-primary font-mono font-medium">{settings.transcodeCacheMaxGb} GB</span>
                                    </div>
                                    <input type="range" min="1" max="50" step="1" value={settings.transcodeCacheMaxGb}
                                        aria-label={t('settings.transcode_cache_limit')}
                                        onChange={e => {
                                            const gb = parseInt(e.target.value);
                                            updateSetting('transcodeCacheMaxGb', gb);
                                            invoke('cmd_set_transcode_cache_limit', { maxGb: gb }).catch(() => {});
                                        }}
                                        className="w-full h-1.5 rounded-full appearance-none bg-telegram-border accent-telegram-primary cursor-pointer" />
                                </div>

                                <div className="flex items-center justify-between gap-3 rounded-lg bg-telegram-hover/50 p-3">
                                    <div className="flex min-w-0 items-center gap-2">
                                        <Database className="h-4 w-4 shrink-0 text-telegram-subtext" />
                                        <div className="min-w-0">
                                            <p className="text-sm font-medium text-telegram-text">{t('settings.offline_cache')}</p>
                                            <p className="text-xs text-telegram-subtext">{t('settings.offline_cache_desc')}</p>
                                            <p className="mt-1 text-xs font-mono text-telegram-primary">
                                                {offlineCacheError
                                                    ? <span role="alert" className="text-red-400">{t('settings.failed_prefix', { error: offlineCacheError })}</span>
                                                    : offlineCache
                                                    ? t('settings.offline_cache_usage', {
                                                        count: offlineCache.file_count,
                                                        used: formatBytes(offlineCache.total_bytes),
                                                        limit: formatBytes(offlineCache.max_bytes),
                                                    })
                                                    : t('common.loading')}
                                            </p>
                                        </div>
                                    </div>
                                    <div className="flex shrink-0 items-center gap-1.5">
                                        <button
                                            type="button"
                                            onClick={fetchOfflineCache}
                                            disabled={offlineCacheLoading}
                                            className="rounded-md p-1.5 text-telegram-subtext transition hover:bg-telegram-hover hover:text-telegram-text disabled:opacity-50"
                                            title={t('settings.refresh_offline_cache')}
                                        >
                                            <RefreshCw className={`h-3.5 w-3.5 ${offlineCacheLoading ? 'animate-spin' : ''}`} />
                                        </button>
                                        <button
                                            type="button"
                                            disabled={offlineCacheLoading || !offlineCache || offlineCache.file_count === 0}
                                            onClick={async () => {
                                                const ok = await confirm({
                                                    title: t('settings.clear_offline_cache_title'),
                                                    message: t('settings.clear_offline_cache_desc'),
                                                    confirmText: t('settings.clear'),
                                                    variant: 'danger',
                                                });
                                                if (!ok) return;
                                                setOfflineCacheLoading(true);
                                                try {
                                                    await invoke('cmd_clean_preview_cache');
                                                    clearImageMemoryCaches();
                                                    await fetchOfflineCache();
                                                    toast.success(t('settings.offline_cache_cleared'));
                                                } catch {
                                                    toast.error(t('settings.cache_clear_failed'));
                                                } finally {
                                                    setOfflineCacheLoading(false);
                                                }
                                            }}
                                            className="rounded-lg bg-red-500/10 px-3 py-1.5 text-xs font-medium text-red-400 transition hover:bg-red-500/20 disabled:cursor-not-allowed disabled:opacity-50"
                                        >
                                            {t('settings.clear')}
                                        </button>
                                    </div>
                                </div>

                                <div className="flex items-center justify-between p-3 rounded-lg bg-telegram-hover/50">
                                    <div className="flex items-center gap-2">
                                        <Trash2 className="w-4 h-4 text-telegram-subtext" />
                                        <div>
                                            <p className="text-sm text-telegram-text font-medium">{t('settings.clear_local_cache')}</p>
                                            <p className="text-xs text-telegram-subtext">{t('settings.clear_local_cache_desc')}</p>
                                        </div>
                                    </div>
                                    <button
                                        disabled={clearing}
                                        onClick={async () => {
                                            const ok = await confirm({
                                                title: t('settings.clear_cache_title'),
                                                message: t('settings.clear_cache_desc'),
                                                confirmText: t('settings.clear'),
                                                variant: 'danger',
                                            });
                                            if (!ok) return;
                                            setClearing(true);
                                            try {
                                                await invoke('cmd_clean_cache');
                                                clearImageMemoryCaches();
                                                await fetchOfflineCache();
                                                toast.success(t('settings.cache_cleared'));
                                            } catch {
                                                toast.error(t('settings.cache_clear_failed'));
                                            } finally {
                                                setClearing(false);
                                            }
                                        }}
                                        className="px-3 py-1.5 rounded-lg text-xs font-medium bg-red-500/10 text-red-400 hover:bg-red-500/20 transition disabled:opacity-50 disabled:cursor-not-allowed"
                                    >
                                        {clearing ? t('settings.clearing') : t('settings.clear')}
                                    </button>
                                </div>

                                {/* Transcode Cache */}
                                <div className="p-3 rounded-lg bg-telegram-hover/50 space-y-3">
                                    <div className="flex items-center justify-between">
                                        <div className="flex items-center gap-2">
                                            <HardDrive className="w-4 h-4 text-telegram-subtext" />
                                            <div>
                                                <p className="text-sm text-telegram-text font-medium">{t('settings.transcode_cache')}</p>
                                                <p className="text-xs text-telegram-subtext">
                                                    {transcodeCache
                                                        ? `${(transcodeCache.total_bytes / 1048576).toFixed(1)} MB / ${(transcodeCache.max_bytes / 1073741824).toFixed(1)} GB${transcodeCache.scan_in_progress ? ' · Inspecting…' : ''}`
                                                        : cacheLoading
                                                            ? t('common.loading')
                                                            : cacheError
                                                                ? t('settings.failed_prefix', { error: cacheError })
                                                                : t('settings.no_transcoded_cached')}
                                                </p>
                                            </div>
                                        </div>
                                        <div className="flex items-center gap-1.5">
                                            <button
                                                onClick={() => void fetchTranscodeCache(true)}
                                                disabled={cacheLoading || transcodeCache?.scan_in_progress || clearingVariant !== null}
                                                className="p-1.5 rounded-md hover:bg-telegram-hover text-telegram-subtext hover:text-telegram-text transition"
                                                title={t('settings.refresh_links')}
                                            >
                                                <RefreshCw className={`w-3 h-3 ${cacheLoading || transcodeCache?.scan_in_progress ? 'animate-spin' : ''}`} />
                                            </button>
                                            <button
                                                disabled={cacheLoading || clearingVariant !== null || (!cacheError && (!transcodeCache || transcodeCache.entries.length === 0))}
                                                onClick={async () => {
                                                    const ok = await confirm({
                                                        title: t('settings.clear_transcode_title'),
                                                        message: t('settings.clear_transcode_message'),
                                                        confirmText: t('settings.clear_all'),
                                                        variant: 'danger',
                                                    });
                                                    if (!ok) return;
                                                    setClearingVariant('__all__');
                                                    try {
                                                        const msg = await invoke<string>('cmd_clear_transcode_cache', {});
                                                        toast.success(msg);
                                                        await fetchTranscodeCache(true);
                                                    } catch (e) {
                                                        toast.error(t('settings.failed_prefix', { error: e }));
                                                    } finally {
                                                        setClearingVariant(null);
                                                    }
                                                }}
                                                className="px-2.5 py-1 rounded-md text-[10px] font-medium bg-red-500/10 text-red-400 hover:bg-red-500/20 transition disabled:opacity-50 disabled:cursor-not-allowed"
                                            >
                                                {clearingVariant === '__all__' ? t('settings.clearing') : t('settings.clear_all')}
                                            </button>
                                        </div>
                                    </div>

                                    <FfmpegInstallNotice available={transcodeCapabilities?.available} />

                                    {/* Cache entries list */}
                                    {cacheLoading && !transcodeCache ? (
                                        <div className="flex items-center justify-center py-2" role="status" aria-label={t('common.loading')}>
                                            <RefreshCw className="w-3 h-3 text-telegram-subtext animate-spin" />
                                        </div>
                                    ) : cacheError ? (
                                        <div className="rounded-md border border-red-500/20 bg-red-500/5 p-2.5 text-center" role="alert">
                                            <p className="break-words text-[11px] text-red-300">{t('settings.failed_prefix', { error: cacheError })}</p>
                                            <button type="button" onClick={() => void fetchTranscodeCache(true)} className="quiet-control mt-2 px-2.5 py-1 text-[10px] text-app-text">
                                                <RefreshCw className="h-3 w-3" />
                                                {t('settings.retry_encryption_check')}
                                            </button>
                                        </div>
                                    ) : transcodeCache && transcodeCache.entries.length > 0 ? (
                                        <div className="space-y-1.5 max-h-[200px] overflow-y-auto pr-1 custom-scrollbar">
                                            {/* Group HLS variants by file_key (exclude originals, which are cleared via per-file Clear or Clear All) */}
                                            {(() => {
                                                const grouped: Record<string, CacheEntry[]> = {};
                                                for (const e of transcodeCache.entries) {
                                                    // Skip original entries — they're cleared via per-file or Clear All only
                                                    if (e.quality === 'original') continue;
                                                    if (!grouped[e.file_key]) grouped[e.file_key] = [];
                                                    grouped[e.file_key].push(e);
                                                }
                                                return Object.entries(grouped).map(([fileKey, entries]) => (
                                                    <div key={fileKey} className="p-2 rounded bg-telegram-bg/50 border border-telegram-border/30">
                                                        <div className="flex items-center justify-between mb-1">
                                                            <span className="text-[10px] font-mono text-telegram-subtext truncate max-w-[180px]" title={fileKey}>
                                                                {fileKey}
                                                            </span>
                                                            <button
                                                                disabled={clearingVariant !== null}
                                                                onClick={async () => {
                                                                    setClearingVariant(fileKey);
                                                                    try {
                                                                        const msg = await invoke<string>('cmd_clear_transcode_cache', { fileKey });
                                                                        toast.success(msg);
                                                                        await fetchTranscodeCache();
                                                                    } catch (e) {
                                                                        toast.error(t('settings.failed_prefix', { error: e }));
                                                                    } finally {
                                                                        setClearingVariant(null);
                                                                    }
                                                                }}
                                                                className="text-[9px] text-red-400/60 hover:text-red-400 transition px-1 py-0.5 rounded hover:bg-red-500/10 disabled:opacity-30"
                                                                title={t('settings.clear_variants_for', { key: fileKey })}
                                                            >
                                                                {clearingVariant === fileKey ? '...' : t('settings.clear')}
                                                            </button>
                                                        </div>
                                                        <div className="flex flex-wrap gap-1">
                                                            {entries.map(e => (
                                                                <button
                                                                    key={`${e.file_key}:${e.quality}`}
                                                                    disabled={clearingVariant !== null}
                                                                    onClick={async () => {
                                                                        const variantKey = `${e.file_key}:${e.quality}`;
                                                                        setClearingVariant(variantKey);
                                                                        try {
                                                                            const msg = await invoke<string>('cmd_clear_transcode_cache', { fileKey: e.file_key, quality: e.quality });
                                                                            toast.success(msg);
                                                                            await fetchTranscodeCache();
                                                                        } catch (err) {
                                                                            toast.error(t('settings.failed_prefix', { error: err }));
                                                                        } finally {
                                                                            setClearingVariant(null);
                                                                        }
                                                                    }}
                                                                    className={`flex items-center gap-1 px-1.5 py-0.5 rounded text-[9px] font-medium transition ${
                                                                        e.playlist_exists
                                                                            ? 'bg-emerald-500/10 text-emerald-400 hover:bg-red-500/10 hover:text-red-400 border border-emerald-500/20'
                                                                            : 'bg-amber-500/10 text-amber-400/60 border border-amber-500/20'
                                                                    } disabled:opacity-30`}
                                                                    title={`${e.quality} — ${(e.size_bytes / 1048576).toFixed(2)} MB${e.playlist_exists ? ' (ready)' : ' (partial)'}`}
                                                                >
                                     {e.quality === 'original' ? t('settings.original') : e.quality}
                                                                    <span className="text-[8px] opacity-60">{e.playlist_exists ? '✓' : '~'}</span>
                                                                </button>
                                                            ))}
                                                        </div>
                                                    </div>
                                                ));
                                            })()}
                                        </div>
                                    ) : transcodeCache && transcodeCache.entries.length === 0 ? (
                                        <p className="text-[11px] text-telegram-subtext/50 text-center py-2">{t('settings.no_transcoded_cached')}</p>
                                    ) : null}
                                </div>
                            </section>

                            {/* Updates Section */}
                            <section className="space-y-3">
                                <h3 className="text-xs font-semibold text-telegram-subtext uppercase tracking-wider flex items-center gap-2">
                                    <Sparkles className="w-3.5 h-3.5" />
                                    {t('settings.updates')}
                                </h3>

                                <div className="p-3 rounded-lg bg-telegram-hover/50 space-y-3">
                                    <div className="flex items-center justify-between">
                                        <div className="flex items-center gap-2">
                                            <Download className="w-4 h-4 text-telegram-subtext" />
                                            <div>
                                                <p className="text-sm text-telegram-text font-medium">{t('settings.check_for_updates')}</p>
                                                <p className="text-xs text-telegram-subtext">
                                                    {installationInfo?.packageManager === 'microsoft-store' ? 'Microsoft Store' : updateVersion ? t('settings.update_available', { version: updateVersion }) : t('settings.check_updates_desc')}
                                                </p>
                                            </div>
                                        </div>
                                        {updateAvailable && !updateDownloading ? (
                                            <button
                                                onClick={handleInstallUpdate}
                                                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium bg-telegram-primary text-white hover:bg-telegram-primary/90 transition"
                                            >
                                                <Download className="w-3 h-3" />
                                                {installationInfo?.managedByPackageManager ? t('common.open') : t('settings.update_restart')}
                                            </button>
                                        ) : updateDownloading ? (
                                            <div className="flex items-center gap-2">
                                                <RefreshCw className="w-3.5 h-3.5 text-telegram-primary animate-spin" />
                                                <span className="text-xs text-telegram-primary font-mono">{updateProgress}%</span>
                                            </div>
                                        ) : (
                                            <button
                                                onClick={handleCheckForUpdates}
                                                disabled={updateChecking}
                                                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium bg-telegram-primary/10 text-telegram-primary hover:bg-telegram-primary/20 transition disabled:opacity-50"
                                            >
                                                <RefreshCw className={`w-3 h-3 ${updateChecking ? 'animate-spin' : ''}`} />
                                                {updateChecking ? t('settings.checking') : t('settings.check_now')}
                                            </button>
                                        )}
                                    </div>
                                    {updateDownloading && (
                                        <div className="w-full h-1.5 bg-telegram-border rounded-full overflow-hidden">
                                            <div
                                                className="h-full bg-telegram-primary rounded-full transition-all duration-300"
                                                style={{ width: `${updateProgress}%` }}
                                            />
                                        </div>
                                    )}
                                </div>
                            </section>

                                    </GeneralSettingsTab>
                                )}

                                {activeTab === 'privacy' && (
                                    <PrivacySettingsTab
                                        crashReportingEnabled={settings.crashReportingEnabled}
                                        onCrashReportingChange={() => updateSettings({ crashReportingEnabled: !settings.crashReportingEnabled, crashReportingConsentSeen: true })}
                                        settings={settings}
                                        onSettingsChange={updateSettings}
                                        onSettingsSyncEnabledChange={enabled => updateSetting('telegramSettingsSyncEnabled', enabled)}
                                    />
                                )}

                                {activeTab === 'advanced' && (
                                    <AdvancedSettingsTab
                                        onOpenApi={() => { setShowGeneralAdvanced(true); setActiveTab('general'); }}
                                        onOpenWebDav={() => setActiveTab('webdav')}
                                        onOpenProxy={() => setActiveTab('proxy')}
                                        onOpenVpn={() => setActiveTab('vpn')}
                                    />
                                )}

                                {activeTab === 'proxy' && (
                                    <ProxySettingsTab>
                                <h3 className="text-xs font-semibold text-telegram-subtext uppercase tracking-wider flex items-center gap-2">
                                    <Shield className="w-3.5 h-3.5" />
                                    {t('settings.proxy_config')}
                                </h3>

                                {/* Enable Proxy */}
                                <div className="flex items-center justify-between p-3 rounded-lg bg-telegram-hover/50">
                                    <div className="flex items-center gap-2">
                                        <div className={`w-2.5 h-2.5 rounded-full ${
                                            !settings.proxyEnabled || !settings.proxyLiveStateEnabled
                                                ? 'bg-gray-500' 
                                                : !proxyStatus 
                                                    ? 'bg-amber-400 animate-pulse shadow-[0_0_6px_rgba(251,191,36,0.5)]' 
                                                    : proxyStatus.reachable 
                                                        ? 'bg-green-400 shadow-[0_0_6px_rgba(74,222,128,0.5)]' 
                                                        : 'bg-red-500 shadow-[0_0_6px_rgba(239,68,68,0.5)]'
                                        }`} />
                                        <div>
                                            <div className="flex items-center gap-2">
                                                <p className="text-sm text-telegram-text font-medium">{t('common.enable_proxy')}</p>
                                                {settings.proxyEnabled && (
                                                    <span className="text-[10px] px-1.5 py-0.5 rounded bg-white/5 text-telegram-subtext font-mono">
                                                        {!settings.proxyLiveStateEnabled
                                                            ? t('settings.proxy_status_off') || 'Off'
                                                            : !proxyStatus 
                                                                ? t('settings.proxy_status_checking') || 'Checking…' 
                                                                : proxyStatus.reachable 
                                                                    ? `${t('settings.proxy_status_connected') || 'Connected'} (${proxyStatus.latency_ms}ms)` 
                                                                    : t('settings.proxy_status_unreachable') || 'Unreachable'}
                                                    </span>
                                                )}
                                            </div>
                                            <p className="text-xs text-telegram-subtext">{t('settings.enable_proxy_desc')}</p>
                                        </div>
                                    </div>
                                    <button
                                        type="button" role="switch" aria-checked={settings.proxyEnabled} aria-label={t('common.enable_proxy')}
                                        onClick={() => updateSetting('proxyEnabled', !settings.proxyEnabled)}
                                        className={`relative w-11 h-6 rounded-full transition-colors duration-200 ${settings.proxyEnabled ? 'bg-telegram-primary' : 'bg-telegram-border'}`}
                                    >
                                        <span className={`absolute top-0.5 start-0.5 w-5 h-5 rounded-full bg-white shadow transition-transform duration-200 ${settings.proxyEnabled ? 'translate-x-5 rtl:-translate-x-5' : 'translate-x-0'}`} />
                                    </button>
                                </div>

                                {/* Live Connection Monitoring */}
                                {settings.proxyEnabled && (
                                    <div className="flex items-center justify-between p-3 rounded-lg bg-telegram-hover/50">
                                        <div>
                                            <p className="text-sm text-telegram-text font-medium">{t('settings.live_state') || 'Live Connection Monitoring'}</p>
                                            <p className="text-xs text-telegram-subtext">{t('settings.live_state_desc') || 'Periodically check connectivity and display latency'}</p>
                                        </div>
                                        <button
                                            type="button" role="switch" aria-checked={settings.proxyLiveStateEnabled} aria-label={t('settings.live_state')}
                                            onClick={() => updateSetting('proxyLiveStateEnabled', !settings.proxyLiveStateEnabled)}
                                            className={`relative w-11 h-6 rounded-full transition-colors duration-200 ${settings.proxyLiveStateEnabled ? 'bg-telegram-primary' : 'bg-telegram-border'}`}
                                        >
                                            <span className={`absolute top-0.5 start-0.5 w-5 h-5 rounded-full bg-white shadow transition-transform duration-200 ${settings.proxyLiveStateEnabled ? 'translate-x-5 rtl:-translate-x-5' : 'translate-x-0'}`} />
                                        </button>
                                    </div>
                                )}

                                {/* Proxy Type */}
                                <div className="flex items-center justify-between p-3 rounded-lg bg-telegram-hover/50">
                                    <div>
                                        <p className="text-sm text-telegram-text font-medium">{t('common.proxy_type')}</p>
                                        <p className="text-xs text-telegram-subtext">
                                            {settings.proxyType === 'socks5' 
                                                ? t('settings.socks5_desc') 
                                                : t('settings.http_bridge_desc') || 'HTTP/HTTPS proxy tunneling via local SOCKS5 bridge.'}
                                        </p>
                                    </div>
                                    <div className="relative">
                                        <select
                                            value={settings.proxyType}
                                            onChange={e => updateSetting('proxyType', e.target.value as 'socks5' | 'http' | 'https')}
                                            aria-label={t('common.proxy_type')}
                                            className="appearance-none bg-telegram-bg border border-telegram-border rounded-md pl-3 pr-8 py-1.5 text-sm text-telegram-text focus:outline-none focus:border-telegram-primary/50 transition cursor-pointer"
                                        >
                                            <option value="socks5">SOCKS5</option>
                                            <option value="http">HTTP</option>
                                            <option value="https">HTTPS</option>
                                        </select>
                                        <ChevronDown className="w-4 h-4 text-telegram-subtext absolute right-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
                                    </div>
                                </div>

                                {/* Host */}
                                <div className="flex items-center justify-between p-3 rounded-lg bg-telegram-hover/50">
                                    <div>
                                        <p className="text-sm text-telegram-text font-medium">{t('common.host')}</p>
                                        <p className="text-xs text-telegram-subtext">{t('settings.host_desc')}</p>
                                    </div>
                                    <input
                                        type="text"
                                        placeholder="e.g. 127.0.0.1"
                                        value={settings.proxyHost}
                                        onChange={e => updateSetting('proxyHost', e.target.value)}
                                        className="w-40 bg-telegram-bg border border-telegram-border rounded-md px-2 py-1 text-sm text-telegram-text text-right focus:outline-none focus:border-telegram-primary/50 transition placeholder:text-telegram-subtext/40"
                                    />
                                </div>

                                {/* Port */}
                                <div className="flex items-center justify-between p-3 rounded-lg bg-telegram-hover/50">
                                    <div>
                                        <p className="text-sm text-telegram-text font-medium">{t('common.port')}</p>
                                        <p className="text-xs text-telegram-subtext">{t('settings.port_desc')}</p>
                                    </div>
                                    <input
                                        type="number"
                                        min="1"
                                        max="65535"
                                        value={settings.proxyPort}
                                        onChange={e => updateSetting('proxyPort', Math.max(1, Math.min(65535, parseInt(e.target.value) || 1080)))}
                                        className="w-20 bg-telegram-bg border border-telegram-border rounded-md px-2 py-1 text-sm text-telegram-text text-center focus:outline-none focus:border-telegram-primary/50 transition"
                                    />
                                </div>

                                {/* SOCKS5/HTTP auth fields */}
                                <div className="flex items-center justify-between p-3 rounded-lg bg-telegram-hover/50">
                                    <div>
                                        <p className="text-sm text-telegram-text font-medium">{t('common.username')}</p>
                                        <p className="text-xs text-telegram-subtext">{t('settings.optional')}</p>
                                    </div>
                                    <input
                                        type="text"
                                        placeholder={t('settings.optional')}
                                        value={settings.proxyUsername}
                                        onChange={e => updateSetting('proxyUsername', e.target.value)}
                                        className="w-40 bg-telegram-bg border border-telegram-border rounded-md px-2 py-1 text-sm text-telegram-text text-right focus:outline-none focus:border-telegram-primary/50 transition placeholder:text-telegram-subtext/40"
                                    />
                                </div>
                                <div className="flex items-center justify-between p-3 rounded-lg bg-telegram-hover/50">
                                    <div>
                                        <p className="text-sm text-telegram-text font-medium">{t('common.password')}</p>
                                        <p className="text-xs text-telegram-subtext">{t('settings.optional')}</p>
                                    </div>
                                    <div className="flex items-center gap-2">
                                        <input
                                            type="password"
                                            placeholder={t('settings.optional')}
                                            value={settings.proxyPassword}
                                            onChange={e => updateSetting('proxyPassword', e.target.value)}
                                            className="w-40 bg-telegram-bg border border-telegram-border rounded-md px-2 py-1 text-sm text-telegram-text text-right focus:outline-none focus:border-telegram-primary/50 transition placeholder:text-telegram-subtext/40"
                                        />
                                        <button
                                            type="button"
                                            aria-label={`${t('settings.clear')} ${t('common.password')}`}
                                            title={`${t('settings.clear')} ${t('common.password')}`}
                                            onClick={async () => {
                                                try {
                                                    await invoke('cmd_clear_proxy_secret');
                                                    updateSetting('proxyPassword', '');
                                                } catch (error) {
                                                    toast.error(userFacingError(error, t));
                                                }
                                            }}
                                            className="rounded-md border border-telegram-border px-2 py-1 text-xs text-telegram-subtext transition hover:border-telegram-primary/50 hover:text-telegram-text"
                                        >
                                            {t('settings.clear')}
                                        </button>
                                    </div>
                                </div>

                                {/* Info note */}
                                <div className="p-3 rounded-lg bg-yellow-500/5 border border-yellow-500/10 space-y-3">
                                    <p className="text-[11px] text-yellow-400/70 leading-relaxed">
                                        {t('settings.proxy_reconnect_note')}
                                    </p>
                                    <div className="flex items-center gap-2">
                                        <button
                                            onClick={async () => {
                                                setReconnecting(true);
                                                try {
                                                    const ok = await invoke<boolean>('cmd_reconnect_with_network_settings');
                                                    if (ok) {
                                                        toast.success(t('settings.reconnect_success_toast'));
                                                    } else {
                                                        toast.error(t('settings.reconnect_failed_toast'));
                                                    }
                                                } catch (e) {
                                                    toast.error(t('settings.reconnect_failed_err_toast', { error: e }));
                                                } finally {
                                                    setReconnecting(false);
                                                }
                                            }}
                                            disabled={reconnecting}
                                            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium bg-telegram-primary/10 text-telegram-primary hover:bg-telegram-primary/20 transition disabled:opacity-50 disabled:cursor-not-allowed"
                                        >
                                            {reconnecting ? (
                                                <>
                                                    <Loader2 className="w-3 h-3 animate-spin" />
                                                    {t('settings.reconnecting')}
                                                </>
                                            ) : (
                                                <>
                                                    <RefreshCw className="w-3 h-3" />
                                                    {t('settings.reconnect_now')}
                                                </>
                                            )}
                                        </button>
                                        <button
                                            onClick={async () => {
                                                setIsTestingProxy(true);
                                                try {
                                                    const success = await invoke<boolean>('cmd_test_proxy_traffic');
                                                    if (success) {
                                                        toast.success(t('settings.proxy_test_success') || 'Proxy connection working!');
                                                    } else {
                                                        toast.error(t('settings.proxy_test_failed') || 'Proxy traffic test failed.');
                                                    }
                                                } catch (e) {
                                                    toast.error(`Error testing proxy: ${e}`);
                                                } finally {
                                                    setIsTestingProxy(false);
                                                }
                                            }}
                                            disabled={isTestingProxy || reconnecting || !settings.proxyEnabled}
                                            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium bg-white/5 text-telegram-text hover:bg-white/10 transition disabled:opacity-30 disabled:cursor-not-allowed"
                                        >
                                            {isTestingProxy ? (
                                                <>
                                                    <Loader2 className="w-3 h-3 animate-spin" />
                                                    {t('settings.proxy_testing') || 'Testing…'}
                                                </>
                                            ) : (
                                                <>
                                                    <Play className="w-3.5 h-3.5" />
                                                    {t('settings.test_connection') || 'Test Connection'}
                                                </>
                                            )}
                                        </button>
                                    </div>
                                </div>
                            </ProxySettingsTab>
                        )}

                        {activeTab === 'vpn' && (
                                    <VpnSettingsTab>
                                <h3 className="text-xs font-semibold text-telegram-subtext uppercase tracking-wider flex items-center gap-2">
                                    <Zap className="w-3.5 h-3.5" />
                                    {t('settings.vpn_optimizer')}
                                    {latencyMs !== null && (
                                        <span className={`ml-auto text-[10px] font-mono px-1.5 py-0.5 rounded-full ${
                                            latencyMs < 0 ? 'bg-red-500/10 text-red-400' :
                                            latencyMs < 100 ? 'bg-green-500/10 text-green-400' :
                                            latencyMs < 300 ? 'bg-yellow-500/10 text-yellow-400' :
                                            'bg-red-500/10 text-red-400'
                                        }`}>
                                            <Activity className="w-3 h-3 inline mr-0.5" />
                                            {latencyMs < 0 ? 'Offline' : `${latencyMs}ms`}
                                        </span>
                                    )}
                                </h3>

                                {/* Master Toggle */}
                                <div className="flex items-center justify-between p-3 rounded-lg bg-telegram-hover/50">
                                    <div className="flex items-center gap-2">
                                        <div className={`w-2 h-2 rounded-full ${settings.vpnMode ? 'bg-emerald-400 shadow-[0_0_6px_rgba(52,211,153,0.5)]' : 'bg-gray-500'}`} />
                                        <div>
                                            <p className="text-sm text-telegram-text font-medium">{t('settings.vpn_mode')}</p>
                                            <p className="text-xs text-telegram-subtext">{t('settings.vpn_mode_desc')}</p>
                                        </div>
                                    </div>
                                    <button
                                        type="button" role="switch" aria-checked={settings.vpnMode} aria-label={t('settings.vpn_mode')}
                                        onClick={() => updateSetting('vpnMode', !settings.vpnMode)}
                                        className={`relative w-11 h-6 rounded-full transition-colors duration-200 ${settings.vpnMode ? 'bg-emerald-500' : 'bg-telegram-border'}`}
                                    >
                                        <span className={`absolute top-0.5 start-0.5 w-5 h-5 rounded-full bg-white shadow transition-transform duration-200 ${settings.vpnMode ? 'translate-x-5 rtl:-translate-x-5' : 'translate-x-0'}`} />
                                    </button>
                                </div>

                                {settings.vpnMode && (<>
                                    {/* Timeout Multiplier */}
                                    <div className="p-3 rounded-lg bg-telegram-hover/50 space-y-2">
                                        <div className="flex items-center justify-between">
                                            <div>
                                                <p className="text-sm text-telegram-text font-medium">{t('settings.timeout_multiplier')}</p>
                                                <p className="text-xs text-telegram-subtext">{t('settings.timeout_multiplier_desc')}</p>
                                            </div>
                                            <span className="text-sm text-telegram-primary font-mono font-medium">{settings.timeoutMultiplier}×</span>
                                        </div>
                                        <input type="range" min="1" max="5" step="1" value={settings.timeoutMultiplier}
                                            aria-label={t('settings.timeout_multiplier')}
                                            onChange={e => updateSetting('timeoutMultiplier', parseInt(e.target.value))}
                                            className="w-full h-1.5 rounded-full appearance-none bg-telegram-border accent-telegram-primary cursor-pointer" />
                                    </div>

                                    {/* Retry Attempts */}
                                    <div className="p-3 rounded-lg bg-telegram-hover/50 space-y-2">
                                        <div className="flex items-center justify-between">
                                            <div>
                                                <p className="text-sm text-telegram-text font-medium">{t('settings.retry_attempts')}</p>
                                                <p className="text-xs text-telegram-subtext">{t('settings.retry_attempts_desc')}</p>
                                            </div>
                                            <span className="text-sm text-telegram-primary font-mono font-medium">{settings.retryAttempts}</span>
                                        </div>
                                        <input type="range" min="0" max="5" step="1" value={settings.retryAttempts}
                                            aria-label={t('settings.retry_attempts')}
                                            onChange={e => updateSetting('retryAttempts', parseInt(e.target.value))}
                                            className="w-full h-1.5 rounded-full appearance-none bg-telegram-border accent-telegram-primary cursor-pointer" />
                                    </div>

                                    {/* Backoff Settings */}
                                    <div className="p-3 rounded-lg bg-telegram-hover/50 space-y-2">
                                        <p className="text-sm text-telegram-text font-medium">{t('settings.retry_backoff')}</p>
                                        <div className="flex items-center justify-between">
                                            <p className="text-xs text-telegram-subtext">{t('settings.base_delay')}</p>
                                            <span className="text-xs text-telegram-primary font-mono">{settings.retryBaseBackoffSec}s</span>
                                        </div>
                                        <input type="range" min="0.5" max="5" step="0.5" value={settings.retryBaseBackoffSec}
                                            aria-label={t('settings.base_delay')}
                                            onChange={e => updateSetting('retryBaseBackoffSec', parseFloat(e.target.value))}
                                            className="w-full h-1.5 rounded-full appearance-none bg-telegram-border accent-telegram-primary cursor-pointer" />
                                        <div className="flex items-center justify-between">
                                            <p className="text-xs text-telegram-subtext">{t('settings.max_delay')}</p>
                                            <span className="text-xs text-telegram-primary font-mono">{settings.retryMaxBackoffSec}s</span>
                                        </div>
                                        <input type="range" min="8" max="60" step="2" value={settings.retryMaxBackoffSec}
                                            aria-label={t('settings.max_delay')}
                                            onChange={e => updateSetting('retryMaxBackoffSec', parseInt(e.target.value))}
                                            className="w-full h-1.5 rounded-full appearance-none bg-telegram-border accent-telegram-primary cursor-pointer" />
                                    </div>

                                    {/* Adaptive Polling */}
                                    <div className="p-3 rounded-lg bg-telegram-hover/50 space-y-2">
                                        <div className="flex items-center justify-between">
                                            <div>
                                                <p className="text-sm text-telegram-text font-medium">{t('settings.adaptive_polling')}</p>
                                                <p className="text-xs text-telegram-subtext">{t('settings.adaptive_polling_desc')}</p>
                                            </div>
                                            <button
                                                type="button" role="switch" aria-checked={settings.adaptivePolling} aria-label={t('settings.adaptive_polling')}
                                                onClick={() => updateSetting('adaptivePolling', !settings.adaptivePolling)}
                                                className={`relative w-11 h-6 rounded-full transition-colors duration-200 ${settings.adaptivePolling ? 'bg-telegram-primary' : 'bg-telegram-border'}`}
                                            >
                                                <span className={`absolute top-0.5 start-0.5 w-5 h-5 rounded-full bg-white shadow transition-transform duration-200 ${settings.adaptivePolling ? 'translate-x-5 rtl:-translate-x-5' : 'translate-x-0'}`} />
                                            </button>
                                        </div>
                                        {settings.adaptivePolling && (<>
                                            <div className="flex items-center justify-between">
                                                <p className="text-xs text-telegram-subtext">{t('settings.min_interval')}</p>
                                                <span className="text-xs text-telegram-primary font-mono">{settings.pollingMinSec}s</span>
                                            </div>
                                            <input type="range" min="10" max="30" step="5" value={settings.pollingMinSec}
                                                aria-label={t('settings.min_interval')}
                                                onChange={e => updateSetting('pollingMinSec', parseInt(e.target.value))}
                                                className="w-full h-1.5 rounded-full appearance-none bg-telegram-border accent-telegram-primary cursor-pointer" />
                                            <div className="flex items-center justify-between">
                                                <p className="text-xs text-telegram-subtext">{t('settings.max_interval')}</p>
                                                <span className="text-xs text-telegram-primary font-mono">{settings.pollingMaxSec}s</span>
                                            </div>
                                            <input type="range" min="45" max="120" step="15" value={settings.pollingMaxSec}
                                                aria-label={t('settings.max_interval')}
                                                onChange={e => updateSetting('pollingMaxSec', parseInt(e.target.value))}
                                                className="w-full h-1.5 rounded-full appearance-none bg-telegram-border accent-telegram-primary cursor-pointer" />
                                        </>)}
                                    </div>

                                    {/* Preferred DC */}
                                    <div className="flex items-center justify-between p-3 rounded-lg bg-telegram-hover/50">
                                        <div>
                                            <p className="text-sm text-telegram-text font-medium">{t('settings.preferred_dc')}</p>
                                            <p className="text-xs text-telegram-subtext">{t('settings.preferred_dc_desc')}</p>
                                        </div>
                                        <div className="relative">
                                            <select
                                                value={settings.preferredDC}
                                                onChange={e => updateSetting('preferredDC', e.target.value as typeof settings.preferredDC)}
                                                aria-label={t('settings.preferred_dc')}
                                                className="appearance-none bg-telegram-bg border border-telegram-border rounded-md pl-3 pr-8 py-1.5 text-sm text-telegram-text focus:outline-none focus:border-telegram-primary/50 transition cursor-pointer"
                                            >
                                                <option value="auto">{t('settings.auto')}</option>
                                                <option value="dc1">DC 1</option>
                                                <option value="dc2">DC 2</option>
                                                <option value="dc3">DC 3</option>
                                                <option value="dc4">DC 4</option>
                                                <option value="dc5">DC 5</option>
                                            </select>
                                            <ChevronDown className="w-4 h-4 text-telegram-subtext absolute right-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
                                        </div>
                                    </div>

                                    {/* DC Fallback Attempts */}
                                    <div className="p-3 rounded-lg bg-telegram-hover/50 space-y-2">
                                        <div className="flex items-center justify-between">
                                            <div>
                                                <p className="text-sm text-telegram-text font-medium">{t('settings.dc_fallback_attempts')}</p>
                                                <p className="text-xs text-telegram-subtext">{t('settings.dc_fallback_desc')}</p>
                                            </div>
                                            <span className="text-sm text-telegram-primary font-mono font-medium">{settings.dcFallbackAttempts}</span>
                                        </div>
                                        <input type="range" min="1" max="4" step="1" value={settings.dcFallbackAttempts}
                                            aria-label={t('settings.dc_fallback_attempts')}
                                            onChange={e => updateSetting('dcFallbackAttempts', parseInt(e.target.value))}
                                            className="w-full h-1.5 rounded-full appearance-none bg-telegram-border accent-telegram-primary cursor-pointer" />
                                    </div>

                                    {/* Flood Wait */}
                                    <div className="flex items-center justify-between p-3 rounded-lg bg-telegram-hover/50">
                                        <div>
                                            <p className="text-sm text-telegram-text font-medium">{t('settings.respect_flood')}</p>
                                            <p className="text-xs text-telegram-subtext">{t('settings.respect_flood_desc')}</p>
                                        </div>
                                        <button
                                            type="button" role="switch" aria-checked={settings.floodWaitRespect} aria-label={t('settings.respect_flood')}
                                            onClick={() => updateSetting('floodWaitRespect', !settings.floodWaitRespect)}
                                            className={`relative w-11 h-6 rounded-full transition-colors duration-200 ${settings.floodWaitRespect ? 'bg-telegram-primary' : 'bg-telegram-border'}`}
                                        >
                                            <span className={`absolute top-0.5 start-0.5 w-5 h-5 rounded-full bg-white shadow transition-transform duration-200 ${settings.floodWaitRespect ? 'translate-x-5 rtl:-translate-x-5' : 'translate-x-0'}`} />
                                        </button>
                                    </div>

                                    {/* Peer Cache Size */}
                                    <div className="p-3 rounded-lg bg-telegram-hover/50 space-y-2">
                                        <div className="flex items-center justify-between">
                                            <div>
                                                <p className="text-sm text-telegram-text font-medium">{t('settings.peer_cache_size')}</p>
                                                <p className="text-xs text-telegram-subtext">{t('settings.peer_cache_desc')}</p>
                                            </div>
                                            <span className="text-sm text-telegram-primary font-mono font-medium">{settings.peerCacheSize}</span>
                                        </div>
                                        <input type="range" min="100" max="2000" step="100" value={settings.peerCacheSize}
                                            aria-label={t('settings.peer_cache_size')}
                                            onChange={e => updateSetting('peerCacheSize', parseInt(e.target.value))}
                                            className="w-full h-1.5 rounded-full appearance-none bg-telegram-border accent-telegram-primary cursor-pointer" />
                                    </div>

                                    {/* Bandwidth Throttle */}
                                    <div className="p-3 rounded-lg bg-telegram-hover/50 space-y-2">
                                        <p className="text-sm text-telegram-text font-medium flex items-center gap-1.5">
                                            <Gauge className="w-3.5 h-3.5 text-telegram-subtext" />
                                            {t('settings.bandwidth_throttle')}
                                        </p>
                                        <div className="flex items-center justify-between">
                                            <p className="text-xs text-telegram-subtext">{t('settings.upload_limit')}</p>
                                            <span className="text-xs text-telegram-primary font-mono">
                                                {settings.bandwidthLimitUpKBs === 0 ? t('settings.unlimited') : `${settings.bandwidthLimitUpKBs} KB/s`}
                                            </span>
                                        </div>
                                        <input type="range" min="0" max="5120" step="128" value={settings.bandwidthLimitUpKBs}
                                            aria-label={t('settings.upload_limit')}
                                            onChange={e => updateSetting('bandwidthLimitUpKBs', parseInt(e.target.value))}
                                            className="w-full h-1.5 rounded-full appearance-none bg-telegram-border accent-telegram-primary cursor-pointer" />
                                        <div className="flex items-center justify-between">
                                            <p className="text-xs text-telegram-subtext">{t('settings.download_limit')}</p>
                                            <span className="text-xs text-telegram-primary font-mono">
                                                {settings.bandwidthLimitDownKBs === 0 ? t('settings.unlimited') : `${settings.bandwidthLimitDownKBs} KB/s`}
                                            </span>
                                        </div>
                                        <input type="range" min="0" max="5120" step="128" value={settings.bandwidthLimitDownKBs}
                                            aria-label={t('settings.download_limit')}
                                            onChange={e => updateSetting('bandwidthLimitDownKBs', parseInt(e.target.value))}
                                            className="w-full h-1.5 rounded-full appearance-none bg-telegram-border accent-telegram-primary cursor-pointer" />
                                    </div>

                                    {/* Chunk Size */}
                                    <div className="flex items-center justify-between p-3 rounded-lg bg-telegram-hover/50">
                                        <div>
                                            <p className="text-sm text-telegram-text font-medium">{t('settings.transfer_chunk_size')}</p>
                                            <p className="text-xs text-telegram-subtext">{t('settings.chunk_size_desc')}</p>
                                        </div>
                                        <div className="relative">
                                            <select
                                                value={settings.chunkSizeKb}
                                                onChange={e => updateSetting('chunkSizeKb', parseInt(e.target.value))}
                                                aria-label={t('settings.transfer_chunk_size')}
                                                className="appearance-none bg-telegram-bg border border-telegram-border rounded-md pl-3 pr-8 py-1.5 text-sm text-telegram-text focus:outline-none focus:border-telegram-primary/50 transition cursor-pointer"
                                            >
                                                <option value={128}>128 KB</option>
                                                <option value={256}>256 KB</option>
                                                <option value={512}>512 KB</option>
                                            </select>
                                            <ChevronDown className="w-4 h-4 text-telegram-subtext absolute right-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
                                        </div>
                                    </div>

                                    {/* Keep-Alive */}
                                    <div className="p-3 rounded-lg bg-telegram-hover/50 space-y-2">
                                        <div className="flex items-center justify-between">
                                            <div>
                                                <p className="text-sm text-telegram-text font-medium">{t('settings.keep_alive')}</p>
                                                <p className="text-xs text-telegram-subtext">{t('settings.keep_alive_desc')}</p>
                                            </div>
                                            <span className="text-sm text-telegram-primary font-mono font-medium">
                                                {settings.keepAliveIntervalSec === 0 ? t('settings.off') : `${settings.keepAliveIntervalSec}s`}
                                            </span>
                                        </div>
                                        <input type="range" min="0" max="120" step="15" value={settings.keepAliveIntervalSec}
                                            aria-label={t('settings.keep_alive')}
                                            onChange={e => updateSetting('keepAliveIntervalSec', parseInt(e.target.value))}
                                            className="w-full h-1.5 rounded-full appearance-none bg-telegram-border accent-telegram-primary cursor-pointer" />
                                    </div>

                                    {/* Archive Size Limit */}
                                    <div className="p-3 rounded-lg bg-telegram-hover/50 space-y-2">
                                        <div className="flex items-center justify-between">
                                            <div>
                                                <p className="text-sm text-telegram-text font-medium">{t('settings.bulk_archive_limit')}</p>
                                                <p className="text-xs text-telegram-subtext">{t('settings.bulk_archive_desc')}</p>
                                            </div>
                                            <span className="text-sm text-telegram-primary font-mono font-medium">
                                                {settings.archiveMaxBytes === 0 ? t('settings.unlimited') : `${settings.archiveMaxBytes} MiB`}
                                            </span>
                                        </div>
                                        <input type="range" min="0" max="2048" step="64" value={settings.archiveMaxBytes}
                                            aria-label={t('settings.bulk_archive_limit')}
                                            onChange={e => updateSetting('archiveMaxBytes', parseInt(e.target.value))}
                                            className="w-full h-1.5 rounded-full appearance-none bg-telegram-border accent-telegram-primary cursor-pointer" />
                                    </div>

                                    {/* Auto-Detect VPN */}
                                    <div className="flex items-center justify-between p-3 rounded-lg bg-telegram-hover/50">
                                        <div className="flex items-center gap-2">
                                            <Wifi className="w-4 h-4 text-telegram-subtext" />
                                            <div>
                                                <p className="text-sm text-telegram-text font-medium">{t('settings.auto_detect_vpn')}</p>
                                                <p className="text-xs text-telegram-subtext">
                                                    {vpnDetected === true ? t('settings.vpn_detected') : vpnDetected === false ? t('settings.no_vpn_detected') : t('settings.checking')}
                                                </p>
                                            </div>
                                        </div>
                                        <button
                                            type="button" role="switch" aria-checked={settings.autoDetectVpn} aria-label={t('settings.auto_detect_vpn')}
                                            onClick={() => updateSetting('autoDetectVpn', !settings.autoDetectVpn)}
                                            className={`relative w-11 h-6 rounded-full transition-colors duration-200 ${settings.autoDetectVpn ? 'bg-telegram-primary' : 'bg-telegram-border'}`}
                                        >
                                            <span className={`absolute top-0.5 start-0.5 w-5 h-5 rounded-full bg-white shadow transition-transform duration-200 ${settings.autoDetectVpn ? 'translate-x-5 rtl:-translate-x-5' : 'translate-x-0'}`} />
                                        </button>
                                    </div>
                                </>)}
                                    </VpnSettingsTab>
                                )}

                                {activeTab === 'webdav' && (
                                    <WebDavSettingsTab>
                                        <div className="flex items-start gap-3 rounded-lg border border-telegram-primary/20 bg-telegram-primary/5 p-3">
                                            <HardDrive className="mt-0.5 h-4 w-4 shrink-0 text-telegram-primary" />
                                            <div>
                                                <h3 className="text-sm font-semibold text-telegram-text">{t('settings.webdav_title')}</h3>
                                                <p className="mt-1 text-xs leading-relaxed text-telegram-subtext">{t('settings.webdav_description')}</p>
                                                <p className="mt-1 text-[11px] text-telegram-primary">{t('settings.webdav_local_only')}</p>
                                                <p className="mt-2 text-xs leading-relaxed text-telegram-subtext"><strong className="text-telegram-text">Use WebDAV, not SMB.</strong> Connect with the complete generated <code>/dav/&lt;token&gt;/</code> URL. Finder's Guest/anonymous login has no token and will show an empty location; no guest account is created.</p>
                                            </div>
                                        </div>
                                        <button type="button" onClick={() => setAccessTransparency('webdav')} className="quiet-control flex w-full items-center justify-center gap-2 border border-app-border-subtle px-3 py-2 text-xs font-medium text-app-accent"><Info className="h-3.5 w-3.5" aria-hidden="true" />Understand WebDAV permissions</button>

                                        {!webDavSettings.supported ? (
                                            <div className="rounded-lg border border-amber-500/20 bg-amber-500/5 p-4 text-sm text-amber-300">
                                                {t('settings.webdav_mobile_unavailable')}
                                            </div>
                                        ) : (
                                            <>
                                                <div className="flex items-center justify-between rounded-lg bg-telegram-hover/50 p-3">
                                                    <div className="flex items-center gap-2">
                                                        <div className={`h-2.5 w-2.5 rounded-full ${
                                                            webDavSettings.running
                                                                ? 'bg-green-400 shadow-[0_0_6px_rgba(74,222,128,0.5)]'
                                                                : webDavSettings.enabled
                                                                    ? 'animate-pulse bg-amber-400'
                                                                    : 'bg-gray-500'
                                                        }`} />
                                                        <div>
                                                            <p className="text-sm font-medium text-telegram-text">{t('settings.enable_webdav')}</p>
                                                            <p className="text-xs text-telegram-subtext">
                                                                {webDavSettings.running
                                                                    ? t('settings.webdav_running', { port: webDavSettings.port })
                                                                    : t('settings.webdav_stopped')}
                                                            </p>
                                                        </div>
                                                    </div>
                                                    <button
                                                        type="button"
                                                        role="switch"
                                                        aria-checked={webDavSettings.enabled}
                                                        onClick={handleWebDavToggle}
                                                        disabled={webDavLoading || webDavGenerating}
                                                        aria-label={t('settings.enable_webdav')}
                                                        className={`relative h-6 w-11 rounded-full transition-colors duration-200 ${webDavSettings.enabled ? 'bg-telegram-primary' : 'bg-telegram-border'} disabled:opacity-50`}
                                                    >
                                                        <span className={`absolute start-0.5 top-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform duration-200 ${webDavSettings.enabled ? 'translate-x-5 rtl:-translate-x-5' : 'translate-x-0'}`} />
                                                    </button>
                                                </div>

                                                <div className="flex items-center justify-between rounded-lg bg-telegram-hover/50 p-3">
                                                    <div>
                                                        <p className="text-sm font-medium text-telegram-text">{t('common.port')}</p>
                                                        <p className="text-xs text-telegram-subtext">{t('settings.webdav_port_desc')}</p>
                                                    </div>
                                                    <input
                                                        type="number"
                                                        inputMode="numeric"
                                                        min="1024"
                                                        max="65535"
                                                        value={webDavPort}
                                                        onChange={event => setWebDavPort(event.target.value)}
                                                        onBlur={handleWebDavPortApply}
                                                        onKeyDown={event => {
                                                            if (event.key === 'Enter') {
                                                                event.currentTarget.blur();
                                                            }
                                                        }}
                                                        disabled={webDavLoading || webDavGenerating}
                                                        aria-label={t('common.port')}
                                                        className="w-24 rounded-md border border-telegram-border bg-telegram-bg px-2 py-1.5 text-center font-mono text-sm text-telegram-text outline-none transition focus:border-telegram-primary/50 disabled:opacity-50"
                                                    />
                                                </div>

                                                <div className="flex items-center justify-between rounded-lg bg-telegram-hover/50 p-3">
                                                    <div className="max-w-[75%]">
                                                        <p className="text-sm font-medium text-telegram-text">{t('settings.webdav_allow_changes')}</p>
                                                        <p className="text-xs text-telegram-subtext">{t('settings.webdav_allow_changes_desc')}</p>
                                                    </div>
                                                    <button
                                                        type="button"
                                                        role="switch"
                                                        aria-checked={webDavSettings.write_enabled}
                                                        onClick={handleWebDavWriteToggle}
                                                        disabled={webDavLoading || webDavGenerating}
                                                        aria-label={t('settings.webdav_allow_changes')}
                                                        className={`relative h-6 w-11 shrink-0 rounded-full transition-colors duration-200 ${webDavSettings.write_enabled ? 'bg-telegram-primary' : 'bg-telegram-border'} disabled:opacity-50`}
                                                    >
                                                        <span className={`absolute start-0.5 top-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform duration-200 ${webDavSettings.write_enabled ? 'translate-x-5 rtl:-translate-x-5' : 'translate-x-0'}`} />
                                                    </button>
                                                </div>

                                                <div className="space-y-3 rounded-lg bg-telegram-hover/50 p-3">
                                                    <div className="flex items-center justify-between gap-3">
                                                        <div className="flex min-w-0 items-center gap-2">
                                                            <Key className="h-4 w-4 shrink-0 text-telegram-subtext" />
                                                            <div className="min-w-0">
                                                                <p className="text-sm font-medium text-telegram-text">{t('settings.webdav_connection_link')}</p>
                                                                <p className="text-xs text-telegram-subtext">
                                                                    {webDavSettings.token_set
                                                                        ? t('settings.webdav_link_configured')
                                                                        : t('settings.webdav_link_unset')}
                                                                </p>
                                                            </div>
                                                        </div>
                                                        <button
                                                            onClick={() => void generateWebDavLink(true)}
                                                            disabled={webDavLoading || webDavGenerating}
                                                            className="flex shrink-0 items-center gap-1.5 rounded-lg bg-telegram-primary/10 px-3 py-1.5 text-xs font-medium text-telegram-primary transition hover:bg-telegram-primary/20 disabled:opacity-50"
                                                        >
                                                            <RefreshCw className={`h-3 w-3 ${webDavGenerating ? 'animate-spin' : ''}`} />
                                                            {webDavSettings.token_set ? t('settings.regenerate') : t('settings.generate')}
                                                        </button>
                                                    </div>

                                                    {generatedWebDavUrl && (
                                                        <div className="rounded-lg border border-yellow-500/20 bg-telegram-bg p-2.5">
                                                            <p className="mb-1.5 text-[10px] font-semibold uppercase tracking-wider text-yellow-400/80">
                                                                {t('settings.webdav_copy_alert')}
                                                            </p>
                                                            <div className="flex items-center gap-2">
                                                                <code className="min-w-0 flex-1 select-all overflow-x-auto rounded bg-telegram-hover px-2 py-1.5 font-mono text-xs text-telegram-text">
                                                                    {generatedWebDavUrl}
                                                                </code>
                                                                <button
                                                                    onClick={handleCopyWebDavUrl}
                                                                    className="shrink-0 rounded-md p-1.5 text-telegram-subtext transition hover:bg-telegram-hover hover:text-telegram-text"
                                                                    title={t('settings.webdav_connection_link')}
                                                                >
                                                                    {webDavUrlCopied
                                                                        ? <Check className="h-4 w-4 text-green-400" />
                                                                        : <Copy className="h-4 w-4" />}
                                                                </button>
                                                            </div>
                                                        </div>
                                                    )}
                                                </div>

                                                <div className="flex items-center gap-2 text-[11px] text-telegram-subtext">
                                                    {(webDavLoading || webDavGenerating) && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                                                    <span>{webDavSettings.write_enabled ? t('settings.webdav_changes_enabled') : t('settings.webdav_read_only')}</span>
                                                </div>

                                                {webDavSettings.last_error && (
                                                    <div className="rounded-lg border border-red-500/20 bg-red-500/5 p-3">
                                                        <p className="text-xs font-medium text-red-300">{t('settings.webdav_last_error')}</p>
                                                        <p className="mt-1 break-words font-mono text-[11px] text-red-300/80">{webDavSettings.last_error}</p>
                                                    </div>
                                                )}
                                            </>
                                        )}
                                    </WebDavSettingsTab>
                                )}

                                {activeTab === 'encryption' && <EncryptionSettingsTab />}

                                {activeTab === 'sharing' && (
                                    <SharingSettingsTab>
                                        <div className="flex items-center justify-between">
                                            <h3 className="text-xs font-semibold text-telegram-subtext uppercase tracking-wider flex items-center gap-2">
                                                <Link className="w-3.5 h-3.5 text-telegram-primary" />
                                                {t('settings.shared_links', { count: shares.length })}
                                            </h3>
                                            <button 
                                                onClick={fetchShares} 
                                                className="text-telegram-subtext hover:text-telegram-text p-1 rounded hover:bg-telegram-hover transition"
                                                title={t('settings.refresh_links')}
                                            >
                                                <RefreshCw className={`w-3.5 h-3.5 ${refreshing ? 'animate-spin' : ''}`} />
                                            </button>
                                        </div>

                                        <div className="bg-telegram-hover/30 border border-telegram-border/50 rounded-lg p-3 space-y-2">
                                            <div className="text-[11px] font-semibold text-telegram-text flex items-center gap-1">🌐 {t('settings.ip_override')}</div>
                                            <input
                                                type="text"
                                                placeholder="e.g. 100.115.22.45 or my-pc:14201"
                                                value={globalDomain}
                                                onChange={(e) => setGlobalDomain(e.target.value)}
                                                className="w-full bg-telegram-surface border border-telegram-border rounded-md px-2.5 py-1.5 text-xs text-telegram-text focus:outline-none focus:border-telegram-primary/50 placeholder:text-telegram-subtext/40"
                                            />
                                            <p className="text-[10px] text-telegram-subtext">
                                                {t('settings.ip_override_desc')}
                                            </p>
                                        </div>

                                        {shares.length === 0 ? (
                                            <div className="py-8 text-center space-y-2">
                                                <Link className="w-8 h-8 text-telegram-subtext/40 mx-auto" />
                                                <p className="text-sm font-medium text-telegram-text">{t('settings.no_active_links')}</p>
                                                <p className="text-xs text-telegram-subtext">{t('settings.no_active_links_desc')}</p>
                                            </div>
                                        ) : (
                                            <div className="space-y-2 max-h-[300px] overflow-y-auto pr-1 custom-scrollbar">
                                                {shares.map((share) => {
                                                    const isExpired = share.expires_at ? (share.expires_at < Math.floor(Date.now() / 1000)) : false;
                                                    return (
                                                        <div key={share.id} className="p-3 rounded-lg bg-telegram-hover/40 border border-telegram-border/50 flex flex-col gap-2 relative">
                                                              <div className="flex justify-between items-start gap-4">
                                                                <div className="min-w-0 flex-1">
                                                                    <div className="text-xs font-semibold text-telegram-text truncate" title={share.file_name}>
                                                                        {share.file_name}
                                                                    </div>
                                                                    <div className="flex gap-2 items-center mt-1 flex-wrap text-[10px]">
                                                                        <span className="text-telegram-subtext">
                                                                            {new Date(share.created_at * 1000).toLocaleDateString()}
                                                                        </span>
                                                                        <span className="w-1 h-1 rounded-full bg-telegram-border" />
                                                                        {share.has_password ? (
                                                                            <span className="text-emerald-400 bg-emerald-500/10 px-1.5 py-0.5 rounded flex items-center gap-0.5 font-medium">
                                                                                <Key className="w-2.5 h-2.5" /> {t('settings.protected')}
                                                                            </span>
                                                                        ) : (
                                                                            <span className="text-blue-400 bg-blue-500/10 px-1.5 py-0.5 rounded font-medium">{t('settings.public')}</span>
                                                                        )}
                                                                        <span className="w-1 h-1 rounded-full bg-telegram-border" />
                                                                        {share.expires_at ? (
                                                                            isExpired ? (
                                                                                <span className="text-red-400 bg-red-500/10 px-1.5 py-0.5 rounded font-medium">{t('settings.expired')}</span>
                                                                            ) : (
                                                                                <span className="text-amber-400 bg-amber-500/10 px-1.5 py-0.5 rounded font-medium">
                                                                                    {t('settings.expires_at', { date: new Date(share.expires_at * 1000).toLocaleDateString() })}
                                                                                </span>
                                                                            )
                                                                        ) : (
                                                                            <span className="text-teal-400 bg-teal-500/10 px-1.5 py-0.5 rounded font-medium">{t('settings.never_expires')}</span>
                                                                        )}
                                                                    </div>
                                                                </div>
                                                                
                                                                <div className="flex gap-1">
                                                                    <button
                                                                        onClick={() => handleCopyShare(share.id)}
                                                                        className={`p-1.5 rounded bg-telegram-surface border border-telegram-border text-telegram-text hover:bg-telegram-hover transition ${
                                                                            copiedId === share.id ? 'text-emerald-400 border-emerald-500/30 bg-emerald-500/5' : ''
                                                                        }`}
                                                                        title={t('settings.copy_share_link')}
                                                                    >
                                                                        {copiedId === share.id ? <Check className="w-3.5 h-3.5" /> : <Copy className="w-3.5 h-3.5" />}
                                                                    </button>
                                                                    <button
                                                                        onClick={() => handleRevokeShare(share.id)}
                                                                        className="p-1.5 rounded bg-telegram-surface border border-telegram-border text-red-400 hover:bg-red-500/10 hover:border-red-500/30 transition"
                                                                        title={t('settings.revoke_link')}
                                                                    >
                                                                        <Trash2 className="w-3.5 h-3.5" />
                                                                    </button>
                                                                </div>
                                                            </div>
                                                        </div>
                                                    );
                                                })}
                                            </div>
                                        )}
                                    </SharingSettingsTab>
                                )}
                                {activeTab === 'themes' && <ThemeSettingsTab />}
                                {activeTab === 'sync' && <SyncSettingsPanel />}
                                {activeTab === 'about' && (
                                    <AboutSettingsTab
                                        appVersion={appVersion}
                                        diagnosticsLoading={diagLoading}
                                        t={t}
                                        onCopyDiagnostics={async () => {
                                            setDiagLoading(true);
                                            try {
                                                const diagnostics = await invoke<string>('cmd_get_system_diagnostics');
                                                await navigator.clipboard.writeText(diagnostics);
                                                toast.success(t('settings.diagnostics_copied'));
                                            } catch (error) {
                                                toast.error(t('settings.diagnostics_copy_failed', { error }));
                                            } finally {
                                                setDiagLoading(false);
                                            }
                                        }}
                                    />
                                )}
                            </AnimatePresence>
                        </div>
                        </div>

                        {/* Footer */}
                        <div className="flex items-center justify-between border-t border-app-border-subtle px-6 py-3">
                            <button
                                onClick={resetSettings}
                                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs text-telegram-subtext hover:text-red-400 hover:bg-red-500/10 transition font-medium"
                            >
                                <RotateCcw className="w-3.5 h-3.5" />
                                {t('settings.reset_defaults')}
                            </button>
                            <button
                                onClick={onClose}
                                className="px-4 py-1.5 rounded-lg text-xs font-medium bg-telegram-primary text-app-accent-contrast hover:bg-telegram-primary/90 transition"
                            >
                                {t('settings.done')}
                            </button>
                        </div>
                    </motion.div>
                </motion.div>
            )}
            {isOpen && accessTransparency && <AccessTransparencyDialog service={accessTransparency} onClose={() => setAccessTransparency(null)} />}
        </AnimatePresence>
    );
}
