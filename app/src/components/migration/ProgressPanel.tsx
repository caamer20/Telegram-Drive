import React from 'react';
import { useTranslation } from 'react-i18next';
import { MigrationJobDetail, ItemProgressPayload, CooldownPayload } from '../../types';
import { Play, XCircle, RotateCcw, AlertTriangle, CheckCircle2, Clock } from 'lucide-react';

interface ProgressPanelProps {
    detail: MigrationJobDetail;
    activeProgresses: Record<number, ItemProgressPayload>;
    cooldown: CooldownPayload | null;
    onStart: () => void;
    onStop: () => void;
    onRetryAllFailed: () => void;
}

export const ProgressPanel: React.FC<ProgressPanelProps> = ({
    detail,
    activeProgresses,
    cooldown,
    onStart,
    onStop,
    onRetryAllFailed,
}) => {
    const { t } = useTranslation();
    const { job, stats } = detail;

    const formatBytes = (bytes: number) => {
        if (bytes === 0) return '0 B';
        const k = 1024;
        const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
        const i = Math.floor(Math.log(bytes) / Math.log(k));
        return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
    };

    const totalCompleted = stats.completed_telegram + stats.completed_local;
    const overallPercent = stats.total_files > 0
        ? Math.min(100, Math.round((totalCompleted / stats.total_files) * 100))
        : 0;

    const isActive = job.state === 'running';

    return (
        <div className="bg-slate-900/60 rounded-xl border border-slate-800 p-5 space-y-5">
            {/* Top Bar: Status Badge & Control Buttons */}
            <div className="flex flex-wrap items-center justify-between gap-4">
                <div className="flex items-center gap-3">
                    <span className={`px-3 py-1 rounded-full text-xs font-semibold uppercase tracking-wider ${
                        job.state === 'running' ? 'bg-blue-500/20 text-blue-400 border border-blue-500/30 animate-pulse' :
                        job.state === 'completed' ? 'bg-emerald-500/20 text-emerald-400 border border-emerald-500/30' :
                        job.state === 'completed_with_errors' ? 'bg-amber-500/20 text-amber-400 border border-amber-500/30' :
                        job.state === 'waiting_for_quota' ? 'bg-purple-500/20 text-purple-400 border border-purple-500/30' :
                        job.state === 'stopped' ? 'bg-slate-500/20 text-slate-400 border border-slate-500/30' :
                        job.state === 'failed' ? 'bg-rose-500/20 text-rose-400 border border-rose-500/30' :
                        'bg-slate-800 text-slate-300 border border-slate-700'
                    }`}>
                        {t(`migration.job_state_${job.state}`, job.state.toUpperCase())}
                    </span>

                    {cooldown && cooldown.seconds_remaining > 0 && (
                        <div className="flex items-center gap-1.5 px-3 py-1 bg-amber-500/10 text-amber-400 border border-amber-500/20 rounded-full text-xs">
                            <AlertTriangle className="w-3.5 h-3.5" />
                            <span>{t('migration.cooldown_active', { seconds: cooldown.seconds_remaining })}</span>
                        </div>
                    )}
                </div>

                <div className="flex flex-wrap items-center gap-2">
                    {!isActive && job.state !== 'completed' && (
                        <button
                            onClick={onStart}
                            className="inline-flex items-center gap-2 px-4 py-2 bg-gradient-to-r from-blue-600 to-indigo-600 hover:from-blue-500 hover:to-indigo-500 text-white rounded-lg text-xs font-semibold shadow-lg transition-all"
                        >
                            <Play className="w-4 h-4 fill-current" />
                            {t('migration.btn_start', 'Start Migration')}
                        </button>
                    )}

                    {isActive && (
                        <button
                            onClick={onStop}
                            className="inline-flex items-center gap-2 px-3.5 py-2 bg-slate-800 hover:bg-rose-900/50 text-slate-300 hover:text-rose-300 border border-slate-700 rounded-lg text-xs font-medium transition-colors"
                        >
                            <XCircle className="w-4 h-4" />
                            {t('migration.btn_stop', 'Stop')}
                        </button>
                    )}

                    {stats.failed_files > 0 && !isActive && (
                        <button
                            onClick={onRetryAllFailed}
                            className="inline-flex items-center gap-2 px-3.5 py-2 bg-slate-800 hover:bg-slate-700 text-slate-200 border border-slate-700 rounded-lg text-xs font-medium transition-colors"
                        >
                            <RotateCcw className="w-4 h-4" />
                            {t('migration.btn_retry_all', 'Retry Failed')}
                        </button>
                    )}
                </div>
            </div>

            {/* Stats Summary Grid */}
            <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
                <div className="p-3 bg-slate-950/60 rounded-lg border border-slate-800/80">
                    <div className="flex items-center gap-2 text-slate-400 text-xs font-medium mb-1">
                        <Clock className="w-3.5 h-3.5 text-blue-400" />
                        {t('migration.stats_total', 'Total Files')}
                    </div>
                    <p className="text-lg font-bold text-slate-100">{stats.total_files}</p>
                    <p className="text-xs text-slate-500">{formatBytes(stats.total_bytes)}</p>
                </div>

                <div className="p-3 bg-slate-950/60 rounded-lg border border-slate-800/80">
                    <div className="flex items-center gap-2 text-slate-400 text-xs font-medium mb-1">
                        <CheckCircle2 className="w-3.5 h-3.5 text-emerald-400" />
                        {t('migration.stats_completed', 'Completed')}
                    </div>
                    <p className="text-lg font-bold text-emerald-400">{totalCompleted}</p>
                    <p className="text-xs text-slate-500">{formatBytes(stats.completed_bytes)}</p>
                </div>

                <div className="p-3 bg-slate-950/60 rounded-lg border border-slate-800/80">
                    <div className="flex items-center gap-2 text-slate-400 text-xs font-medium mb-1">
                        <Clock className="w-3.5 h-3.5 text-purple-400" />
                        {t('migration.stats_waiting', 'Waiting Quota')}
                    </div>
                    <p className="text-lg font-bold text-purple-400">{stats.waiting_files}</p>
                    <p className="text-xs text-slate-500">{t('migration.pending', 'Pending: {{count}}', { count: stats.pending_files })}</p>
                </div>

                <div className="p-3 bg-slate-950/60 rounded-lg border border-slate-800/80">
                    <div className="flex items-center gap-2 text-slate-400 text-xs font-medium mb-1">
                        <AlertTriangle className="w-3.5 h-3.5 text-rose-400" />
                        {t('migration.stats_failed', 'Failed')}
                    </div>
                    <p className="text-lg font-bold text-rose-400">{stats.failed_files}</p>
                    <p className="text-xs text-slate-500">{t('migration.folders', 'Folders: {{count}}', { count: stats.total_folders })}</p>
                </div>
            </div>

            {/* Overall Progress Bar */}
            <div className="space-y-1.5">
                <div className="flex justify-between text-xs font-medium text-slate-300">
                    <span>{t('migration.overall_progress', 'Overall Progress')}</span>
                    <span>{overallPercent}%</span>
                </div>
                <div className="w-full h-2.5 bg-slate-950 rounded-full overflow-hidden border border-slate-800">
                    <div
                        className="h-full bg-gradient-to-r from-blue-500 via-indigo-500 to-emerald-500 transition-all duration-300"
                        style={{ width: `${overallPercent}%` }}
                    />
                </div>
            </div>

            {/* Current Active Progresses */}
            {Object.values(activeProgresses).length > 0 && (
                <div className="space-y-3">
                    {Object.values(activeProgresses).map(progress => (
                        <div key={progress.item_id} className="p-4 bg-blue-950/20 border border-blue-900/40 rounded-xl space-y-2">
                            <div className="flex justify-between items-center text-xs">
                                <span className="font-semibold text-blue-300 truncate max-w-[70%]" title={progress.item_name}>
                                    {progress.phase === 'downloading' ? t('migration.phase_downloading', 'Downloading') :
                                     progress.phase === 'processing' ? t('migration.phase_processing', 'Processing') :
                                     t('migration.phase_uploading', 'Uploading')}: {progress.item_name}
                                </span>
                                <span className="text-blue-400 font-mono font-bold">{progress.percent}%</span>
                            </div>
                            <div className="w-full h-2 bg-slate-950 rounded-full overflow-hidden">
                                <div
                                    className="h-full bg-blue-500 transition-all duration-200"
                                    style={{ width: `${progress.percent}%` }}
                                />
                            </div>
                            <div className="flex justify-between text-[11px] text-slate-400 font-mono">
                                <span>{formatBytes(progress.bytes_done)} / {formatBytes(progress.bytes_total)}</span>
                            </div>
                        </div>
                    ))}
                </div>
            )}
        </div>
    );
};
