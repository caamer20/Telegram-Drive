import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { useTranslation } from 'react-i18next';
import { useQueryClient } from '@tanstack/react-query';
import {
    MsAccountInfo,
    OneDriveItem,
    MigrationJobSummary,
    MigrationJobDetail,
    MigrationStats,
    ItemProgressPayload,
    CooldownPayload,
    JobStatePayload,
    ItemCompletePayload,
    StatsPayload,
    DailyMigrationQuota,
    MigrationActivity,
    ProcessingLogEntry,
} from '../types';
import { mergeActivity } from '../components/migration/transferState';

// Legacy local types (removed from main types.ts but still used in this hook)
interface ScanProgressPayload {
    phase: string;
    pages_scanned: number;
    discovered_files: number;
    discovered_folders: number;
    elapsed_ms: number;
}

interface AutoMigrationProfile {
    id: number;
    account_id: string;
    enabled: boolean;
    default_telegram_dest_id?: number | null;
    default_telegram_dest_name?: string | null;
    local_temp_dir?: string | null;
    last_auto_scan_at?: number | null;
    created_at: number;
    updated_at: number;
    active_job_id?: number | null;
    pause_reason?: string | null;
}

interface AutoMigrationStatus {
    profile: AutoMigrationProfile | null;
    account: MsAccountInfo | null;
    active_job: MigrationJobDetail | null;
    scan_progress: ScanProgressPayload | null;
}

const formatLogBytes = (bytes: number): string => {
    if (bytes <= 0) return '0 B';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    const index = Math.min(units.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)));
    return `${(bytes / (1024 ** index)).toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
};

const activityToProcessingLog = (entry: MigrationActivity): ProcessingLogEntry => ({
    id: `activity-${entry.id}`,
    timestamp: entry.created_at * 1000,
    category: entry.phase === 'downloading'
        ? 'download'
        : entry.phase === 'processing'
            ? 'processing'
            : entry.phase === 'uploading'
                ? 'upload'
                : entry.phase === 'scan'
                    ? 'scan'
                    : 'system',
    level: entry.status === 'failed'
        ? 'error'
        : entry.status === 'completed'
            ? 'success'
            : entry.phase === 'quota'
                ? 'warning'
                : 'info',
    message_key: entry.message || 'migration.log_persisted_activity',
    params: {
        name: entry.item_name || '',
        phase: entry.phase,
        status: entry.status,
    },
});

export function useMigration() {
    const { t } = useTranslation();
    const queryClient = useQueryClient();
    const [msAccount, setMsAccount] = useState<MsAccountInfo | null>(null);
    const [jobs, setJobs] = useState<MigrationJobSummary[]>([]);
    const [currentJobDetail, setCurrentJobDetail] = useState<MigrationJobDetail | null>(null);
    const [activeProgresses, setActiveProgresses] = useState<Record<number, ItemProgressPayload>>({});
    const [cooldown, setCooldown] = useState<CooldownPayload | null>(null);
    const [loading, setLoading] = useState<boolean>(false);
    const [snapshotLoading, setSnapshotLoading] = useState<boolean>(false);
    const [scanProgress, setScanProgress] = useState<ScanProgressPayload | null>(null);
    const [scanSnapshotItems, setScanSnapshotItems] = useState<OneDriveItem[]>([]);
    const [autoProfile, setAutoProfile] = useState<AutoMigrationProfile | null>(null);
    const [dailyQuota, setDailyQuota] = useState<DailyMigrationQuota | null>(null);
    const [migrationActivity, setMigrationActivity] = useState<MigrationActivity[]>([]);
    const [processingLogs, setProcessingLogs] = useState<ProcessingLogEntry[]>([]);
    const scanRequestInFlight = useRef(false);
    const processingLogSequence = useRef(0);
    const processingLogJobId = useRef<number | null>(null);

    const appendProcessingLog = useCallback((
        entry: Omit<ProcessingLogEntry, 'id' | 'timestamp'>,
    ) => {
        const nextEntry: ProcessingLogEntry = {
            ...entry,
            id: `${Date.now()}-${++processingLogSequence.current}`,
            timestamp: Date.now(),
        };
        setProcessingLogs(previous => [...previous, nextEntry].slice(-300));
    }, []);

    const clearProcessingLogs = useCallback(() => {
        setProcessingLogs([]);
    }, []);

    const loadScanSnapshot = useCallback(async () => {
        try {
            const items = await invoke<OneDriveItem[]>('cmd_migration_get_scan_snapshot');
            setScanSnapshotItems(items);
            return items;
        } catch (e) {
            console.error('Failed to load stopped scan snapshot:', e);
            setScanSnapshotItems([]);
            return [];
        }
    }, []);

    // Fetch MS connection status
    const checkMsStatus = useCallback(async () => {
        try {
            const res = await invoke<MsAccountInfo | null>('cmd_migration_ms_status');
            setMsAccount(res);
            return res;
        } catch (e) {
            console.error('Failed to get MS status:', e);
            return null;
        }
    }, []);

    // Connect Microsoft
    const connectMicrosoft = useCallback(async (clientId?: string, tenant?: string, redirectUri?: string) => {
        setLoading(true);
        try {
            const res = await invoke<MsAccountInfo>('cmd_migration_ms_connect', {
                clientId: clientId || null,
                tenant: tenant || null,
                redirectUri: redirectUri || null,
            });
            setMsAccount(res);
            toast.success(t('migration.connected_as', { name: res.account_name }));

            const persistedStatus = await invoke<AutoMigrationStatus>('cmd_migration_get_auto_status');
            setAutoProfile(persistedStatus.profile);
            setScanProgress(persistedStatus.scan_progress);
            if (persistedStatus.active_job) {
                setCurrentJobDetail(persistedStatus.active_job);
            }
            if (persistedStatus.scan_progress?.phase === 'stopped') {
                await loadScanSnapshot();
            }
            setSnapshotLoading(false);

            return res;
        } catch (e: any) {
            toast.error(t('migration.connect_failed', { error: String(e) }));
            throw e;
        } finally {
            setLoading(false);
        }
    }, [loadScanSnapshot, t]);




    // Disconnect Microsoft
    const disconnectMicrosoft = useCallback(async () => {
        setLoading(true);
        try {
            await invoke('cmd_migration_ms_disconnect');
            setMsAccount(null);
            setCurrentJobDetail(null);
            setActiveProgresses({});
            setMigrationActivity([]);
            setProcessingLogs([]);
            setAutoProfile(null);
            setDailyQuota(null);
            setCooldown(null);
            setScanProgress(null);
            setScanSnapshotItems([]);
            toast.info(t('migration.disconnected'));
        } catch (e: any) {
            toast.error(String(e));
        } finally {
            setLoading(false);
        }
    }, [t]);

    const switchMicrosoftAccount = useCallback(async () => {
        await disconnectMicrosoft();
        return connectMicrosoft();
    }, [connectMicrosoft, disconnectMicrosoft]);

    // List OneDrive folder contents (lazy loading)
    const listOneDriveFolders = useCallback(async (parentId?: string): Promise<OneDriveItem[]> => {
        try {
            return await invoke<OneDriveItem[]>('cmd_migration_list_onedrive_folders', {
                parentId: parentId || null,
            });
        } catch (e: any) {
            toast.error(t('migration.list_folders_failed', { error: String(e) }));
            return [];
        }
    }, [t]);

    // Fetch job list
    const refreshJobs = useCallback(async () => {
        try {
            const list = await invoke<MigrationJobSummary[]>('cmd_migration_get_jobs');
            setJobs(list);
            return list;
        } catch (e: any) {
            console.error('Failed to get jobs:', e);
            return [];
        }
    }, []);

    // Fetch job detail
    const loadJob = useCallback(async (jobId: number) => {
        try {
            const detail = await invoke<MigrationJobDetail>('cmd_migration_get_job', { jobId });
            setCurrentJobDetail(detail);
            return detail;
        } catch (e: any) {
            console.error('Failed to load job detail:', e);
            return null;
        }
    }, []);

    // Create job
    const createJob = useCallback(async () => {
        setLoading(true);
        try {
            const job = await invoke<{ id: number }>('cmd_migration_create_job');
            await refreshJobs();
            const detail = await loadJob(job.id);
            toast.success(t('migration.job_created'));
            return detail;
        } catch (e: any) {
            toast.error(t('migration.create_job_failed', { error: String(e) }));
            return null;
        } finally {
            setLoading(false);
        }
    }, [refreshJobs, loadJob, t]);

    // Delete job
    const deleteJob = useCallback(async (jobId: number) => {
        try {
            await invoke('cmd_migration_delete_job', { jobId });
            if (currentJobDetail?.job.id === jobId) {
                setCurrentJobDetail(null);
            }
            await refreshJobs();
            toast.info(t('migration.job_deleted'));
        } catch (e: any) {
            toast.error(String(e));
        }
    }, [currentJobDetail, refreshJobs, t]);

    // Config setters
    const setOneDriveFolder = useCallback(async (jobId: number, folderId: string, folderPath: string) => {
        try {
            await invoke('cmd_migration_set_onedrive_folder', { jobId, folderId, folderPath });
            await loadJob(jobId);
        } catch (e: any) {
            toast.error(String(e));
        }
    }, [loadJob]);

    const setTelegramDestination = useCallback(async (jobId: number, destinationId: number | null, destinationName: string) => {
        try {
            await invoke('cmd_migration_set_telegram_destination', {
                jobId,
                destinationId,
                destinationName,
            });
            await loadJob(jobId);
        } catch (e: any) {
            toast.error(String(e));
        }
    }, [loadJob]);

    const setLocalDir = useCallback(async (jobId: number, localDir: string) => {
        try {
            await invoke('cmd_migration_set_local_dir', { jobId, localDir });
            await loadJob(jobId);
        } catch (e: any) {
            toast.error(String(e));
        }
    }, [loadJob]);

    // Scan folder snapshot
    const scan = useCallback(async (jobId: number) => {
        setLoading(true);
        try {
            const stats = await invoke<MigrationStats>('cmd_migration_scan', { jobId });
            await loadJob(jobId);
            toast.success(t('migration.scan_complete', { count: stats.total_files }));
            return stats;
        } catch (e: any) {
            toast.error(t('migration.scan_failed', { error: String(e) }));
            return null;
        } finally {
            setLoading(false);
        }
    }, [loadJob, t]);

    // Migration Controls
    const startMigration = useCallback(async (jobId: number) => {
        try {
            await invoke('cmd_migration_start', { jobId });
            toast.info(t('migration.job_started'));
        } catch (e: any) {
            toast.error(String(e));
        }
    }, [t]);

    const pauseMigration = useCallback(async (jobId: number) => {
        try {
            await invoke('cmd_migration_pause', { jobId });
            toast.info(t('migration.pausing'));
        } catch (e: any) {
            toast.error(String(e));
        }
    }, [t]);

    const resumeMigration = useCallback(async (jobId: number) => {
        try {
            await invoke('cmd_migration_resume', { jobId });
            toast.info(t('migration.resuming'));
        } catch (e: any) {
            toast.error(String(e));
        }
    }, [t]);

    const cancelMigration = useCallback(async (jobId: number) => {
        try {
            await invoke('cmd_migration_cancel', { jobId });
            toast.info(t('migration.cancelling'));
        } catch (e: any) {
            toast.error(String(e));
        }
    }, [t]);

    const retryItem = useCallback(async (jobId: number, itemId: number) => {
        try {
            await invoke('cmd_migration_retry_item', { jobId, itemId });
            await loadJob(jobId);
            toast.info(t('migration.item_retried'));
        } catch (e: any) {
            toast.error(String(e));
        }
    }, [loadJob, t]);

    const retryAllFailed = useCallback(async (jobId: number) => {
        try {
            const count = await invoke<number>('cmd_migration_retry_all_failed', { jobId });
            await loadJob(jobId);
            toast.info(t('migration.retried_all', { count }));
        } catch (e: any) {
            toast.error(String(e));
        }
    }, [loadJob, t]);

    // Initial mount & Event Listeners
    useEffect(() => {
        checkMsStatus().then(account => {
            if (account) {
                refreshJobs();
                void queryClient.invalidateQueries({ queryKey: ['files'] });
                void queryClient.invalidateQueries({ queryKey: ['bandwidth'] });
            }
        });

        const unlisteners: UnlistenFn[] = [];
        let disposed = false;
        const retainUnlistener = (unlisten: UnlistenFn) => {
            if (disposed) {
                unlisten();
            } else {
                unlisteners.push(unlisten);
            }
        };

        listen<JobStatePayload>('migration:job-state', (e) => {
            loadJob(e.payload.job_id);
            refreshJobs();
            appendProcessingLog({
                category: 'job',
                level: e.payload.state === 'failed' ? 'error' : e.payload.state === 'completed' ? 'success' : 'info',
                message_key: 'migration.log_job_state',
                params: {
                    job: e.payload.job_id,
                    previous: e.payload.previous_state,
                    state: e.payload.state,
                },
            });
        }).then(retainUnlistener);

        let lastProgressTime = 0;
        const lastLoggedProgressBucket = new Map<string, number>();
        listen<ItemProgressPayload>('migration:item-progress', (e) => {
            const now = Date.now();
            if (now - lastProgressTime > 200 || e.payload.percent === 100) {
                lastProgressTime = now;
                setActiveProgresses(prev => {
                    return { ...prev, [e.payload.item_id]: { ...e.payload, timestamp: e.payload.timestamp ?? now } };
                });
            }

            const progressKey = `${e.payload.job_id}:${e.payload.item_id}:${e.payload.phase}`;
            const progressBucket = Math.floor(Math.max(0, Math.min(100, e.payload.percent)) / 5);
            if (lastLoggedProgressBucket.get(progressKey) !== progressBucket || e.payload.percent === 100) {
                lastLoggedProgressBucket.set(progressKey, progressBucket);
                appendProcessingLog({
                    category: e.payload.phase === 'downloading'
                        ? 'download'
                        : e.payload.phase === 'uploading'
                            ? 'upload'
                            : 'processing',
                    level: 'info',
                    message_key: e.payload.phase === 'downloading'
                        ? 'migration.log_item_downloading'
                        : e.payload.phase === 'uploading'
                            ? 'migration.log_item_uploading'
                            : e.payload.phase === 'analyzing'
                                ? 'migration.log_item_analyzing'
                                : 'migration.log_item_processing',
                    params: {
                        name: e.payload.item_name,
                        percent: Math.round(e.payload.percent),
                        done: formatLogBytes(e.payload.bytes_done),
                        total: formatLogBytes(e.payload.bytes_total),
                        speed: `${formatLogBytes(e.payload.speed_bytes_per_sec)}/s`,
                    },
                });
            }
        }).then(retainUnlistener);

        listen<ItemCompletePayload>('migration:item-complete', (e) => {
            void queryClient.invalidateQueries({ queryKey: ['bandwidth'] });
            if (e.payload.status === 'completed_telegram' || e.payload.status === 'completed_local') {
                void queryClient.invalidateQueries({ queryKey: ['files'] });
            }
            setActiveProgresses(prev => {
                const next = { ...prev };
                delete next[e.payload.item_id];
                return next;
            });
            if (e.payload.status === 'failed') {
                toast.error(`${e.payload.item_name}: ${e.payload.error_message || 'Failed'}`);
            }
            appendProcessingLog({
                category: 'system',
                level: e.payload.status === 'failed'
                    ? 'error'
                    : 'success',
                message_key: e.payload.status === 'failed'
                    ? 'migration.log_item_failed'
                    : 'migration.log_item_completed',
                params: {
                    name: e.payload.item_name,
                    error: e.payload.error_message || e.payload.error_type || '',
                },
            });
            // Update item status locally in state to prevent heavy full-DB reload
            setCurrentJobDetail(prev => {
                if (!prev || prev.job.id !== e.payload.job_id) return prev;
                const updatedFiles = prev.files.map(f =>
                    f.id === e.payload.item_id ? { ...f, pipeline_stage: e.payload.status } : f
                );
                return { ...prev, files: updatedFiles };
            });
        }).then(retainUnlistener);



        listen<StatsPayload>('migration:stats', (e) => {
            setCurrentJobDetail(prev =>
                prev?.job.id === e.payload.job_id ? { ...prev, stats: e.payload.stats } : prev
            );
        }).then(retainUnlistener);

        listen<CooldownPayload>('migration:cooldown', (e) => {
            setCooldown(e.payload);
            if (e.payload.seconds_remaining > 0) {
                appendProcessingLog({
                    category: 'system',
                    level: 'warning',
                    message_key: 'migration.log_cooldown',
                    params: {
                        job: e.payload.job_id,
                        seconds: e.payload.seconds_remaining,
                    },
                });
            }
        }).then(retainUnlistener);

        listen<MigrationActivity>('migration:activity', (e) => {
            setMigrationActivity(previous => mergeActivity(
                e.payload.phase === 'scan' && e.payload.status === 'started'
                    ? previous.filter(entry => !(entry.job_id === 0 && entry.status === 'starting'))
                    : previous,
                e.payload,
            ));
        }).then(retainUnlistener);

        listen<ScanProgressPayload>('migration:scan-progress', (e) => {
            setScanProgress(e.payload);
            if (
                e.payload.phase === 'starting'
                || e.payload.phase === 'enumerating'
                || e.payload.phase === 'building_snapshot'
                || e.payload.phase === 'stopping'
            ) {
                setSnapshotLoading(true);
            } else if (
                e.payload.phase === 'failed'
                || e.payload.phase === 'stopped'
                || e.payload.phase === 'completed'
            ) {
                setSnapshotLoading(false);
            }
            if (e.payload.phase === 'stopped') {
                void loadScanSnapshot();
            }
            appendProcessingLog({
                category: 'scan',
                level: e.payload.phase === 'failed' ? 'error' : 'info',
                message_key: e.payload.phase === 'failed'
                    ? 'migration.log_scan_failed'
                    : e.payload.phase === 'stopped'
                        ? 'migration.log_scan_stopped'
                    : e.payload.phase === 'building_snapshot'
                        ? 'migration.log_snapshot_building'
                        : 'migration.log_scan_progress',
                params: {
                    pages: e.payload.pages_scanned,
                    files: e.payload.discovered_files,
                    folders: e.payload.discovered_folders,
                    seconds: Math.floor(e.payload.elapsed_ms / 1000),
                },
            });
        }).then(retainUnlistener);

        listen<{ error: string }>('migration:pipeline-error', (e) => {
            scanRequestInFlight.current = false;
            setLoading(false);
            setSnapshotLoading(false);
            setMigrationActivity(previous => mergeActivity(
                previous.filter(entry => !(entry.job_id === 0 && entry.status === 'starting')),
                {
                    id: -Date.now(),
                    job_id: 0,
                    item_id: null,
                    item_name: null,
                    phase: 'failed',
                    status: 'failed',
                    attempt: 0,
                    revision: 0,
                    message: t('migration.pipeline_start_failed_activity', {
                        error: e.payload.error,
                    }),
                    created_at: Math.floor(Date.now() / 1000),
                },
            ));
            appendProcessingLog({
                category: 'system',
                level: 'error',
                message_key: 'migration.log_scan_start_failed',
                params: { error: e.payload.error },
            });
            toast.error(t('migration.pipeline_start_failed', {
                error: e.payload.error,
            }));
        }).then(retainUnlistener);

        listen<{ job_id: number }>('migration:snapshot-ready', (e) => {
            setScanSnapshotItems([]);
            void loadJob(e.payload.job_id).finally(() => {
                setSnapshotLoading(false);
            });
            void refreshJobs();
        }).then(retainUnlistener);

        return () => {
            disposed = true;
            unlisteners.forEach(fn => fn());
        };
    }, [checkMsStatus, refreshJobs, loadJob, loadScanSnapshot, appendProcessingLog, queryClient, t]);

    const getAutoStatus = useCallback(async () => {
        try {
            const res = await invoke<AutoMigrationStatus>('cmd_migration_get_auto_status');
            setAutoProfile(res.profile);
            if (res.account) {
                setMsAccount(res.account);
            }
            if (res.active_job) {
                setCurrentJobDetail(res.active_job);
            }
            setScanProgress(res.scan_progress);
            const scanIsActive = res.scan_progress?.phase === 'starting'
                || res.scan_progress?.phase === 'enumerating'
                || res.scan_progress?.phase === 'building_snapshot'
                || res.scan_progress?.phase === 'stopping';
            if (scanIsActive) {
                setSnapshotLoading(true);
            } else if (
                res.active_job
                || res.scan_progress?.phase === 'failed'
                || res.scan_progress?.phase === 'stopped'
            ) {
                setSnapshotLoading(false);
            }
            return res;
        } catch (e) {
            console.error('Failed to get auto status:', e);
            return null;
        }
    }, []);

    const toggleAuto = useCallback(async (enabled: boolean) => {
        setLoading(true);
        try {
            const res = await invoke<AutoMigrationProfile>('cmd_migration_toggle_auto', { enabled });
            setAutoProfile(res);
            toast.success(enabled ? t('migration.auto_enabled', 'Auto Migration enabled') : t('migration.auto_disabled', 'Auto Migration disabled'));
            return res;
        } catch (e: any) {
            toast.error(String(e));
            return null;
        } finally {
            setLoading(false);
        }
    }, [t]);

    const updateAutoSettings = useCallback(async (destId?: number, destName?: string, tempDir?: string) => {
        setLoading(true);
        try {
            const res = await invoke<AutoMigrationProfile>('cmd_migration_update_auto_settings', {
                destId: destId || null,
                destName: destName || null,
                tempDir: tempDir || null,
            });
            setAutoProfile(res);
            toast.success(t('migration.settings_saved', 'Settings saved'));
            return res;
        } catch (e: any) {
            toast.error(String(e));
            throw e;
        } finally {
            setLoading(false);
        }
    }, [t]);

    const getDailyQuota = useCallback(async () => {
        try {
            const res = await invoke<DailyMigrationQuota>('cmd_migration_get_daily_quota');
            setDailyQuota(res);
            return res;
        } catch (e) {
            console.error('Failed to get daily quota:', e);
            return null;
        }
    }, []);

    const getActivity = useCallback(async (jobId: number) => {
        try {
            const entries = await invoke<MigrationActivity[]>('cmd_migration_get_activity', {
                jobId,
                limit: 100,
            });
            setMigrationActivity(entries);
            const isSameJob = processingLogJobId.current === jobId;
            processingLogJobId.current = jobId;
            const persistedLogs = entries
                .slice()
                .reverse()
                .map(activityToProcessingLog);
            setProcessingLogs(previous => {
                const base = isSameJob
                    ? previous.filter(entry => !entry.id.startsWith('activity-'))
                    : [];
                return [...base, ...persistedLogs]
                    .sort((left, right) => left.timestamp - right.timestamp)
                    .slice(-300);
            });
            return entries;
        } catch (e) {
            console.error('Failed to get migration activity:', e);
            return [];
        }
    }, []);

    const executeRescanAuto = useCallback(async (reset: boolean) => {
        setLoading(true);
        setSnapshotLoading(true);
        scanRequestInFlight.current = true;
        setScanProgress({
            phase: 'starting',
            pages_scanned: 0,
            discovered_files: 0,
            discovered_folders: 0,
            elapsed_ms: 0,
        });
        setScanSnapshotItems([]);
        setProcessingLogs([]);
        processingLogJobId.current = null;
        appendProcessingLog({
            category: 'scan',
            level: 'info',
            message_key: 'migration.log_scan_started',
            params: { reset: reset ? 1 : 0 },
        });
        setMigrationActivity(previous => mergeActivity(previous, {
            id: -Date.now(),
            job_id: 0,
            item_id: null,
            item_name: null,
            phase: 'scan',
            status: 'starting',
            attempt: 0,
            revision: 0,
            message: t('migration.pipeline_starting_activity', 'Đang khởi động pipeline quét và migrate tuần tự...'),
            created_at: Math.floor(Date.now() / 1000),
        }));
        try {
            await invoke<void>('cmd_migration_rescan_auto', { reset });
            await getAutoStatus();
            toast.info(t(
                'migration.pipeline_start_accepted',
                'Pipeline đã được tiếp nhận và đang chạy nền.',
            ));
            return true;
        } catch (e: any) {
            setScanProgress((previous: any) => previous
                ? { ...previous, phase: 'failed' }
                : null);
            setSnapshotLoading(false);
            setMigrationActivity((previous: any) => previous.filter(
                (entry: any) => !(entry.job_id === 0 && entry.status === 'starting'),
            ));
            appendProcessingLog({
                category: 'system',
                level: 'error',
                message_key: 'migration.log_scan_start_failed',
                params: { error: String(e) },
            });
            toast.error(String(e));
            return null;
        } finally {
            scanRequestInFlight.current = false;
            setLoading(false);
        }
    }, [appendProcessingLog, getAutoStatus, t]);

    const rescanAuto = useCallback(
        () => executeRescanAuto(false),
        [executeRescanAuto],
    );

    const resetAndRescanAuto = useCallback(
        () => executeRescanAuto(true),
        [executeRescanAuto],
    );

    const stopAutoScan = useCallback(async () => {
        setScanProgress((previous: any) => previous
            ? { ...previous, phase: 'stopping' }
            : {
                phase: 'stopping',
                pages_scanned: 0,
                discovered_files: 0,
                discovered_folders: 0,
                elapsed_ms: 0,
            });
        try {
            const progress = await invoke<ScanProgressPayload>('cmd_migration_stop_auto_scan');
            setScanProgress(progress);
            toast.info(t('migration.scan_stop_requested', 'Đang dừng quét và lưu checkpoint...'));
            return progress;
        } catch (e: any) {
            setSnapshotLoading(false);
            toast.error(String(e));
            return null;
        }
    }, [t]);

    const deleteMigrationItem = useCallback(async (jobId: number, itemId: number) => {
        setLoading(true);
        try {
            await invoke('cmd_migration_delete_item', { jobId, itemId });
            toast.success(t('migration.item_deleted_onedrive', 'Đã xóa file trên OneDrive và hủy đồng bộ'));
            await loadJob(jobId);
        } catch (e: any) {

            toast.error(String(e));
        } finally {
            setLoading(false);
        }
    }, [loadJob, t]);

    const renameMigrationItem = useCallback(async (jobId: number, itemId: number, newName: string) => {
        setLoading(true);
        try {
            await invoke('cmd_migration_rename_item', { jobId, itemId, newName });
            toast.success(t('migration.item_renamed', 'File renamed'));
            await loadJob(jobId);
        } catch (e: any) {
            toast.error(String(e));
        } finally {
            setLoading(false);
        }
    }, [loadJob, t]);

    const syncSingleItem = useCallback(async (jobId: number, itemId: number) => {
        setLoading(true);
        try {
            await invoke('cmd_migration_sync_single_item', { jobId, itemId });
            toast.success(t('migration.sync_single_started', 'Đã bắt đầu đồng bộ file đã chọn'));
        } catch (e: any) {
            toast.error(String(e));
        } finally {
            setLoading(false);
        }
    }, [t]);

    const syncScanSnapshotItem = useCallback(async (sourceItemId: string) => {
        setLoading(true);
        try {
            const jobId = await invoke<number>('cmd_migration_sync_scan_snapshot_item', {
                sourceItemId,
            });
            await loadJob(jobId);
            await refreshJobs();
            toast.success(t(
                'migration.checkpoint_migration_started',
                'Đã bắt đầu migrate file đã quét',
            ));
            return jobId;
        } catch (e: any) {
            toast.error(String(e));
            return null;
        } finally {
            setLoading(false);
        }
    }, [loadJob, refreshJobs, t]);

    const queueSelectedItems = useCallback(async (sourceItemIds: string[], actionType: 'download' | 'sync') => {
        setLoading(true);
        try {
            const jobId = await invoke<number>('cmd_migration_queue_selected_items', {
                sourceItemIds,
                actionType,
            });
            await loadJob(jobId);
            await refreshJobs();
            toast.success(
                actionType === 'download'
                    ? t('migration.bulk_download_started', 'Đã bắt đầu tải xuống các mục đã chọn')
                    : t('migration.bulk_sync_started', 'Đã bắt đầu đồng bộ các mục đã chọn'),
            );
            return jobId;
        } catch (e: any) {
            toast.error(String(e));
            return null;
        } finally {
            setLoading(false);
        }
    }, [loadJob, refreshJobs, t]);

    useEffect(() => {
        if (!msAccount) return;

        void (async () => {
            setSnapshotLoading(true);
            const status = await getAutoStatus();
            void getDailyQuota();
            if (!status) {
                setSnapshotLoading(false);
                return;
            }
            if (status.scan_progress?.phase === 'stopped') {
                await loadScanSnapshot();
            } else if (status.active_job) {
                setCurrentJobDetail(status.active_job);
                setScanSnapshotItems([]);
            } else {
                // The profile can legitimately have no active_job_id after a completed
                // migration. Restore the latest persisted job so the file list survives
                // a page reload/tab switch instead of appearing empty.
                const persistedJobs = await refreshJobs();
                const latestJob = persistedJobs.find(job => job.total_files > 0);
                if (latestJob) {
                    await loadJob(latestJob.id);
                }
            }
            const scanIsActive = status.scan_progress?.phase === 'starting'
                || status.scan_progress?.phase === 'enumerating'
                || status.scan_progress?.phase === 'building_snapshot'
                || status.scan_progress?.phase === 'stopping';
            if (!scanIsActive) {
                setSnapshotLoading(false);
            }
        })();
    }, [msAccount?.account_email, getAutoStatus, getDailyQuota, loadScanSnapshot, refreshJobs, loadJob]);

    useEffect(() => {
        const scanIsActive = scanProgress?.phase === 'starting'
            || scanProgress?.phase === 'enumerating'
            || scanProgress?.phase === 'building_snapshot'
            || scanProgress?.phase === 'stopping';
        if (!msAccount || !scanIsActive) return;

        let cancelled = false;
        const refreshAuthoritativeState = async () => {
            const status = await getAutoStatus();
            if (cancelled || !status) return;
            if (status.active_job || status.scan_progress?.phase === 'failed') {
                setSnapshotLoading(false);
            }
        };
        const intervalId = window.setInterval(() => {
            void refreshAuthoritativeState();
        }, 2000);

        return () => {
            cancelled = true;
            window.clearInterval(intervalId);
        };
    }, [
        msAccount?.account_email,
        scanProgress?.phase,
        currentJobDetail?.job.id,
        getAutoStatus,
    ]);

    useEffect(() => {
        const jobId = currentJobDetail?.job.id;
        if (!jobId) {
            setMigrationActivity([]);
            return;
        }

        void getActivity(jobId);
        const shouldPoll = currentJobDetail?.job.state === 'running'
            || scanProgress?.phase === 'starting'
            || scanProgress?.phase === 'enumerating'
            || scanProgress?.phase === 'building_snapshot'
            || scanProgress?.phase === 'stopping';
        if (!shouldPoll) return;

        const intervalId = window.setInterval(() => {
            void getActivity(jobId);
        }, 2000);
        return () => window.clearInterval(intervalId);
    }, [
        currentJobDetail?.job.id,
        currentJobDetail?.job.state,
        scanProgress?.phase,
        getActivity,
    ]);

    return {
        msAccount,
        jobs,
        currentJobDetail,
        activeProgresses,
        cooldown,
        loading,
        snapshotLoading,
        scanProgress,
        scanSnapshotItems,
        autoProfile,
        dailyQuota,
        migrationActivity,
        processingLogs,
        connectMicrosoft,
        disconnectMicrosoft,
        switchMicrosoftAccount,
        checkMsStatus,
        listOneDriveFolders,
        refreshJobs,
        loadJob,
        createJob,
        deleteJob,
        setOneDriveFolder,
        setTelegramDestination,
        setLocalDir,
        scan,
        startMigration,
        pauseMigration,
        resumeMigration,
        cancelMigration,
        retryItem,
        retryAllFailed,
        getAutoStatus,
        toggleAuto,
        updateAutoSettings,
        getDailyQuota,
        getActivity,
        rescanAuto,
        resetAndRescanAuto,
        stopAutoScan,
        deleteMigrationItem,
        renameMigrationItem,
        syncSingleItem,
        syncScanSnapshotItem,
        queueSelectedItems,
        clearProcessingLogs,
    };
}
