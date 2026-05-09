import { DownloadItem } from "../../types";
import { Download, Check, X, AlertCircle, RotateCcw } from "lucide-react";

function formatBytes(bytes: number): string {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`;
}

interface DownloadQueueProps {
    items: DownloadItem[];
    onClearFinished: () => void;
    onCancelAll: () => void;
    onCancelItem: (id: string) => void;
    onRetryItem: (id: string) => void;
}

export function DownloadQueue({ items, onClearFinished, onCancelAll, onCancelItem, onRetryItem }: DownloadQueueProps) {
    if (items.length === 0) return null;

    const activeCount = items.filter(i => i.status === 'pending' || i.status === 'downloading').length;
    const completedCount = items.filter(i => i.status === 'success').length;

    return (
        <div className="fixed bottom-4 right-4 w-80 bg-telegram-surface border border-telegram-border rounded-xl shadow-2xl overflow-hidden z-[100]">
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
                <div className="flex gap-2">
                    {activeCount > 0 && (
                        <button onClick={onCancelAll} className="text-xs text-red-400 hover:text-red-300 transition-colors">Cancel All</button>
                    )}
                    {completedCount > 0 && (
                        <button onClick={onClearFinished} className="text-xs text-telegram-primary hover:text-telegram-text transition-colors">
                            Clear Finished
                        </button>
                    )}
                </div>
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
                                {item.status === 'cancelled' && <div className="w-4 h-4 rounded-full bg-gray-500/20 flex items-center justify-center"><X className="w-3 h-3 text-gray-400" /></div>}
                            </div>
                            <div className="flex-1 truncate text-telegram-subtext" title={item.filename}>
                                {item.filename}
                            </div>
                            {item.status === 'downloading' && (
                                <button onClick={() => onCancelItem(item.id)} className="text-gray-400 hover:text-red-400 transition-colors flex-shrink-0" title="Cancel">
                                    <X className="w-3.5 h-3.5" />
                                </button>
                            )}
                            {item.status === 'pending' && (
                                <button onClick={() => onCancelItem(item.id)} className="text-gray-400 hover:text-red-400 transition-colors flex-shrink-0" title="Remove">
                                    <X className="w-3.5 h-3.5" />
                                </button>
                            )}
                            {(item.status === 'error' || item.status === 'cancelled') && (
                                <button onClick={() => onRetryItem(item.id)} className="text-gray-400 hover:text-blue-400 transition-colors flex-shrink-0" title="Retry">
                                    <RotateCcw className="w-3.5 h-3.5" />
                                </button>
                            )}
                        </div>
                        {item.status === 'downloading' && (
                            <>
                                <div className="w-full bg-telegram-border h-1 mt-1 rounded-full overflow-hidden">
                                    {item.progress !== undefined ? (
                                        <div
                                            className="bg-telegram-secondary h-full rounded-full transition-all duration-300"
                                            style={{ width: `${item.progress}%` }}
                                        />
                                    ) : (
                                        <div className="bg-telegram-secondary h-full w-full animate-progress-indeterminate" />
                                    )}
                                </div>
                                <div className="flex justify-between text-[10px] text-telegram-subtext mt-0.5">
                                    <span>
                                        {item.uploadedBytes !== undefined && item.totalBytes !== undefined
                                            ? `${formatBytes(item.uploadedBytes)} / ${formatBytes(item.totalBytes)}`
                                            : item.progress !== undefined ? `${item.progress}%` : ''}
                                    </span>
                                    <span>
                                        {item.speedBytesPerSec !== undefined && item.speedBytesPerSec > 0
                                            ? `${formatBytes(item.speedBytesPerSec)}/s`
                                            : ''}
                                    </span>
                                </div>
                            </>
                        )}
                        {item.status === 'error' && item.error && (
                            <div className="flex items-center gap-1 text-xs text-red-400 mt-1">
                                <AlertCircle className="w-3 h-3 flex-shrink-0" />
                                <span className="truncate">{item.error}</span>
                            </div>
                        )}
                        {item.status === 'cancelled' && <div className="text-xs text-gray-400 mt-0.5">Cancelled</div>}
                    </div>
                ))}
            </div>
        </div>
    )
}
