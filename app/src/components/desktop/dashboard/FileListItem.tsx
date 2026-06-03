import { useState } from 'react';
import { Folder, MoreVertical, Check } from 'lucide-react';
import { TelegramFile } from '../../../types';
import { FileTypeIcon } from '../../shared/FileTypeIcon';
import { useVideoMetadata } from '../../../hooks/useVideoMetadata';
import { useCachedVariants } from '../../../hooks/useCachedVariants';
import { VideoMetaBadge } from '../../shared/VideoMetaBadge';

interface FileListItemProps {
    file: TelegramFile;
    selectedIds: number[];
    onFileClick: (e: React.MouseEvent, id: number) => void;
    handleContextMenu: (e: React.MouseEvent, file: TelegramFile) => void;
    onDragStart?: (fileId: number) => void;
    onDragEnd?: () => void;
    onDrop?: (e: React.DragEvent, folderId: number) => void;
}

export function FileListItem({
    file, selectedIds, onFileClick, handleContextMenu,
    onDragStart, onDragEnd, onDrop
}: FileListItemProps) {
    const [isDragOver, setIsDragOver] = useState(false);
    const isFolder = file.type === 'folder';

    // Lazy video metadata badge (.mp4 only)
    const { data: videoMeta, isLoading: videoMetaLoading } = useVideoMetadata(
        file.id,
        file.folder_id ?? null,
        file.name,
    );

    // Cached HLS variants
    const { data: cachedVariants } = useCachedVariants(
        file.id,
        file.folder_id ?? null,
        file.name,
    );
    const cachedQualities = (cachedVariants || []).filter(v => v.available).map(v => v.quality);

    return (
        <div
            onClick={(e) => onFileClick(e, file.id)}
            onContextMenu={(e) => handleContextMenu(e, file)}
            draggable
            onDragStart={(e) => {
                if (onDragStart) onDragStart(file.id);
                e.dataTransfer.setData('application/x-telegram-file-id', file.id.toString());
                e.dataTransfer.effectAllowed = 'move';
            }}
            onDragEnd={() => {
                if (onDragEnd) onDragEnd();
            }}
            onDragOver={(e) => {
                if (isFolder) {
                    e.preventDefault();
                    e.stopPropagation();
                    if (!isDragOver) setIsDragOver(true);
                }
            }}
            onDragLeave={(e) => {
                if (isFolder) {
                    e.preventDefault();
                    e.stopPropagation();
                    setIsDragOver(false);
                }
            }}
            onDrop={(e) => {
                if (isFolder && onDrop) {
                    e.preventDefault();
                    e.stopPropagation();
                    setIsDragOver(false);
                    onDrop(e, file.id);
                }
            }}
            className={`group grid grid-cols-[2rem_minmax(0,1fr)_6rem_8rem_auto] gap-4 items-center px-4 py-3 rounded-lg cursor-pointer border border-transparent transition-all hover:bg-telegram-hover relative
                ${selectedIds.includes(file.id) ? 'bg-telegram-primary/10 border-telegram-primary/20' : ''}
                ${isDragOver ? 'ring-2 ring-telegram-primary bg-telegram-primary/20' : ''}
            `}
        >
            <div className="flex justify-center">
                {isFolder ? <Folder className="w-5 h-5 text-telegram-primary" /> : <FileTypeIcon filename={file.name} className="w-5 h-5" />}
            </div>
            <div className="min-w-0 truncate text-sm text-telegram-text font-medium pr-2">
                <span>{file.name}</span>
                <span className="hidden sm:inline">
                    <VideoMetaBadge metadata={videoMeta} isLoading={videoMetaLoading} />
                </span>
                {cachedQualities.length > 0 && (
                    <span className="hidden sm:inline-flex items-center gap-0.5 ml-1.5">
                        {cachedQualities.map((q) => (
                            <span key={q} className="inline-flex items-center gap-0.5 text-[9px] font-medium text-emerald-400 bg-emerald-500/10 px-1 py-0.5 rounded">
                                <Check className="w-2.5 h-2.5" />
                                {q}
                            </span>
                        ))}
                    </span>
                )}
            </div>
            <div className="min-w-0 text-right text-xs text-telegram-subtext truncate">{file.sizeStr}</div>
            <div className="min-w-0 text-right text-xs text-telegram-subtext font-mono opacity-50 truncate pr-2">{file.created_at || '-'}</div>

            {/* 3-dot Menu Button */}
            <button
                onClick={(e) => {
                    e.stopPropagation();
                    handleContextMenu(e, file);
                }}
                className="justify-self-end opacity-0 group-hover:opacity-100 p-1 bg-telegram-surface hover:bg-telegram-hover border border-telegram-border shadow-md rounded text-telegram-subtext hover:text-telegram-text transition-all z-10"
                title="Actions"
            >
                <MoreVertical className="w-4 h-4" />
            </button>
        </div>
    );
}
