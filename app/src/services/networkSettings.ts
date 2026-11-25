import { invoke } from '@tauri-apps/api/core';
import type { Settings } from '../types/settings';
import { resolveLegacyProxyPassword } from './settingsPersistence';

// Both commands persist the same native config file. Preserve user-edit order,
// including across Settings remounts, and allow later writes after a failure.
let pendingApply: Promise<unknown> = Promise.resolve();

function apply<T = string>(command: string, args?: { req: Record<string, unknown> }, legacyAction?: 'import' | 'clear'): Promise<T> {
    const operation = pendingApply.then(async () => {
        if (legacyAction) await resolveLegacyProxyPassword(legacyAction);
        return invoke<T>(command, args);
    });
    pendingApply = operation.catch(() => undefined);
    return operation;
}

export function clearProxyPassword(): Promise<void> {
    return apply<void>('cmd_clear_proxy_secret', undefined, 'clear');
}

export function applyProxySettings(settings: Settings): Promise<string> {
    return apply('cmd_apply_proxy_settings', {
        req: {
            enabled: settings.proxyEnabled,
            proxy_type: settings.proxyType,
            host: settings.proxyHost,
            port: settings.proxyPort,
            username: settings.proxyUsername,
            password: settings.proxyPassword,
        },
    }, 'import');
}

export function applyVpnSettings(settings: Settings): Promise<string> {
    return apply('cmd_apply_vpn_settings', {
        req: {
            enabled: settings.vpnMode,
            timeout_multiplier: settings.timeoutMultiplier,
            retry_attempts: settings.retryAttempts,
            retry_base_backoff_ms: Math.round(settings.retryBaseBackoffSec * 1000),
            retry_max_backoff_ms: Math.round(settings.retryMaxBackoffSec * 1000),
            // Retain historical fields when round-tripping stored/synced settings.
            adaptive_polling: settings.adaptivePolling,
            polling_min_sec: settings.pollingMinSec,
            polling_max_sec: settings.pollingMaxSec,
            preferred_dc: settings.preferredDC,
            dc_fallback_attempts: settings.dcFallbackAttempts,
            flood_wait_respect: settings.floodWaitRespect,
            peer_cache_size: settings.peerCacheSize,
            bandwidth_limit_up_kbs: settings.bandwidthLimitUpKBs,
            bandwidth_limit_down_kbs: settings.bandwidthLimitDownKBs,
            bandwidth_schedule: settings.bandwidth_schedule,
            bandwidth_windows: settings.bandwidthWindows,
            chunk_size_kb: settings.chunkSizeKb,
            keep_alive_interval_sec: settings.keepAliveIntervalSec,
            auto_detect_vpn: settings.autoDetectVpn,
            archive_max_bytes: settings.archiveMaxBytes * 1024 * 1024,
        },
    });
}
