import React from 'react';
import { useTranslation } from 'react-i18next';
import { MigrationItem } from '../../types';
import { RotateCw, CheckCircle2, AlertCircle, SkipForward, Clock, Download, Upload } from 'lucide-react';

interface FileTableProps {
    files: MigrationItem[];
    onRetryItem: (itemId: number) => void;
}

export const FileTable: React.FC<FileTableProps> = ({ files, onRetryItem }) => {
    const { t } = useTranslation();

    const formatBytes = (bytes: number) => {
        if (bytes === 0) return '0 B';
        const k = 1024;
        const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
        const i = Math.floor(Math.log(bytes) / Math.log(k));
        return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
    };

    const renderStatusBadge = (item: MigrationItem) => {
        switch (item.pipeline_stage) {
            case 'completed':
                return (
                    <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-xs font-medium bg-emerald-500/10 text-emerald-400 border border-emerald-500/20">
                        <CheckCircle2 className="w-3.5 h-3.5" />
                        {t('migration.status_completed', 'Completed')}
                    </span>
                );
            case 'skipped_duplicate':
                return (
                    <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-xs font-medium bg-amber-500/10 text-amber-400 border border-amber-500/20">
                        <SkipForward className="w-3.5 h-3.5" />
                        {t('migration.status_skipped', 'Skipped (Duplicate)')}
                    </span>
                );
            case 'downloading':
                return (
                    <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-xs font-medium bg-blue-500/10 text-blue-400 border border-blue-500/20 animate-pulse">
                        <Download className="w-3.5 h-3.5" />
                        {t('migration.status_downloading', 'Downloading')}
                    </span>
                );
            case 'uploading':
                return (
                    <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-xs font-medium bg-indigo-500/10 text-indigo-400 border border-indigo-500/20 animate-pulse">
                        <Upload className="w-3.5 h-3.5" />
                        {t('migration.status_uploading', 'Uploading')}
                    </span>
                );
            case 'failed':
                return (
                    <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-xs font-medium bg-rose-500/10 text-rose-400 border border-rose-500/20">
                        <AlertCircle className="w-3.5 h-3.5" />
                        {t('migration.status_failed', 'Failed')}
                    </span>
                );
            default:
                return (
                    <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-xs font-medium bg-slate-500/10 text-slate-400 border border-slate-500/20">
                        <Clock className="w-3.5 h-3.5" />
                        {t('migration.status_pending', 'Pending')}
                    </span>
                );
        }
    };

    if (files.length === 0) {
        return (
            <div className="p-8 text-center text-slate-400 bg-slate-900/50 rounded-xl border border-slate-800">
                <p>{t('migration.no_files_scanned', 'No files in snapshot. Select a folder and click Scan.')}</p>
            </div>
        );
    }

    return (
        <div className="bg-slate-900/50 rounded-xl border border-slate-800 overflow-hidden">
            <div className="max-h-[400px] overflow-y-auto custom-scrollbar">
                <table className="w-full text-left text-sm text-slate-300">
                    <thead className="text-xs uppercase bg-slate-950/80 text-slate-400 sticky top-0 backdrop-blur-md border-b border-slate-800">
                        <tr>
                            <th className="px-4 py-3 font-semibold">{t('migration.th_name', 'File Name')}</th>
                            <th className="px-4 py-3 font-semibold">{t('migration.th_path', 'Relative Path')}</th>
                            <th className="px-4 py-3 font-semibold">{t('migration.th_size', 'Size')}</th>
                            <th className="px-4 py-3 font-semibold">{t('migration.th_status', 'Status')}</th>
                            <th className="px-4 py-3 font-semibold text-right">{t('migration.th_actions', 'Actions')}</th>
                        </tr>
                    </thead>
                    <tbody className="divide-y divide-slate-800/50">
                        {files.map((item) => (
                            <tr key={item.id} className="hover:bg-slate-800/30 transition-colors">
                                <td className="px-4 py-3 font-medium text-slate-100 truncate max-w-[200px]" title={item.name}>
                                    {item.name}
                                </td>
                                <td className="px-4 py-3 text-slate-400 font-mono text-xs truncate max-w-[250px]" title={item.path}>
                                    {item.path}
                                </td>
                                <td className="px-4 py-3 whitespace-nowrap text-slate-400">
                                    {formatBytes(item.size)}
                                </td>
                                <td className="px-4 py-3 whitespace-nowrap">
                                    {renderStatusBadge(item)}
                                    {item.last_error && (
                                        <p className="text-xs text-rose-400 mt-1 max-w-[250px] truncate" title={item.last_error}>
                                            {item.last_error}
                                        </p>
                                    )}
                                </td>
                                <td className="px-4 py-3 whitespace-nowrap text-right">
                                    {item.pipeline_stage === 'failed' && (
                                        <button
                                            onClick={() => onRetryItem(item.id)}
                                            className="inline-flex items-center gap-1 px-2.5 py-1 rounded-md text-xs font-medium bg-slate-800 hover:bg-slate-700 text-slate-200 border border-slate-700 transition-colors"
                                            title={t('migration.retry', 'Retry')}
                                        >
                                            <RotateCw className="w-3 h-3" />
                                            {t('migration.retry', 'Retry')}
                                        </button>
                                    )}
                                </td>
                            </tr>
                        ))}
                    </tbody>
                </table>
            </div>
        </div>
    );
};
