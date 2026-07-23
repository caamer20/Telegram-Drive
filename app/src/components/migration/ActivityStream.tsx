import React from 'react';
import {
    ArrowDownToLine,
    ArrowUpFromLine,
    CheckCircle2,
    ShieldAlert,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { MigrationActivity } from '../../types';

interface ActivityStreamProps {
    entries: MigrationActivity[];
}

export const ActivityStream: React.FC<ActivityStreamProps> = ({ entries }) => {
    const { t } = useTranslation();
    if (entries.length === 0) {
        return (
            <div className="py-8 text-center text-xs text-slate-500">
                {t('migration.no_activity')}
            </div>
        );
    }

    return (
        <div className="space-y-2 max-h-64 overflow-y-auto pr-1 custom-scrollbar">
            {entries.slice(0, 50).map(entry => (
                <div
                    key={entry.id}
                    className="flex items-center justify-between p-3 bg-slate-950/70 border border-slate-800/80 rounded-xl text-xs"
                >
                    <div className="flex items-center gap-3 truncate">
                        {entry.status === 'completed' && (
                            <CheckCircle2 className="w-4 h-4 text-emerald-400 shrink-0" />
                        )}
                        {entry.status === 'failed' && (
                            <ShieldAlert className="w-4 h-4 text-rose-400 shrink-0" />
                        )}
                        {entry.phase === 'downloading' && (
                            <ArrowDownToLine className="w-4 h-4 text-blue-400 shrink-0" />
                        )}
                        {entry.phase === 'uploading' && (
                            <ArrowUpFromLine className="w-4 h-4 text-emerald-400 shrink-0" />
                        )}
                        <div className="min-w-0">
                            <span className="text-slate-200 font-medium truncate block">
                                {entry.item_name || entry.message || t('migration.snapshot_activity')}
                            </span>
                            <span className="text-slate-500">
                                {new Date(entry.created_at * 1000).toLocaleString()}
                            </span>
                        </div>
                    </div>
                    <span
                        className={`px-2 py-0.5 rounded-full text-[10px] font-semibold shrink-0 ${
                            entry.status === 'completed'
                                ? 'bg-emerald-500/10 text-emerald-400 border border-emerald-500/20'
                                : entry.status === 'failed'
                                    ? 'bg-rose-500/10 text-rose-400 border border-rose-500/20'
                                    : 'bg-blue-500/10 text-blue-400 border border-blue-500/20'
                        }`}
                    >
                        {entry.phase}: {entry.status}
                    </span>
                </div>
            ))}
        </div>
    );
};
