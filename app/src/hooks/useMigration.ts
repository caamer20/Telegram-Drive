import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { useTranslation } from 'react-i18next';
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
    AutoMigrationProfile,
    DailyMigrationQuota,
} from '../types';


export function useMigration() {
    const { t } = useTranslation();
    const [msAccount, setMsAccount] = useState<MsAccountInfo | null>(null);
    const [jobs, setJobs] = useState<MigrationJobSummary[]>([]);
    const [currentJobDetail, setCurrentJobDetail] = useState<MigrationJobDetail | null>(null);
    const [itemProgress, setItemProgress] = useState<ItemProgressPayload | null>(null);
    const [cooldown, setCooldown] = useState<CooldownPayload | null>(null);
    const [loading, setLoading] = useState<boolean>(false);

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
            return res;
        } catch (e: any) {
            toast.error(t('migration.connect_failed', { error: String(e) }));
            throw e;
        } finally {
            setLoading(false);
        }
    }, [t]);




    // Disconnect Microsoft
    const disconnectMicrosoft = useCallback(async () => {
        setLoading(true);
        try {
            await invoke('cmd_migration_ms_disconnect');
            setMsAccount(null);
            toast.info(t('migration.disconnected'));
        } catch (e: any) {
            toast.error(String(e));
        } finally {
            setLoading(false);
        }
    }, [t]);

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
        checkMsStatus();
        refreshJobs().then(list => {
            if (list.length > 0) {
                loadJob(list[0].id);
            }
        });

        const unlisteners: UnlistenFn[] = [];

        listen<JobStatePayload>('migration:job-state', (e) => {
            if (currentJobDetail && currentJobDetail.job.id === e.payload.job_id) {
                loadJob(e.payload.job_id);
            }
            refreshJobs();
        }).then(fn => unlisteners.push(fn));

        listen<ItemProgressPayload>('migration:item-progress', (e) => {
            setItemProgress(e.payload);
        }).then(fn => unlisteners.push(fn));

        listen<ItemCompletePayload>('migration:item-complete', (e) => {
            setItemProgress(null);
            if (e.payload.status === 'failed') {
                toast.error(`${e.payload.item_name}: ${e.payload.error_message || 'Failed'}`);
            }
            if (currentJobDetail && currentJobDetail.job.id === e.payload.job_id) {
                loadJob(e.payload.job_id);
            }
        }).then(fn => unlisteners.push(fn));

        listen<StatsPayload>('migration:stats', (e) => {
            if (currentJobDetail && currentJobDetail.job.id === e.payload.job_id) {
                setCurrentJobDetail(prev => prev ? { ...prev, stats: e.payload.stats } : null);
            }
        }).then(fn => unlisteners.push(fn));

        listen<CooldownPayload>('migration:cooldown', (e) => {
            setCooldown(e.payload);
        }).then(fn => unlisteners.push(fn));

        return () => {
            unlisteners.forEach(fn => fn());
        };
    }, [checkMsStatus, refreshJobs, loadJob, currentJobDetail]);

    const [autoProfile, setAutoProfile] = useState<AutoMigrationProfile | null>(null);
    const [dailyQuota, setDailyQuota] = useState<DailyMigrationQuota | null>(null);

    const getAutoStatus = useCallback(async () => {
        try {
            const res = await invoke<AutoMigrationProfile | null>('cmd_migration_get_auto_status');
            setAutoProfile(res);
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
            throw e;
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

    useEffect(() => {
        getAutoStatus();
        getDailyQuota();
    }, [getAutoStatus, getDailyQuota]);

    return {
        msAccount,
        jobs,
        currentJobDetail,
        itemProgress,
        cooldown,
        loading,
        autoProfile,
        dailyQuota,
        connectMicrosoft,
        disconnectMicrosoft,
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
    };
}

