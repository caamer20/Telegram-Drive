import { QueueItem } from "../../types";
import { X, RotateCcw, AlertCircle } from "lucide-react";

function formatBytes(bytes: number): string {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`;
}

interface UploadQueueProps {
    items: QueueItem[];
    onClearFinished: () => void;
    onCancelAll: () => void;
    onCancelItem: (id: string) => void;
    onRetryItem: (id: string) => void;
}

export function UploadQueue({ items, onClearFinished, onCancelAll, onCancelItem, onRetryItem }: UploadQueueProps) {
    if (items.length === 0) return null;

    const hasPendingOrActive = items.some(i => i.status === 'pending' || i.status === 'uploading');

    return (
        <div className="fixed bottom-4 right-4 w-80 bg-telegram-surface border border-telegram-border rounded-xl shadow-2xl overflow-hidden z-[100]">
            <div className="p-3 border-b border-telegram-border bg-telegram-hover flex justify-between items-center">
                <h4 className="text-sm font-medium text-telegram-text">Uploads</h4>
                <div className="flex gap-2">
                    {hasPendingOrActive && (
                        <button onClick={onCancelAll} className="text-xs text-red-400 hover:text-red-300 transition-colors">Cancel All</button>
                    )}
                    <button onClick={onClearFinished} className="text-xs text-telegram-primary hover:text-telegram-text transition-colors">Clear Finished</button>
                </div>
            </div>
            <div className="max-h-60 overflow-y-auto p-2 space-y-2">
                {items.map(item => (
                    <div key={item.id} className="flex flex-col gap-1 p-2 bg-telegram-hover rounded">
                        <div className="flex items-center gap-3 text-sm">
                            <div className={`w-2 h-2 rounded-full flex-shrink-0 ${item.status === 'pending' ? 'bg-yellow-500' :
                                item.status === 'uploading' ? 'bg-blue-500 animate-pulse' :
                                    item.status === 'cancelled' ? 'bg-gray-500' :
                                        item.status === 'error' ? 'bg-red-500' : 'bg-green-500'
                                }`} />
                            <div className="flex-1 truncate text-telegram-subtext" title={item.path}>
                                {item.path.split('/').pop()}
                            </div>
                            {item.status === 'uploading' && (
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
                        {item.status === 'uploading' && (
                            <>
                                <div className="w-full bg-telegram-border h-1 mt-1 rounded-full overflow-hidden">
                                    {item.progress !== undefined ? (
                                        <div
                                            className="bg-blue-500 h-full rounded-full transition-all duration-300"
                                            style={{ width: `${item.progress}%` }}
                                        />
                                    ) : (
                                        <div className="bg-blue-500 h-full w-full animate-progress-indeterminate" />
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
