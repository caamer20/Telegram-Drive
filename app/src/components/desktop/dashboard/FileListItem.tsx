import { useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Folder, MoreVertical, Check } from 'lucide-react';
import { useDraggable, useDroppable } from '@dnd-kit/core';
import { TelegramFile } from '../../../types';
import { FileTypeIcon } from '../../shared/FileTypeIcon';
import { useVideoMetadata } from '../../../hooks/useVideoMetadata';
import { useCachedVariants } from '../../../hooks/useCachedVariants';
import { VideoMetaBadge } from '../../shared/VideoMetaBadge';
import { EncryptionBadge } from '../../shared/EncryptionBadge';
import { describeFileActions } from './fileActionDescriptors';


interface FileListItemProps {
    file: TelegramFile;
    selectedIds: number[];
    onFileClick: (e: React.MouseEvent, file: TelegramFile) => void;
    handleContextMenu: (e: React.MouseEvent, file: TelegramFile) => void;
    disableDrag?: boolean;
}

export function FileListItem({
    file, selectedIds, onFileClick, handleContextMenu, disableDrag = false
}: FileListItemProps) {
    const { t, i18n } = useTranslation();
    const timestamp = Date.parse(file.created_at || '');
    const dateLabel = Number.isFinite(timestamp) ? new Date(timestamp).toLocaleDateString(i18n.resolvedLanguage, { month: 'short', day: 'numeric', year: 'numeric' }) : (file.created_at || '—');
    const { isFolder } = describeFileActions(file);
    const fileIds = selectedIds.includes(file.id) ? selectedIds : [file.id];
    const {
        attributes,
        listeners,
        setNodeRef: setDraggableNodeRef,
        isDragging,
    } = useDraggable({
        id: `telegram-file-${file.folder_id ?? 'home'}-${file.id}`,
        disabled: isFolder || disableDrag,
        data: { kind: 'telegram-files', fileIds, label: file.name },
    });
    const {
        setNodeRef: setDroppableNodeRef,
        isOver,
        active: dragActive,
    } = useDroppable({
        id: `content-folder-${file.id}`,
        disabled: !isFolder,
        data: { kind: 'content-folder', folderId: file.id },
    });
    const setNodeRef = useCallback((node: HTMLDivElement | null) => {
        setDraggableNodeRef(node);
        setDroppableNodeRef(node);
    }, [setDraggableNodeRef, setDroppableNodeRef]);
    const isFileDragOver = isFolder && isOver && dragActive?.data.current?.kind === 'telegram-files';

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
            ref={setNodeRef}
            data-file-row
            onClick={(e) => onFileClick(e, file)}
            onContextMenu={(e) => handleContextMenu(e, file)}
            style={{ opacity: isDragging ? 0.45 : undefined }}
            {...(!isFolder ? attributes : {})}
            {...(!isFolder ? listeners : {})}
            aria-disabled={undefined}
            role="group"
            aria-label={file.name}
            className={`group grid h-10 cursor-pointer grid-cols-[1.75rem_minmax(0,1fr)_2rem] items-center gap-3 border-b border-app-border-subtle px-3 transition-colors hover:bg-app-hover sm:grid-cols-[1.75rem_minmax(0,2fr)_6rem_8rem_2rem]
                ${selectedIds.includes(file.id) ? 'bg-app-selected' : ''}
                ${isFileDragOver ? 'bg-app-selected ring-2 ring-inset ring-app-accent' : ''}
            `}
        >
            <div className="flex justify-center">
                {isFolder ? <Folder className="h-4 w-4 text-app-accent" /> : <FileTypeIcon filename={file.name} className="h-4 w-4" />}
            </div>
            <div className="min-w-0 truncate text-ui font-medium text-app-text">
                <span>{file.name}</span>
                <EncryptionBadge state={file.encryption_state ?? 'plain'} className="ms-1.5 align-middle" />
                <VideoMetaBadge metadata={videoMeta} isLoading={videoMetaLoading} />
                {cachedQualities.length > 0 && (
                    <span className="inline-flex items-center gap-0.5 ml-1.5">
                        {cachedQualities.map(q => (
                            <span key={q} className="inline-flex items-center gap-0.5 rounded bg-emerald-500/10 px-1 py-0.5 text-badge font-medium text-emerald-400">
                                <Check className="w-2.5 h-2.5" />
                                {q}
                            </span>
                        ))}
                    </span>
                )}
            </div>
            <div className="hidden truncate text-end text-metadata text-app-text-secondary sm:block">{file.sizeStr}</div>
            <div className="hidden truncate text-end text-metadata tabular-nums text-app-text-tertiary sm:block" title={file.created_at}>{dateLabel}</div>

            {/* 3-dot Menu Button — in grid flow, not absolutely positioned */}
            <div className="flex justify-end">
                <button
                    onClick={(e) => {
                        e.stopPropagation();
                        handleContextMenu(e, file);
                    }}
                    className="quiet-control flex h-7 w-7 items-center justify-center border border-transparent text-app-text-secondary hover:border-app-border hover:text-app-text"
                    aria-label={t('workspace.file_actions', { name: file.name })}
                >
                    <MoreVertical className="h-3.5 w-3.5" />
                </button>
            </div>
        </div>
    );
}
