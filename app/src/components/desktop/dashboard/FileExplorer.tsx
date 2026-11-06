import { sourceFolder } from '../../../services/fileIdentity';
import { useState, useMemo, useCallback, useRef, useEffect } from 'react';
import { Plus, ArrowUpDown, ArrowUp, ArrowDown, FolderUp } from 'lucide-react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useTranslation } from 'react-i18next';
import { useSettings } from '../../../context/SettingsContext';
import { FileCard } from './FileCard';
import { EmptyState } from './EmptyState';
import { TelegramFile, TelegramFolder } from '../../../types';
import { ContextMenu } from './ContextMenu';
import { FileListItem } from './FileListItem';
import { Skeleton } from '../../ui';
import { quietMetrics } from '../../../design/contracts';

export type SortField = 'name' | 'size' | 'date';
export type SortDirection = 'asc' | 'desc';

const GRID_GAP = quietMetrics.fileGrid.gap;
const MIN_CARD_WIDTH = quietMetrics.fileGrid.minimumCardWidth;
const MIN_CARD_HEIGHT = quietMetrics.fileGrid.minimumCardHeight;
const LIST_ROW_HEIGHT = quietMetrics.listRowHeight.desktop;

interface FileExplorerProps {
    files: TelegramFile[];
    loading: boolean;
    error: Error | null;
    viewMode: 'grid' | 'list';
    selectedIds: number[];
    activeFolderId: number | null;
    onFileClick: (e: React.MouseEvent, file: TelegramFile, orderedFiles: TelegramFile[]) => void;
    onDelete: (file: TelegramFile) => void;
    onDownload: (file: TelegramFile) => void;
    onPreview: (file: TelegramFile, orderedFiles?: TelegramFile[]) => void;
    onManualUpload: () => void;
    onFolderUpload: () => void;
    showFolderUpload: boolean;
    onToggleSelection: (id: number) => void;
    onShare?: (file: TelegramFile) => void;
    onRename?: (file: TelegramFile) => void;
    onFileMove?: (file: TelegramFile) => void;
    folders?: TelegramFolder[];
    cardScale: number;
    sortField: SortField;
    sortDirection: SortDirection;
    onSortChange: (field: SortField) => void;
    onToggleFavorite?: (file: TelegramFile) => void;
    onTogglePinned?: (file: TelegramFile) => void;
    syncProgress?: { active: boolean; count: number };
    selectionDisabled?: boolean;
}


function useGridColumns(containerRef: React.RefObject<HTMLDivElement | null>) {
    const [columns, setColumns] = useState(4);
    const [containerWidth, setContainerWidth] = useState(800);

    useEffect(() => {
        if (!containerRef.current) return;

        const updateColumns = () => {
            const el = containerRef.current;
            if (!el) return;
            // clientWidth includes padding — subtract it so card size
            // calculations match the actual grid content area.
            const cs = getComputedStyle(el);
            const padX = parseFloat(cs.paddingLeft) + parseFloat(cs.paddingRight);
            const width = el.clientWidth - padX;
            setContainerWidth(width > 0 ? width : 800);
            if (width < 640) setColumns(2);
            else if (width < 768) setColumns(3);
            else if (width < 1024) setColumns(4);
            else if (width < 1280) setColumns(5);
            else setColumns(6);
        };

        updateColumns();
        const observer = new ResizeObserver(updateColumns);
        observer.observe(containerRef.current);
        return () => observer.disconnect();
    }, [containerRef]);

    return { columns, containerWidth };
}

export function FileExplorer({
    files, loading, error, viewMode, selectedIds, activeFolderId,
    onFileClick, onDelete, onDownload, onPreview, onManualUpload, onFolderUpload, showFolderUpload, onToggleSelection, onShare, onRename, onFileMove,
    folders, cardScale, sortField, sortDirection, onSortChange, onToggleFavorite, onTogglePinned, syncProgress, selectionDisabled = false
}: FileExplorerProps) {
    const [contextMenu, setContextMenu] = useState<{ x: number; y: number; file: TelegramFile } | null>(null);
    const { t } = useTranslation();
    const { settings } = useSettings();

    const parentRef = useRef<HTMLDivElement>(null);
    const { columns: baseColumns, containerWidth } = useGridColumns(parentRef);

    // Scale columns by cardScale: higher scale = fewer columns = larger cards.
    // Keep a compact protected footprint so 50–75% zoom can add columns while
    // long names, metadata, thumbnails, and actions remain clipped per card.
    const desiredColumns = Math.max(1, Math.round(baseColumns / cardScale));
    const safeColumns = Math.max(1, Math.floor((containerWidth + GRID_GAP) / (MIN_CARD_WIDTH + GRID_GAP)));
    const columns = Math.min(desiredColumns, safeColumns);

    const cardWidth = (containerWidth - (GRID_GAP * (columns - 1))) / columns;
    const cardHeight = Math.max(MIN_CARD_HEIGHT, cardWidth * 0.75); // 4:3 until the protected minimum

    const handleContextMenu = useCallback((e: React.MouseEvent, file: TelegramFile) => {
        e.preventDefault();
        e.stopPropagation();
        setContextMenu({ x: e.clientX, y: e.clientY, file });
    }, []);

    const sortedFiles = useMemo(() => {
        return [...files].sort((a, b) => {
            let comparison = 0;
            switch (sortField) {
                case 'name':
                    comparison = a.name.localeCompare(b.name, settings.language, { numeric: true, sensitivity: 'base' });
                    break;
                case 'size':
                    comparison = (a.size || 0) - (b.size || 0);
                    break;
                case 'date':
                    comparison = (a.created_at || '').localeCompare(b.created_at || '');
                    break;
            }
            return sortDirection === 'asc' ? comparison : -comparison;
        });
    }, [files, settings.language, sortField, sortDirection]);

    const handlePreviewRequest = useCallback((file: TelegramFile) => {
        onPreview(file, sortedFiles);
    }, [onPreview, sortedFiles]);


    const gridRows = useMemo(() => {
        const rows: (TelegramFile | 'upload' | 'upload-folder')[][] = [];
        const tail: ('upload' | 'upload-folder')[] = ['upload'];
        if (showFolderUpload) tail.push('upload-folder');
        const itemsWithUpload: (TelegramFile | 'upload' | 'upload-folder')[] = [...sortedFiles, ...tail];
        for (let i = 0; i < itemsWithUpload.length; i += columns) {
            rows.push(itemsWithUpload.slice(i, i + columns));
        }
        return rows;
    }, [sortedFiles, columns, showFolderUpload]);


    const listItems = useMemo(() => {
        const tail: ('upload' | 'upload-folder')[] = ['upload'];
        if (showFolderUpload) tail.push('upload-folder');
        return [...sortedFiles, ...tail];
    }, [sortedFiles, activeFolderId, showFolderUpload]);


    const gridVirtualizer = useVirtualizer({
        count: gridRows.length,
        getScrollElement: () => parentRef.current,
        estimateSize: useCallback(() => cardHeight, [cardHeight]),
        overscan: 2,
        gap: GRID_GAP,
    });

    const listVirtualizer = useVirtualizer({
        count: listItems.length,
        getScrollElement: () => parentRef.current,
        estimateSize: () => LIST_ROW_HEIGHT,
        overscan: 5,
    });

    useEffect(() => {
        if (parentRef.current) {
            parentRef.current.scrollTop = 0;
        }
        gridVirtualizer.scrollToOffset(0);
        listVirtualizer.scrollToOffset(0);
    }, [activeFolderId, gridVirtualizer, listVirtualizer]);

    // Remeasure the grid virtualizer when columns or cardHeight changes to prevent overlapping
    useEffect(() => {
        gridVirtualizer.measure();
    }, [columns, cardHeight, gridVirtualizer]);

    const SortIcon = ({ field }: { field: SortField }) => {
        if (sortField !== field) return <ArrowUpDown className="h-3 w-3 opacity-40" />;
        return sortDirection === 'asc'
            ? <ArrowUp className="h-3 w-3 text-app-accent" />
            : <ArrowDown className="h-3 w-3 text-app-accent" />;
    };

    if (loading || (!error && syncProgress?.active && files.length === 0)) {
        return (
            <div className="custom-scrollbar flex-1 overflow-hidden p-3" aria-label={t('common.loading')}>
                {viewMode === 'grid' ? (
                    <div className="grid grid-cols-[repeat(auto-fill,minmax(120px,1fr))] gap-3">
                        {Array.from({ length: 12 }, (_, index) => (
                            <div key={index} className="h-[90px] overflow-hidden rounded-container border border-app-border-subtle bg-app-surface/35 p-3">
                                <Skeleton className="h-12 w-12" />
                                <Skeleton className="mt-5 h-3 w-4/5" />
                                <Skeleton className="mt-2 h-2.5 w-2/5" />
                            </div>
                        ))}
                    </div>
                ) : (
                    <div className="border-y border-app-border-subtle">
                        {Array.from({ length: 10 }, (_, index) => (
                            <div key={index} className="flex h-10 items-center gap-3 border-b border-app-border-subtle px-3 last:border-b-0">
                                <Skeleton className="h-4 w-4" />
                                <Skeleton className="h-3 w-[min(22rem,55%)]" />
                                <Skeleton className="ms-auto h-3 w-16" />
                            </div>
                        ))}
                    </div>
                )}
            </div>
        );
    }

    if (error) {
        return <div className="flex flex-1 items-center justify-center p-3 text-ui text-app-danger">Error loading files</div>;
    }

    if (files.length === 0) {
        return (
            <div className="flex-1 overflow-auto p-3">
                <EmptyState onUpload={onManualUpload} />
            </div>
        );
    }

    return (
        <div
            ref={parentRef}
            data-testid="file-explorer"
            className="custom-scrollbar flex-1 overflow-auto p-3"
        >
            {syncProgress?.active && syncProgress.count > 0 && (
                <div className="sticky top-0 z-10 mx-auto mb-3 flex w-fit items-center gap-2 rounded-full border border-app-accent/20 bg-app-surface-raised px-3 py-1.5 text-xs text-app-text-secondary shadow-lg" role="status" aria-live="polite">
                    <span className="h-2 w-2 animate-pulse rounded-full bg-app-accent" />
                    Syncing {syncProgress.count.toLocaleString()} messages…
                </div>
            )}
            {viewMode === 'grid' ? (
                <>
                    <div
                        className="relative w-full"
                        style={{ height: `${gridVirtualizer.getTotalSize()}px` }}
                    >
                        {gridVirtualizer.getVirtualItems().map((virtualRow) => {
                            const row = gridRows[virtualRow.index];
                            return (
                                <div
                                    key={virtualRow.key}
                                    className="absolute top-0 left-0 w-full grid"
                                    style={{
                                        height: `${cardHeight}px`,
                                        transform: `translateY(${virtualRow.start}px)`,
                                        gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
                                        gap: `${GRID_GAP}px`,
                                    }}
                                >
                                    {row.map((item) => {
                                        if (item === 'upload') {
                                            return (
                                                <button
                                                    key="upload"
                                                    onClick={(e) => { e.stopPropagation(); onManualUpload(); }}
                                                    className="quiet-control group flex min-w-0 flex-col items-center justify-center overflow-hidden border border-dashed border-app-border-subtle bg-app-surface/25 text-app-text-secondary hover:border-app-accent/45 hover:bg-app-surface/50 hover:text-app-accent"
                                                    style={{ height: `${cardHeight}px` }}
                                                >
                                                    <Plus className="mb-1.5 h-6 w-6" />
                                                    <span className="text-ui font-medium">{t('common.upload_file')}</span>
                                                </button>
                                            );
                                        }
                                        if (item === 'upload-folder') {
                                            return (
                                                <button
                                                    key="upload-folder"
                                                    onClick={(e) => { e.stopPropagation(); onFolderUpload(); }}
                                                    className="quiet-control group flex min-w-0 flex-col items-center justify-center overflow-hidden border border-dashed border-app-border-subtle bg-app-surface/25 text-app-text-secondary hover:border-app-accent/45 hover:bg-app-surface/50 hover:text-app-accent"
                                                    style={{ height: `${cardHeight}px` }}
                                                >
                                                    <FolderUp className="mb-1.5 h-6 w-6" />
                                                    <span className="text-ui font-medium">{t('common.upload_folder')}</span>
                                                </button>
                                            );
                                        }
                                        const file = item;
                                        return (
                                            <FileCard
                                                key={`${file.folder_id ?? 'home'}:${file.id}`}
                                                file={file}
                                                isSelected={selectedIds.includes(file.id)}
                                                onClick={(e) => onFileClick(e, file, sortedFiles)}
                                                onContextMenu={(e) => handleContextMenu(e, file)}
                                                onDelete={() => onDelete(file)}
                                                onDownload={() => onDownload(file)}
                                                onPreview={() => handlePreviewRequest(file)}
                                                activeFolderId={sourceFolder(file, activeFolderId)}
                                                height={cardHeight}
                                                onToggleSelection={selectionDisabled ? undefined : () => onToggleSelection(file.id)}
                                                onShare={onShare ? () => onShare(file) : undefined}
                                                selectedIds={selectedIds}
                                                disableDrag={selectionDisabled}
                                            />
                                        );
                                    })}
                                </div>
                            );
                        })}
                    </div>
                </>
            ) : (
                <div className="flex w-full flex-col overflow-hidden border-y border-app-border-subtle">
                    {/* List Header */}
                    <div className="grid h-8 grid-cols-[1.75rem_minmax(0,1fr)_2rem] sm:grid-cols-[1.75rem_minmax(0,2fr)_6rem_8rem_2rem] items-center gap-3 border-b border-app-border-subtle bg-app-surface-sunken/20 px-3 text-metadata font-medium text-app-text-secondary select-none">
                        <div aria-hidden="true" />
                        <button onClick={() => onSortChange('name')} className="flex items-center gap-1 transition-colors hover:text-app-text">
                            {t('common.name')} <SortIcon field="name" />
                        </button>
                        <button onClick={() => onSortChange('size')} className="hidden sm:flex items-center justify-end gap-1 transition-colors hover:text-app-text">
                            {t('common.size')} <SortIcon field="size" />
                        </button>
                        <button onClick={() => onSortChange('date')} className="hidden sm:flex items-center justify-end gap-1 transition-colors hover:text-app-text">
                            {t('common.date')} <SortIcon field="date" />
                        </button>
                        <div aria-hidden="true" />
                    </div>

                    <div
                        className="relative w-full"
                        style={{ height: `${listVirtualizer.getTotalSize()}px` }}
                    >
                        {listVirtualizer.getVirtualItems().map((virtualItem) => {
                            const item = listItems[virtualItem.index];
                            if (item === 'upload') {
                                return (
                                    <div
                                        key="upload"
                                        className="absolute top-0 left-0 w-full"
                                        style={{ transform: `translateY(${virtualItem.start}px)` }}
                                    >
                                        <button
                                            onClick={(e) => { e.stopPropagation(); onManualUpload(); }}
                                        className="flex h-10 w-full cursor-pointer items-center gap-3 border-b border-dashed border-app-border-subtle px-3 text-app-text-secondary hover:bg-app-hover hover:text-app-text"
                                        >
                                            <div className="flex h-5 w-5 items-center justify-center"><Plus className="h-3.5 w-3.5" /></div>
                                            <span className="text-ui font-medium">{t('common.upload_file')}...</span>
                                        </button>
                                    </div>
                                );
                            }
                            if (item === 'upload-folder') {
                                return (
                                    <div
                                        key="upload-folder"
                                        className="absolute top-0 left-0 w-full"
                                        style={{ transform: `translateY(${virtualItem.start}px)` }}
                                    >
                                        <button
                                            onClick={(e) => { e.stopPropagation(); onFolderUpload(); }}
                                        className="flex h-10 w-full cursor-pointer items-center gap-3 border-b border-dashed border-app-border-subtle px-3 text-app-text-secondary hover:bg-app-hover hover:text-app-text"
                                        >
                                            <div className="flex h-5 w-5 items-center justify-center"><FolderUp className="h-3.5 w-3.5" /></div>
                                            <span className="text-ui font-medium">{t('common.upload_folder')}...</span>
                                        </button>
                                    </div>
                                );
                            }
                            const file = item;
                            return (
                                <div
                                    key={`${file.folder_id ?? 'home'}:${file.id}`}
                                    className="absolute top-0 left-0 w-full"
                                    style={{ transform: `translateY(${virtualItem.start}px)` }}
                                >
                                    <FileListItem
                                        file={file}
                                        selectedIds={selectedIds}
                                        onFileClick={(e, clickedFile) => onFileClick(e, clickedFile, sortedFiles)}
                                        handleContextMenu={handleContextMenu}
                                        disableDrag={selectionDisabled}
                                    />
                                </div>
                            );
                        })}
                    </div>
                </div>
            )}

            {contextMenu && (
                <ContextMenu
                    x={contextMenu.x}
                    y={contextMenu.y}
                    file={contextMenu.file}
                    onClose={() => setContextMenu(null)}
                    onDownload={() => {
                        onDownload(contextMenu.file);
                        setContextMenu(null);
                    }}
                    onDelete={() => {
                        onDelete(contextMenu.file);
                        setContextMenu(null);
                    }}
                    onPreview={() => {
                        if (contextMenu.file.type === 'folder') {
                             onFileClick({ preventDefault: () => { }, stopPropagation: () => { } } as React.MouseEvent, contextMenu.file, sortedFiles);
                        } else {
                            handlePreviewRequest(contextMenu.file);
                        }
                        setContextMenu(null);
                    }}
                    onShare={onShare ? () => {
                        onShare(contextMenu.file);
                        setContextMenu(null);
                    } : undefined}
                    onRename={onRename ? () => {
                        onRename(contextMenu.file);
                        setContextMenu(null);
                    } : undefined}
                    onMove={onFileMove ? () => {
                        onFileMove(contextMenu.file);
                        setContextMenu(null);
                    } : undefined}
                    folders={folders}
                    activeFolderId={activeFolderId}
                    onToggleFavorite={onToggleFavorite ? () => {
                        onToggleFavorite(contextMenu.file);
                        setContextMenu(null);
                    } : undefined}
                    onTogglePinned={onTogglePinned ? () => {
                        onTogglePinned(contextMenu.file);
                        setContextMenu(null);
                    } : undefined}
                />
            )}
        </div>
    )
}
