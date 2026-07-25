import React, { useState, useCallback, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { useMigrationContext } from '../../context/MigrationContext';
import { Cloud, Play, Pause, Square, RotateCcw, FolderOpen, FileJson, Shield, AlertTriangle } from 'lucide-react';

interface PreflightResult {
    job_id: number;
    total_files: number;
    total_bytes: number;
    video_count: number;
    video_bytes: number;
    image_count: number;
    image_bytes: number;
    other_count: number;
    other_bytes: number;
    telegram_bytes: number;
    local_bytes: number;
    quota_remaining: number;
    disk_available: number;
    warnings: string[];
    valid: boolean;
}

interface BackupV2Status {
    job_id: number;
    state: string;
    is_active: boolean;
    stats: {
        total_items: number;
        completed_telegram: number;
        completed_local: number;
        skipped_duplicates: number;
        failed_items: number;
        reconciliation_required: number;
        waiting_for_quota: number;
    };
    quota_used: number;
    quota_remaining: number;
    flood_wait_secs: number | null;
    manifest_state: string;
}

const formatBytes = (bytes: number): string => {
    if (bytes <= 0) return '0 B';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    const index = Math.min(units.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)));
    return `${(bytes / (1024 ** index)).toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
};


export const BackupV2Page: React.FC = () => {
    const { t } = useTranslation();
    const {
        msAccount,
        connectMicrosoft,
        switchMicrosoftAccount,
        listOneDriveFolders,
        loading,
    } = useMigrationContext();

    // ---- Configuration state ----
    const [sourceFolderId, setSourceFolderId] = useState<string>('root');
    const [sourceFolderPath, setSourceFolderPath] = useState<string>('/');
    const [localBackupDir, _setLocalBackupDir] = useState<string>('/Volumes/DATASTORE/Temo');
    const [workspaceDir, setWorkspaceDir] = useState<string>('/Volumes/DATASTORE/Temo/_workspace');
    const [folders, setFolders] = useState<Array<{ id: string; name: string; path: string }>>([]);
    const [_foldersLoading, setFoldersLoading] = useState(false);

    // ---- Preflight state ----
    const [preflightResult, setPreflightResult] = useState<PreflightResult | null>(null);
    const [preflightRunning, setPreflightRunning] = useState(false);

    // ---- Job state ----
    const [jobId, setJobId] = useState<number | null>(null);
    const [isRunning, setIsRunning] = useState(false);
    const [isPaused, setIsPaused] = useState(false);
    const [status, setStatus] = useState<BackupV2Status | null>(null);
    const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

    // ---- Load OneDrive folders ----
    const loadFolders = useCallback(async (parentId?: string) => {
        setFoldersLoading(true);
        try {
            const items = await listOneDriveFolders(parentId);
            setFolders(items.filter(f => f.item_type === 'folder').map(f => ({
                id: f.id,
                name: f.name,
                path: f.path || f.name,
            })));
        } catch (e: any) {
            toast.error(String(e));
        } finally {
            setFoldersLoading(false);
        }
    }, [listOneDriveFolders]);

    useEffect(() => {
        if (msAccount) {
            void loadFolders();
        }
    }, [msAccount, loadFolders]);

    // ---- Polling status ----
    const startPolling = useCallback((jid: number) => {
        if (pollRef.current) clearInterval(pollRef.current);
        pollRef.current = setInterval(async () => {
            try {
                const s = await invoke<BackupV2Status>('cmd_backup_v2_get_status', { jobId: jid });
                setStatus(s);
                if (s.state === 'completed' || s.state === 'failed') {
                    setIsRunning(false);
                    setIsPaused(false);
                    if (pollRef.current) {
                        clearInterval(pollRef.current);
                        pollRef.current = null;
                    }
                }
            } catch {
                // ignore poll errors
            }
        }, 2000);
    }, []);

    useEffect(() => {
        return () => {
            if (pollRef.current) clearInterval(pollRef.current);
        };
    }, []);

    // ---- Preflight ----
    const handlePreflight = useCallback(async () => {
        if (!sourceFolderId) {
            toast.error(t('migration.v2_select_folder', 'Vui lòng chọn thư mục nguồn'));
            return;
        }
        if (!localBackupDir) {
            toast.error(t('migration.v2_select_backup', 'Vui lòng chọn thư mục backup local'));
            return;
        }
        const ws = workspaceDir || localBackupDir + '/_TelegramDrive_Workspace';
        setPreflightResult(null);
        try {
            const result = await invoke<PreflightResult>('cmd_backup_v2_preflight', {
                sourceFolderId,
                sourceFolderPath: sourceFolderPath || '/',
                telegramDestinationId: null,
                telegramDestinationName: "Saved Messages",
                localBackupDir,
                workspaceDir: ws,
            });
            setPreflightResult(result);
            setJobId(result.job_id);
            if (!workspaceDir) setWorkspaceDir(ws);
            toast.success(t('migration.v2_preflight_ok', 'Preflight hoàn tất'));
        } catch (e: any) {
            toast.error(String(e));
        } finally {
            setPreflightRunning(false);
        }
    }, [sourceFolderId, sourceFolderPath, null, "Saved Messages", localBackupDir, workspaceDir, t]);

    // ---- Start ----
    const handleStart = useCallback(async () => {
        if (!jobId) return;
        setIsRunning(true);
        setIsPaused(false);
        try {
            await invoke('cmd_backup_v2_start', { jobId });
            toast.success(t('migration.v2_started', 'Backup đã bắt đầu'));
            startPolling(jobId);
        } catch (e: any) {
            setIsRunning(false);
            toast.error(String(e));
        }
    }, [jobId, startPolling, t]);

    // ---- Controls ----
    const handlePause = useCallback(async () => {
        try {
            await invoke('cmd_backup_v2_pause');
            setIsPaused(true);
            toast.info(t('migration.v2_paused', 'Đã tạm dừng'));
        } catch (e: any) {
            toast.error(String(e));
        }
    }, [t]);

    const handleResume = useCallback(async () => {
        try {
            await invoke('cmd_backup_v2_resume');
            setIsPaused(false);
            toast.info(t('migration.v2_resumed', 'Đã tiếp tục'));
        } catch (e: any) {
            toast.error(String(e));
        }
    }, [t]);

    const handleStop = useCallback(async () => {
        try {
            await invoke('cmd_backup_v2_stop');
            setIsRunning(false);
            setIsPaused(false);
            toast.info(t('migration.v2_stopped', 'Đã dừng backup'));
        } catch (e: any) {
            toast.error(String(e));
        }
    }, [t]);

    // ---- Open folders ----
    const openLocalBackup = useCallback(() => {
        if (localBackupDir) {
            void invoke('cmd_open_in_finder', { path: localBackupDir }).catch(() => {});
        }
    }, [localBackupDir]);

    // ---- Connect gate ----
    if (!msAccount) {
        return (
            <div className="flex-1 flex items-center justify-center bg-slate-950 p-6">
                <div className="text-center max-w-md space-y-4">
                    <Cloud className="w-12 h-12 text-slate-500 mx-auto" />
                    <h2 className="text-xl font-bold text-white">
                        {t('migration.connect_title', 'Kết nối OneDrive')}
                    </h2>
                    <p className="text-slate-400 text-sm">
                        {t('migration.connect_desc', 'Kết nối tài khoản Microsoft để bắt đầu backup dữ liệu từ OneDrive.')}
                    </p>
                    <button
                        onClick={() => { void connectMicrosoft(); }}
                        disabled={loading}
                        className="px-6 py-3 bg-blue-600 hover:bg-blue-700 text-white rounded-lg font-semibold transition-colors disabled:opacity-50"
                    >
                        {loading ? t('common.connecting', 'Đang kết nối...') : t('migration.connect_onedrive', 'Kết nối OneDrive')}
                    </button>
                </div>
            </div>
        );
    }

    // ---- Progress view ----
    if (isRunning || (status && status.state !== 'pending')) {
        const s = status;
        const completed = s ? (s.stats.completed_telegram + s.stats.completed_local + s.stats.skipped_duplicates) : 0;
        const total = s?.stats.total_items || preflightResult?.total_files || 0;
        const pct = total > 0 ? Math.round((completed / total) * 100) : 0;

        return (
            <div className="flex-1 h-full overflow-y-auto custom-scrollbar bg-slate-950 p-6 space-y-6 text-slate-100">
                {/* Header */}
                <div className="flex items-center justify-between pb-4 border-b border-slate-800">
                    <div className="flex items-center gap-3">
                        <div className="p-3 bg-gradient-to-br from-blue-600 to-indigo-600 rounded-xl">
                            <Cloud className="w-6 h-6 text-white" />
                        </div>
                        <div>
                            <h1 className="text-xl font-bold text-white">OneDrive Backup</h1>
                            <p className="text-xs text-slate-400">
                                {s?.state === 'completed' ? 'Hoàn tất' : s?.state === 'running' ? 'Đang chạy' : s?.state || '...'}
                                {isPaused ? ' (Đã tạm dừng)' : ''}
                            </p>
                        </div>
                    </div>
                    {/* Account */}
                    <div className="text-xs text-slate-400">
                        {msAccount.account_name} ({msAccount.account_email})
                        <button onClick={() => { void switchMicrosoftAccount(); }} className="ml-2 text-blue-400 hover:underline">
                            {t('common.switch', 'Đổi')}
                        </button>
                    </div>
                </div>

                {/* Progress bar */}
                <div className="space-y-2">
                    <div className="flex justify-between text-sm">
                        <span className="text-slate-300">{completed} / {total} items</span>
                        <span className="text-slate-400">{pct}%</span>
                    </div>
                    <div className="w-full bg-slate-800 rounded-full h-3 overflow-hidden">
                        <div
                            className="h-full bg-gradient-to-r from-blue-500 to-indigo-500 rounded-full transition-all duration-500"
                            style={{ width: `${pct}%` }}
                        />
                    </div>
                </div>

                {/* Stats grid */}
                <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
                    <StatCard label="Telegram" value={s?.stats.completed_telegram || 0} color="text-green-400" />
                    <StatCard label="Local" value={s?.stats.completed_local || 0} color="text-blue-400" />
                    <StatCard label="Đã skip" value={s?.stats.skipped_duplicates || 0} color="text-yellow-400" />
                    <StatCard label="Lỗi" value={s?.stats.failed_items || 0} color="text-red-400" />
                    <StatCard label="Cần đối chiếu" value={s?.stats.reconciliation_required || 0} color="text-orange-400" />
                    <StatCard label="Chờ quota" value={s?.stats.waiting_for_quota || 0} color="text-purple-400" />
                    <StatCard label="Quota đã dùng" value={formatBytes(s?.quota_used || 0)} color="text-slate-300" />
                    <StatCard label="Quota còn lại" value={formatBytes(s?.quota_remaining || 0)} color="text-slate-300" />
                </div>

                {/* Flood wait */}
                {s?.flood_wait_secs && s.flood_wait_secs > 0 && (
                    <div className="flex items-center gap-2 p-3 bg-amber-900/30 border border-amber-800 rounded-lg text-amber-300 text-sm">
                        <AlertTriangle className="w-4 h-4" />
                        Flood wait: {s.flood_wait_secs}s còn lại
                    </div>
                )}

                {/* Safety notice */}
                <div className="flex items-center gap-2 p-3 bg-emerald-900/20 border border-emerald-800/50 rounded-lg text-emerald-300 text-xs">
                    <Shield className="w-4 h-4 flex-shrink-0" />
                    OneDrive source sẽ không bị xóa, di chuyển hoặc chỉnh sửa.
                </div>

                {/* Controls */}
                <div className="flex flex-wrap gap-2">
                    {isRunning && !isPaused && (
                        <button onClick={handlePause} className="inline-flex items-center gap-2 px-4 py-2 bg-amber-600 hover:bg-amber-700 text-white rounded-lg text-sm font-semibold">
                            <Pause className="w-4 h-4" /> Tạm dừng
                        </button>
                    )}
                    {isRunning && isPaused && (
                        <button onClick={handleResume} className="inline-flex items-center gap-2 px-4 py-2 bg-emerald-600 hover:bg-emerald-700 text-white rounded-lg text-sm font-semibold">
                            <Play className="w-4 h-4" /> Tiếp tục
                        </button>
                    )}
                    {isRunning && (
                        <button onClick={handleStop} className="inline-flex items-center gap-2 px-4 py-2 bg-red-600 hover:bg-red-700 text-white rounded-lg text-sm font-semibold">
                            <Square className="w-4 h-4" /> Dừng
                        </button>
                    )}
                    {!isRunning && (
                        <>
                            <button onClick={openLocalBackup} className="inline-flex items-center gap-2 px-4 py-2 bg-slate-800 hover:bg-slate-700 text-slate-200 rounded-lg text-sm">
                                <FolderOpen className="w-4 h-4" /> Mở backup
                            </button>
                            <button
                                onClick={() => { void invoke('cmd_backup_v2_retry_manifest', { jobId }).catch(() => {}); }}
                                className="inline-flex items-center gap-2 px-4 py-2 bg-slate-800 hover:bg-slate-700 text-slate-200 rounded-lg text-sm"
                            >
                                <FileJson className="w-4 h-4" /> Retry manifest
                            </button>
                            <button
                                onClick={() => { setPreflightResult(null); setJobId(null); setIsRunning(false); setStatus(null); }}
                                className="inline-flex items-center gap-2 px-4 py-2 bg-slate-800 hover:bg-slate-700 text-slate-200 rounded-lg text-sm"
                            >
                                <RotateCcw className="w-4 h-4" /> Backup mới
                            </button>
                        </>
                    )}
                </div>
            </div>
        );
    }

    // ---- Configuration view (default) ----
    return (
        <div className="flex-1 h-full overflow-y-auto custom-scrollbar bg-slate-950 p-6 space-y-6 text-slate-100">
            {/* Header */}
            <div className="flex flex-wrap items-center justify-between gap-4 pb-4 border-b border-slate-800">
                <div className="flex items-center gap-3">
                    <div className="p-3 bg-gradient-to-br from-blue-600 to-indigo-600 rounded-xl shadow-lg shadow-blue-500/10">
                        <Cloud className="w-6 h-6 text-white" />
                    </div>
                    <div>
                        <h1 className="text-xl font-bold tracking-tight text-white">OneDrive Backup</h1>
                        <p className="text-xs text-slate-400">
                            Sao lưu dữ liệu từ OneDrive sang Telegram Drive — an toàn, không xóa nguồn
                        </p>
                    </div>
                </div>
                <div className="text-xs text-slate-400">
                    {msAccount.account_name}
                    <button onClick={() => { void switchMicrosoftAccount(); }} className="ml-2 text-blue-400 hover:underline">
                        {t('common.switch', 'Đổi')}
                    </button>
                </div>
            </div>

            {/* E1: Configuration */}
            <section className="space-y-4">
                <h2 className="text-lg font-semibold text-white flex items-center gap-2">
                    <span className="w-6 h-6 rounded-full bg-blue-600 text-white text-xs flex items-center justify-center">1</span>
                    Cấu hình
                </h2>

                {/* OneDrive folder */}
                <div className="space-y-2">
                    <label className="text-sm text-slate-400">Thư mục nguồn OneDrive</label>
                    <select
                        value={sourceFolderId}
                        onChange={(e) => {
                            const opt = e.target.selectedOptions[0];
                            setSourceFolderId(e.target.value);
                            setSourceFolderPath(opt?.dataset.path || '/');
                        }}
                        className="w-full p-2.5 bg-slate-900 border border-slate-700 rounded-lg text-white text-sm focus:border-blue-500 outline-none"
                    >
                        <option value="root" data-path="/">📁 Tất cả (Root)</option>
                        {folders.map(f => (
                            <option key={f.id} value={f.id} data-path={f.path}>{f.name}</option>
                        ))}
                    </select>
                </div>

                {/* Local backup dir — hardcoded */}
                <div className="space-y-2">
                    <label className="text-sm text-slate-400">Thư mục backup local (file không gửi Telegram)</label>
                    <div className="w-full p-2.5 bg-slate-900 border border-slate-700 rounded-lg text-slate-400 text-sm font-mono">
                        /Volumes/DATASTORE/Temo
                    </div>
                </div>

                {/* Workspace dir — hardcoded */}
                <div className="space-y-2">
                    <label className="text-sm text-slate-400">Thư mục workspace (tạm, download & xử lý)</label>
                    <div className="w-full p-2.5 bg-slate-900 border border-slate-700 rounded-lg text-slate-400 text-sm font-mono">
                        /Volumes/DATASTORE/Temo/_workspace
                    </div>
                </div>

                {/* Safety notice */}
                <div className="flex items-center gap-2 p-3 bg-emerald-900/20 border border-emerald-800/50 rounded-lg text-emerald-300 text-xs">
                    <Shield className="w-4 h-4 flex-shrink-0" />
                    OneDrive source sẽ không bị xóa, di chuyển hoặc chỉnh sửa.
                </div>
            </section>

            {/* E2: Preflight */}
            <section className="space-y-4">
                <h2 className="text-lg font-semibold text-white flex items-center gap-2">
                    <span className="w-6 h-6 rounded-full bg-indigo-600 text-white text-xs flex items-center justify-center">2</span>
                    Preflight
                </h2>

                <button
                    onClick={handlePreflight}
                    disabled={preflightRunning || !sourceFolderId}
                    className="px-5 py-2.5 bg-indigo-600 hover:bg-indigo-700 disabled:opacity-40 text-white rounded-lg text-sm font-semibold transition-colors"
                >
                    {preflightRunning ? 'Đang kiểm tra...' : 'Chạy Preflight'}
                </button>

                {preflightResult && (
                    <div className="grid grid-cols-2 md:grid-cols-3 gap-3 p-4 bg-slate-900 rounded-lg border border-slate-800">
                        <PreflightStat label="Tổng files" value={String(preflightResult.total_files)} />
                        <PreflightStat label="Tổng dung lượng" value={formatBytes(preflightResult.total_bytes)} />
                        <PreflightStat label="Video" value={`${preflightResult.video_count} (${formatBytes(preflightResult.video_bytes)})`} />
                        <PreflightStat label="Ảnh" value={`${preflightResult.image_count} (${formatBytes(preflightResult.image_bytes)})`} />
                        <PreflightStat label="File khác (local)" value={`${preflightResult.other_count} (${formatBytes(preflightResult.other_bytes)})`} />
                        <PreflightStat label="Telegram bytes" value={formatBytes(preflightResult.telegram_bytes)} />
                        <PreflightStat label="Quota còn lại" value={formatBytes(preflightResult.quota_remaining)} />
                        <PreflightStat label="Disk available" value={formatBytes(preflightResult.disk_available)} />
                    </div>
                )}

                {preflightResult?.warnings && preflightResult.warnings.length > 0 && (
                    <div className="space-y-1">
                        {preflightResult.warnings.map((w, i) => (
                            <div key={i} className="text-xs text-amber-400 flex items-center gap-1">
                                <AlertTriangle className="w-3 h-3" /> {w}
                            </div>
                        ))}
                    </div>
                )}
            </section>

            {/* E3: Start button */}
            {preflightResult && (
                <section className="pt-4 border-t border-slate-800">
                    <button
                        onClick={handleStart}
                        className="px-6 py-3 bg-emerald-600 hover:bg-emerald-700 text-white rounded-lg font-semibold inline-flex items-center gap-2 transition-colors"
                    >
                        <Play className="w-5 h-5" /> Bắt đầu Backup
                    </button>
                </section>
            )}
        </div>
    );
};

// ---- Sub-components ----

const StatCard: React.FC<{ label: string; value: number | string; color: string }> = ({ label, value, color }) => (
    <div className="p-3 bg-slate-900 rounded-lg border border-slate-800">
        <div className="text-xs text-slate-500">{label}</div>
        <div className={`text-lg font-bold ${color}`}>{value}</div>
    </div>
);

const PreflightStat: React.FC<{ label: string; value: string }> = ({ label, value }) => (
    <div>
        <div className="text-xs text-slate-500">{label}</div>
        <div className="text-sm font-semibold text-white">{value}</div>
    </div>
);
