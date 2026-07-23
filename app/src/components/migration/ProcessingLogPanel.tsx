import React, { useEffect, useRef } from 'react';
import { Terminal, Trash2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { ProcessingLogEntry } from '../../types';

interface ProcessingLogPanelProps {
    entries: ProcessingLogEntry[];
    onClear: () => void;
}

const levelColor: Record<ProcessingLogEntry['level'], string> = {
    info: 'text-sky-300',
    success: 'text-emerald-300',
    warning: 'text-amber-300',
    error: 'text-rose-300',
};

const categoryColor: Record<ProcessingLogEntry['category'], string> = {
    scan: 'bg-blue-500/10 text-blue-300 border-blue-500/20',
    download: 'bg-cyan-500/10 text-cyan-300 border-cyan-500/20',
    upload: 'bg-emerald-500/10 text-emerald-300 border-emerald-500/20',
    job: 'bg-purple-500/10 text-purple-300 border-purple-500/20',
    system: 'bg-slate-700/40 text-slate-300 border-slate-600/40',
};

export const ProcessingLogPanel: React.FC<ProcessingLogPanelProps> = ({ entries, onClear }) => {
    const { t } = useTranslation();
    const scrollRef = useRef<HTMLDivElement>(null);

    useEffect(() => {
        const element = scrollRef.current;
        if (element) {
            element.scrollTop = element.scrollHeight;
        }
    }, [entries]);

    return (
        <section className="bg-slate-900 border border-slate-800 rounded-2xl overflow-hidden">
            <div className="flex items-center justify-between gap-3 px-5 py-4 border-b border-slate-800">
                <div>
                    <h3 className="text-sm font-bold text-white flex items-center gap-2">
                        <Terminal className="w-4 h-4 text-cyan-400" />
                        {t('migration.processing_log_title', 'Log xử lý chi tiết')}
                    </h3>
                    <p className="text-xs text-slate-500 mt-1">
                        {t('migration.processing_log_description', 'Theo dõi từng bước scan, download, upload và trạng thái job theo thời gian thực.')}
                    </p>
                </div>
                <button
                    onClick={onClear}
                    disabled={entries.length === 0}
                    className="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-lg border border-slate-700 text-slate-400 hover:text-white hover:bg-slate-800 disabled:opacity-40 disabled:cursor-not-allowed"
                >
                    <Trash2 className="w-3.5 h-3.5" />
                    {t('migration.clear_log', 'Xóa log')}
                </button>
            </div>

            <div
                ref={scrollRef}
                className="h-80 overflow-y-auto bg-slate-950/90 p-4 font-mono text-xs custom-scrollbar"
            >
                {entries.length === 0 ? (
                    <div className="h-full flex items-center justify-center text-slate-600">
                        {t('migration.no_processing_log', 'Chưa có sự kiện xử lý trong phiên này.')}
                    </div>
                ) : (
                    <div className="space-y-1.5">
                        {entries.map(entry => (
                            <div key={entry.id} className="grid grid-cols-[72px_76px_1fr] gap-2 items-start">
                                <span className="text-slate-600 tabular-nums">
                                    {new Date(entry.timestamp).toLocaleTimeString([], {
                                        hour: '2-digit',
                                        minute: '2-digit',
                                        second: '2-digit',
                                    })}
                                </span>
                                <span className={`inline-flex justify-center px-1.5 py-0.5 rounded border text-[10px] uppercase ${categoryColor[entry.category]}`}>
                                    {t(`migration.log_category_${entry.category}`, entry.category)}
                                </span>
                                <span className={levelColor[entry.level]}>
                                    {t(entry.message_key, entry.params)}
                                </span>
                            </div>
                        ))}
                    </div>
                )}
            </div>
        </section>
    );
};
