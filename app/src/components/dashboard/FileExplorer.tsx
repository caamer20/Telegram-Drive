import { useState, useMemo } from 'react';
import { Plus, Folder, Eye, HardDrive, ArrowUpDown, ArrowUp, ArrowDown } from 'lucide-react';
import { FileCard } from './FileCard';
import { EmptyState } from './EmptyState';
import { TelegramFile } from '../../types';
import { FileTypeIcon } from '../FileTypeIcon';

type SortField = 'name' | 'size' | 'date';
type SortDirection = 'asc' | 'desc';

interface FileExplorerProps {
    files: TelegramFile[];
    loading: boolean;
    error: any;
    viewMode: 'grid' | 'list';
    selectedIds: number[];
    activeFolderId: number | null;
    onFileClick: (e: React.MouseEvent, id: number) => void;
    onDelete: (id: number) => void;
    onDownload: (id: number, name: string) => void;
    onPreview: (file: TelegramFile) => void;
    onManualUpload: () => void;
    onSelectionClear: () => void;
}

export function FileExplorer({
    files, loading, error, viewMode, selectedIds, activeFolderId,
    onFileClick, onDelete, onDownload, onPreview, onManualUpload, onSelectionClear
}: FileExplorerProps) {
    const [sortField, setSortField] = useState<SortField>('name');
    const [sortDirection, setSortDirection] = useState<SortDirection>('asc');

    const sortedFiles = useMemo(() => {
        return [...files].sort((a, b) => {
            let comparison = 0;
            switch (sortField) {
                case 'name':
                    comparison = a.name.localeCompare(b.name);
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
        <div className="flex-1 p-6 overflow-auto custom-scrollbar" onClick={(e) => {
            if (e.target === e.currentTarget) onSelectionClear();
        }}>
            {viewMode === 'grid' ? (
                <>
                    {/* Sort Controls for Grid */}
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
                    </div>

                    <div className="grid grid-cols-2 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-4 animate-in fade-in duration-300">
                        {sortedFiles.map((file) => (
                            <FileCard
                                key={file.id}
                                file={file}
                                isSelected={selectedIds.includes(file.id)}
                                onClick={(e) => onFileClick(e, file.id)}
                                onDelete={() => onDelete(file.id)}
                                onDownload={() => onDownload(file.id, file.name)}
                                onPreview={() => onPreview(file)}
                            />
                        ))}

                        {/* Upload Button */}
                        <button
                            onClick={(e) => { e.stopPropagation(); onManualUpload(); }}
                            className="aspect-[4/3] border-2 border-dashed border-telegram-border rounded-xl flex flex-col items-center justify-center text-telegram-subtext hover:border-telegram-primary hover:text-telegram-primary transition-all group"
                        >
                            <Plus className="w-8 h-8 mb-2 group-hover:scale-110 transition-transform" />
                            <span className="text-sm font-medium">Upload File</span>
                        </button>
                    </div>
                </>
            ) : (
                <div className="flex flex-col gap-1 w-full animate-in fade-in duration-300">
                    {/* List Header with Sortable Columns */}
                    <div className="grid grid-cols-[2rem_2fr_6rem_8rem] gap-4 px-4 py-2 text-xs font-semibold text-telegram-subtext border-b border-telegram-border mb-2 select-none items-center">
                        <div className="text-center">#</div>
                        <button onClick={() => handleSort('name')} className="flex items-center gap-1 hover:text-telegram-text transition-colors">
                            Name <SortIcon field="name" />
                        </button>
                        <button onClick={() => handleSort('size')} className="flex items-center gap-1 justify-end hover:text-telegram-text transition-colors">
                            Size <SortIcon field="size" />
                        </button>
                        <button onClick={() => handleSort('date')} className="flex items-center gap-1 justify-end hover:text-telegram-text transition-colors">
                            Date <SortIcon field="date" />
                        </button>
                    </div>

                    {sortedFiles.map((file) => (
                        <div
                            key={file.id}
                            onClick={(e) => onFileClick(e, file.id)}
                            onContextMenu={(e) => {
                                e.preventDefault();
                                if (!selectedIds.includes(file.id)) onFileClick(e, file.id);
                            }}
                            draggable
                            onDragStart={(e) => {
                                e.dataTransfer.setData("application/x-telegram-file-id", file.id.toString());
                            }}
                            className={`group grid grid-cols-[2rem_2fr_6rem_8rem] gap-4 items-center px-4 py-3 rounded-lg cursor-pointer border border-transparent transition-all hover:bg-telegram-hover ${selectedIds.includes(file.id) ? 'bg-telegram-primary/10 border-telegram-primary/20' : ''}`}
                        >
                            <div className="flex justify-center">
                                {file.type === 'folder' ? <Folder className="w-5 h-5 text-telegram-primary" /> : <FileTypeIcon filename={file.name} className="w-5 h-5" />}
                            </div>
                            <div className="truncate text-sm text-telegram-text font-medium relative pr-8">
                                {file.name}
                                {/* List Actions */}
                                <div className="absolute right-0 top-1/2 -translate-y-1/2 opacity-0 group-hover:opacity-100 flex items-center bg-telegram-surface border border-telegram-border shadow-lg rounded px-1">
                                    <button onClick={(e) => { e.stopPropagation(); onPreview(file) }} className="p-1 hover:text-telegram-text text-telegram-subtext" title="Preview"><Eye className="w-4 h-4" /></button>
                                    <button onClick={(e) => { e.stopPropagation(); onDownload(file.id, file.name) }} className="p-1 hover:text-telegram-text text-telegram-subtext" title="Download"><HardDrive className="w-4 h-4" /></button>
                                    <button onClick={(e) => { e.stopPropagation(); onDelete(file.id) }} className="p-1 hover:text-red-400 text-telegram-subtext" title="Delete"><Plus className="w-4 h-4 rotate-45" /></button>
                                </div>
                            </div>
                            <div className="text-right text-xs text-telegram-subtext truncate">{file.sizeStr}</div>
                            <div className="text-right text-xs text-telegram-subtext font-mono opacity-50 truncate">{file.created_at || '-'}</div>
                        </div>
                    ))}

                    {activeFolderId === null && (
                        <button
                            onClick={(e) => { e.stopPropagation(); onManualUpload(); }}
                            className="flex items-center gap-4 px-4 py-3 rounded-lg cursor-pointer border border-dashed border-telegram-border text-telegram-subtext hover:text-telegram-text hover:bg-telegram-hover mt-2"
                        >
                            <div className="w-5 h-5 flex items-center justify-center"><Plus className="w-4 h-4" /></div>
                            <span className="text-sm font-medium">Upload File...</span>
                        </button>
                    )}
                </div>
            )}
        </div>
    )
}
