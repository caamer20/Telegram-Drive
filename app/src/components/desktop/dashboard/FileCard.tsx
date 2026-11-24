import {useTranslation} from 'react-i18next';
import { sourceFolder } from '../../../services/fileIdentity';
import { useCallback, useState, useEffect, useSyncExternalStore } from 'react';
import { Folder, Eye, Trash2, Link, Check } from 'lucide-react';
import { useDraggable, useDroppable } from '@dnd-kit/core';
import { TelegramFile } from '../../../types';
import { forgetThumbnail, getCachedThumbnail, loadThumbnail, subscribeImageCache, imageCacheVersion } from '../../../services/imagePreviewCache';
import { FileTypeIcon } from '../../shared/FileTypeIcon';
import { useVideoMetadata } from '../../../hooks/useVideoMetadata';
import { useCachedVariants } from '../../../hooks/useCachedVariants';
import { VideoMetaBadge } from '../../shared/VideoMetaBadge';
import { Skeleton } from '../../ui';
import { EncryptionBadge } from '../../shared/EncryptionBadge';
import { describeFileActions } from './fileActionDescriptors';
import i18n from '../../../i18n';

interface FileCardProps {
    file: TelegramFile;
    onDelete: () => void;
    onDownload: () => void;
    onPreview?: () => void;
    onShare?: () => void;
    isSelected: boolean;
    onClick?: (e: React.MouseEvent) => void;
    onContextMenu?: (e: React.MouseEvent) => void;
    activeFolderId?: number | null;
    height?: number;
    onToggleSelection?: () => void;
    selectedIds?: number[];
    disableDrag?: boolean;
}

// Check if file is an image type that can have a thumbnail
function hasThumbnail(filename: string): boolean {
    const ext = filename.split('.').pop()?.toLowerCase() || '';
    return ['jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp', 'mp4', 'mkv', 'webm', 'mov', 'avi', 'm4v'].includes(ext);
}


export function FileCard({ file, onDelete, onDownload, onPreview, onShare, isSelected, onClick, onContextMenu, activeFolderId, height, onToggleSelection, selectedIds, disableDrag = false }: FileCardProps) {
    useTranslation();
    const sourceFolderId = sourceFolder(file, activeFolderId ?? null);
    const thumbnailVersion = useSyncExternalStore(subscribeImageCache, imageCacheVersion, imageCacheVersion);
    const actions = describeFileActions(file);
    const { isFolder } = actions;
    const [thumbnail, setThumbnail] = useState<string | null>(null);
    const [thumbnailLoading, setThumbnailLoading] = useState(false);
    const [thumbnailReady, setThumbnailReady] = useState(false);
    const fileIds = selectedIds?.includes(file.id) ? selectedIds : [file.id];
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
        sourceFolderId,
        file.name,
    );

    // Cached HLS variants
    const { data: cachedVariants } = useCachedVariants(
        file.id,
        sourceFolderId,
        file.name,
    );
    const cachedQualities = (cachedVariants || []).filter(v => v.available).map(v => v.quality);

    // Lazy load thumbnail for image files
    useEffect(() => {
        if (isFolder || !hasThumbnail(file.name)) return;

        let cancelled = false;
        const cached = getCachedThumbnail(file.id, sourceFolderId);
        setThumbnail(cached);
        setThumbnailLoading(!cached);
        setThumbnailReady(Boolean(cached));

        loadThumbnail(file.id, sourceFolderId).then((result) => {
            if (!cancelled && result) {
                if (result !== cached) setThumbnailReady(false);
                setThumbnail(result);
            }
        }).catch(() => {
            // Silently fail - will show icon instead
        }).finally(() => {
            if (!cancelled) setThumbnailLoading(false);
        });

        return () => { cancelled = true; };
    }, [file, sourceFolderId, isFolder, thumbnailVersion]);

    return (
        <div
            ref={setNodeRef}
            className="file-card-container relative h-full min-w-0 overflow-hidden"
            style={{ opacity: isDragging ? 0.45 : undefined }}
            {...(!isFolder ? attributes : {})}
            {...(!isFolder ? listeners : {})}
            onKeyDown={event => { if (!isFolder && event.target === event.currentTarget) listeners?.onKeyDown?.(event); }}
            aria-disabled={undefined}
            role="group"
            aria-label={file.name}
            onContextMenu={onContextMenu}
            onClick={onClick}
        >
            <div
                className={`group relative h-full w-full min-w-0 cursor-pointer overflow-hidden rounded-container border transition-[border-color,background-color,box-shadow]
                ${isSelected ? 'border-app-accent bg-app-selected ring-1 ring-app-accent' : 'border-transparent bg-app-surface/45 hover:border-app-border hover:bg-app-surface/70'}
                ${isFileDragOver ? 'bg-app-selected ring-2 ring-app-accent' : ''}`}
                style={height ? { height: `${height}px` } : { aspectRatio: '4/3' }}
            >
                {/* Thumbnail or Icon */}
                {thumbnail ? (
                    <div className="absolute inset-0">
                        {!thumbnailReady && <Skeleton className="absolute inset-0 rounded-none" />}
                        <img
                            src={thumbnail}
                            alt={file.name}
                            loading="lazy"
                            decoding="async"
                            className={`h-full w-full object-cover transition-opacity duration-300 motion-reduce:transition-none ${thumbnailReady ? 'opacity-100' : 'opacity-0'}`}
                            onLoad={() => setThumbnailReady(true)}
                            onError={() => {
                                forgetThumbnail(file.id, sourceFolderId);
                                setThumbnail(null);
                                setThumbnailReady(false);
                            }}
                        />
                        {/* Gradient overlay for text readability */}
                        <div className="absolute inset-0 bg-gradient-to-t from-black/70 via-transparent to-transparent" />
                    </div>
                ) : (
                    <div className="file-card-icon absolute inset-x-0 bottom-12 top-0 flex items-center justify-center p-3">
                        {isFolder ? (
                            <Folder className="h-10 w-10 max-h-full max-w-full shrink-0 text-app-accent" strokeWidth={1.6} />
                        ) : thumbnailLoading && hasThumbnail(file.name) ? (
                            <Skeleton className="h-10 w-10 shrink-0" />
                        ) : (
                            <FileTypeIcon filename={file.name} size="lg" className="h-10 w-10 max-h-full max-w-full shrink-0" />
                        )}
                    </div>
                )}

                {/* Selection Checkmark */}
                <button
                    type="button"
                    aria-label={isSelected ? `Deselect ${file.name}` : `Select ${file.name}`}
                    aria-pressed={isSelected}
                    disabled={!onToggleSelection}
                    onClick={(e) => {
                        e.stopPropagation();
                        if (onToggleSelection) onToggleSelection();
                    }}
                    className={`absolute start-2 top-2 z-10 flex h-[22px] w-[22px] cursor-pointer items-center justify-center rounded-full border transition-opacity ${isSelected ? 'border-app-accent bg-app-accent text-app-accent-contrast' : 'border-white/55 bg-black/35 text-white opacity-0 group-hover:opacity-100 focus-visible:opacity-100'}`}
                >
                    {isSelected && <Check className="h-3 w-3" />}
                </button>

                {/* File info overlay at bottom */}
                <div className={`file-card-info absolute inset-x-0 bottom-0 z-[1] min-h-12 overflow-hidden px-2.5 py-2 ${thumbnail ? 'text-white' : 'text-app-text'}`}>
                    <h2 className="block w-full min-w-0 truncate text-ui font-medium" title={file.name}>{file.name}</h2>
                    <div className="file-card-metadata mt-0.5 flex h-4 w-full min-w-0 items-center gap-1.5 overflow-hidden whitespace-nowrap">
                        <p className={`shrink-0 text-metadata ${thumbnail ? 'text-white/70' : 'text-app-text-secondary'}`}>{file.sizeStr}</p>
                        <EncryptionBadge state={file.encryption_state ?? 'plain'} className="shrink-0" />
                        <VideoMetaBadge metadata={videoMeta} isLoading={videoMetaLoading} />
                        {cachedQualities.length > 0 && (
                            <span className="inline-flex min-w-0 items-center gap-0.5 overflow-hidden">
                                {cachedQualities.map(q => (
                                    <span key={q} className="inline-flex items-center gap-0.5 rounded bg-emerald-500/10 px-1 py-0.5 text-badge font-medium text-emerald-400">
                                        <Check className="h-2.5 w-2.5" />
                                        {q}
                                    </span>
                                ))}
                            </span>
                        )}
                    </div>
                </div>

                {/* Quick actions on hover */}
                <div className="file-card-actions absolute end-2 top-2 z-10 flex max-w-[calc(100%-2.75rem)] gap-0.5 overflow-hidden rounded-control border border-white/10 bg-black/55 p-0.5 opacity-0 backdrop-blur-md transition-opacity group-hover:opacity-100 focus-within:opacity-100">
                    <button type="button" aria-label={i18n.t('common.action_name',{action:i18n.t('files.preview'),name:file.name})} onClick={(e) => { e.stopPropagation(); if (onPreview) onPreview() }} className="quiet-control file-action-btn flex h-7 w-7 items-center justify-center text-white/80 hover:text-white" title={i18n.t("files.preview")}>
                        <Eye className="h-3.5 w-3.5" />
                    </button>
                    <button type="button" aria-label={i18n.t('common.action_name',{action:i18n.t('files.download'),name:file.name})} onClick={(e) => { e.stopPropagation(); onDownload() }} className="quiet-control file-action-btn flex h-7 w-7 items-center justify-center text-white/80 hover:text-white" title={i18n.t("files.download")}>
                        <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="w-3 h-3"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"></path><polyline points="7 10 12 15 17 10"></polyline><line x1="12" y1="15" x2="12" y2="3"></line></svg>
                    </button>
                    {actions.canShare && onShare && (
                        <button type="button" aria-label={i18n.t('common.action_name',{action:i18n.t('files.share'),name:file.name})} onClick={(e) => { e.stopPropagation(); onShare() }} className="quiet-control file-action-btn flex h-7 w-7 items-center justify-center text-white/80 hover:text-white" title={i18n.t("files.share")}>
                            <Link className="h-3.5 w-3.5" />
                        </button>
                    )}
                    <button type="button" aria-label={i18n.t('common.action_name',{action:i18n.t('files.delete'),name:file.name})} onClick={(e) => { e.stopPropagation(); onDelete() }} className="quiet-control file-action-btn flex h-7 w-7 items-center justify-center text-white/80 hover:bg-red-500/70 hover:text-white" title={i18n.t("files.delete")}>
                        <Trash2 className="h-3.5 w-3.5" />
                    </button>
                </div>
            </div>
        </div>
    )
}
