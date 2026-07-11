import { useState, useMemo, useCallback, useRef, useEffect } from 'react';
import { Plus, ArrowUpDown, ArrowUp, ArrowDown, FolderUp, ZoomIn, ZoomOut } from 'lucide-react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useTranslation } from 'react-i18next';
import { useSettings } from '../../../context/SettingsContext';
import { FileCard } from './FileCard';
import { EmptyState } from './EmptyState';
import { TelegramFile, TelegramFolder } from '../../../types';
import { ContextMenu } from './ContextMenu';
import { FileListItem } from './FileListItem';

type SortField = 'name' | 'size' | 'date';
type SortDirection = 'asc' | 'desc';

interface FileExplorerProps {
    files: TelegramFile[];
    loading: boolean;
    error: Error | null;
    viewMode: 'grid' | 'list';
    selectedIds: number[];
    activeFolderId: number | null;
    onFileClick: (e: React.MouseEvent, id: number, orderedFiles: TelegramFile[]) => void;
    onDelete: (id: number) => void;
    onDownload: (id: number, name: string) => void;
    onPreview: (file: TelegramFile, orderedFiles?: TelegramFile[]) => void;
    onManualUpload: () => void;
    onFolderUpload: () => void;
    showFolderUpload: boolean;
    onToggleSelection: (id: number) => void;
    onDrop?: (e: React.DragEvent, folderId: number) => void;
    onDragStart?: (fileIds: number[]) => void;
    onDragEnd?: () => void;
    onShare?: (file: TelegramFile) => void;
    onRename?: (file: TelegramFile) => void;
    onFileMove?: (file: TelegramFile) => void;
    folders?: TelegramFolder[];
    cardScale: number;
    onCardScaleChange: (scale: number) => void;
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
    onFileClick, onDelete, onDownload, onPreview, onManualUpload, onFolderUpload, showFolderUpload, onToggleSelection, onDrop, onDragStart, onDragEnd, onShare, onRename, onFileMove,
    folders, cardScale, onCardScaleChange
}: FileExplorerProps) {
    const [sortField, setSortField] = useState<SortField>('name');
    const [sortDirection, setSortDirection] = useState<SortDirection>('asc');
    const [contextMenu, setContextMenu] = useState<{ x: number; y: number; file: TelegramFile } | null>(null);
    const { t } = useTranslation();
    const { settings } = useSettings();

    const parentRef = useRef<HTMLDivElement>(null);
    const { columns: baseColumns, containerWidth } = useGridColumns(parentRef);

    // Scale columns by cardScale: higher scale = fewer columns = larger cards
    const columns = Math.max(1, Math.round(baseColumns / cardScale));

    const GAP = 6;
    const cardWidth = (containerWidth - (GAP * (columns - 1))) / columns;
    const cardHeight = cardWidth * 0.75; // aspect-[4/3]

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
    }, [files, sortField, sortDirection]);

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
        gap: GAP,
    });

    const listVirtualizer = useVirtualizer({
        count: listItems.length,
        getScrollElement: () => parentRef.current,
        estimateSize: () => 48,
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

    const handleSort = (field: SortField) => {
        if (sortField === field) {
            setSortDirection(d => d === 'asc' ? 'desc' : 'asc');
        } else {
            setSortField(field);
            setSortDirection('asc');
        }
    };

    const SortIcon = ({ field }: { field: SortField }) => {
        if (sortField !== field) return <ArrowUpDown className="w-3 h-3 opacity-30" />;
        return sortDirection === 'asc'
            ? <ArrowUp className="w-3 h-3 text-telegram-primary" />
            : <ArrowDown className="w-3 h-3 text-telegram-primary" />;
    };

    if (loading) {
        return (
            <div className="flex-1 p-6 flex justify-center items-center text-telegram-subtext flex-col gap-4">
                <div className="w-8 h-8 border-4 border-telegram-primary border-t-transparent rounded-full animate-spin"></div>
                Loading your files...
            </div>
        )
    }

    if (error) {
        return <div className="flex-1 p-6 flex justify-center items-center text-red-400">Error loading files</div>
    }

    if (files.length === 0) {
        return (
            <div className="flex-1 p-6 overflow-auto">
                <EmptyState onUpload={onManualUpload} />
            </div>
        );
    }

    return (
        <div
            ref={parentRef}
            className="flex-1 p-6 overflow-auto custom-scrollbar"
        >
            {viewMode === 'grid' ? (
                <>

                    <div className="flex items-center gap-2 mb-4 text-xs text-telegram-subtext">
                        <span>Sort by:</span>
                        <button
                            onClick={() => handleSort('name')}
                            className={`px-2 py-1 rounded flex items-center gap-1 hover:bg-white/5 ${sortField === 'name' ? 'text-telegram-primary' : ''}`}
                        >
                            Name <SortIcon field="name" />
                        </button>
                        <button
                            onClick={() => handleSort('size')}
                            className={`px-2 py-1 rounded flex items-center gap-1 hover:bg-white/5 ${sortField === 'size' ? 'text-telegram-primary' : ''}`}
                        >
                            Size <SortIcon field="size" />
                        </button>
                        <button
                            onClick={() => handleSort('date')}
                            className={`px-2 py-1 rounded flex items-center gap-1 hover:bg-white/5 ${sortField === 'date' ? 'text-telegram-primary' : ''}`}
                        >
                            Date <SortIcon field="date" />
                        </button>

                        {/* Zoom slider */}
                        <div className="ml-auto flex items-center gap-1.5">
                            <button
                                onClick={() => onCardScaleChange(Math.max(0.5, cardScale - 0.25))}
                                className="p-1 rounded hover:bg-white/10 text-telegram-subtext hover:text-telegram-text transition-colors"
                                title="Smaller thumbnails"
                                disabled={cardScale <= 0.5}
                            >
                                <ZoomOut className="w-3.5 h-3.5" />
                            </button>
                            <input
                                type="range"
                                min="0.5"
                                max="2"
                                step="0.25"
                                value={cardScale}
                                onChange={(e) => onCardScaleChange(parseFloat(e.target.value))}
                                className="w-20 h-1 bg-telegram-border rounded-full appearance-none cursor-pointer [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:w-3 [&::-webkit-slider-thumb]:h-3 [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-telegram-primary [&::-webkit-slider-thumb]:cursor-pointer [&::-webkit-slider-thumb]:transition-transform [&::-webkit-slider-thumb]:hover:scale-125"
                                title={`Thumbnail zoom: ${Math.round(cardScale * 100)}%`}
                            />
                            <button
                                onClick={() => onCardScaleChange(Math.min(2, cardScale + 0.25))}
                                className="p-1 rounded hover:bg-white/10 text-telegram-subtext hover:text-telegram-text transition-colors"
                                title="Larger thumbnails"
                                disabled={cardScale >= 2}
                            >
                                <ZoomIn className="w-3.5 h-3.5" />
                            </button>
                            <span className="text-[10px] text-telegram-subtext/60 w-10 text-right tabular-nums">{Math.round(cardScale * 100)}%</span>
                        </div>
                    </div>


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
                                        gap: `${GAP}px`,
                                    }}
                                >
                                    {row.map((item) => {
                                        if (item === 'upload') {
                                            return (
                                                <button
                                                    key="upload"
                                                    onClick={(e) => { e.stopPropagation(); onManualUpload(); }}
                                                    className="border-2 border-dashed border-telegram-border rounded-xl flex flex-col items-center justify-center text-telegram-subtext hover:border-telegram-primary hover:text-telegram-primary transition-all group overflow-hidden"
                                                    style={{ height: `${cardHeight}px` }}
                                                >
                                                    <Plus className="w-8 h-8 mb-2 group-hover:scale-110 transition-transform" />
                                                    <span className="text-sm font-medium">{t('common.upload_file')}</span>
                                                </button>
                                            );
                                        }
                                        if (item === 'upload-folder') {
                                            return (
                                                <button
                                                    key="upload-folder"
                                                    onClick={(e) => { e.stopPropagation(); onFolderUpload(); }}
                                                    className="border-2 border-dashed border-telegram-border rounded-xl flex flex-col items-center justify-center text-telegram-subtext hover:border-telegram-primary hover:text-telegram-primary transition-all group overflow-hidden"
                                                    style={{ height: `${cardHeight}px` }}
                                                >
                                                    <FolderUp className="w-8 h-8 mb-2 group-hover:scale-110 transition-transform" />
                                                    <span className="text-sm font-medium">{t('common.upload_folder')}</span>
                                                </button>
                                            );
                                        }
                                        const file = item;
                                        return (
                                            <FileCard
                                                key={file.id}
                                                file={file}
                                                isSelected={selectedIds.includes(file.id)}
                                                onClick={(e) => onFileClick(e, file.id, sortedFiles)}
                                                onContextMenu={(e) => handleContextMenu(e, file)}
                                                onDelete={() => onDelete(file.id)}
                                                onDownload={() => onDownload(file.id, file.name)}
                                                onPreview={() => handlePreviewRequest(file)}
                                                onDrop={onDrop}
                                                onDragStart={onDragStart}
                                                onDragEnd={onDragEnd}
                                                activeFolderId={activeFolderId}
                                                height={cardHeight}
                                                onToggleSelection={() => onToggleSelection(file.id)}
                                                onShare={onShare ? () => onShare(file) : undefined}
                                                selectedIds={selectedIds}
                                            />
                                        );
                                    })}
                                </div>
                            );
                        })}
                    </div>
                </>
            ) : (
                <div className="flex flex-col w-full">
                    {/* List Header */}
                    <div className="grid grid-cols-[2rem_2fr_6rem_8rem] gap-4 px-4 py-2 text-xs font-semibold text-telegram-subtext border-b border-telegram-border mb-2 select-none items-center">
                        <div className="text-center">#</div>
                        <button onClick={() => handleSort('name')} className="flex items-center gap-1 hover:text-telegram-text transition-colors">
                            {t('common.name')} <SortIcon field="name" />
                        </button>
                        <button onClick={() => handleSort('size')} className="flex items-center gap-1 justify-end hover:text-telegram-text transition-colors">
                            {t('common.size')} <SortIcon field="size" />
                        </button>
                        <button onClick={() => handleSort('date')} className="flex items-center gap-1 justify-end hover:text-telegram-text transition-colors">
                            {t('common.date')} <SortIcon field="date" />
                        </button>
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
                                            className="flex items-center gap-4 px-4 py-3 rounded-lg cursor-pointer border border-dashed border-telegram-border text-telegram-subtext hover:text-telegram-text hover:bg-telegram-hover w-full"
                                        >
                                            <div className="w-5 h-5 flex items-center justify-center"><Plus className="w-4 h-4" /></div>
                                            <span className="text-sm font-medium">{t('common.upload_file')}...</span>
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
                                            className="flex items-center gap-4 px-4 py-3 rounded-lg cursor-pointer border border-dashed border-telegram-border text-telegram-subtext hover:text-telegram-text hover:bg-telegram-hover w-full"
                                        >
                                            <div className="w-5 h-5 flex items-center justify-center"><FolderUp className="w-4 h-4" /></div>
                                            <span className="text-sm font-medium">{t('common.upload_folder')}...</span>
                                        </button>
                                    </div>
                                );
                            }
                            const file = item;
                            return (
                                <div
                                    key={file.id}
                                    className="absolute top-0 left-0 w-full"
                                    style={{ transform: `translateY(${virtualItem.start}px)` }}
                                >
                                    <FileListItem
                                        file={file}
                                        selectedIds={selectedIds}
                                        onFileClick={(e, id) => onFileClick(e, id, sortedFiles)}
                                        handleContextMenu={handleContextMenu}
                                        onDragStart={onDragStart}
                                        onDragEnd={onDragEnd}
                                        onDrop={onDrop}
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
                        onDownload(contextMenu.file.id, contextMenu.file.name);
                        setContextMenu(null);
                    }}
                    onDelete={() => {
                        onDelete(contextMenu.file.id);
                        setContextMenu(null);
                    }}
                    onPreview={() => {
                        if (contextMenu.file.type === 'folder') {
                            onFileClick({ preventDefault: () => { }, stopPropagation: () => { } } as React.MouseEvent, contextMenu.file.id, sortedFiles);
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
                />
            )}
        </div>
    )
}
