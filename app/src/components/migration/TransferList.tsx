import React from 'react';
import { ArrowDownToLine, ArrowUpFromLine, Clapperboard } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { ItemProgressPayload, MigrationItem } from '../../types';

interface TransferListProps {
    phase: 'downloading' | 'processing' | 'uploading';
    item: MigrationItem | null;
    progress: ItemProgressPayload | null;
}

function formatBytes(bytes: number): string {
    if (bytes <= 0) return '0 B';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    const index = Math.min(units.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)));
    return `${(bytes / 1024 ** index).toFixed(2)} ${units[index]}`;
}

export const TransferList: React.FC<TransferListProps> = ({ phase, item, progress }) => {
    const { t } = useTranslation();
    const downloading = phase === 'downloading';
    const processing = phase === 'processing';
    const title = downloading
        ? t('migration.download_list')
        : processing
            ? t('migration.processing_list')
            : t('migration.upload_list');
    const icon = downloading
        ? <ArrowDownToLine className="w-5 h-5 text-sky-400" />
        : processing
            ? <Clapperboard className="w-5 h-5 text-violet-400" />
            : <ArrowUpFromLine className="w-5 h-5 text-emerald-400" />;
    const bar = downloading
        ? 'from-sky-500 to-blue-500'
        : processing
            ? 'from-violet-500 to-fuchsia-400'
            : 'from-emerald-500 to-teal-400';
    const progressMatches = !progress
        || progress.phase === phase
        || (processing && progress.phase === 'analyzing');

    return (
        <section className="bg-slate-900 border border-slate-800 rounded-2xl p-5">
            <h3 className="text-sm font-bold text-white flex items-center gap-2">
                {icon}
                {title}
            </h3>
            {item && progressMatches ? (
                <div className="mt-4 p-4 rounded-xl bg-slate-950 border border-slate-800">
                    <div className="flex items-center justify-between gap-4 text-xs">
                        <div className="min-w-0">
                            <p className="font-semibold text-slate-200 truncate">{item.name}</p>
                            <p className="text-slate-500 mt-1">{formatBytes(item.size_bytes)}</p>
                        </div>
                        <span className="font-bold text-slate-200">{progress?.percent ?? 0}%</span>
                    </div>
                    <div className="mt-3 h-2.5 rounded-full overflow-hidden bg-slate-800">
                        <div
                            className={`h-full bg-gradient-to-r ${bar} transition-all duration-300`}
                            style={{ width: `${progress?.percent ?? 0}%` }}
                        />
                    </div>
                </div>
            ) : (
                <div className="mt-4 py-8 text-center text-xs text-slate-500 rounded-xl border border-dashed border-slate-800">
                    {t('migration.no_active_transfer')}
                </div>
            )}
        </section>
    );
};
