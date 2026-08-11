import type { DownloadEvent, Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';

const PENDING_UPDATE_KEY = 'telegram-drive.pending-update';
const LAST_SEEN_VERSION_KEY = 'telegram-drive.last-seen-version';

export type UpdateInstallPhase = 'downloading' | 'verifying' | 'installing';

export interface WhatsNewDetails {
    version: string;
    body?: string;
    updated: boolean;
}

interface PendingUpdate {
    fromVersion: string;
    toVersion: string;
    body?: string;
    createdAt: string;
}

function readLocalStorage(key: string): string | null {
    try {
        return localStorage.getItem(key);
    } catch {
        return null;
    }
}

function writeLocalStorage(key: string, value: string): void {
    try {
        localStorage.setItem(key, value);
    } catch {
        // Storage can be unavailable in private browsing or locked-down shells.
    }
}

function removeLocalStorage(key: string): void {
    try {
        localStorage.removeItem(key);
    } catch {
        // Best-effort cleanup should not mask the original update error.
    }
}

function writePendingUpdate(update: Update): void {
    const pending: PendingUpdate = {
        fromVersion: update.currentVersion,
        toVersion: update.version,
        body: update.body,
        createdAt: new Date().toISOString(),
    };
    writeLocalStorage(PENDING_UPDATE_KEY, JSON.stringify(pending));
}

export async function installVerifiedUpdate(
    update: Update,
    onProgress: (progress: number) => void,
    onPhase?: (phase: UpdateInstallPhase) => void,
): Promise<void> {
    let downloadedBytes = 0;
    let contentLength: number | undefined;
    let finished = false;

    onPhase?.('downloading');
    await update.download((event: DownloadEvent) => {
        if (event.event === 'Started') {
            contentLength = event.data.contentLength;
            downloadedBytes = 0;
        } else if (event.event === 'Progress') {
            downloadedBytes += event.data.chunkLength;
            if (contentLength && contentLength > 0) {
                onProgress(Math.min(99, Math.round((downloadedBytes / contentLength) * 100)));
            }
        } else if (event.event === 'Finished') {
            finished = true;
        }
    });

    onPhase?.('verifying');
    if (!finished) {
        throw new Error('The update download did not finish. Your current version was left unchanged.');
    }
    if (contentLength && downloadedBytes !== contentLength) {
        throw new Error(
            `Update verification failed: expected ${contentLength} bytes but received ${downloadedBytes}. Your current version was left unchanged.`,
        );
    }

    // Tauri verifies the updater signature during download. Installation is only
    // attempted after its Finished event and our byte-count consistency check.
    onProgress(100);
    writePendingUpdate(update);
    onPhase?.('installing');
    try {
        await update.install();
        await relaunch();
    } catch (error) {
        removeLocalStorage(PENDING_UPDATE_KEY);
        throw error;
    }
}

export function consumeWhatsNew(currentVersion: string): WhatsNewDetails | null {
    const lastSeenVersion = readLocalStorage(LAST_SEEN_VERSION_KEY);
    writeLocalStorage(LAST_SEEN_VERSION_KEY, currentVersion);

    const rawPending = readLocalStorage(PENDING_UPDATE_KEY);
    if (rawPending) {
        try {
            const pending = JSON.parse(rawPending) as PendingUpdate;
            if (pending.toVersion === currentVersion) {
                removeLocalStorage(PENDING_UPDATE_KEY);
                return { version: currentVersion, body: pending.body, updated: true };
            }
        } catch {
            removeLocalStorage(PENDING_UPDATE_KEY);
        }
    }

    if (lastSeenVersion && lastSeenVersion !== currentVersion) {
        return { version: currentVersion, updated: true };
    }
    return null;
}
