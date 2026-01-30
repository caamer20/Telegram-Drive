import { QueueItem } from "../../types";

interface UploadQueueProps {
    items: QueueItem[];
    onClearFinished: () => void;
}

export function UploadQueue({ items, onClearFinished }: UploadQueueProps) {
    if (items.length === 0) return null;

    return (
        <div className="fixed bottom-4 right-4 w-80 bg-telegram-surface border border-telegram-border rounded-xl shadow-2xl overflow-hidden z-[100]">
            <div className="p-3 border-b border-telegram-border bg-telegram-hover flex justify-between items-center">
                <h4 className="text-sm font-medium text-telegram-text">Uploads</h4>
                <button onClick={onClearFinished} className="text-xs text-telegram-primary hover:text-telegram-text transition-colors">Clear Finished</button>
            </div>
            <div className="max-h-60 overflow-y-auto p-2 space-y-2">
                {items.map(item => (
                    <div key={item.id} className="flex flex-col gap-1 p-2 bg-telegram-hover rounded">
                        <div className="flex items-center gap-3 text-sm">
                            <div className={`w-2 h-2 rounded-full ${item.status === 'pending' ? 'bg-yellow-500' :
                                item.status === 'uploading' ? 'bg-blue-500 animate-pulse' :
                                    item.status === 'error' ? 'bg-red-500' : 'bg-green-500'
                                }`} />
                            <div className="flex-1 truncate text-telegram-subtext" title={item.path}>
                                {item.path.split('/').pop()}
                            </div>
                            {item.status === 'error' && <div className="text-xs text-red-400">Error</div>}
                        </div>
                        {item.status === 'uploading' && (
                            <div className="w-full bg-telegram-border h-1 mt-1 rounded-full overflow-hidden">
                                <div className="bg-blue-500 h-full w-full animate-progress-indeterminate"></div>
                            </div>
                        )}
                    </div>
                ))}
            </div>
        </div>
    )
}
