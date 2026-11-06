import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { DownloadItem, QueueItem } from '../types';
import type { DownloadCollisionPolicy, DownloadOutcome } from '../types/transfers';
import type { VideoUploadMode } from '../types/settings';

export type TransferDirection = 'upload' | 'download';
export type TransferKind = 'local_upload' | 'url_upload' | 'download';
export type TransferStatus =
    | 'pending'
    | 'paused'
    | 'waiting_for_network'
    | 'cooldown'
    | 'downloading'
    | 'uploading'
    | 'encrypting'
    | 'decrypting'
    | 'verifying'
    | 'waiting_for_unlock'
    | 'completed'
    | 'failed'
    | 'cancelled';

export interface DesktopTransferJob {
    id: string;
    ownerId?: string | null;
    direction: TransferDirection;
    kind: TransferKind;
    status: TransferStatus;
    path?: string;
    url?: string;
    folderId: number | null;
    messageId?: number;
    filename: string;
    savePath?: string;
    collisionPolicy?: DownloadCollisionPolicy;
    downloadOutcome?: DownloadOutcome;
    protectionMode?: string;
    protectMetadata?: boolean;
    videoUploadMode?: VideoUploadMode;
    videoFilenameCaption?: boolean;
    tempZipPath?: string;
    /** Set when another part of the app, such as Folder Sync, runs the transfer. */
    origin?: string;
    progress: number;
    transferredBytes: number;
    totalBytes: number;
    speedBytesPerSec: number;
    error?: string;
    retryAt?: number;
    queuePosition: number;
    revision: number;
    createdAt: number;
    updatedAt: number;
}

export interface DesktopTransferRequest {
    id: string;
    ownerId?: string | null;
    direction: TransferDirection;
    kind: TransferKind;
    path?: string;
    url?: string;
    folderId: number | null;
    messageId?: number;
    filename: string;
    savePath?: string;
    collisionPolicy?: DownloadCollisionPolicy;
    protectionMode?: string;
    promptToken?: number;
    protectMetadata?: boolean;
    videoUploadMode?: VideoUploadMode;
    videoFilenameCaption?: boolean;
    tempZipPath?: string;
    totalBytes?: number;
    initialStatus?: TransferStatus;
}

const uploadStatus = (status: TransferStatus): QueueItem['status'] => {
    if (status === 'completed') return 'success';
    if (status === 'failed') return 'error';
    if (status === 'cooldown') return 'waiting_for_network';
    return status;
};

const downloadStatus = (status: TransferStatus): DownloadItem['status'] => {
    if (status === 'completed') return 'success';
    if (status === 'failed') return 'error';
    if (status === 'uploading' || status === 'encrypting') return 'downloading';
    return status;
};

export const transferJobToUploadItem = (job: DesktopTransferJob): QueueItem => ({
    id: job.id,
    ownerId: job.ownerId ?? undefined,
    path: job.path || job.filename,
    url: job.url,
    folderId: job.folderId,
    status: uploadStatus(job.status),
    error: job.error,
    progress: job.progress,
    uploadedBytes: job.transferredBytes,
    totalBytes: job.totalBytes,
    speedBytesPerSec: job.speedBytesPerSec,
    tempZipPath: job.tempZipPath,
    protection: job.protectionMode ? {
        mode: job.protectionMode as NonNullable<QueueItem['protection']>['mode'],
        protectMetadata: job.protectMetadata,
    } : undefined,
    videoUploadMode: job.videoUploadMode,
    videoFilenameCaption: job.videoFilenameCaption,
});

export const transferJobToDownloadItem = (job: DesktopTransferJob): DownloadItem => ({
    id: job.id,
    ownerId: job.ownerId ?? undefined,
    messageId: job.messageId || 0,
    filename: job.filename,
    folderId: job.folderId,
    status: downloadStatus(job.status),
    error: job.error,
    progress: job.progress,
    downloadedBytes: job.transferredBytes,
    totalBytes: job.totalBytes,
    speedBytesPerSec: job.speedBytesPerSec,
    savePath: job.savePath,
    collisionPolicy: job.collisionPolicy ?? 'keep_both',
    downloadOutcome: job.downloadOutcome,
    protectionMode: job.protectionMode as DownloadItem['protectionMode'],
});

export const uploadItemToTransferRequest = (item: QueueItem): DesktopTransferRequest => ({
    id: item.id,
    ownerId: item.ownerId,
    direction: 'upload',
    kind: item.url ? 'url_upload' : 'local_upload',
    path: item.url ? undefined : item.path,
    url: item.url,
    folderId: item.folderId,
    filename: item.path.split(/[\\/]/).pop() || item.path,
    protectionMode: item.protection?.mode,
    promptToken: item.protection?.promptToken,
    protectMetadata: item.protection?.protectMetadata,
    videoUploadMode: item.videoUploadMode,
    videoFilenameCaption: item.videoFilenameCaption,
    tempZipPath: item.tempZipPath,
    totalBytes: item.totalBytes,
    initialStatus: normalizeInitialStatus(item.status),
});

export const downloadItemToTransferRequest = (item: DownloadItem): DesktopTransferRequest => ({
    id: item.id,
    ownerId: item.ownerId,
    direction: 'download',
    kind: 'download',
    folderId: item.folderId,
    messageId: item.messageId,
    filename: item.filename,
    savePath: item.savePath,
    collisionPolicy: item.collisionPolicy ?? 'keep_both',
    protectionMode: item.protectionMode,
    promptToken: item.promptToken,
    totalBytes: item.totalBytes,
    initialStatus: normalizeInitialStatus(item.status),
});

const normalizeInitialStatus = (status: QueueItem['status'] | DownloadItem['status']): TransferStatus => {
    if (status === 'success') return 'completed';
    if (status === 'error') return 'failed';
    if (status === 'uploading' || status === 'downloading' || status === 'encrypting'
        || status === 'decrypting' || status === 'verifying') return 'pending';
    return status;
};

export async function listDesktopTransfers(): Promise<DesktopTransferJob[]> {
    return invoke<DesktopTransferJob[]>('cmd_transfer_list');
}

export async function enqueueDesktopTransfers(
    requests: DesktopTransferRequest[],
): Promise<DesktopTransferJob[]> {
    if (requests.length === 0) return [];
    return invoke<DesktopTransferJob[]>('cmd_transfer_enqueue_many', { requests });
}

export async function configureDesktopTransferLimits(maxUploads: number, maxDownloads: number) {
    await invoke('cmd_transfer_set_limits', {
        maxUploads: Math.max(1, maxUploads),
        maxDownloads: Math.max(1, maxDownloads),
    });
}

export async function transferItemAction(
    action: 'pause' | 'resume' | 'cancel' | 'retry',
    id: string,
    ownerId?: string,
): Promise<DesktopTransferJob> {
    return invoke<DesktopTransferJob>(`cmd_transfer_${action}`, { id, ownerId });
}

/** Recheck strict failed-upload eligibility in the native mutation lock. */
export async function retryFailedUpload(id: string, ownerId: string, promptToken?: number): Promise<{ job: DesktopTransferJob; retried: boolean }> {
    return invoke<{ job: DesktopTransferJob; retried: boolean }>('cmd_transfer_retry_failed_upload', { id, ownerId, promptToken });
}

export async function transferBulkAction(
    action: 'pause' | 'resume' | 'cancel',
    direction: TransferDirection,
    ownerId?: string,
): Promise<DesktopTransferJob[]> {
    return invoke<DesktopTransferJob[]>(`cmd_transfer_${action}_all`, { direction, ownerId });
}

export async function clearTerminalTransfers(
    direction: TransferDirection,
    includeFailedAndCancelled: boolean,
    ownerId?: string,
): Promise<string[]> {
    return invoke<string[]>('cmd_transfer_clear_terminal', { direction, includeFailedAndCancelled, ownerId });
}

export async function supplyTransferPromptToken(id: string, promptToken: number): Promise<void> {
    await invoke('cmd_transfer_supply_prompt_token', { id, promptToken });
}

export async function listenToDesktopTransfers(
    onUpsert: (job: DesktopTransferJob) => void,
    onRemove: (id: string) => void,
): Promise<UnlistenFn> {
    const unlistenUpsert = await listen<DesktopTransferJob>('transfer-upserted', event => {
        onUpsert(event.payload);
    });
    let unlistenRemove: UnlistenFn;
    try {
        unlistenRemove = await listen<string>('transfer-removed', event => {
            onRemove(event.payload);
        });
    } catch (error) {
        unlistenUpsert();
        throw error;
    }
    return () => {
        unlistenUpsert();
        unlistenRemove();
    };
}
