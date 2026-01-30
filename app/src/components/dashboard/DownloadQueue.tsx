import { DownloadItem } from "../../types";
import { Download, Check, X, AlertCircle } from "lucide-react";

interface DownloadQueueProps {
    items: DownloadItem[];
    onClearFinished: () => void;
}

export function DownloadQueue({ items, onClearFinished }: DownloadQueueProps) {
    if (items.length === 0) return null;

    const activeCount = items.filter(i => i.status === 'pending' || i.status === 'downloading').length;
    const completedCount = items.filter(i => i.status === 'success').length;

    return (
        <div className="fixed bottom-4 left-4 w-80 bg-telegram-surface border border-telegram-border rounded-xl shadow-2xl overflow-hidden z-[100]">
            <div className="p-3 border-b border-telegram-border bg-telegram-hover flex justify-between items-center">
                <div className="flex items-center gap-2">
                    <Download className="w-4 h-4 text-telegram-secondary" />
                    <h4 className="text-sm font-medium text-telegram-text">Downloads</h4>
                    {activeCount > 0 && (
                        <span className="text-xs px-1.5 py-0.5 bg-telegram-secondary/20 text-telegram-secondary rounded-full">
                            {activeCount} active
                        </span>
                    )}
                </div>
                {completedCount > 0 && (
                    <button onClick={onClearFinished} className="text-xs text-telegram-primary hover:text-telegram-text transition-colors">
                        Clear Finished
                    </button>
                )}
            </div>
            <div className="max-h-60 overflow-y-auto p-2 space-y-2">
                {items.map(item => (
                    <div key={item.id} className="flex flex-col gap-1 p-2 bg-telegram-hover rounded">
                        <div className="flex items-center gap-3 text-sm">
                            <div className="flex-shrink-0">
                                {item.status === 'pending' && <div className="w-4 h-4 rounded-full bg-yellow-500/20 flex items-center justify-center"><div className="w-2 h-2 bg-yellow-500 rounded-full" /></div>}
                                {item.status === 'downloading' && <div className="w-4 h-4 rounded-full border-2 border-telegram-secondary border-t-transparent animate-spin" />}
                                {item.status === 'success' && <div className="w-4 h-4 rounded-full bg-green-500/20 flex items-center justify-center"><Check className="w-3 h-3 text-green-500" /></div>}
                                {item.status === 'error' && <div className="w-4 h-4 rounded-full bg-red-500/20 flex items-center justify-center"><X className="w-3 h-3 text-red-500" /></div>}
                            </div>
                            <div className="flex-1 truncate text-telegram-subtext" title={item.filename}>
                                {item.filename}
                            </div>
                        </div>
                        {item.status === 'downloading' && (
                            <div className="w-full bg-telegram-border h-1 mt-1 rounded-full overflow-hidden">
                                <div className="bg-telegram-secondary h-full w-full animate-progress-indeterminate"></div>
                            </div>
                        )}
                        {item.status === 'error' && item.error && (
                            <div className="flex items-center gap-1 text-xs text-red-400 mt-1">
                                <AlertCircle className="w-3 h-3" />
                                <span className="truncate">{item.error}</span>
                            </div>
                        )}
                    </div>
                ))}
            </div>
        </div>
    )
}
