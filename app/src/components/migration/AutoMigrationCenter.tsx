import React, { useState, useMemo, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import {
    Cloud,
    CheckCircle2,
    HardDrive,
    Zap,
    Settings,
    ShieldAlert,
    RefreshCw,
    Activity,
    Folder,
    FileText,
    ListFilter,
    Search,
    Trash2,
    Edit3,
    Check,
    X,
    Filter,
    ChevronLeft,
    ChevronRight,
    ArrowUpDown,
    FileType,
    ArrowUp,
    ArrowDown,
    Play,
    Square,
} from 'lucide-react';

import {
    MsAccountInfo,
    AutoMigrationProfile,
    DailyMigrationQuota,
    MigrationJobDetail,
    ItemProgressPayload,
    MigrationItem,
    MigrationActivity,
    ScanProgressPayload,
    ProcessingLogEntry,
    OneDriveItem,
} from '../../types';
import { selectTransferLists } from './transferState';
import { ConnectionGate } from './ConnectionGate';
import { TransferList } from './TransferList';
import { ActivityStream } from './ActivityStream';
import { ProcessingLogPanel } from './ProcessingLogPanel';

interface AutoMigrationCenterProps {
    msAccount: MsAccountInfo | null;
    autoProfile: AutoMigrationProfile | null;
    dailyQuota: DailyMigrationQuota | null;
    currentJobDetail: MigrationJobDetail | null;
    itemProgress: ItemProgressPayload | null;
    migrationActivity: MigrationActivity[];
    loading: boolean;
    snapshotLoading: boolean;
    scanProgress: ScanProgressPayload | null;
    scanSnapshotItems: OneDriveItem[];
    processingLogs: ProcessingLogEntry[];
    onConnectMs: () => void;
    onSwitchMs: () => void;
    onOpenSettings: () => void;
    onRefresh: () => void;
    onResetScan: () => void;
    onStopScan: () => void;
    onClearProcessingLogs: () => void;
    onDeleteItem?: (jobId: number, itemId: number) => void;
    onRenameItem?: (jobId: number, itemId: number, newName: string) => void;
    onSyncSingleItem?: (jobId: number, itemId: number) => void;
    onSyncCheckpointItem?: (sourceItemId: string) => void;
}

const ITEMS_PER_PAGE = 50;

const getFileCategory = (filename: string): string => {
    const ext = filename.split('.').pop()?.toLowerCase() || '';
    if (['png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp', 'svg', 'heic'].includes(ext)) return 'image';
    if (['mp4', 'mkv', 'avi', 'mov', 'webm', 'flv', 'wmv', 'm4v', '3gp'].includes(ext)) return 'video';
    if (['pdf', 'doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx', 'txt', 'csv', 'md'].includes(ext)) return 'document';
    if (['zip', 'rar', '7z', 'tar', 'gz', 'bz2', 'iso'].includes(ext)) return 'archive';
    return 'other';
};

const getParentFolderDisplay = (item: MigrationItem): string => {
    if (item.source_path && item.source_path.includes('/')) {
        const parts = item.source_path.split('/');
        parts.pop(); // Remove file name
        const parent = parts.join('/');
        return parent ? parent : '/ (Root)';
    }
    return '/ (Root)';
};

const getStatusRank = (state: string): number => {
    switch (state) {
        case 'downloading': return 1;
        case 'uploading': return 2;
        case 'pending': return 3;
        case 'failed': return 4;
        case 'completed': return 5;
        case 'skipped_duplicate': return 6;
        default: return 7;
    }
};

const renderItemStatusBadge = (state: string) => {
    switch (state) {
        case 'completed':
            return <span className="px-2 py-0.5 rounded-full text-[10px] font-semibold bg-emerald-500/10 text-emerald-400 border border-emerald-500/20">Completed</span>;
        case 'skipped_duplicate':
            return <span className="px-2 py-0.5 rounded-full text-[10px] font-semibold bg-sky-500/10 text-sky-400 border border-sky-500/20">Skipped Duplicate</span>;
        case 'downloading':
            return <span className="px-2 py-0.5 rounded-full text-[10px] font-semibold bg-blue-500/10 text-blue-400 border border-blue-500/20 animate-pulse">Downloading</span>;
        case 'uploading':
            return <span className="px-2 py-0.5 rounded-full text-[10px] font-semibold bg-emerald-500/10 text-emerald-300 border border-emerald-500/20 animate-pulse">Uploading</span>;
        case 'failed':
            return <span className="px-2 py-0.5 rounded-full text-[10px] font-semibold bg-rose-500/10 text-rose-400 border border-rose-500/20">Failed</span>;
        default:
            return <span className="px-2 py-0.5 rounded-full text-[10px] font-semibold bg-slate-800 text-slate-400 border border-slate-700">Pending</span>;
    }
};

const formatBytes = (bytes: number) => {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
};

interface FileRowProps {
    file: MigrationItem;
    jobId: number;
    isEditing: boolean;
    editingName: string;
    setEditingName: (name: string) => void;
    onStartRename: (item: MigrationItem) => void;
    onSaveRename: (jobId: number, itemId: number) => void;
    onCancelRename: () => void;
    onDeleteItem?: (jobId: number, itemId: number) => void;
    onSyncSingleItem?: (jobId: number, itemId: number) => void;
    onSyncCheckpointItem?: (sourceItemId: string) => void;
    readOnly: boolean;
}

const FileRow = React.memo<FileRowProps>(({
    file,
    jobId,
    isEditing,
    editingName,
    setEditingName,
    onStartRename,
    onSaveRename,
    onCancelRename,
    onDeleteItem,
    onSyncSingleItem,
    onSyncCheckpointItem,
    readOnly,
}) => {
    const { t } = useTranslation();
    const parentFolder = getParentFolderDisplay(file);

    return (
        <tr className="hover:bg-slate-900/60 transition-colors group">
            {/* File Name / Edit Input */}
            <td className="py-2.5 px-4 font-medium text-slate-200 max-w-xs truncate">
                {isEditing && !readOnly ? (
                    <div className="flex items-center gap-1">
                        <input
                            type="text"
                            value={editingName}
                            onChange={(e) => setEditingName(e.target.value)}
                            className="px-2 py-1 bg-slate-900 border border-blue-500 rounded text-xs text-white focus:outline-none w-full"
                            autoFocus
                        />
                        <button
                            onClick={() => onSaveRename(jobId, file.id)}
                            className="p-1 text-emerald-400 hover:text-white shrink-0"
                            title="Lưu tên mới"
                        >
                            <Check className="w-4 h-4" />
                        </button>
                        <button
                            onClick={onCancelRename}
                            className="p-1 text-slate-400 hover:text-white shrink-0"
                            title="Hủy"
                        >
                            <X className="w-4 h-4" />
                        </button>
                    </div>
                ) : (
                    <div className="flex items-center gap-2 truncate">
                        <FileText className="w-4 h-4 text-blue-400 shrink-0" />
                        <span className="truncate" title={file.name}>{file.name}</span>
                    </div>
                )}
            </td>

            {/* Source Path Parent Folder Badge */}
            <td className="py-2.5 px-4 text-slate-400">
                <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md bg-slate-900 border border-slate-800 text-[11px] font-mono">
                    <Folder className="w-3.5 h-3.5 text-amber-400/80 shrink-0" />
                    <span className="truncate max-w-[220px]" title={parentFolder}>
                        {parentFolder}
                    </span>
                </span>
            </td>

            {/* Size */}
            <td className="py-2.5 px-4 text-slate-300 font-mono text-[11px]">
                {formatBytes(file.size_bytes)}
            </td>

            {/* Status */}
            <td className="py-2.5 px-4">
                {readOnly && file.job_id === 0 ? (
                    <span className="inline-flex items-center rounded-full border border-sky-500/20 bg-sky-500/10 px-2 py-0.5 text-[10px] font-semibold text-sky-300">
                        {t('migration.scanned_checkpoint', 'Đã quét')}
                    </span>
                ) : renderItemStatusBadge(file.state)}
            </td>

            {/* Actions */}
            <td className="py-2.5 px-4 text-right">
                <div className="flex items-center justify-end gap-1">
                    {readOnly ? (
                        file.job_id === 0 && file.source_item_id && onSyncCheckpointItem ? (
                            <button
                                onClick={() => onSyncCheckpointItem(file.source_item_id!)}
                                className="p-1.5 bg-blue-600/20 hover:bg-blue-600 text-blue-400 hover:text-white rounded-lg transition-colors border border-blue-500/30 flex items-center gap-1 text-[11px] font-medium"
                                title={t('migration.migrate_checkpoint_file', 'Migrate thủ công file này')}
                            >
                                <Play className="w-3 h-3 fill-current" />
                                <span>{t('migration.migrate_now', 'Migrate')}</span>
                            </button>
                        ) : file.state === 'failed' && onSyncSingleItem ? (
                            <button
                                onClick={() => onSyncSingleItem(file.job_id, file.id)}
                                className="p-1.5 bg-rose-600/20 hover:bg-rose-600 text-rose-400 hover:text-white rounded-lg transition-colors border border-rose-500/30 flex items-center gap-1 text-[11px] font-medium"
                                title={t('migration.retry_checkpoint_file', 'Thử migrate lại file này')}
                            >
                                <Play className="w-3 h-3 fill-current" />
                                <span>{t('migration.retry', 'Thử lại')}</span>
                            </button>
                        ) : file.state === 'completed' || file.state === 'skipped_duplicate' ? (
                            <span className="text-[10px] text-emerald-400">
                                {t('migration.migrated', 'Đã migrate')}
                            </span>
                        ) : (
                            <span className="text-[10px] text-slate-500">
                                {t('migration.migration_in_progress', 'Đang xử lý')}
                            </span>
                        )
                    ) : (
                        <>
                    {/* Manual Sync Single File Button */}
                    {onSyncSingleItem && (
                        <button
                            onClick={() => onSyncSingleItem(jobId, file.id)}
                            className="p-1.5 bg-blue-600/20 hover:bg-blue-600 text-blue-400 hover:text-white rounded-lg transition-colors border border-blue-500/30 flex items-center gap-1 text-[11px] font-medium"
                            title="Đồng bộ thủ công tệp này ngay"
                        >
                            <Play className="w-3 h-3 fill-current" />
                            <span>Sync</span>
                        </button>
                    )}

                    <button
                        onClick={() => onStartRename(file)}
                        className="p-1.5 hover:bg-slate-800 text-slate-400 hover:text-blue-400 rounded-lg transition-colors"
                        title="Sửa tên file"
                    >
                        <Edit3 className="w-3.5 h-3.5" />
                    </button>

                    {onDeleteItem && (
                        <button
                            onClick={() => onDeleteItem(jobId, file.id)}
                            className="p-1.5 hover:bg-rose-500/10 text-slate-400 hover:text-rose-400 rounded-lg transition-colors"
                            title="Xóa file trên OneDrive & Hủy đồng bộ"
                        >
                            <Trash2 className="w-3.5 h-3.5" />
                        </button>
                    )}
                        </>
                    )}
                </div>
            </td>
        </tr>
    );
});


export const AutoMigrationCenter: React.FC<AutoMigrationCenterProps> = ({
    msAccount,
    dailyQuota,
    currentJobDetail,
    itemProgress,
    migrationActivity,
    loading,
    snapshotLoading,
    scanProgress,
    scanSnapshotItems,
    processingLogs,
    onConnectMs,
    onSwitchMs,
    onOpenSettings,
    onRefresh,
    onResetScan,
    onStopScan,
    onClearProcessingLogs,
    onDeleteItem,
    onRenameItem,
    onSyncSingleItem,
    onSyncCheckpointItem,
}) => {

    const { t } = useTranslation();
    const [searchQuery, setSearchQuery] = useState<string>('');
    const [statusFilter, setStatusFilter] = useState<string>('all');
    const [fileTypeFilter, setFileTypeFilter] = useState<string>('all');
    const [sizeFilter, setSizeFilter] = useState<string>('all');
    const [sortOption, setSortOption] = useState<string>('newest');

    const [editingItemId, setEditingItemId] = useState<number | null>(null);
    const [editingName, setEditingName] = useState<string>('');
    const [currentPage, setCurrentPage] = useState<number>(1);

    // Calculate quota percentage
    const uploadedGB = dailyQuota ? (dailyQuota.uploaded_bytes / (1024 * 1024 * 1024)).toFixed(2) : '0.00';
    const limitGB = dailyQuota ? (dailyQuota.limit_bytes / (1024 * 1024 * 1024)).toFixed(0) : '250';
    const quotaPercent = dailyQuota && dailyQuota.limit_bytes > 0
        ? Math.min(100, (dailyQuota.uploaded_bytes / dailyQuota.limit_bytes) * 100)
        : 0;

    const isQuotaExceeded = quotaPercent >= 100;

    // Current job stats
    const stats = currentJobDetail?.stats;
    const isJobRunning = currentJobDetail?.job.state === 'running';
    const isAutoRunning = isJobRunning && currentJobDetail?.job.job_origin === 'auto';
    const isPipelineStarting = scanProgress?.phase === 'starting';
    const isScanActive = scanProgress?.phase === 'enumerating'
        || scanProgress?.phase === 'building_snapshot'
        || scanProgress?.phase === 'stopping';
    const isPipelineBusy = isPipelineStarting || isScanActive;
    const isScanStopping = scanProgress?.phase === 'stopping';
    const isScanStopped = scanProgress?.phase === 'stopped';
    const isPartialSnapshot = isScanStopped && scanSnapshotItems.length > 0;

    const handleStartRename = useCallback((item: MigrationItem) => {
        setEditingItemId(item.id);
        setEditingName(item.name);
    }, []);

    const handleSaveRename = useCallback((jobId: number, itemId: number) => {
        if (editingName.trim() && onRenameItem) {
            onRenameItem(jobId, itemId, editingName.trim());
        }
        setEditingItemId(null);
        setEditingName('');
    }, [editingName, onRenameItem]);

    const handleCancelRename = useCallback(() => {
        setEditingItemId(null);
        setEditingName('');
    }, []);

    const handleHeaderSort = (field: 'name' | 'path' | 'size' | 'status') => {
        setCurrentPage(1);
        if (field === 'name') {
            setSortOption(prev => prev === 'name_asc' ? 'name_desc' : 'name_asc');
        } else if (field === 'path') {
            setSortOption(prev => prev === 'path_asc' ? 'path_desc' : 'path_asc');
        } else if (field === 'size') {
            setSortOption(prev => prev === 'size_desc' ? 'size_asc' : 'size_desc');
        } else if (field === 'status') {
            setSortOption(prev => prev === 'status_asc' ? 'status_desc' : 'status_asc');
        }
    };

    // Filter ONLY actual files (exclude folder entries)
    const partialSnapshotEntries = useMemo<MigrationItem[]>(
        () => {
            const activeItemsBySourceId = new Map(
                (currentJobDetail?.files || [])
                    .filter(item => item.source_item_id)
                    .map(item => [item.source_item_id!, item]),
            );
            return scanSnapshotItems.map((item, index) => {
                const activeItem = activeItemsBySourceId.get(item.id);
                if (activeItem) {
                    const liveState = itemProgress?.job_id === activeItem.job_id
                        && itemProgress.item_id === activeItem.id
                        ? itemProgress.phase
                        : activeItem.state;
                    return { ...activeItem, state: liveState, queue_position: index };
                }
                return {
                    id: -(index + 1),
                    job_id: 0,
                    item_type: item.item_type,
                    name: item.name,
                    source_path: item.path || item.name,
                    source_item_id: item.id,
                    size_bytes: item.size,
                    source_etag: item.etag,
                    source_last_modified: item.last_modified,
                    source_fingerprint_type: item.quickxor_hash
                        ? 'onedrive_quickxor'
                        : item.sha1_hash
                            ? 'onedrive_sha1'
                            : null,
                    source_fingerprint_value: item.quickxor_hash || item.sha1_hash || null,
                    state: 'pending',
                    attempt_count: 0,
                    created_at: 0,
                    queue_position: index,
                };
            });
        },
        [currentJobDetail?.files, itemProgress, scanSnapshotItems],
    );

    const rawFileEntries = useMemo(() => {
        const source = isPartialSnapshot
            ? partialSnapshotEntries
            : currentJobDetail?.files || [];
        return source.filter(item => item.item_type === 'file');
    }, [currentJobDetail?.files, isPartialSnapshot, partialSnapshotEntries]);

    // Multi-Filter & Sort with useMemo
    const filteredFiles = useMemo(() => {
        let files = [...rawFileEntries];

        // Search Query
        if (searchQuery.trim()) {
            const query = searchQuery.toLowerCase().trim();
            files = files.filter(file => {
                const parentFolder = getParentFolderDisplay(file);
                return file.name.toLowerCase().includes(query) ||
                    file.source_path.toLowerCase().includes(query) ||
                    parentFolder.toLowerCase().includes(query);
            });
        }

        // Status Filter
        if (statusFilter !== 'all') {
            files = files.filter(file => file.state === statusFilter);
        }

        // File Type Filter
        if (fileTypeFilter !== 'all') {
            files = files.filter(file => getFileCategory(file.name) === fileTypeFilter);
        }

        // File Size Filter
        if (sizeFilter !== 'all') {
            files = files.filter(file => {
                const s = file.size_bytes;
                if (sizeFilter === 'small') return s < 10 * 1024 * 1024;
                if (sizeFilter === 'medium') return s >= 10 * 1024 * 1024 && s < 100 * 1024 * 1024;
                if (sizeFilter === 'large') return s >= 100 * 1024 * 1024 && s < 1024 * 1024 * 1024;
                if (sizeFilter === 'huge') return s >= 1024 * 1024 * 1024;
                return true;
            });
        }

        // Sorting
        files.sort((a, b) => {
            switch (sortOption) {
                case 'newest': return b.id - a.id;
                case 'oldest': return a.id - b.id;
                case 'size_desc': return b.size_bytes - a.size_bytes;
                case 'size_asc': return a.size_bytes - b.size_bytes;
                case 'name_asc': return a.name.localeCompare(b.name);
                case 'name_desc': return b.name.localeCompare(a.name);
                case 'path_asc': return getParentFolderDisplay(a).localeCompare(getParentFolderDisplay(b));
                case 'path_desc': return getParentFolderDisplay(b).localeCompare(getParentFolderDisplay(a));
                case 'type_asc': return getFileCategory(a.name).localeCompare(getFileCategory(b.name));
                case 'status_asc': return getStatusRank(a.state) - getStatusRank(b.state);
                case 'status_desc': return getStatusRank(b.state) - getStatusRank(a.state);
                default: return b.id - a.id;
            }
        });

        return files;
    }, [rawFileEntries, searchQuery, statusFilter, fileTypeFilter, sizeFilter, sortOption]);

    const totalPages = Math.max(1, Math.ceil(filteredFiles.length / ITEMS_PER_PAGE));

    const paginatedFiles = useMemo(() => {
        const start = (currentPage - 1) * ITEMS_PER_PAGE;
        return filteredFiles.slice(start, start + ITEMS_PER_PAGE);
    }, [filteredFiles, currentPage]);

    const transferLists = useMemo(
        () => selectTransferLists(currentJobDetail, itemProgress),
        [currentJobDetail, itemProgress],
    );

    const renderSortIcon = (field: 'name' | 'path' | 'size' | 'status') => {
        if (field === 'name') {
            if (sortOption === 'name_asc') return <ArrowUp className="w-3 h-3 text-blue-400" />;
            if (sortOption === 'name_desc') return <ArrowDown className="w-3 h-3 text-blue-400" />;
        }
        if (field === 'path') {
            if (sortOption === 'path_asc') return <ArrowUp className="w-3 h-3 text-blue-400" />;
            if (sortOption === 'path_desc') return <ArrowDown className="w-3 h-3 text-blue-400" />;
        }
        if (field === 'size') {
            if (sortOption === 'size_desc') return <ArrowDown className="w-3 h-3 text-blue-400" />;
            if (sortOption === 'size_asc') return <ArrowUp className="w-3 h-3 text-blue-400" />;
        }
        if (field === 'status') {
            if (sortOption === 'status_asc') return <ArrowUp className="w-3 h-3 text-blue-400" />;
            if (sortOption === 'status_desc') return <ArrowDown className="w-3 h-3 text-blue-400" />;
        }
        return <ArrowUpDown className="w-3 h-3 text-slate-600 opacity-0 group-hover:opacity-100 transition-opacity" />;
    };

    if (!msAccount) {
        return <ConnectionGate loading={loading} onConnect={onConnectMs} />;
    }

    return (
        <div className="space-y-6">
            {/* Top Control & Status Banner */}
            <div className="bg-slate-900 border border-slate-800 rounded-2xl p-6 shadow-xl relative overflow-hidden">
                <div className="absolute top-0 right-0 w-96 h-96 bg-blue-500/5 rounded-full blur-3xl pointer-events-none" />

                <div className="flex flex-col md:flex-row items-start md:items-center justify-between gap-6 relative z-10">
                    <div className="flex items-center gap-4">
                        <div className="w-14 h-14 rounded-2xl bg-gradient-to-br from-blue-600 to-indigo-700 flex items-center justify-center shadow-lg shadow-blue-500/20 shrink-0">
                            <Zap className="w-7 h-7 text-white animate-pulse" />
                        </div>
                        <div>
                            <div className="flex items-center gap-2">
                                <h2 className="text-xl font-bold text-white tracking-tight">
                                    {t('migration.auto_title', 'Smart Auto-Migration Center')}
                                </h2>
                                <span className="inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-semibold bg-blue-500/10 text-blue-400 border border-blue-500/20">
                                    Zero-Click Sync
                                </span>
                            </div>
                            <p className="text-sm text-slate-400 mt-1">
                                {t('migration.auto_subtitle', 'Tự động quét và đồng bộ dữ liệu từ OneDrive sang Telegram Drive ngầm 100%')}
                            </p>
                        </div>
                    </div>

                    {/* Explicit sequential pipeline control */}
                    <div className="flex items-center gap-3 bg-slate-950/80 p-3 rounded-2xl border border-slate-800 shrink-0">
                        <span className="text-sm font-medium text-slate-300">
                            {isAutoRunning || isPipelineBusy
                                ? t('migration.pipeline_running', 'Đang quét và migrate tuần tự')
                                : t('migration.pipeline_idle', 'Sẵn sàng quét và migrate')}
                        </span>
                        <button
                            onClick={onRefresh}
                            disabled={loading || isAutoRunning || isPipelineBusy}
                            className="inline-flex items-center gap-1.5 px-3 py-2 bg-blue-600 hover:bg-blue-500 text-white rounded-lg text-xs font-semibold transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
                        >
                            <Play className="w-3.5 h-3.5 fill-current" />
                            {t('migration.start_pipeline', 'Quét & migrate')}
                        </button>
                    </div>
                </div>

                {/* Account & Settings Row */}
                <div className="grid grid-cols-1 md:grid-cols-3 gap-4 mt-6 pt-6 border-t border-slate-800/80">
                    {/* Account Card */}
                    <div className="bg-slate-950/60 rounded-xl p-4 border border-slate-800 flex items-center justify-between">
                        <div className="flex items-center gap-3">
                            <Cloud className="w-5 h-5 text-sky-400 shrink-0" />
                            <div className="truncate">
                                <p className="text-xs text-slate-400 font-medium">{t('migration.ms_account', 'OneDrive Account')}</p>
                                <p className="text-sm font-semibold text-slate-200 truncate">
                                    {msAccount ? msAccount.account_name : t('migration.not_connected', 'Chưa kết nối')}
                                </p>
                            </div>
                        </div>
                        {msAccount ? (
                            <div className="flex items-center gap-2 shrink-0">
                                <span className="flex h-2.5 w-2.5 relative">
                                    <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75"></span>
                                    <span className="relative inline-flex rounded-full h-2.5 w-2.5 bg-emerald-500"></span>
                                </span>
                                <button
                                    onClick={onSwitchMs}
                                    disabled={loading || snapshotLoading || isJobRunning}
                                    className="px-2.5 py-1.5 bg-slate-900 hover:bg-slate-800 text-slate-300 hover:text-white rounded-lg text-xs font-semibold border border-slate-700 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
                                >
                                    {t('migration.switch_account', 'Đổi tài khoản')}
                                </button>
                            </div>
                        ) : (
                            <button
                                onClick={onConnectMs}
                                className="px-3 py-1 bg-blue-600 hover:bg-blue-500 text-white rounded-lg text-xs font-semibold transition-colors"
                            >
                                Connect
                            </button>
                        )}
                    </div>

                    {/* Daily Quota Card */}
                    <div className="bg-slate-950/60 rounded-xl p-4 border border-slate-800">
                        <div className="flex items-center justify-between mb-2">
                            <div className="flex items-center gap-2">
                                <HardDrive className="w-4 h-4 text-purple-400" />
                                <span className="text-xs font-medium text-slate-300">
                                    {t('migration.daily_quota', 'Quota hôm nay (Giới hạn 250GB)')}
                                </span>
                            </div>
                            <span className="text-xs font-bold text-purple-400">
                                {uploadedGB} / {limitGB} GB
                            </span>
                        </div>
                        <div className="w-full bg-slate-800 rounded-full h-2 overflow-hidden">
                            <div
                                className={`h-full transition-all duration-500 ${
                                    isQuotaExceeded ? 'bg-rose-500' : quotaPercent > 80 ? 'bg-amber-500' : 'bg-purple-500'
                                }`}
                                style={{ width: `${quotaPercent}%` }}
                            />
                        </div>
                        {isQuotaExceeded && (
                            <p className="text-[11px] text-amber-400 mt-1.5 flex items-center gap-1">
                                <ShieldAlert className="w-3.5 h-3.5 shrink-0" />
                                Đã đạt giới hạn an toàn 250GB/ngày. Tự động dừng để bảo vệ nick Telegram.
                            </p>
                        )}
                    </div>

                    {/* Quick Settings & Controls */}
                    <div className="bg-slate-950/60 rounded-xl p-4 border border-slate-800 flex items-center justify-between">
                        <div className="flex items-center gap-2 text-xs text-slate-400">
                            <Activity className="w-4 h-4 text-emerald-400" />
                            <span>
                                {isAutoRunning
                                    ? t('migration.status_running', 'Đang tự động đồng bộ...')
                                    : isJobRunning
                                        ? t('migration.status_manual_running', 'Đang migrate thủ công...')
                                    : t('migration.status_idle', 'Hệ thống ở trạng thái chờ')}
                            </span>
                        </div>
                        <div className="flex items-center gap-2">
                            {isScanActive ? (
                                <button
                                    onClick={onStopScan}
                                    disabled={false}
                                    className="inline-flex items-center gap-1.5 px-2.5 py-2 bg-rose-500/10 hover:bg-rose-500/20 text-rose-400 rounded-lg transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
                                    title={t('migration.stop_scan', 'Dừng quét')}
                                >
                                    <Square className="w-4 h-4" />
                                    <span>
                                        {isScanStopping
                                            ? t('migration.stopping_scan', 'Đang dừng...')
                                            : t('migration.stop_scan', 'Dừng quét')}
                                    </span>
                                </button>
                            ) : (
                                <>
                                    <button
                                        onClick={onRefresh}
                                        disabled={loading || isJobRunning || isPipelineBusy}
                                        className="inline-flex items-center gap-1.5 px-2.5 py-2 hover:bg-slate-800 text-slate-400 hover:text-white rounded-lg transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
                                        title={isScanStopped
                                            ? t('migration.resume_scan', 'Tiếp tục quét')
                                            : t('migration.rescan', 'Quét lại')}
                                    >
                                        <RefreshCw className="w-4 h-4" />
                                        <span>
                                            {isScanStopped
                                                ? t('migration.resume_scan', 'Tiếp tục quét')
                                                : t('migration.rescan', 'Quét lại')}
                                        </span>
                                    </button>
                                    {isScanStopped && (
                                        <button
                                            onClick={onResetScan}
                                            disabled={loading || isJobRunning}
                                            className="inline-flex items-center gap-1.5 px-2.5 py-2 bg-rose-500/10 hover:bg-rose-500/20 text-rose-400 rounded-lg transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
                                            title={t('migration.reset_scan', 'Xóa tất cả & quét lại')}
                                        >
                                            <Trash2 className="w-4 h-4" />
                                            <span>{t('migration.reset_scan', 'Xóa tất cả & quét lại')}</span>
                                        </button>
                                    )}
                                </>
                            )}
                            <button
                                onClick={onOpenSettings}
                                className="p-2 hover:bg-slate-800 text-slate-400 hover:text-white rounded-lg transition-colors"
                                title="Advanced Settings"
                            >
                                <Settings className="w-4 h-4" />
                            </button>
                        </div>
                    </div>
                </div>
            </div>

            <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
                <TransferList
                    phase="downloading"
                    item={transferLists.downloading[0] ?? null}
                    progress={itemProgress}
                />
                <TransferList
                    phase="uploading"
                    item={transferLists.uploading[0] ?? null}
                    progress={itemProgress}
                />
            </div>

            {/* Flattened File List View (Danh Sách File Trải Phẳng theo File & Thao Tác Trực Tiếp) */}
            <div className="bg-slate-900 border border-slate-800 rounded-2xl p-5 space-y-4">
                <div className="flex flex-col md:flex-row md:items-center justify-between gap-4">
                    <div className="flex items-center gap-2">
                        <ListFilter className="w-5 h-5 text-blue-400" />
                        <div>
                            <h3 className="text-sm font-bold text-white flex items-center gap-2">
                                {t('migration.flattened_files_title', 'Danh Sách File OneDrive Trải Phẳng')}
                                <span className="text-xs font-normal text-slate-400 bg-slate-950 px-2 py-0.5 rounded-full border border-slate-800">
                                    {filteredFiles.length} tệp
                                </span>
                            </h3>
                            <p className="text-xs text-slate-400">
                                Danh sách tất cả file trên OneDrive (Cột Thư Mục Gốc thể hiện đường dẫn chứa file)
                            </p>
                        </div>
                    </div>

                    {/* Filter & Sort Controls Row */}
                    <div className="flex flex-wrap items-center gap-2">
                        {/* Search Input */}
                        <div className="relative">
                            <Search className="w-4 h-4 absolute left-3 top-2.5 text-slate-400" />
                            <input
                                type="text"
                                value={searchQuery}
                                onChange={(e) => {
                                    setSearchQuery(e.target.value);
                                    setCurrentPage(1);
                                }}
                                placeholder="Tìm kiếm file hoặc thư mục..."
                                className="pl-9 pr-3 py-1.5 bg-slate-950 border border-slate-800 rounded-xl text-xs text-slate-200 focus:outline-none focus:border-blue-500 w-48"
                            />
                        </div>

                        {/* Status Filter */}
                        <div className="flex items-center gap-1 bg-slate-950 px-2.5 py-1 rounded-xl border border-slate-800 text-xs">
                            <Filter className="w-3.5 h-3.5 text-slate-400" />
                            <select
                                value={statusFilter}
                                onChange={(e) => {
                                    setStatusFilter(e.target.value);
                                    setCurrentPage(1);
                                }}
                                className="bg-transparent text-slate-200 focus:outline-none cursor-pointer"
                            >
                                <option value="all" className="bg-slate-900">Trạng thái: Tất cả</option>
                                <option value="pending" className="bg-slate-900">Chờ xử lý</option>
                                <option value="completed" className="bg-slate-900">Đã xong</option>
                                <option value="skipped_duplicate" className="bg-slate-900">Bỏ qua trùng</option>
                                <option value="failed" className="bg-slate-900">Thất bại</option>
                            </select>
                        </div>

                        {/* File Type Filter */}
                        <div className="flex items-center gap-1 bg-slate-950 px-2.5 py-1 rounded-xl border border-slate-800 text-xs">
                            <FileType className="w-3.5 h-3.5 text-slate-400" />
                            <select
                                value={fileTypeFilter}
                                onChange={(e) => {
                                    setFileTypeFilter(e.target.value);
                                    setCurrentPage(1);
                                }}
                                className="bg-transparent text-slate-200 focus:outline-none cursor-pointer"
                            >
                                <option value="all" className="bg-slate-900">Loại: Tất cả</option>
                                <option value="image" className="bg-slate-900">🖼️ Hình ảnh</option>
                                <option value="video" className="bg-slate-900">🎬 Video</option>
                                <option value="document" className="bg-slate-900">📄 Tài liệu</option>
                                <option value="archive" className="bg-slate-900">📦 File nén</option>
                                <option value="other" className="bg-slate-900">📁 Khác</option>
                            </select>
                        </div>

                        {/* File Size Filter */}
                        <div className="flex items-center gap-1 bg-slate-950 px-2.5 py-1 rounded-xl border border-slate-800 text-xs">
                            <HardDrive className="w-3.5 h-3.5 text-slate-400" />
                            <select
                                value={sizeFilter}
                                onChange={(e) => {
                                    setSizeFilter(e.target.value);
                                    setCurrentPage(1);
                                }}
                                className="bg-transparent text-slate-200 focus:outline-none cursor-pointer"
                            >
                                <option value="all" className="bg-slate-900">Dung lượng: Tất cả</option>
                                <option value="small" className="bg-slate-900">&lt; 10 MB</option>
                                <option value="medium" className="bg-slate-900">10 MB - 100 MB</option>
                                <option value="large" className="bg-slate-900">100 MB - 1 GB</option>
                                <option value="huge" className="bg-slate-900">&gt; 1 GB</option>
                            </select>
                        </div>

                        {/* Extended Sorting Options */}
                        <div className="flex items-center gap-1 bg-slate-950 px-2.5 py-1 rounded-xl border border-slate-800 text-xs">
                            <ArrowUpDown className="w-3.5 h-3.5 text-blue-400" />
                            <select
                                value={sortOption}
                                onChange={(e) => {
                                    setSortOption(e.target.value);
                                    setCurrentPage(1);
                                }}
                                className="bg-transparent text-slate-200 focus:outline-none cursor-pointer font-medium"
                            >
                                <option value="newest" className="bg-slate-900">Xếp: Mới nhất</option>
                                <option value="oldest" className="bg-slate-900">Xếp: Cũ nhất</option>
                                <option value="name_asc" className="bg-slate-900">Tên file (A → Z)</option>
                                <option value="name_desc" className="bg-slate-900">Tên file (Z → A)</option>
                                <option value="size_desc" className="bg-slate-900">Dung lượng (Lớn → Nhỏ)</option>
                                <option value="size_asc" className="bg-slate-900">Dung lượng (Nhỏ → Lớn)</option>
                                <option value="path_asc" className="bg-slate-900">Thư mục gốc (A → Z)</option>
                                <option value="type_asc" className="bg-slate-900">Loại file (Hình ảnh, Video...)</option>
                                <option value="status_asc" className="bg-slate-900">Trạng thái đồng bộ</option>
                            </select>
                        </div>
                    </div>
                </div>

                {isScanStopped && scanProgress && (
                    <p className="text-[11px] text-amber-300/90">
                        {t('migration.scan_stopped_summary', {
                            pages: scanProgress.pages_scanned,
                            files: scanProgress.discovered_files,
                            folders: scanProgress.discovered_folders,
                        })}
                    </p>
                )}

                {/* Flattened File Table with Clickable Sorting Headers & Pagination */}
                <div className="bg-slate-950/80 border border-slate-800/80 rounded-xl overflow-hidden font-sans text-xs space-y-2">
                    <div className="max-h-96 overflow-y-auto custom-scrollbar">
                        <table className="w-full text-left border-collapse">
                            <thead>
                                <tr className="border-b border-slate-800 bg-slate-950 text-slate-400 font-semibold sticky top-0 z-10 select-none">
                                    <th
                                        onClick={() => handleHeaderSort('name')}
                                        className="py-3 px-4 cursor-pointer hover:bg-slate-900 hover:text-white transition-colors group"
                                    >
                                        <div className="flex items-center gap-1.5">
                                            <span>Tên File</span>
                                            {renderSortIcon('name')}
                                        </div>
                                    </th>
                                    <th
                                        onClick={() => handleHeaderSort('path')}
                                        className="py-3 px-4 cursor-pointer hover:bg-slate-900 hover:text-white transition-colors group"
                                    >
                                        <div className="flex items-center gap-1.5">
                                            <span>Thư Mục Gốc</span>
                                            {renderSortIcon('path')}
                                        </div>
                                    </th>
                                    <th
                                        onClick={() => handleHeaderSort('size')}
                                        className="py-3 px-4 cursor-pointer hover:bg-slate-900 hover:text-white transition-colors group"
                                    >
                                        <div className="flex items-center gap-1.5">
                                            <span>Kích Thước</span>
                                            {renderSortIcon('size')}
                                        </div>
                                    </th>
                                    <th
                                        onClick={() => handleHeaderSort('status')}
                                        className="py-3 px-4 cursor-pointer hover:bg-slate-900 hover:text-white transition-colors group"
                                    >
                                        <div className="flex items-center gap-1.5">
                                            <span>Trạng Thái</span>
                                            {renderSortIcon('status')}
                                        </div>
                                    </th>
                                    <th className="py-3 px-4 text-right">Thao Tác</th>
                                </tr>
                            </thead>
                            <tbody className="divide-y divide-slate-800/60">
                                {snapshotLoading ? (
                                    <tr>
                                        <td colSpan={5} className="py-12 text-center">
                                            <div className="flex flex-col items-center gap-3 text-slate-300">
                                                <RefreshCw className="w-7 h-7 text-blue-400 animate-spin" />
                                                <div>
                                                    <p className="text-sm font-semibold">
                                                        {t('migration.loading_onedrive_files', 'Đang lấy cây thư mục từ OneDrive...')}
                                                    </p>
                                                    <p className="text-xs text-slate-500 mt-1">
                                                        {scanProgress
                                                            ? t('migration.scan_progress_summary', {
                                                                pages: scanProgress.pages_scanned,
                                                                files: scanProgress.discovered_files,
                                                                folders: scanProgress.discovered_folders,
                                                                seconds: Math.floor(scanProgress.elapsed_ms / 1000),
                                                            })
                                                            : t('migration.loading_onedrive_files_description', 'Danh sách file sẽ xuất hiện ngay khi snapshot hoàn tất.')}
                                                    </p>
                                                    {scanProgress?.phase === 'building_snapshot' && (
                                                        <p className="text-xs text-blue-300 mt-1">
                                                            {t('migration.building_snapshot', 'Đang sắp xếp và lưu danh sách file...')}
                                                        </p>
                                                    )}
                                                </div>
                                                <div className="w-64 max-w-full h-1.5 overflow-hidden rounded-full bg-slate-800">
                                                    <div className="h-full w-1/3 rounded-full bg-blue-500 animate-[pulse_1s_ease-in-out_infinite]" />
                                                </div>
                                            </div>
                                        </td>
                                    </tr>
                                ) : paginatedFiles.length > 0 ? (
                                    paginatedFiles.map((file) => (
                                        <FileRow
                                            key={file.id}
                                            file={file}
                                            jobId={currentJobDetail?.job.id || 0}
                                            isEditing={editingItemId === file.id}
                                            editingName={editingName}
                                            setEditingName={setEditingName}
                                            onStartRename={handleStartRename}
                                            onSaveRename={handleSaveRename}
                                            onCancelRename={handleCancelRename}
                                            onDeleteItem={onDeleteItem}
                                            onSyncSingleItem={onSyncSingleItem}
                                            onSyncCheckpointItem={onSyncCheckpointItem}
                                            readOnly={isPartialSnapshot}
                                        />

                                    ))
                                ) : (
                                    <tr>
                                        <td colSpan={5} className="py-8 text-center text-slate-500">
                                            {searchQuery || statusFilter !== 'all' || fileTypeFilter !== 'all' || sizeFilter !== 'all'
                                                ? 'Không tìm thấy file phù hợp với các bộ lọc đã chọn.'
                                                : t('migration.no_snapshot_files', 'Chưa có snapshot OneDrive. Nhấn “Quét lại” để thử lấy danh sách file.')}
                                        </td>
                                    </tr>
                                )}
                            </tbody>
                        </table>
                    </div>

                    {/* Pagination Controls */}
                    {filteredFiles.length > ITEMS_PER_PAGE && (
                        <div className="flex items-center justify-between p-3 border-t border-slate-800/80 text-xs text-slate-400 bg-slate-950">
                            <span>
                                Hiển thị {((currentPage - 1) * ITEMS_PER_PAGE) + 1} - {Math.min(currentPage * ITEMS_PER_PAGE, filteredFiles.length)} / {filteredFiles.length} tệp
                            </span>
                            <div className="flex items-center gap-2">
                                <button
                                    onClick={() => setCurrentPage(prev => Math.max(1, prev - 1))}
                                    disabled={currentPage === 1}
                                    className="p-1 rounded bg-slate-900 border border-slate-800 hover:bg-slate-800 disabled:opacity-40 disabled:cursor-not-allowed"
                                >
                                    <ChevronLeft className="w-4 h-4" />
                                </button>
                                <span>Trang {currentPage} / {totalPages}</span>
                                <button
                                    onClick={() => setCurrentPage(prev => Math.min(totalPages, prev + 1))}
                                    disabled={currentPage === totalPages}
                                    className="p-1 rounded bg-slate-900 border border-slate-800 hover:bg-slate-800 disabled:opacity-40 disabled:cursor-not-allowed"
                                >
                                    <ChevronRight className="w-4 h-4" />
                                </button>
                            </div>
                        </div>
                    )}
                </div>
            </div>

            {/* Live Activity & Completed Stats */}
            <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
                {/* Stats Summary */}
                <div className="bg-slate-900 border border-slate-800 rounded-2xl p-5 space-y-4">
                    <h3 className="text-sm font-bold text-white flex items-center gap-2">
                        <Activity className="w-4 h-4 text-blue-400" />
                        {t('migration.stats_overview', 'Thống kê Tiến độ')}
                    </h3>
                    <div className="space-y-3">
                        <div className="flex justify-between items-center text-xs p-2.5 bg-slate-950 rounded-xl">
                            <span className="text-slate-400">Đã hoàn thành</span>
                            <span className="font-bold text-emerald-400">
                                {stats ? `${stats.completed_files} files` : '0 files'}
                            </span>
                        </div>
                        <div className="flex justify-between items-center text-xs p-2.5 bg-slate-950 rounded-xl">
                            <span className="text-slate-400">Bỏ qua trùng lặp</span>
                            <span className="font-bold text-sky-400">
                                {stats ? `${stats.skipped_duplicates} files` : '0 files'}
                            </span>
                        </div>
                        <div className="flex justify-between items-center text-xs p-2.5 bg-slate-950 rounded-xl">
                            <span className="text-slate-400">Đang chờ xử lý</span>
                            <span className="font-bold text-amber-400">
                                {stats ? `${stats.pending_files} files` : '0 files'}
                            </span>
                        </div>
                        <div className="flex justify-between items-center text-xs p-2.5 bg-slate-950 rounded-xl">
                            <span className="text-slate-400">Thất bại / Lỗi</span>
                            <span className="font-bold text-rose-400">
                                {stats ? `${stats.failed_files} files` : '0 files'}
                            </span>
                        </div>
                    </div>
                </div>

                {/* Live File Stream */}
                <div className="md:col-span-2 bg-slate-900 border border-slate-800 rounded-2xl p-5 space-y-4">
                    <div className="flex items-center justify-between">
                        <h3 className="text-sm font-bold text-white flex items-center gap-2">
                            <CheckCircle2 className="w-4 h-4 text-emerald-400" />
                            {t('migration.recent_activity', 'Nhật ký Hoạt động Tự động')}
                        </h3>
                        <span className="text-xs text-slate-500 font-medium">Real-time Stream</span>
                    </div>

                    <ActivityStream entries={migrationActivity} />
                </div>
            </div>

            <ProcessingLogPanel
                entries={processingLogs}
                onClear={onClearProcessingLogs}
            />
        </div>
    );
};
