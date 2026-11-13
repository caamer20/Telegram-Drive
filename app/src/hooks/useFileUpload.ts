import i18n from '../i18n';
import { useState, useEffect, useLayoutEffect, useRef } from 'react';
import { useActionScope } from './useActionScope';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { useQueryClient } from '@tanstack/react-query';
import { toast } from 'sonner';
import { DropUploadResult, DroppedPathValidation, QueueItem, UploadProtectionIntent } from '../types';
import { isAndroidPlatform, showFileDialogFallback, pickWithFallback } from '../utils';
import { useSettings } from '../context/SettingsContext';
import type { Store } from '@tauri-apps/plugin-store';
import { useTranslation } from 'react-i18next';
import { useUploadChoice } from '../context/UploadChoiceContext';
import { useConfirm } from '../context/ConfirmContext';
import { triggerHaptic } from '../services/feedback';
import { isTransientNetworkError, restoreUploadQueue, serializeUploadQueue } from '../services/transferQueuePolicy';
import { announceSupporterValueMoment } from '../services/supporterVisibility';
import { userFacingError } from '../services/userFacingError';
import { invalidateFolderFileQueries } from '../services/fileListRefresh';
import { effectiveVideoUploadMode } from '../services/videoUploadMode';
import {
    clearTerminalTransfers,
    configureDesktopTransferLimits,
    enqueueDesktopTransfers,
    listenToDesktopTransfers,
    listDesktopTransfers,
    retryFailedUpload,
    supplyTransferPromptToken,
    transferBulkAction,
    transferItemAction,
    transferJobToUploadItem,
    uploadItemToTransferRequest,
} from '../services/desktopTransferEngine';

interface RetryFailedSummary {
    queued: number;
    skipped: number;
    failed: number;
    remaining: number;
}

type RetryOutcome = 'queued' | 'skipped' | 'stopped';

interface ProgressPayload {
    id: string;
    percent: number;
    uploaded_bytes: number;
    total_bytes: number;
    speed_bytes_per_sec: number;
}

interface RemoteProgressPayload {
    id: string;
    phase: 'downloading' | 'uploading';
    percent: number;
    speed: number;
    uploaded_bytes: number;
    total_bytes: number;
}

export function useFileUpload(
    activeFolderId: number | null,
    store: Store | null,
    androidNetworkAvailable = true,
    androidWaitingReason = 'Waiting for a network connection',
    ownerId?: string,
) {
    const { t } = useTranslation();
    const ownerRef = useRef(ownerId);
    ownerRef.current = ownerId;
    const capture = useActionScope(ownerId ?? null);
    const isCurrent = capture();
    const ownsItem = (item: QueueItem) => isCurrent() && Boolean(ownerId) && item.ownerId === ownerId;
    const requireCurrent = () => { if (!isCurrent()) throw new Error('ACCOUNT_CHANGED'); };
    const runningItemsRef = useRef(new Map<string, () => boolean>());
    const queryClient = useQueryClient();
    const { settings } = useSettings();
    const { chooseUploadProtection } = useUploadChoice();
    const protectionScopeRef = useRef<AbortController | null>(null);
    useLayoutEffect(() => {
        const controller = new AbortController();
        protectionScopeRef.current = controller;
        return () => {
            controller.abort();
            if (protectionScopeRef.current === controller) protectionScopeRef.current = null;
        };
    }, [ownerId]);
    const { confirm, promptSecret } = useConfirm();
    const [uploadQueue, setUploadQueue] = useState<QueueItem[]>([]);
    const [initialized, setInitialized] = useState(false);
    const [retryingFailed, setRetryingFailed] = useState(false);
    const retryingItemsRef = useRef(new Map<string, () => boolean>());
    const retryBatchRef = useRef<{ ownerId: string; stopped: boolean; done: Promise<void> } | null>(null);
    const initializingRef = useRef(false);
    const cancelledRef = useRef<Set<string>>(new Set());
    const pausedRef = useRef<Set<string>>(new Set());
    const networkPausedRef = useRef<Set<string>>(new Set());
    const activeCountRef = useRef(0);
    const persistenceChainRef = useRef<Promise<void>>(Promise.resolve());
    const persistenceHealthyRef = useRef(true);
    const startingItemsRef = useRef<Set<string>>(new Set());
    const uploadQueueRef = useRef(uploadQueue);
    const androidNetworkAvailableRef = useRef(androidNetworkAvailable);
    const desktopRevisionsRef = useRef<Map<string, number>>(new Map());
    const desktopStatusesRef = useRef<Map<string, string>>(new Map());
    uploadQueueRef.current = uploadQueue;
    androidNetworkAvailableRef.current = androidNetworkAvailable;

    useEffect(() => {
        setRetryingFailed(false);
        return () => {
            const batch = retryBatchRef.current;
            if (batch && batch.ownerId === ownerId) {
                batch.stopped = true;
                retryBatchRef.current = null;
            }
        };
    }, [ownerId]);

    // Listen for progress events from Rust
    useEffect(() => {
        if (!isAndroidPlatform) return;
        let unlistenProgress: UnlistenFn | undefined;
        let unlistenRemote: UnlistenFn | undefined;

        listen<ProgressPayload>('upload-progress', (event) => {
            setUploadQueue(q => q.map(i =>
                i.id === event.payload.id && runningItemsRef.current.get(i.id)?.() ? {
                    ...i,
                    progress: event.payload.percent,
                    uploadedBytes: event.payload.uploaded_bytes,
                    totalBytes: event.payload.total_bytes,
                    speedBytesPerSec: event.payload.speed_bytes_per_sec,
                } : i
            ));
        }).then(fn => { unlistenProgress = fn; });

        listen<RemoteProgressPayload>('remote-upload-progress', (event) => {
            setUploadQueue(q => q.map(i =>
                i.id === event.payload.id && runningItemsRef.current.get(i.id)?.() && !['paused', 'cancelled', 'waiting_for_network'].includes(i.status) ? {
                    ...i,
                    status: event.payload.phase,
                    progress: event.payload.percent,
                    speedBytesPerSec: event.payload.speed,
                    uploadedBytes: event.payload.uploaded_bytes,
                    totalBytes: event.payload.total_bytes,
                } : i
            ));
        }).then(fn => { unlistenRemote = fn; });

        return () => {
            unlistenProgress?.();
            unlistenRemote?.();
        };
    }, []);

    useEffect(() => {
        if (isAndroidPlatform) {
            setUploadQueue(queue => queue.map(item => {
                if (item.ownerId === ownerId || !['uploading', 'downloading', 'encrypting', 'verifying'].includes(item.status)) return item;
                pausedRef.current.add(item.id);
                void invoke('cmd_cancel_transfer', { transferId: item.id }).catch(() => undefined);
                return { ...item, status: 'paused', speedBytesPerSec: 0, error: undefined };
            }));
            return;
        }
        setUploadQueue([]);
        desktopRevisionsRef.current.clear();
        desktopStatusesRef.current.clear();
    }, [ownerId]);

    // Desktop queue state is a revisioned projection of the durable Rust engine.
    useEffect(() => {
        if (isAndroidPlatform) return;
        let disposed = false;
        let unlisten: UnlistenFn | undefined;
        const accept = (
            job: Awaited<ReturnType<typeof listDesktopTransfers>>[number],
            notifyTransition = false,
        ) => {
            if (disposed || !ownerId || ownerRef.current !== ownerId || job.ownerId !== ownerId || job.direction !== 'upload') return;
            const knownRevision = desktopRevisionsRef.current.get(job.id) || 0;
            if (job.revision < knownRevision) return;
            const previousStatus = desktopStatusesRef.current.get(job.id);
            desktopRevisionsRef.current.set(job.id, job.revision);
            desktopStatusesRef.current.set(job.id, job.status);
            const item = transferJobToUploadItem(job);
            setUploadQueue(queue => queue.some(candidate => candidate.id === item.id)
                ? queue.map(candidate => candidate.id === item.id ? item : candidate)
                : [...queue, item]);
            if (notifyTransition && previousStatus !== job.status) {
                // Folder Sync reports its own outcomes in the Sync panel; its
                // transfers only refresh the folder they changed.
                if (job.origin) {
                    if (job.status === 'completed') {
                        void invalidateFolderFileQueries(queryClient, job.folderId, job.ownerId);
                    }
                } else if (job.status === 'completed') {
                    triggerHaptic('success');
                    announceSupporterValueMoment('upload_completed');
                    void invalidateFolderFileQueries(queryClient, job.folderId, job.ownerId);
                } else if (job.status === 'failed') {
                    toast.error(i18n.t('transfer_copy.upload_failed',{name:job.filename,error:job.error||i18n.t('transfer_copy.unknown_error')}));
                } else if (job.status === 'waiting_for_unlock') {
                    toast.warning(t('settings.encryption_mode_passphrase'));
                }
            }
        };
        void listenToDesktopTransfers(job => accept(job, true), id => {
            if (disposed || !isCurrent()) return;
            desktopRevisionsRef.current.delete(id);
            desktopStatusesRef.current.delete(id);
            setUploadQueue(queue => queue.filter(item => item.id !== id));
        }).then(async listener => {
            if (disposed) {
                listener();
                return;
            }
            unlisten = listener;
            const jobs = await listDesktopTransfers();
            jobs.forEach(job => accept(job));
        }).catch(error => {
            if (disposed || !isCurrent()) return;
            console.error('[Upload] Could not attach to the desktop transfer engine:', error);
            toast.error(i18n.t('transfer_copy.queue_load_error'));
        });
        return () => {
            disposed = true;
            unlisten?.();
        };
    }, [queryClient, t, ownerId]);

    useEffect(() => {
        if (!store || initialized || initializingRef.current || (!isAndroidPlatform && !ownerId)) return;
        initializingRef.current = true;
        if (!isAndroidPlatform) {
            void store.get<QueueItem[]>('uploadQueue').then(async saved => {
                const pending = saved ? restoreUploadQueue(saved, false) : [];
                if (pending.length > 0) {
                    await enqueueDesktopTransfers(pending.map(uploadItemToTransferRequest));
                    await store.set('uploadQueue', []);
                    await store.save();
                    toast.info(i18n.t('transfer_copy.migrated_uploads',{count:pending.length}));
                }
                setInitialized(true);
            }).catch(error => {
                console.error('[Upload] Could not migrate the desktop recovery queue:', error);
                toast.error(i18n.t('transfer_copy.upload_migrate_failed'));
                setInitialized(true);
            });
            return;
        }
        store.get<QueueItem[]>('uploadQueue').then((saved) => {
            if (saved && saved.length > 0) {
                const pending = restoreUploadQueue(saved, isAndroidPlatform);
                if (pending.length > 0) {
                    setUploadQueue(previous => [
                        ...pending.filter(item => !previous.some(existing => existing.id === item.id)),
                        ...previous,
                    ]);
                    const visibleCount = pending.filter(item => ownerRef.current && item.ownerId === ownerRef.current).length;
                    if (visibleCount) toast.info(i18n.t('transfer_copy.restored_uploads',{count:visibleCount}));
                }
            }
            setInitialized(true);
        }).catch(error => {
            initializingRef.current = false;
            persistenceHealthyRef.current = false;
            console.error('[Upload] Could not restore the saved queue:', error);
        });
    }, [store, initialized, ownerId]);

    useEffect(() => {
        if (!isAndroidPlatform) return;
        if (!store || !initialized) return;
        const pending = serializeUploadQueue(uploadQueue, isAndroidPlatform);
        persistenceChainRef.current = persistenceChainRef.current
            .catch(() => undefined)
            .then(async () => {
                await store.set('uploadQueue', pending);
                await store.save();
                persistenceHealthyRef.current = true;
            })
            .catch(error => {
                persistenceHealthyRef.current = false;
                console.error('[Upload] Could not persist the recovery queue:', error);
            });
    }, [store, uploadQueue, initialized]);

    useEffect(() => {
        if (isAndroidPlatform) return;
        void configureDesktopTransferLimits(
            settings.maxConcurrentUploads || 1,
            settings.maxConcurrentDownloads || 1,
        ).catch(error => console.error('[Transfer] Could not update concurrency limits:', error));
    }, [settings.maxConcurrentDownloads, settings.maxConcurrentUploads]);

    useEffect(() => {
        if (!isAndroidPlatform || !initialized) return;
        const activeStatuses: QueueItem['status'][] = ['uploading', 'downloading', 'encrypting', 'verifying'];
        if (!androidNetworkAvailable) {
            setUploadQueue(queue => {
                let changed = false;
                const next = queue.map(item => {
                    if (!ownsItem(item)) return item;
                    if (item.status !== 'pending' && !activeStatuses.includes(item.status)) return item;
                    changed = true;
                    if (activeStatuses.includes(item.status)) {
                        networkPausedRef.current.add(item.id);
                        void invoke('cmd_cancel_transfer', { transferId: item.id }).catch(() => undefined);
                    }
                    return { ...item, status: 'waiting_for_network' as const, error: androidWaitingReason };
                });
                return changed ? next : queue;
            });
            return;
        }
        setUploadQueue(queue => queue.some(item => ownsItem(item) && item.status === 'waiting_for_network')
            ? queue.map(item => ownsItem(item) && item.status === 'waiting_for_network'
                ? { ...item, status: 'pending' as const, error: undefined }
                : item)
            : queue);
    }, [androidNetworkAvailable, androidWaitingReason, initialized, ownerId]);

    // Process up to maxConcurrentUploads in parallel
    useEffect(() => {
        if (!isAndroidPlatform) return;
        if (isAndroidPlatform && !androidNetworkAvailable) return;
        if (isAndroidPlatform && (!store || !initialized)) return;
        const maxConcurrent = settings.maxConcurrentUploads || 1;
        const available = maxConcurrent - activeCountRef.current;
        if (available <= 0) return;
        const pendingItems = uploadQueue.filter(i => ownsItem(i) && i.status === 'pending').slice(0, available);
        for (const item of pendingItems) {
            if (!isAndroidPlatform) {
                void processItem(item);
                continue;
            }
            if (startingItemsRef.current.has(item.id)) continue;
            startingItemsRef.current.add(item.id);
            void (async () => {
                await persistenceChainRef.current;
                const current = uploadQueueRef.current.find(candidate => candidate.id === item.id);
                if (!current || !ownsItem(current) || current.status !== 'pending' || !androidNetworkAvailableRef.current) return;
                if (!persistenceHealthyRef.current) {
                    setUploadQueue(queue => queue.map(candidate => candidate.id === item.id ? {
                        ...candidate,
                        status: 'error',
                        error: 'Could not save this upload for background recovery. Retry after reopening the app.',
                    } : candidate));
                    return;
                }
                await processItem(current);
            })().finally(() => {
                startingItemsRef.current.delete(item.id);
                if (!isCurrent() && ownerRef.current === item.ownerId) setUploadQueue(queue => [...queue]);
            });
        }
    }, [uploadQueue, settings.maxConcurrentUploads, androidNetworkAvailable, initialized, store, ownerId]);

    const enqueueUploadItems = async (items: QueueItem[]) => {
        if (items.length === 0) return;
        if (!isCurrent() || items.some(item => !ownsItem(item))) throw new Error("ACCOUNT_CHANGED");
        if (isAndroidPlatform) {
            setUploadQueue(previous => isCurrent() ? [...previous, ...items] : previous);
            return;
        }
        const jobs = await enqueueDesktopTransfers(items.map(uploadItemToTransferRequest));
        setUploadQueue(previous => {
            if (!isCurrent() || ownerRef.current !== ownerId) return previous;
            const currentJobs = jobs.filter(job => {
                if (!ownerId || job.ownerId !== ownerId) return false;
                const knownRevision = desktopRevisionsRef.current.get(job.id) || 0;
                return job.revision >= knownRevision;
            });
            const incoming = new Map(currentJobs.map(job => [job.id, transferJobToUploadItem(job)]));
            const next = previous.map(item => incoming.get(item.id) || item);
            const known = new Set(previous.map(item => item.id));
            for (const job of currentJobs) {
                desktopRevisionsRef.current.set(job.id, job.revision);
                if (!known.has(job.id)) next.push(transferJobToUploadItem(job));
            }
            return next;
        });
    };

    const cleanupResumableSource = async (item: QueueItem) => {
        if (item.tempZipPath) {
            try {
                await invoke('cmd_delete_temp_zip', { path: item.tempZipPath });
            } catch {
                // Best-effort cleanup
            }
        }
        if (item.androidStaged) {
            await invoke('cmd_delete_android_staged_upload', { path: item.path }).catch(() => undefined);
        }
    };

    const processItem = async (item: QueueItem) => {
        if (!ownsItem(item)) return;
        runningItemsRef.current.set(item.id, isCurrent);
        let keepTemporaryFileForResume = false;
        const protection: UploadProtectionIntent = item.protection ?? {
            mode: settings.encryptionDefaultMode,
            protectMetadata: settings.encryptionProtectMetadata,
        };
        if (
            (protection.mode === 'passphrase' || protection.mode === 'vault_and_passphrase')
            && !protection.promptToken
        ) {
            setUploadQueue(q => q.map(i => i.id === item.id ? {
                ...i,
                protection,
                status: 'waiting_for_unlock',
                error: 'File passphrase required',
            } : i));
            runningItemsRef.current.delete(item.id);
            return;
        }
        activeCountRef.current++;
        const initialStatus = item.url ? 'downloading' : 'uploading';
        setUploadQueue(q => q.map(i => i.id === item.id ? { ...i, status: initialStatus, progress: 0 } : i));
        try {
            if (item.url) {
                await invoke('cmd_upload_from_url', {
                    ownerId: item.ownerId,
                    url: item.url,
                    folderId: item.folderId,
                    transferId: item.id,
                    protectionMode: protection.mode,
                    promptToken: protection.promptToken,
                    protectMetadata: protection.protectMetadata ?? settings.encryptionProtectMetadata,
                    videoUploadMode: item.videoUploadMode ?? 'file',
                });
            } else {
                await invoke('cmd_upload_file', {
                    ownerId: item.ownerId,
                    path: item.path,
                    folderId: item.folderId,
                    transferId: item.id,
                    protectionMode: protection.mode,
                    promptToken: protection.promptToken,
                    protectMetadata: protection.protectMetadata ?? settings.encryptionProtectMetadata,
                    videoUploadMode: item.videoUploadMode ?? 'file',
                });
            }
            if (!isCurrent()) return;
            // Check if cancelled during upload
            // A resolved backend invocation means the upload completed even if a
            // late pause request raced with its final bytes. Mark it successful
            // so resume cannot create a duplicate Telegram message.
            pausedRef.current.delete(item.id);
            networkPausedRef.current.delete(item.id);
            if (cancelledRef.current.has(item.id)) {
                cancelledRef.current.delete(item.id);
            } else {
                setUploadQueue(q => q.map(i => i.id === item.id ? { ...i, status: 'success', progress: 100 } : i));
                triggerHaptic('success');
                announceSupporterValueMoment('upload_completed');
                void invalidateFolderFileQueries(queryClient, item.folderId, item.ownerId);
            }
            if (!keepTemporaryFileForResume) await cleanupResumableSource(item);
        } catch (e) {
            if (!isCurrent()) return;
            if (networkPausedRef.current.has(item.id)) {
                networkPausedRef.current.delete(item.id);
                keepTemporaryFileForResume = true;
            } else if (pausedRef.current.has(item.id)) {
                pausedRef.current.delete(item.id);
                keepTemporaryFileForResume = true;
            } else if (!cancelledRef.current.has(item.id)) {
                const errMsg = String(e);
                if (errMsg.includes('Transfer cancelled')) {
                    setUploadQueue(q => q.map(i => i.id === item.id ? { ...i, status: 'cancelled' } : i));
                } else if (errMsg.includes('VAULT_LOCKED') || errMsg.includes('KEY_REQUIRED')) {
                    keepTemporaryFileForResume = Boolean(item.tempZipPath || item.androidStaged);
                    setUploadQueue(q => q.map(i => i.id === item.id ? {
                        ...i,
                        status: 'waiting_for_unlock',
                        error: errMsg,
                        protection: i.protection ? { ...i.protection, promptToken: undefined } : protection,
                    } : i));
                    toast.warning(t('settings.encryption_mode_passphrase'));
                } else if (errMsg.includes('FILE_TOO_BIG') || errMsg.includes('too large') || errMsg.includes('2 GB')) {
                    setUploadQueue(q => q.map(i => i.id === item.id ? { ...i, status: 'error', error: errMsg } : i));
                    toast.error(`Upload failed: Telegram has a 2 GB file size limit. Try splitting large folders.`);
                } else if (isAndroidPlatform && isTransientNetworkError(errMsg)) {
                    keepTemporaryFileForResume = true;
                    setUploadQueue(q => q.map(i => i.id === item.id ? {
                        ...i,
                        status: 'waiting_for_network',
                        error: 'Waiting for a stable network connection',
                    } : i));
                } else {
                    keepTemporaryFileForResume = Boolean(item.tempZipPath || item.androidStaged);
                    const displayPath = item.url || item.path;
                    setUploadQueue(q => q.map(i => i.id === item.id ? { ...i, status: 'error', error: errMsg } : i));
                    toast.error(i18n.t('transfer_copy.upload_failed',{name:displayPath.split('/').pop(),error:String(e)}));
                }
            } else if (cancelledRef.current.has(item.id)) {
                cancelledRef.current.delete(item.id);
            }
            if (!keepTemporaryFileForResume) await cleanupResumableSource(item);
        } finally {
            runningItemsRef.current.delete(item.id);
            if (!isCurrent()) {
                pausedRef.current.delete(item.id);
                networkPausedRef.current.delete(item.id);
                cancelledRef.current.delete(item.id);
            }
            activeCountRef.current--;
            // A quickly resumed item may already be pending while the cancelled
            // invocation unwinds. Trigger a pass after releasing the slot.
            setUploadQueue(q => [...q]);
        }
    };

    const stageProtectionForFiles = async (
        count: number,
        requestedMode = settings.encryptionDefaultMode,
        throwOnFailure = false,
        protectMetadata = settings.encryptionProtectMetadata,
    ): Promise<UploadProtectionIntent[] | null> => {
        requireCurrent();
        const mode = requestedMode;
        const base: UploadProtectionIntent = {
            mode,
            protectMetadata,
        };
        if (mode !== 'passphrase' && mode !== 'vault_and_passphrase') {
            return Array.from({ length: count }, () => ({ ...base }));
        }

        const accepted = await confirm({
            title: t('settings.encryption_disclaimer_title'),
            message: t('settings.encryption_disclaimer_body'),
            variant: 'info',
        });
        if (!accepted) return null;
        requireCurrent();
        // One masked request covers the whole batch; the value stays in memory
        // only long enough to stage a single-use token per file.
        const passphrase = await promptSecret({
            title: t('settings.encryption_mode_passphrase'),
            message: t('settings.min_passphrase_length'),
            confirmEntry: true,
            minBytes: 8,
        });
        if (!passphrase) return null;
        requireCurrent();
        try {
            const tokens = await Promise.all(
                Array.from({ length: count }, () => invoke<number>('cmd_stage_file_passphrase', { passphrase })),
            );
            requireCurrent();
            return tokens.map(promptToken => ({ ...base, promptToken }));
        } catch (error) {
            requireCurrent();
            if (throwOnFailure) throw error;
            toast.error(`Could not prepare encrypted upload: ${String(error)}`);
            return null;
        }
    };

    const chooseAndStageProtection = async (count: number): Promise<UploadProtectionIntent[] | null> => {
        requireCurrent();
        const controller = protectionScopeRef.current;
        if (!controller) return null;
        const selection = await chooseUploadProtection(count, { signal: controller.signal, isCurrent });
        if (!isCurrent() || !selection) return null;
        return stageProtectionForFiles(count, selection.mode, false, selection.protectMetadata);
    };

    /** Queues a set of file paths with an explicit, non-secret protection intent. */
    const queueFiles = async (
        paths: string[],
        destinationFolderId: number | null = activeFolderId,
        actionOwnerId: string | undefined = ownerId,
    ): Promise<number> => {
        if (!paths || paths.length === 0) return 0;
        if (!isCurrent() || !actionOwnerId || ownerRef.current !== actionOwnerId) throw new Error("ACCOUNT_CHANGED");
        const protection = await chooseAndStageProtection(paths.length);
        if (!protection) return 0;
        const preparedPaths: Array<{ path: string; androidStaged: boolean }> = [];
        try {
            for (const path of paths) {
                requireCurrent();
                preparedPaths.push(isAndroidPlatform
                    ? { path: await invoke<string>('cmd_stage_android_upload', { path }), androidStaged: true }
                    : { path, androidStaged: false });
                requireCurrent();
            }
        } catch (error) {
            await Promise.all(preparedPaths
                .filter(candidate => candidate.androidStaged)
                .map(candidate => invoke('cmd_delete_android_staged_upload', { path: candidate.path }).catch(() => undefined)));
            if (!isCurrent()) throw new Error('ACCOUNT_CHANGED');
            toast.error(`Android could not preserve the selected file for background recovery: ${String(error)}`);
            return 0;
        }
        const newItems: QueueItem[] = preparedPaths.map((prepared, index) => ({
            id: crypto.randomUUID(),
            ownerId: actionOwnerId,
            path: prepared.path,
            androidStaged: prepared.androidStaged || undefined,
            folderId: destinationFolderId,
            status: 'pending' as const,
            protection: protection[index],
            videoUploadMode: effectiveVideoUploadMode(
                paths[index],
                protection[index],
                settings.videoUploadMode,
            ),
            videoFilenameCaption: settings.videoFilenameCaption,
        }));
        requireCurrent();
        await enqueueUploadItems(newItems);
        toast.info(t('notifications.uploads_queued', { count: paths.length }));
        return paths.length;
    };

    const handleManualUpload = async () => {
        requireCurrent();
        const actionOwnerId = ownerId;
        const destinationFolderId = activeFolderId;
        const paths = await pickWithFallback(
            async () => {
                const selected = await open({ multiple: true, directory: false });
                if (!selected) return null;
                return Array.isArray(selected) ? selected : [selected];
            },
            () => handleManualUpload(),
            {
                errorTitle: 'File picker failed',
                onBrowserPicker: async () => {
                    const fallbackPaths = await showFileDialogFallback({ directory: false, multiple: true });
                    return fallbackPaths.length > 0 ? fallbackPaths : null;
                },
            },
        );
        requireCurrent();
        if (paths && paths.length > 0) {
            await queueFiles(paths, destinationFolderId, actionOwnerId);
        }
    };

    /** Queue files dropped from the OS file manager (drag-and-drop upload) */
    const handleDropUpload = async (paths: string[]): Promise<DropUploadResult> => {
        requireCurrent();
        const actionOwnerId = ownerId;
        if (!paths || paths.length === 0) return { queued: 0, rejected: [] };

        // Snapshot the destination before validation or an encryption prompt can yield.
        const destinationFolderId = activeFolderId;
        let validation: DroppedPathValidation;
        try {
            validation = await invoke<DroppedPathValidation>('cmd_validate_dropped_paths', { paths });
            requireCurrent();
        } catch (error) {
            requireCurrent();
            toast.error(t('settings.failed_prefix', { error: String(error) }));
            return { queued: 0, rejected: [] };
        }

        if (validation.rejected.length > 0) {
            toast.warning(t('notifications.drop_rejected', { count: validation.rejected.length }));
        }

        const queued = await queueFiles(validation.accepted, destinationFolderId, actionOwnerId);
        return {
            queued,
            rejected: validation.rejected,
            cancelled: validation.accepted.length > 0 && queued === 0,
        };
    };

    const handleFolderUpload = async () => {
        requireCurrent();
        const actionOwnerId = ownerId;
        const destinationFolderId = activeFolderId;
        const folderPath = await pickWithFallback(
            async () => {
                const selected = await open({ multiple: false, directory: true, title: i18n.t('common.upload_folder_picker') });
                if (!selected) return null;
                const fp = Array.isArray(selected) ? selected[0] : selected;
                return fp || null;
            },
            () => handleFolderUpload(),
            {
                errorTitle: 'Folder picker failed',
                onBrowserPicker: async () => {
                    const fallbackPaths = await showFileDialogFallback({ directory: true, multiple: true });
                    if (fallbackPaths.length > 0) {
                        // HTML folder picker returns individual file paths, not a folder path.
                        // We can't zip without a folder path, so files upload individually.
                        toast.info('Folder zipping unavailable with browser picker — uploading files individually.');
                        await queueFiles(fallbackPaths, destinationFolderId, actionOwnerId);
                    }
                    return null; // Already handled via queueFiles — signal that the main flow should stop
                },
            },
        );
        requireCurrent();
        if (!folderPath) return;

        const folderName = folderPath.split('/').pop() || folderPath.split('\\').pop() || 'folder';

        if (settings.zipFolders) {
            toast.info(`Zipping "${folderName}"...`);
            try {
                const zipPath = await invoke<string>('cmd_zip_folder', { folderPath });
                if (!isCurrent()) {
                    await invoke('cmd_delete_temp_zip', { path: zipPath }).catch(() => undefined);
                    return;
                }
                const protection = await chooseAndStageProtection(1);
                if (!protection) {
                    await invoke('cmd_delete_temp_zip', { path: zipPath }).catch(() => {});
                    return;
                }
                let uploadPath = zipPath;
                let androidStaged = false;
                if (isAndroidPlatform) {
                    try {
                        uploadPath = await invoke<string>('cmd_stage_android_upload', { path: zipPath });
                        androidStaged = true;
                    } catch (error) {
                        await invoke('cmd_delete_temp_zip', { path: zipPath }).catch(() => undefined);
                        throw error;
                    }
                    await invoke('cmd_delete_temp_zip', { path: zipPath }).catch(() => undefined);
                }
                const item: QueueItem = {
                    id: crypto.randomUUID(),
                    ownerId: actionOwnerId,
                    path: uploadPath,
                    folderId: destinationFolderId,
                    status: 'pending',
                    tempZipPath: androidStaged ? undefined : zipPath,
                    androidStaged: androidStaged || undefined,
                    protection: protection[0],
                };
                if (!isCurrent()) {
                    await cleanupResumableSource(item);
                    return;
                }
                await enqueueUploadItems([item]);
                toast.success(`Queued "${folderName}.zip" for upload`);
            } catch (e) {
                if (!isCurrent()) return;
                console.error('[Upload] Zip error:', e);
                toast.error(`Failed to zip folder: ${e}`);
            }
        } else {
            toast.info(`Folder upload without zipping is not supported. Enable "Zip folders before upload" in Settings.`);
        }
    };

    const stopRetryBatch = async () => {
        const batch = retryBatchRef.current;
        if (batch) batch.stopped = true;
        if (batch) await batch.done;
    };

    const cancelAll = async () => {
        if (!isCurrent()) return;
        await stopRetryBatch();
        if (!isCurrent()) return;
        if (!isAndroidPlatform) {
            void transferBulkAction('cancel', 'upload', ownerId).catch(error => { if (isCurrent()) toast.error(userFacingError(error, t)); });
            toast.info(i18n.t('transfer_copy.uploads_cancelled'));
            return;
        }
        setUploadQueue(q => {
            const activeItems = q.filter(i => ownsItem(i) && ['uploading', 'downloading', 'encrypting', 'verifying'].includes(i.status));
            const removableItems = q.filter(i => ownsItem(i) && ['pending', 'paused', 'waiting_for_network', 'waiting_for_unlock', 'error'].includes(i.status));
            for (const item of activeItems) {
                cancelledRef.current.add(item.id);
                invoke('cmd_cancel_transfer', { transferId: item.id }).catch(() => {});
            }
            for (const item of removableItems) void cleanupResumableSource(item);
            return q
                .filter(i => !removableItems.some(removable => removable.id === i.id))
                .map(i => activeItems.some(active => active.id === i.id) ? { ...i, status: 'cancelled' as const } : i);
        });
        toast.info(i18n.t('transfer_copy.uploads_cancelled'));
    };

    const pauseAll = async () => {
        if (!isCurrent()) return;
        await stopRetryBatch();
        if (!isCurrent()) return;
        if (!isAndroidPlatform) {
            void transferBulkAction('pause', 'upload', ownerId).catch(error => { if (isCurrent()) toast.error(userFacingError(error, t)); });
            toast.info(i18n.t('transfer_copy.uploads_paused'));
            return;
        }
        setUploadQueue(q => q.map(item => {
            if (!ownsItem(item)) return item;
            if (['uploading', 'downloading', 'encrypting', 'verifying'].includes(item.status)) {
                pausedRef.current.add(item.id);
                invoke('cmd_cancel_transfer', { transferId: item.id }).catch(() => {});
                return { ...item, status: 'paused' as const, error: undefined };
            }
            return item.status === 'pending' ? { ...item, status: 'paused' as const } : item;
        }));
        toast.info(i18n.t('transfer_copy.uploads_paused'));
    };

    const resumeAll = () => {
        if (!isCurrent()) return;
        if (!isAndroidPlatform) {
            void transferBulkAction('resume', 'upload', ownerId).catch(error => { if (isCurrent()) toast.error(userFacingError(error, t)); });
            toast.info(i18n.t('transfer_copy.uploads_resumed'));
            return;
        }
        setUploadQueue(q => q.map(item => ownsItem(item) && item.status === 'paused'
            ? { ...item, status: 'pending' as const, error: undefined }
            : item));
        toast.info(i18n.t('transfer_copy.uploads_resumed'));
    };

    const clearFinished = () => {
        if (!isCurrent()) return;
        if (!isAndroidPlatform) {
            void clearTerminalTransfers('upload', true, ownerId).catch(error => { if (isCurrent()) toast.error(userFacingError(error, t)); });
            return;
        }
        setUploadQueue(queue => {
            const removed = queue.filter(item => ownsItem(item) && ['success', 'error', 'cancelled'].includes(item.status)
                && !cancelledRef.current.has(item.id));
            for (const item of removed) void cleanupResumableSource(item);
            return queue.filter(item => !removed.some(candidate => candidate.id === item.id));
        });
    };

    const cancelItem = (id: string) => {
        if (!isCurrent()) return;
        if (!isAndroidPlatform) {
            void transferItemAction('cancel', id, ownerId).catch(error => { if (isCurrent()) toast.error(userFacingError(error, t)); });
            return;
        }
        setUploadQueue(q => {
            const item = q.find(i => ownsItem(i) && i.id === id);
            if (item && ['uploading', 'downloading', 'encrypting', 'verifying'].includes(item.status)) {
                cancelledRef.current.add(id);
                invoke('cmd_cancel_transfer', { transferId: id }).catch(() => {});
                return q.map(i => i.id === id ? { ...i, status: 'cancelled' as const } : i);
            }
            if (item && ['pending', 'paused', 'waiting_for_network', 'waiting_for_unlock', 'error'].includes(item.status)) {
                void cleanupResumableSource(item);
                return q.filter(i => i.id !== id);
            }
            return q;
        });
    };

    const runRetry = async (id: string, failedOnly = false, stopped?: () => boolean): Promise<RetryOutcome> => {
        const retryable = (item: QueueItem) => failedOnly ? item.status === 'error' : ['error', 'cancelled', 'waiting_for_unlock'].includes(item.status);
        const eligible = () => uploadQueueRef.current.find(item => ownsItem(item) && item.id === id && retryable(item));
        if (!isCurrent() || stopped?.()) return 'stopped';
        if (cancelledRef.current.has(id) || retryingItemsRef.current.get(id)?.()) return 'skipped';
        const item = eligible();
        if (!item) return 'skipped';
        retryingItemsRef.current.set(id, isCurrent);
        try {
            let protection = item.protection;
            let promptToken: number | undefined;
            if (protection?.mode === 'passphrase' || protection?.mode === 'vault_and_passphrase') {
                const staged = await stageProtectionForFiles(1, protection.mode, failedOnly);
                requireCurrent();
                if (!staged) return failedOnly ? 'stopped' : 'skipped';
                protection = { ...staged[0], protectMetadata: protection.protectMetadata };
                promptToken = protection.promptToken;
            }
            requireCurrent();
            if (stopped?.()) return 'stopped';
            if (!eligible() || cancelledRef.current.has(id)) return 'skipped';
            if (!isAndroidPlatform) {
                if (failedOnly) {
                    const result = await retryFailedUpload(id, ownerId!, promptToken);
                    return result.retried ? 'queued' : 'skipped';
                }
                if (protection?.promptToken) {
                    await supplyTransferPromptToken(id, protection.promptToken);
                    requireCurrent();
                }
                await transferItemAction('retry', id, ownerId);
                return 'queued';
            }
            setUploadQueue(q => q.map(i =>
                ownsItem(i) && i.id === id
                    ? { ...i, protection, status: 'pending' as const, error: undefined, progress: undefined, uploadedBytes: undefined, totalBytes: undefined, speedBytesPerSec: undefined }
                    : i
            ));
            return 'queued';
        } finally {
            if (retryingItemsRef.current.get(id) === isCurrent) retryingItemsRef.current.delete(id);
        }
    };

    const retryItem = async (id: string) => {
        if (retryBatchRef.current) return;
        try {
            await runRetry(id);
        } catch (error) {
            if (isCurrent() && !String(error).includes('ACCOUNT_CHANGED')) toast.error(userFacingError(error, t));
        }
    };

    const pauseItem = (id: string) => {
        if (!isCurrent()) return;
        if (!isAndroidPlatform) return;
        const item = uploadQueueRef.current.find(candidate => ownsItem(candidate) && candidate.id === id);
        if (!item || !['pending', 'waiting_for_network', 'uploading', 'downloading', 'encrypting', 'verifying'].includes(item.status)) return;
        if (['uploading', 'downloading', 'encrypting', 'verifying'].includes(item.status)) {
            pausedRef.current.add(id);
            void invoke('cmd_cancel_transfer', { transferId: id }).catch(() => undefined);
        }
        setUploadQueue(queue => queue.map(candidate => candidate.id === id
            ? { ...candidate, status: 'paused', speedBytesPerSec: 0, error: undefined }
            : candidate));
    };

    const resumeItem = (id: string) => {
        if (!isCurrent()) return;
        if (!isAndroidPlatform) return;
        setUploadQueue(queue => queue.map(item => ownsItem(item) && item.id === id && item.status === 'paused'
            ? { ...item, status: androidNetworkAvailableRef.current ? 'pending' : 'waiting_for_network', error: androidNetworkAvailableRef.current ? undefined : androidWaitingReason }
            : item));
    };


    const retryFailed = async (): Promise<RetryFailedSummary | undefined> => {
        if (isAndroidPlatform || !isCurrent() || !ownerId || retryBatchRef.current) return;
        const ids = uploadQueueRef.current.filter(item => ownsItem(item) && item.status === 'error').map(item => item.id);
        if (!ids.length) return;
        let finish!: () => void;
        const batch = { ownerId, stopped: false, done: new Promise<void>(resolve => { finish = resolve; }) };
        const summary: RetryFailedSummary = { queued: 0, skipped: 0, failed: 0, remaining: ids.length };
        retryBatchRef.current = batch;
        setRetryingFailed(true);
        let accountChanged = false;
        try {
            for (const id of ids) {
                if (!isCurrent() || batch.stopped) break;
                try {
                    const result = await runRetry(id, true, () => batch.stopped);
                    if (!isCurrent() || result === 'stopped') break;
                    summary[result]++;
                } catch (error) {
                    if (!isCurrent() || String(error).includes('ACCOUNT_CHANGED')) { accountChanged = true; break; }
                    if (String(error).includes('Transfer was not found') || !uploadQueueRef.current.some(item => ownsItem(item) && item.id === id && item.status === 'error')) summary.skipped++;
                    else summary.failed++;
                }
                summary.remaining--;
            }
            if (isCurrent() && !accountChanged) toast.info(t('activity.retry_failed_summary', { ...summary }));
            return summary;
        } finally {
            if (retryBatchRef.current === batch) {
                retryBatchRef.current = null;
                if (isCurrent()) setRetryingFailed(false);
            }
            finish();
        }
    };

    const handleUrlUpload = async (url: string, folderId: number | null) => {
        requireCurrent();
        const actionOwnerId = ownerId;
        if (!url || !url.trim()) return;
        let filename: string;
        try {
            filename = new URL(url).pathname.split('/').pop() || 'remote_file';
        } catch {
            filename = url.split('/').pop() || 'remote_file';
        }
        const protection = await chooseAndStageProtection(1);
        if (!protection) return;
        const item: QueueItem = {
            id: crypto.randomUUID(),
            ownerId: actionOwnerId,
            path: filename,
            url: url.trim(),
            folderId: folderId,
            status: 'pending' as const,
            protection: protection[0],
            // The response may reveal a video filename via Content-Disposition
            // even when the URL itself has no extension. Rust applies this
            // preference only after it knows the downloaded file's real name.
            videoUploadMode: protection[0].mode === 'standard' ? settings.videoUploadMode : 'file',
            videoFilenameCaption: settings.videoFilenameCaption,
        };
        await enqueueUploadItems([item]);
        toast.info(i18n.t('transfer_copy.remote_upload_queued'));
    };

    return {
        uploadQueue: uploadQueue.filter(item => Boolean(ownerId) && item.ownerId === ownerId),
        enqueueUploadItems,
        handleManualUpload,
        handleFolderUpload,
        handleDropUpload,
        handleUrlUpload,
        cancelAll,
        pauseAll,
        resumeAll,
        clearFinished,
        cancelItem,
        retryItem,
        retryFailed,
        retryingFailed,
        pauseItem,
        resumeItem,
    };
}
