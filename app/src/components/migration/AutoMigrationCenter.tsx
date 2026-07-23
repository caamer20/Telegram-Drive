import React from 'react';
import { useTranslation } from 'react-i18next';

import {
    Cloud,
    CheckCircle2,
    HardDrive,
    Zap,
    Settings,
    ArrowDownToLine,
    ArrowUpFromLine,
    ShieldAlert,
    RefreshCw,
    Activity,
} from 'lucide-react';
import {
    MsAccountInfo,
    AutoMigrationProfile,
    DailyMigrationQuota,
    MigrationJobDetail,
    ItemProgressPayload,
} from '../../types';

interface AutoMigrationCenterProps {
    msAccount: MsAccountInfo | null;
    autoProfile: AutoMigrationProfile | null;
    dailyQuota: DailyMigrationQuota | null;
    currentJobDetail: MigrationJobDetail | null;
    itemProgress: ItemProgressPayload | null;
    loading: boolean;
    onToggleAuto: (enabled: boolean) => void;
    onConnectMs: () => void;
    onOpenSettings: () => void;
    onRefresh: () => void;
}

export const AutoMigrationCenter: React.FC<AutoMigrationCenterProps> = ({
    msAccount,
    autoProfile,
    dailyQuota,
    currentJobDetail,
    itemProgress,
    loading,
    onToggleAuto,
    onConnectMs,
    onOpenSettings,
    onRefresh,
}) => {
    const { t } = useTranslation();
    const isAutoEnabled = autoProfile ? autoProfile.enabled : true;

    // Calculate quota percentage
    const uploadedGB = dailyQuota ? (dailyQuota.uploaded_bytes / (1024 * 1024 * 1024)).toFixed(2) : '0.00';
    const limitGB = dailyQuota ? (dailyQuota.limit_bytes / (1024 * 1024 * 1024)).toFixed(0) : '250';
    const quotaPercent = dailyQuota && dailyQuota.limit_bytes > 0
        ? Math.min(100, (dailyQuota.uploaded_bytes / dailyQuota.limit_bytes) * 100)
        : 0;

    const isQuotaExceeded = quotaPercent >= 100;

    // Current job stats
    const stats = currentJobDetail?.stats;
    const isRunning = currentJobDetail?.job.state === 'running';

    return (
        <div className="space-y-6">
            {/* Top Control & Status Banner */}
            <div className="bg-slate-900 border border-slate-800 rounded-2xl p-6 shadow-xl relative overflow-hidden">
                <div className="absolute top-0 right-0 w-96 h-96 bg-blue-500/5 rounded-full blur-3xl pointer-events-none" />

                <div className="flex flex-col md:flex-row items-start md:items-center justify-between gap-6 relative z-10">
                    <div className="flex items-center gap-4">
                        <div className="w-14 h-14 rounded-2xl bg-gradient-to-br from-blue-600 to-indigo-700 flex items-center justify-center shadow-lg shadow-blue-500/20 shrink-0">
                            <Zap className="w-7 h-7 text-white animate-pulse" />
                        </div>
                        <div>
                            <div className="flex items-center gap-2">
                                <h2 className="text-xl font-bold text-white tracking-tight">
                                    {t('migration.auto_title', 'Smart Auto-Migration Center')}
                                </h2>
                                <span className="inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-semibold bg-blue-500/10 text-blue-400 border border-blue-500/20">
                                    Zero-Click Sync
                                </span>
                            </div>
                            <p className="text-sm text-slate-400 mt-1">
                                {t('migration.auto_subtitle', 'Tự động quét và đồng bộ dữ liệu từ OneDrive sang Telegram Drive ngầm 100%')}
                            </p>
                        </div>
                    </div>

                    {/* Master Switch */}
                    <div className="flex items-center gap-4 bg-slate-950/80 p-3 rounded-2xl border border-slate-800 shrink-0">
                        <span className="text-sm font-medium text-slate-300">
                            {isAutoEnabled
                                ? t('migration.auto_status_on', 'Tự động Migrate: BẬT')
                                : t('migration.auto_status_off', 'Tự động Migrate: TẮT')}
                        </span>
                        <button
                            onClick={() => onToggleAuto(!isAutoEnabled)}
                            disabled={loading || !msAccount}
                            className={`relative inline-flex h-7 w-14 items-center rounded-full transition-colors focus:outline-none ${
                                isAutoEnabled ? 'bg-blue-600' : 'bg-slate-700'
                            } ${(!msAccount || loading) ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'}`}
                        >
                            <span
                                className={`inline-block h-5 w-5 transform rounded-full bg-white transition-transform ${
                                    isAutoEnabled ? 'translate-x-8' : 'translate-x-1'
                                }`}
                            />
                        </button>
                    </div>
                </div>

                {/* Account & Settings Row */}
                <div className="grid grid-cols-1 md:grid-cols-3 gap-4 mt-6 pt-6 border-t border-slate-800/80">
                    {/* Account Card */}
                    <div className="bg-slate-950/60 rounded-xl p-4 border border-slate-800 flex items-center justify-between">
                        <div className="flex items-center gap-3">
                            <Cloud className="w-5 h-5 text-sky-400 shrink-0" />
                            <div className="truncate">
                                <p className="text-xs text-slate-400 font-medium">{t('migration.ms_account', 'OneDrive Account')}</p>
                                <p className="text-sm font-semibold text-slate-200 truncate">
                                    {msAccount ? msAccount.account_name : t('migration.not_connected', 'Chưa kết nối')}
                                </p>
                            </div>
                        </div>
                        {msAccount ? (
                            <span className="flex h-2.5 w-2.5 relative">
                                <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75"></span>
                                <span className="relative inline-flex rounded-full h-2.5 w-2.5 bg-emerald-500"></span>
                            </span>
                        ) : (
                            <button
                                onClick={onConnectMs}
                                className="px-3 py-1 bg-blue-600 hover:bg-blue-500 text-white rounded-lg text-xs font-semibold transition-colors"
                            >
                                Connect
                            </button>
                        )}
                    </div>

                    {/* Daily Quota Card */}
                    <div className="bg-slate-950/60 rounded-xl p-4 border border-slate-800">
                        <div className="flex items-center justify-between mb-2">
                            <div className="flex items-center gap-2">
                                <HardDrive className="w-4 h-4 text-purple-400" />
                                <span className="text-xs font-medium text-slate-300">
                                    {t('migration.daily_quota', 'Quota hôm nay (Giới hạn 250GB)')}
                                </span>
                            </div>
                            <span className="text-xs font-bold text-purple-400">
                                {uploadedGB} / {limitGB} GB
                            </span>
                        </div>
                        <div className="w-full bg-slate-800 rounded-full h-2 overflow-hidden">
                            <div
                                className={`h-full transition-all duration-500 ${
                                    isQuotaExceeded ? 'bg-rose-500' : quotaPercent > 80 ? 'bg-amber-500' : 'bg-purple-500'
                                }`}
                                style={{ width: `${quotaPercent}%` }}
                            />
                        </div>
                        {isQuotaExceeded && (
                            <p className="text-[11px] text-amber-400 mt-1.5 flex items-center gap-1">
                                <ShieldAlert className="w-3.5 h-3.5 shrink-0" />
                                Đã đạt giới hạn an toàn 250GB/ngày. Tự động dừng để bảo vệ nick Telegram.
                            </p>
                        )}
                    </div>

                    {/* Quick Settings & Controls */}
                    <div className="bg-slate-950/60 rounded-xl p-4 border border-slate-800 flex items-center justify-between">
                        <div className="flex items-center gap-2 text-xs text-slate-400">
                            <Activity className="w-4 h-4 text-emerald-400" />
                            <span>
                                {isRunning
                                    ? t('migration.status_running', 'Đang tự động đồng bộ...')
                                    : t('migration.status_idle', 'Hệ thống ở trạng thái chờ')}
                            </span>
                        </div>
                        <div className="flex items-center gap-2">
                            <button
                                onClick={onRefresh}
                                className="p-2 hover:bg-slate-800 text-slate-400 hover:text-white rounded-lg transition-colors"
                                title="Refresh"
                            >
                                <RefreshCw className="w-4 h-4" />
                            </button>
                            <button
                                onClick={onOpenSettings}
                                className="p-2 hover:bg-slate-800 text-slate-400 hover:text-white rounded-lg transition-colors"
                                title="Advanced Settings"
                            >
                                <Settings className="w-4 h-4" />
                            </button>
                        </div>
                    </div>
                </div>
            </div>

            {/* Real-time Progress Bar Card */}
            {itemProgress && (
                <div className="bg-gradient-to-br from-slate-900 to-slate-950 border border-blue-500/30 rounded-2xl p-6 shadow-2xl space-y-4">
                    <div className="flex items-center justify-between">
                        <div className="flex items-center gap-3">
                            <div className="p-2 bg-blue-500/10 text-blue-400 rounded-xl border border-blue-500/20">
                                <RefreshCw className="w-5 h-5 animate-spin" />
                            </div>
                            <div>
                                <p className="text-xs font-semibold text-blue-400 uppercase tracking-wider">
                                    {t('migration.processing_file', 'Đang xử lý file')}
                                </p>
                                <h3 className="text-base font-bold text-white truncate max-w-md">
                                    {itemProgress.item_name}
                                </h3>
                            </div>
                        </div>
                        <span className="text-xs font-medium px-2.5 py-1 bg-slate-800 text-slate-300 rounded-lg">
                            File ID: #{itemProgress.item_id}
                        </span>
                    </div>

                    {(() => {
                        const downloadPercent = itemProgress.phase === 'downloading' ? itemProgress.percent : 100;
                        const uploadPercent = itemProgress.phase === 'uploading' ? itemProgress.percent : 0;
                        return (
                            <div className="grid grid-cols-1 md:grid-cols-2 gap-4 pt-2">
                                {/* Download Progress */}
                                <div className="bg-slate-950/80 p-4 rounded-xl border border-slate-800">
                                    <div className="flex items-center justify-between text-xs mb-2">
                                        <span className="text-slate-400 flex items-center gap-1.5 font-medium">
                                            <ArrowDownToLine className="w-4 h-4 text-sky-400" />
                                            Download (OneDrive → Local)
                                        </span>
                                        <span className="font-bold text-sky-400">
                                            {downloadPercent}%
                                        </span>
                                    </div>
                                    <div className="w-full bg-slate-800 rounded-full h-2.5 overflow-hidden">
                                        <div
                                            className="bg-gradient-to-r from-sky-500 to-blue-500 h-full transition-all duration-300 rounded-full"
                                            style={{ width: `${downloadPercent}%` }}
                                        />
                                    </div>
                                </div>

                                {/* Upload Progress */}
                                <div className="bg-slate-950/80 p-4 rounded-xl border border-slate-800">
                                    <div className="flex items-center justify-between text-xs mb-2">
                                        <span className="text-slate-400 flex items-center gap-1.5 font-medium">
                                            <ArrowUpFromLine className="w-4 h-4 text-emerald-400" />
                                            Upload (Local → Telegram)
                                        </span>
                                        <span className="font-bold text-emerald-400">
                                            {uploadPercent}%
                                        </span>
                                    </div>
                                    <div className="w-full bg-slate-800 rounded-full h-2.5 overflow-hidden">
                                        <div
                                            className="bg-gradient-to-r from-emerald-500 to-teal-400 h-full transition-all duration-300 rounded-full"
                                            style={{ width: `${uploadPercent}%` }}
                                        />
                                    </div>
                                </div>
                            </div>
                        );
                    })()}

                </div>
            )}

            {/* Live Activity & Completed Stats */}
            <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
                {/* Stats Summary */}
                <div className="bg-slate-900 border border-slate-800 rounded-2xl p-5 space-y-4">
                    <h3 className="text-sm font-bold text-white flex items-center gap-2">
                        <Activity className="w-4 h-4 text-blue-400" />
                        {t('migration.stats_overview', 'Thống kê Tiến độ')}
                    </h3>
                    <div className="space-y-3">
                        <div className="flex justify-between items-center text-xs p-2.5 bg-slate-950 rounded-xl">
                            <span className="text-slate-400">Đã hoàn thành</span>
                            <span className="font-bold text-emerald-400">
                                {stats ? `${stats.completed_files} files` : '0 files'}
                            </span>
                        </div>
                        <div className="flex justify-between items-center text-xs p-2.5 bg-slate-950 rounded-xl">
                            <span className="text-slate-400">Bỏ qua trùng lặp</span>
                            <span className="font-bold text-sky-400">
                                {stats ? `${stats.skipped_duplicates} files` : '0 files'}
                            </span>
                        </div>
                        <div className="flex justify-between items-center text-xs p-2.5 bg-slate-950 rounded-xl">
                            <span className="text-slate-400">Đang chờ xử lý</span>
                            <span className="font-bold text-amber-400">
                                {stats ? `${stats.pending_files} files` : '0 files'}
                            </span>
                        </div>
                        <div className="flex justify-between items-center text-xs p-2.5 bg-slate-950 rounded-xl">
                            <span className="text-slate-400">Thất bại / Lỗi</span>
                            <span className="font-bold text-rose-400">
                                {stats ? `${stats.failed_files} files` : '0 files'}
                            </span>
                        </div>
                    </div>
                </div>

                {/* Live File Stream */}
                <div className="md:col-span-2 bg-slate-900 border border-slate-800 rounded-2xl p-5 space-y-4">
                    <div className="flex items-center justify-between">
                        <h3 className="text-sm font-bold text-white flex items-center gap-2">
                            <CheckCircle2 className="w-4 h-4 text-emerald-400" />
                            {t('migration.recent_activity', 'Nhật ký Hoạt động Tự động')}
                        </h3>
                        <span className="text-xs text-slate-500 font-medium">Real-time Stream</span>
                    </div>

                    <div className="space-y-2 max-h-64 overflow-y-auto pr-1 custom-scrollbar">
                        {currentJobDetail && currentJobDetail.files.length > 0 ? (
                            currentJobDetail.files
                                .slice(-10)
                                .reverse()
                                .map((file) => (
                                    <div
                                        key={file.id}
                                        className="flex items-center justify-between p-3 bg-slate-950/70 border border-slate-800/80 rounded-xl text-xs"
                                    >
                                        <div className="flex items-center gap-3 truncate">
                                            {file.state === 'completed' && <CheckCircle2 className="w-4 h-4 text-emerald-400 shrink-0" />}
                                            {file.state === 'skipped_duplicate' && <Zap className="w-4 h-4 text-sky-400 shrink-0" />}
                                            {file.state === 'failed' && <ShieldAlert className="w-4 h-4 text-rose-400 shrink-0" />}
                                            {file.state === 'downloading' && <ArrowDownToLine className="w-4 h-4 text-blue-400 shrink-0 animate-bounce" />}
                                            {file.state === 'uploading' && <ArrowUpFromLine className="w-4 h-4 text-emerald-400 shrink-0 animate-bounce" />}
                                            <span className="text-slate-200 font-medium truncate">{file.name}</span>
                                        </div>
                                        <span
                                            className={`px-2 py-0.5 rounded-full text-[10px] font-semibold shrink-0 ${
                                                file.state === 'completed'
                                                    ? 'bg-emerald-500/10 text-emerald-400 border border-emerald-500/20'
                                                    : file.state === 'skipped_duplicate'
                                                    ? 'bg-sky-500/10 text-sky-400 border border-sky-500/20'
                                                    : file.state === 'failed'
                                                    ? 'bg-rose-500/10 text-rose-400 border border-rose-500/20'
                                                    : 'bg-blue-500/10 text-blue-400 border border-blue-500/20'
                                            }`}
                                        >
                                            {file.state}
                                        </span>
                                    </div>
                                ))
                        ) : (
                            <div className="py-8 text-center text-xs text-slate-500">
                                Chưa có nhật ký hoạt động gần đây.
                            </div>
                        )}
                    </div>
                </div>
            </div>
        </div>
    );
};
