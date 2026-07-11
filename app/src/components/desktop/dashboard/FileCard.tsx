import { motion } from 'framer-motion';
import { useState, useEffect } from 'react';
import { Folder, Eye, Trash2, Link, Check } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { TelegramFile } from '../../../types';
import { createDragGhost } from '../../../utils';
import { FileTypeIcon } from '../../shared/FileTypeIcon';
import { useVideoMetadata } from '../../../hooks/useVideoMetadata';
import { useCachedVariants } from '../../../hooks/useCachedVariants';
import { VideoMetaBadge } from '../../shared/VideoMetaBadge';

interface FileCardProps {
    file: TelegramFile;
    onDelete: () => void;
    onDownload: () => void;
    onPreview?: () => void;
    onShare?: () => void;
    isSelected: boolean;
    onClick?: (e: React.MouseEvent) => void;
    onContextMenu?: (e: React.MouseEvent) => void;
    onDrop?: (e: React.DragEvent, folderId: number) => void;
    onDragStart?: (fileIds: number[]) => void;
    onDragEnd?: () => void;
    activeFolderId?: number | null;
    height?: number;
    onToggleSelection?: () => void;
    selectedIds?: number[];
}

// Check if file is an image type that can have a thumbnail
function isImageFile(filename: string): boolean {
    const ext = filename.split('.').pop()?.toLowerCase() || '';
    return ['jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp'].includes(ext);
}


export function FileCard({ file, onDelete, onDownload, onPreview, onShare, isSelected, onClick, onContextMenu, onDrop, onDragStart, onDragEnd, activeFolderId, height, onToggleSelection, selectedIds }: FileCardProps) {
    const isFolder = file.type === 'folder';
    const [isDragOver, setIsDragOver] = useState(false);
    const [thumbnail, setThumbnail] = useState<string | null>(null);
    const [thumbnailLoading, setThumbnailLoading] = useState(false);

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

    // Lazy load thumbnail for image files
    useEffect(() => {
        if (isFolder || !isImageFile(file.name)) return;

        let cancelled = false;
        setThumbnailLoading(true);

        invoke<string>('cmd_get_thumbnail', {
            messageId: file.id,
            folderId: activeFolderId
        }).then((result) => {
            if (!cancelled && result) {
                setThumbnail(result);
            }
        }).catch(() => {
            // Silently fail - will show icon instead
        }).finally(() => {
            if (!cancelled) setThumbnailLoading(false);
        });

        return () => { cancelled = true; };
    }, [file.id, file.name, activeFolderId, isFolder]);

    return (
        <div
            className="relative"
            draggable={!isFolder}
            onContextMenu={onContextMenu}
            onClick={onClick}
            onDragStart={!isFolder ? (e: any) => {
                const idsToDrag = selectedIds && selectedIds.includes(file.id) ? selectedIds : [file.id];
                if (onDragStart) onDragStart(idsToDrag);
                e.dataTransfer.setData("application/x-telegram-file-ids", JSON.stringify(idsToDrag));
                e.dataTransfer.effectAllowed = 'move';
                const dragCount = idsToDrag.length;
                const ghost = createDragGhost(file.name, isFolder, dragCount);
                e.dataTransfer.setDragImage(ghost, 0, 0);
                requestAnimationFrame(() => ghost.remove());
            } : undefined}
            onDragEnd={!isFolder ? () => {
                if (onDragEnd) onDragEnd();
            } : undefined}
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
        >
            <motion.div
                whileHover={{ y: -4 }}
                className={`group cursor-pointer bg-telegram-surface rounded-xl overflow-hidden border hover:shadow-[0_4px_20px_rgba(0,0,0,0.2)] transition-all relative
                ${isSelected ? 'border-telegram-primary bg-telegram-primary/5 ring-1 ring-telegram-primary' : 'border-telegram-border hover:border-telegram-primary/50'}
                ${isDragOver ? 'ring-2 ring-telegram-primary bg-telegram-primary/20 scale-105' : ''}`}
                style={height ? { height: `${height}px` } : { aspectRatio: '4/3' }}
            >
                {/* Thumbnail or Icon */}
                {thumbnail ? (
                    <div className="absolute inset-0">
                        <img
                            src={thumbnail}
                            alt={file.name}
                            className="w-full h-full object-contain"
                        />
                        {/* Gradient overlay for text readability */}
                        <div className="absolute inset-0 bg-gradient-to-t from-black/70 via-transparent to-transparent" />
                    </div>
                ) : (
                    // Reserve space for the file-info overlay below (name + size/badges)
                    // so the centered icon doesn't visually collide with that text on
                    // shorter cards — most noticeable with video files, whose metadata
                    // badge makes that overlay taller than other file types'.
                    <div className="absolute top-0 left-0 right-0 bottom-16 flex items-center justify-center p-4">
                        {isFolder ? (
                            <Folder className="w-12 h-12 text-telegram-primary" />
                        ) : thumbnailLoading && isImageFile(file.name) ? (
                            <div className="w-8 h-8 border-2 border-telegram-primary/30 border-t-telegram-primary rounded-full animate-spin" />
                        ) : (
                            <FileTypeIcon filename={file.name} size="lg" />
                        )}
                    </div>
                )}

                {/* Selection Checkmark */}
                <div
                    onClick={(e) => {
                        e.stopPropagation();
                        if (onToggleSelection) onToggleSelection();
                    }}
                    className={`absolute top-2 left-2 w-5 h-5 rounded-full border flex items-center justify-center transition-all z-10 cursor-pointer ${isSelected ? 'bg-telegram-primary border-telegram-primary' : 'border-white/50 bg-black/30 opacity-0 group-hover:opacity-100'}`}
                >
                    {isSelected && <div className="w-1.5 h-1.5 bg-black rounded-full" />}
                </div>

                {/* File info overlay at bottom */}
                <div className={`absolute bottom-0 left-0 right-0 p-3 ${thumbnail ? 'text-white' : 'text-telegram-text'}`}>
                    <h3 className="text-sm font-medium truncate w-full" title={file.name}>{file.name}</h3>
                    <div className="flex items-center gap-2 mt-0.5">
                        <p className={`text-xs ${thumbnail ? 'text-white/70' : 'text-telegram-subtext'}`}>{file.sizeStr}</p>
                        <VideoMetaBadge metadata={videoMeta} isLoading={videoMetaLoading} />
                        {cachedQualities.length > 0 && (
                            <span className="inline-flex items-center gap-0.5">
                                {cachedQualities.map(q => (
                                    <span key={q} className="inline-flex items-center gap-0.5 text-[9px] font-medium text-emerald-400 bg-emerald-500/10 px-1 py-0.5 rounded">
                                        <Check className="w-2.5 h-2.5" />
                                        {q}
                                    </span>
                                ))}
                            </span>
                        )}
                    </div>
                </div>

                {/* Quick actions on hover */}
                <div className="absolute top-2 right-2 opacity-0 group-hover:opacity-100 transition-opacity flex gap-1 z-10">
                    <button onClick={(e) => { e.stopPropagation(); if (onPreview) onPreview() }} className="file-action-btn p-1 bg-black/50 rounded-full hover:bg-telegram-primary hover:text-white text-white/70" title="Preview">
                        <Eye className="w-3 h-3" />
                    </button>
                    <button onClick={(e) => { e.stopPropagation(); onDownload() }} className="file-action-btn p-1 bg-black/50 rounded-full hover:bg-green-500 hover:text-white text-white/70" title="Download">
                        <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="w-3 h-3"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"></path><polyline points="7 10 12 15 17 10"></polyline><line x1="12" y1="15" x2="12" y2="3"></line></svg>
                    </button>
                    {!isFolder && onShare && (
                        <button onClick={(e) => { e.stopPropagation(); onShare() }} className="file-action-btn p-1 bg-black/50 rounded-full hover:bg-telegram-primary hover:text-white text-white/70" title="Share">
                            <Link className="w-3 h-3" />
                        </button>
                    )}
                    <button onClick={(e) => { e.stopPropagation(); onDelete() }} className="file-action-btn p-1 bg-black/50 rounded-full hover:bg-red-500 hover:text-white text-white/70" title="Delete">
                        <Trash2 className="w-3 h-3" />
                    </button>
                </div>
            </motion.div>
        </div>
    )
}
