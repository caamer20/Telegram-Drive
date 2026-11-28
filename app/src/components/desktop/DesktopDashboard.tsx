import { clearImageMemoryCaches } from '../../services/imagePreviewCache';
import { sourceFolder } from '../../services/fileIdentity';
import { lazy, useState, useEffect, useLayoutEffect, useCallback, useMemo, useRef } from 'react';
import { ORGANIZE_FILES_EVENT } from '../../services/workspace';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { useTranslation } from 'react-i18next';
import {
    closestCenter,
    DndContext,
    DragEndEvent,
    DragOverlay,
    DragStartEvent,
    KeyboardSensor,
    PointerSensor,
    useSensor,
    useSensors,
} from '@dnd-kit/core';
import { arrayMove, sortableKeyboardCoordinates } from '@dnd-kit/sortable';

import { TelegramFile, BandwidthStats, type SmartView, type StorageInsightResult, type DriveStats } from '../../types';
import { tempDir } from '@tauri-apps/api/path';
import { formatBytes, isMediaFile, isPdfFile, isArchiveFile, isImageFile, copyToClipboard, sanitizeFilename } from '../../utils';

// Components
import { Sidebar } from './dashboard/Sidebar';
import { TopBar } from './dashboard/TopBar';
import { FileExplorer, type SortDirection, type SortField } from './dashboard/FileExplorer';
import { TransferCenter } from './dashboard/TransferCenter';
import { ExternalDropBlocker } from './dashboard/ExternalDropBlocker';
import type { SettingsTab } from './dashboard/SettingsModal';
import { RenameFileModal } from './dashboard/RenameFileModal';
import { DesktopAdBanner } from './dashboard/DesktopAdBanner';
import { KeyboardShortcutsDialog } from './dashboard/KeyboardShortcutsDialog';
import { DriveConceptTour } from './dashboard/DriveConceptTour';
import { LazyFeatureBoundary } from '../shared/LazyFeatureBoundary';
import { SyncDashboard } from './sync/SyncDashboard';
import { Files, Library, Link, Copy, Check, X, Loader2, Share2, PieChart, Trash2, Sparkles, Search, FolderPlus, FolderUp, Download, ArrowRightLeft, CheckSquare } from 'lucide-react';

// Hooks
import { useTelegramConnection } from '../../hooks/useTelegramConnection';
import { useFileOperations } from '../../hooks/useFileOperations';
import { useFileUpload } from '../../hooks/useFileUpload';
import { useFileDownload } from '../../hooks/useFileDownload';
import { useFileSharing } from '../../hooks/useFileSharing';
import { useKeyboardShortcuts } from '../../hooks/useKeyboardShortcuts';
import { useGlobalFileSearch } from '../../hooks/useGlobalFileSearch';
import { useSettings } from '../../context/SettingsContext';
import { useActionScope } from '../../hooks/useActionScope';
import { useSupporter } from '../../context/SupporterContext';
import { DEFAULT_SEARCH_FILTERS, filterAndRankFiles, type FileSearchFilters } from '../../services/fileSearch';
import { useSupporterPrompt } from '../../hooks/useSupporterPrompt';
import { type SupporterPromptTrigger } from '../../services/supporterVisibility';
import { markDesktopFrontendReady, markDesktopFrontendUnready, type DesktopNavigationRequest } from '../../services/desktopLifecycle';
import { fileQueryKey, invalidateFolderFileQueries, refreshFolderFiles, updateFileQueryData } from '../../services/fileListRefresh';
import { getAdjacentPreview, previewFileKey, samePreviewFile as sameFile } from '../../services/previewNavigation';
import i18n from '../../i18n';

const LazyRenameFolderModal = lazy(() => import('./dashboard/RenameFolderModal').then(module => ({ default: module.RenameFolderModal })));
const LazyShareDialog = lazy(() => import('./dashboard/ShareDialog').then(module => ({ default: module.ShareDialog })));
const RemoteUploadModal = lazy(() => import('./dashboard/RemoteUploadModal').then(module => ({ default: module.RemoteUploadModal })));
const LazyPreviewModal = lazy(() => import('./dashboard/PreviewModal').then((module) => ({ default: module.PreviewModal })));
const LazyMediaPlayer = lazy(() => import('./dashboard/MediaPlayer').then((module) => ({ default: module.MediaPlayer })));
const LazyPdfViewer = lazy(() => import('./dashboard/PdfViewer').then((module) => ({ default: module.PdfViewer })));
const LazyArchiveViewerModal = lazy(() => import('./dashboard/ArchiveViewerModal').then((module) => ({ default: module.ArchiveViewerModal })));
const LazySettingsModal = lazy(() => import('./dashboard/SettingsModal').then((module) => ({ default: module.SettingsModal })));
const SupporterOfferDialog = lazy(() => import('../shared/SupporterOfferDialog').then(module => ({ default: module.SupporterOfferDialog })));
const LazyHelpCenterDialog = lazy(() => import('./dashboard/HelpCenterDialog').then((module) => ({ default: module.HelpCenterDialog })));
const LazyWorkspaceHub = lazy(() => import('../workspace/WorkspaceHub').then(module => ({ default: module.WorkspaceHub })));
const LazyHomeDashboard = lazy(() => import('./dashboard/HomeDashboard').then(module => ({ default: module.HomeDashboard })));
const LazyTrashView = lazy(() => import('./dashboard/TrashView').then(module => ({ default: module.TrashView })));
const LazyStorageStatsModal = lazy(() => import('./dashboard/StorageStatsModal').then(module => ({ default: module.StorageStatsModal })));
const LazyPropertiesModal = lazy(() => import('./dashboard/PropertiesModal').then(module => ({ default: module.PropertiesModal })));
const LazyVaultModal = lazy(() => import('./dashboard/VaultModal').then(module => ({ default: module.VaultModal })));
const LazyFolderTreePicker = lazy(() => import('./dashboard/FolderTreePicker').then(module => ({ default: module.FolderTreePicker })));


export function Dashboard({ onLogout }: { onLogout: () => void }) {
    const queryClient = useQueryClient();
    const { t } = useTranslation();
    const [workspaceKeys, setWorkspaceKeys] = useState<string[] | null>(null);
    useEffect(() => {
        const organize = (event: Event) => setWorkspaceKeys((event as CustomEvent<{ keys: string[] }>).detail.keys);
        window.addEventListener(ORGANIZE_FILES_EVENT, organize);
        return () => window.removeEventListener(ORGANIZE_FILES_EVENT, organize);
    }, []);


    const {
        store, folders, groups, activeFolderId, setActiveFolderId, isSyncing, isConnected,
        handleLogout, handleSyncFolders, handleCreateFolder, handleFolderDelete,
        handleFolderRename, handleFolderToggleVisibility, handleExportFolderInvite,
        handleCreateGroup, handleDeleteGroup, handleUpdateGroup, handleAssignFolderToGroup,
        handleReorderFolders, handleUpdateGroupOrder,
        refreshFolders,
        accountId,
    } = useTelegramConnection(onLogout);


    useEffect(() => {
        let closed = false;
        let stop: (() => void) | undefined;
        void listen<{ ownerId: string; folderId: number | null }>('file-inventory-changed', ({ payload }) => {
            if (closed || payload.ownerId !== accountId) return;
            clearImageMemoryCaches();
            void queryClient.invalidateQueries({ queryKey: fileQueryKey(payload.ownerId, payload.folderId), exact: true });
        }).then(unlisten => { if (closed) unlisten(); else stop = unlisten; });
        let stopDriveChanged: (() => void) | undefined;
        void listen('drive-changed', () => {
            if (closed) return;
            clearImageMemoryCaches();
            void queryClient.invalidateQueries({ queryKey: ['files'] });
            void queryClient.invalidateQueries({ queryKey: ['driveStats'] });
            void queryClient.invalidateQueries({ queryKey: ['trashStats'] });
            void queryClient.invalidateQueries({ queryKey: ['trashedFolders'] });
            void queryClient.invalidateQueries({ queryKey: ['trashList'] });
            void queryClient.invalidateQueries({ queryKey: ['folderColors'] });
            void refreshFolders();
        }).then(unlisten => { if (closed) unlisten(); else stopDriveChanged = unlisten; });
        return () => { closed = true; stop?.(); stopDriveChanged?.(); };
    }, [accountId, queryClient, refreshFolders]);

    const { settings, updateSetting, updateSettings, isLoaded: settingsLoaded } = useSettings();
    const captureMutationScope = useActionScope(accountId);
    const { status: supporterStatus } = useSupporter();

    useEffect(() => {
        if (sessionStorage.getItem('telegram-drive-recovered-session') !== 'true') return;
        sessionStorage.removeItem('telegram-drive-recovered-session');
        const timer = window.setTimeout(() => {
            toast.success('We recovered your session — transfers are still queued.');
        }, 500);
        return () => window.clearTimeout(timer);
    }, []);
    const viewMode = settings.viewMode;
    const setViewMode = (mode: 'grid' | 'list') => updateSetting('viewMode', mode);

    const [previewFile, setPreviewFile] = useState<TelegramFile | null>(null);
    const [selectedIds, setSelectedIds] = useState<number[]>([]);
    const [moveRequest, setMoveRequest] = useState<{ ownerId: string; files: TelegramFile[] } | null>(null);
    const showMoveModal = moveRequest?.ownerId === accountId;
    const [showSettings, setShowSettings] = useState(false);
    const settingsModuleRequested = useRef(false);
    if (showSettings) settingsModuleRequested.current = true;
    const [settingsInitialTab, setSettingsInitialTab] = useState<SettingsTab>('general');
    const [focusSupporter, setFocusSupporter] = useState(false);
    const [transferCenterOpenRequest, setTransferCenterOpenRequest] = useState(0);
    const [showShortcuts, setShowShortcuts] = useState(false);
    const [showHelp, setShowHelp] = useState(false);
    const [supporterOfferTrigger, setSupporterOfferTrigger] = useState<SupporterPromptTrigger | null>(null);
    const [createFolderRequest, setCreateFolderRequest] = useState(0);
    const [activeSmartView, setActiveSmartView] = useState<SmartView | null>('recents');
    const [insightSummary, setInsightSummary] = useState<{ scope: string; count: number; complete: boolean } | null>(null);
    const [searchTerm, setSearchTerm] = useState("");
    const [searchFilters, setSearchFilters] = useState<FileSearchFilters>(DEFAULT_SEARCH_FILTERS);
    const search = useGlobalFileSearch(searchTerm, searchFilters, accountId);
    const searchResults = search.results;
    const [folderSyncProgress, setFolderSyncProgress] = useState({ active: false, count: 0 });
    const fileLoadSequenceRef = useRef(0);
    const fileLoadScope = JSON.stringify([accountId, activeSmartView, activeFolderId]);
    const fileLoadScopeRef = useRef({ key: fileLoadScope, generation: 0 });
    if (fileLoadScopeRef.current.key !== fileLoadScope) {
        fileLoadScopeRef.current = { key: fileLoadScope, generation: fileLoadScopeRef.current.generation + 1 };
    }
    useEffect(() => {
        setFolderSyncProgress({ active: false, count: 0 });
        return () => { fileLoadSequenceRef.current++; };
    }, [fileLoadScope]);
    useEffect(() => {
        if (!accountId || activeSmartView) return;
        let disposed = false;
        const touch = () => {
            void invoke<boolean>('cmd_touch_file_inventory', { ownerId: accountId, folderId: activeFolderId }).then(present => {
                if (!present && !disposed) void invalidateFolderFileQueries(queryClient, activeFolderId, accountId);
            }).catch(() => {});
        };
        touch();
        const timer = window.setInterval(touch, 60_000);
        return () => { disposed = true; window.clearInterval(timer); };
    }, [accountId, activeFolderId, activeSmartView, queryClient]);
    useEffect(() => {
        let disposed = false;
        let stop: (() => void) | undefined;
        void listen('vault-locked', () => {
            if (disposed) return;
            fileLoadScopeRef.current.generation++;
            fileLoadSequenceRef.current++;
            setPreviewFile(file => file?.encryption_state === 'encrypted_unlocked' ? null : file);
            setMoveRequest(null);
            setFolderSyncProgress({ active: false, count: 0 });
            void queryClient.cancelQueries({ queryKey: ['files'] }).then(() => {
                if (disposed) return;
                queryClient.setQueriesData<TelegramFile[]>({ queryKey: ['files'] }, files =>
                    files?.filter(file => file.encryption_state !== 'encrypted_unlocked'));
                void queryClient.invalidateQueries({ queryKey: ['files'] });
            });
        }).then(unlisten => { if (disposed) unlisten(); else stop = unlisten; });
        return () => { disposed = true; stop?.(); };
    }, [queryClient]);
    const [cardScale, setCardScale] = useState(1.0);
    const sortField: SortField = settings.fileSortField;
    const sortDirection: SortDirection = settings.fileSortDirection;
    const [internalDrag, setInternalDrag] = useState<{ ownerId: string; fileIds: number[]; files: TelegramFile[]; label: string; isCurrent: () => boolean } | null>(null);
    const dragSensors = useSensors(
        useSensor(PointerSensor, { activationConstraint: { distance: 8 } }),
        useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
    );

    const handleSortChange = (field: SortField) => {
        if (field === sortField) {
            updateSettings({
                fileSortField: field,
                fileSortDirection: sortDirection === 'asc' ? 'desc' : 'asc',
            });
            return;
        }
        updateSettings({ fileSortField: field, fileSortDirection: 'asc' });
    };
    const [showRemoteUpload, setShowRemoteUpload] = useState(false);
    const [playingFile, setPlayingFile] = useState<TelegramFile | null>(null);
    const [localPreview, setLocalPreview] = useState<{ key: string; path: string } | null>(null);
    const [pdfFile, setPdfFile] = useState<TelegramFile | null>(null);
    const [archiveViewFile, setArchiveViewFile] = useState<TelegramFile | null>(null);
    // Feature overlays ported from the earlier UI: home dashboard, trash,
    // storage statistics, custom background, copy-to-folder and properties.
    const [showHome, setShowHome] = useState(true);
    const [homeSelectedIds, setHomeSelectedIds] = useState<number[]>([]);
    const [folderMoveTarget, setFolderMoveTarget] = useState<{ id: number; name: string } | null>(null);
    const [folderCopySource, setFolderCopySource] = useState<{ id: number; name: string } | null>(null);
    const [copyRequest, setCopyRequest] = useState<{ files: TelegramFile[] } | null>(null);
    const toggleHomeSelect = useCallback((id: number) => {
        setHomeSelectedIds(prev => prev.includes(id) ? prev.filter(x => x !== id) : [...prev, id]);
    }, []);
    const clearHomeSelection = useCallback(() => setHomeSelectedIds([]), []);
    const selectAllFolders = useCallback(() => {
        setHomeSelectedIds(folders.filter(f => !f.parent_id).map(f => f.id));
    }, [folders]);
    const bulkDeleteFolders = useCallback(async () => {
        const ids = [...homeSelectedIds];
        for (const id of ids) {
            const f = folders.find(x => x.id === id);
            await handleFolderDelete(id, f?.name ?? '');
        }
        setHomeSelectedIds([]);
        await refreshFolders();
        void queryClient.invalidateQueries({ queryKey: ['files'] });
        void queryClient.invalidateQueries({ queryKey: ['driveStats'] });
    }, [homeSelectedIds, folders, handleFolderDelete, refreshFolders, queryClient]);
    const bulkShareFolders = useCallback(async () => {
        if (homeSelectedIds.length !== 1) { toast.info('Select a single folder to share'); return; }
        try {
            const info = await handleExportFolderInvite(homeSelectedIds[0]);
            await copyToClipboard(info.link);
            toast.success('Invite link copied');
        } catch { /* errors surfaced elsewhere */ }
        setHomeSelectedIds([]);
    }, [homeSelectedIds, handleExportFolderInvite]);


    const [showTrash, setShowTrash] = useState(false);
    const [showStorageStats, setShowStorageStats] = useState(false);
    const [showVault, setShowVault] = useState(false);
    const [copyTarget, setCopyTarget] = useState<TelegramFile | null>(null);
    const [propertiesTarget, setPropertiesTarget] = useState<TelegramFile | null>(null);
    const { data: driveStatsForStats } = useQuery({
        queryKey: ['driveStats'],
        queryFn: () => invoke<DriveStats>('cmd_get_drive_stats'),
        enabled: !!accountId,
        refetchInterval: showStorageStats ? 30000 : false,
    });
    const {
        shareFile, shareOwnerId, setShareFile, bulkShareLinks, bulkShareLoading, bulkShareCopied,
        setBulkShareLinks, createBulkShares, handleCopyBulkLink, handleNativeShareBulkLink,
    } = useFileSharing(accountId, activeFolderId);
    const [previewContextFiles, setPreviewContextFiles] = useState<TelegramFile[]>([]);
    const [previewContextIndex, setPreviewContextIndex] = useState(-1);
    const [renameFolder, setRenameFolder] = useState<{ id: number; name: string } | null>(null);
    const [renameRequest, setRenameRequest] = useState<{ ownerId: string; file: TelegramFile } | null>(null);
    const renameFileTarget = renameRequest?.ownerId === accountId ? renameRequest.file : null;
    useEffect(() => { setSelectedIds([]); setMoveRequest(null); setRenameRequest(null); setInternalDrag(null); }, [accountId]);

    useEffect(() => {
        let cancelled = false;
        let unlisten: (() => void) | undefined;
        let unlistenBackgroundHint: (() => void) | undefined;
        const initializeBridge = async () => {
            const [disposeNavigation, disposeBackgroundHint] = await Promise.all([
                listen<DesktopNavigationRequest>('desktop-navigation-request', ({ payload }) => {
                    if (payload.target === 'transfers') {
                        setTransferCenterOpenRequest(value => value + 1);
                    } else if (payload.target === 'settings') {
                        setSettingsInitialTab('general');
                        setShowSettings(true);
                    }
                }),
                listen('desktop-background-hint', () => {
                    toast.info(t('settings.desktop_background_hint'));
                }),
            ]);
            if (cancelled) {
                disposeNavigation();
                disposeBackgroundHint();
                return;
            }
            unlisten = disposeNavigation;
            unlistenBackgroundHint = disposeBackgroundHint;
            await markDesktopFrontendReady();
        };
        void initializeBridge().catch(() => {
            // Browser previews do not expose the desktop event or command bridge.
        });
        return () => {
            cancelled = true;
            unlisten?.();
            unlistenBackgroundHint?.();
            void markDesktopFrontendUnready().catch(() => {});
        };
    }, [t]);

    useEffect(() => {
        const openSettings = (event: Event) => {
            const tab = (event as CustomEvent<{ tab?: SettingsTab }>).detail?.tab ?? 'general';
            setSettingsInitialTab(tab);
            setShowSettings(true);
        };
        window.addEventListener('telegram-drive-open-settings', openSettings);
        return () => window.removeEventListener('telegram-drive-open-settings', openSettings);
    }, []);

    const { data: allFiles = [], isLoading, error } = useQuery({
        queryKey: fileQueryKey(accountId, activeFolderId, activeSmartView ?? 'folder'),
        queryFn: async ({ signal }) => {
            if (!accountId) throw new Error('ACCOUNT_REQUIRED');
            const requestSequence = ++fileLoadSequenceRef.current;
            const generation = fileLoadScopeRef.current.generation;
            const isCurrent = () => !signal.aborted
                && fileLoadScopeRef.current.generation === generation
                && fileLoadSequenceRef.current === requestSequence;
            const check = () => { if (!isCurrent()) throw new DOMException('File refresh was cancelled', 'AbortError'); };
            if (activeSmartView) {
                let localFiles: TelegramFile[];
                if (activeSmartView === 'offline') {
                    localFiles = await invoke<TelegramFile[]>('cmd_get_offline_files', { ownerId: accountId, limit: 250 });
                } else if (activeSmartView === 'large' || activeSmartView === 'old' || activeSmartView === 'duplicates') {
                    const insight = await invoke<StorageInsightResult>('cmd_get_storage_insight', {
                        view: activeSmartView,
                        ownerId: accountId,
                        largeThresholdBytes: 100 * 1024 * 1024,
                        oldFileDays: 365,
                    });
                    check();
                    setInsightSummary({ scope: fileLoadScope, count: insight.scanned_count, complete: insight.complete });
                    localFiles = insight.files;
                } else {
                    localFiles = await invoke<TelegramFile[]>('cmd_get_file_activity', { ownerId: accountId, view: activeSmartView, limit: 250 });
                }
                check();
                return localFiles.map((file) => ({ ...file, sizeStr: formatBytes(file.size), type: 'file' as const }));
            }
            const queryKey = fileQueryKey(accountId, activeFolderId);
            return refreshFolderFiles({
                ownerId: accountId,
                folderId: activeFolderId,
                requestId: `desktop-${crypto.randomUUID()}`,
                signal,
                isCurrent,
                cachedFiles: queryClient.getQueryData<TelegramFile[]>(queryKey),
                onFiles: files => queryClient.setQueryData(queryKey, files),
                onProgress: setFolderSyncProgress,
            });
        },
        enabled: !!store && !!accountId,
        staleTime: 5 * 60_000,
        refetchOnWindowFocus: false,
        refetchOnReconnect: false,
    });

    const filteringFiles = searchTerm.trim().length > 0 || searchFilters.type !== 'all' || searchFilters.size !== 'any' || searchFilters.date !== 'any';
    const displayedFiles = useMemo(() => {
        if (searchFilters.scope === 'all') return searchResults;
        return filterAndRankFiles(allFiles, searchTerm, searchFilters);
    }, [allFiles, searchResults, searchTerm, searchFilters]);
    const isCrossFolderView = activeSmartView !== null
        || searchFilters.scope === 'all';
    useEffect(() => { if (isCrossFolderView) setSelectedIds([]); }, [isCrossFolderView]);

    const folderAggregates = useMemo(() => {
        const children = new Map<number, number[]>();
        for (const f of folders) {
            const parent = f.parent_id ?? null;
            if (parent == null) continue;
            const list = children.get(parent) ?? [];
            list.push(f.id);
            children.set(parent, list);
        }
        const cache = new Map<number, { files: number; size: number; folders: number }>();
        const compute = (id: number, seen: Set<number>): { files: number; size: number; folders: number } => {
            const cached = cache.get(id);
            if (cached) return cached;
            if (seen.has(id)) return { files: 0, size: 0, folders: 0 };
            seen.add(id);
            const own = driveStatsForStats?.folders?.[String(id)];
            let files = own?.total_files ?? 0;
            let size = own?.total_size ?? 0;
            let nested = 0;
            for (const childId of children.get(id) ?? []) {
                const child = compute(childId, seen);
                files += child.files;
                size += child.size;
                nested += 1 + child.folders;
            }
            const result = { files, size, folders: nested };
            cache.set(id, result);
            return result;
        };
        const map = new Map<number, { files: number; size: number; folders: number }>();
        for (const f of folders) map.set(f.id, compute(f.id, new Set<number>()));
        return map;
    }, [folders, driveStatsForStats]);

    const folderRows = useMemo<TelegramFile[]>(() => {
        if (activeSmartView || searchTerm.trim()) return [];
        const parentId = activeFolderId ?? null;
        return folders
            .filter(f => (parentId === null ? !f.parent_id : f.parent_id === parentId))
            .map(f => {
                const agg = folderAggregates.get(f.id);
                return {
                    id: f.id,
                    name: f.name,
                    size: agg?.size ?? 0,
                    sizeStr: agg ? `${agg.files} files · ${agg.folders} folders · ${formatBytes(agg.size)}` : '',
                    type: 'folder' as const,
                    folder_id: parentId,
                    created_at: '',
                } as unknown as TelegramFile;
            });
    }, [folders, activeFolderId, activeSmartView, searchTerm, folderAggregates]);

    const filesWithFolders = useMemo(
        () => (folderRows.length ? [...folderRows, ...displayedFiles] : displayedFiles),
        [folderRows, displayedFiles],
    );

    const handleManualSync = useCallback(async () => {
        await handleSyncFolders();
        if (activeSmartView === null) {
            await queryClient.invalidateQueries({
                queryKey: fileQueryKey(accountId, activeFolderId),
                exact: true,
            });
        }
    }, [accountId, activeFolderId, activeSmartView, handleSyncFolders, queryClient]);

    const { data: bandwidth } = useQuery({
        queryKey: ['bandwidth'],
        queryFn: () => invoke<BandwidthStats>('cmd_get_bandwidth'),
        refetchInterval: 5000,
        enabled: !!store
    });


    const { uploadQueue, handleManualUpload, handleFolderUpload, handleDropUpload, handleUrlUpload, clearFinished: clearUploads, cancelAll: cancelUploads, pauseAll: pauseUploads, resumeAll: resumeUploads, cancelItem: cancelUploadItem, retryItem: retryUploadItem, retryFailed: retryFailedUploads, retryingFailed: retryingFailedUploads } = useFileUpload(activeFolderId, store, undefined, undefined, accountId ?? undefined);
    const { downloadQueue, queueDownload, queueBulkDownload, clearFinished: clearDownloads, cancelAll: cancelDownloads, pauseAll: pauseDownloads, resumeAll: resumeDownloads, cancelItem: cancelDownloadItem, retryItem: retryDownloadItem } = useFileDownload(store, undefined, undefined, accountId ?? undefined);

    const showSupporterOffer = useSupporterPrompt(
        Boolean(!accountId || isLoading || isSyncing || previewFile || playingFile || pdfFile || archiveViewFile
            || localPreview || showSettings || showMoveModal || shareFile || showRemoteUpload || showHelp
            || showShortcuts || renameFolder || renameFileTarget || bulkShareLinks || bulkShareLoading
            || internalDrag || selectedIds.length || workspaceKeys !== null || searchTerm
            || uploadQueue.some(item => ['pending', 'uploading', 'downloading', 'encrypting', 'verifying'].includes(item.status))
            || downloadQueue.some(item => ['pending', 'cooldown', 'downloading', 'decrypting', 'verifying'].includes(item.status))),
        setSupporterOfferTrigger,
    );

    const {
        handleDelete, handleBulkDelete, handleBulkDownload,
        handleMoveFiles, handleCopyFile, handleRenameFile: renameOwnedFile, handleDownloadFolder

    } = useFileOperations(activeFolderId, selectedIds, setSelectedIds, displayedFiles, queueBulkDownload, accountId);

    const loadFolderFiles = useCallback(async (folderId: number): Promise<TelegramFile[]> => {
        try {
            const res = await invoke<{ files: TelegramFile[] }>('cmd_get_files', { folderId });
            return res?.files ?? [];
        } catch {
            return [];
        }
    }, []);

    const bulkDownloadFolders = useCallback(async () => {
        if (!homeSelectedIds.length) return;
        const ids = [...homeSelectedIds];
        setHomeSelectedIds([]);
        for (const id of ids) {
            const files = await loadFolderFiles(id);
            if (!files.length) continue;
            await queueBulkDownload(files, id);
        }
    }, [homeSelectedIds, loadFolderFiles, queueBulkDownload]);

    const openFolderMove = useCallback(() => {
        if (homeSelectedIds.length !== 1) { toast.info('Select a single folder to move'); return; }
        const folder = folders.find(f => f.id === homeSelectedIds[0]);
        if (!folder) return;
        setFolderMoveTarget({ id: folder.id, name: folder.name });
    }, [homeSelectedIds, folders]);

    // Protect already-uploaded files: download the original, re-upload it
    // through the vault, then remove the plaintext copy.
    const handleEncryptSelected = useCallback(async () => {
        if (!accountId || !selectedIds.length) return;
        const targets = displayedFiles.filter(file => selectedIds.includes(file.id)
            && file.type !== 'folder'
            && (file.encryption_state ?? 'plain') === 'plain');
        if (!targets.length) {
            toast.info('Selected files are already protected');
            return;
        }
        let tempRoot: string;
        try {
            tempRoot = await tempDir();
        } catch {
            toast.error('Temporary folder is unavailable');
            return;
        }
        const backslash = String.fromCharCode(92);
        const separator = tempRoot.includes(backslash) ? backslash : '/';
        const base = tempRoot.endsWith(separator) ? tempRoot : `${tempRoot}${separator}`;
        let done = 0;
        for (const file of targets) {
            const tempPath = `${base}${crypto.randomUUID()}-${sanitizeFilename(file.name)}`;
            const folderId = file.folder_id ?? activeFolderId;
            try {
                await invoke('cmd_download_file', { req: { message_id: file.id, save_path: tempPath, folder_id: folderId } });
                await invoke('cmd_upload_file', {
                    ownerId: accountId,
                    path: tempPath,
                    folderId,
                    transferId: crypto.randomUUID(),
                    protectionMode: 'vault',
                    protectMetadata: settings.encryptionProtectMetadata,
                    videoUploadMode: 'file',
                });
                await invoke('cmd_delete_file', { messageId: file.id, folderId, ownerId: accountId });
                done += 1;
            } catch (error) {
                toast.error(`Could not encrypt ${file.name}: ${String(error)}`);
            } finally {
                void invoke('cmd_remove_temp_file', { path: tempPath }).catch(() => undefined);
            }
        }
        if (done > 0) {
            toast.success(`${done} file${done === 1 ? '' : 's'} encrypted into the vault`);
            setSelectedIds([]);
            await refreshFolders();
            void queryClient.invalidateQueries({ queryKey: ['files'] });
            void queryClient.invalidateQueries({ queryKey: ['driveStats'] });
        }
    }, [accountId, selectedIds, displayedFiles, activeFolderId, settings.encryptionProtectMetadata, refreshFolders, queryClient]);

    const commitFolderMove = useCallback(async (targetFolderId: number | null) => {
        const folder = folderMoveTarget;
        setFolderMoveTarget(null);
        if (!folder) return;
        try {
            await invoke('cmd_move_folder', { folderId: folder.id, newParentId: targetFolderId });
            await queryClient.invalidateQueries({ queryKey: ['folders'] });
            void queryClient.invalidateQueries({ queryKey: ['files'] });
            void queryClient.invalidateQueries({ queryKey: ['driveStats'] });
            await refreshFolders();
            setHomeSelectedIds([]);
            toast.success('Folder moved.');
        } catch {
            toast.error('Failed to move folder.');
        }
    }, [folderMoveTarget, queryClient, refreshFolders]);

    const descendantIdsOf = useCallback((folderId: number) => {
        const output: number[] = [];
        const walk = (parentId: number) => {
            for (const folder of folders) {
                if (folder.parent_id !== parentId) continue;
                output.push(folder.id);
                walk(folder.id);
            }
        };
        walk(folderId);
        return output;
    }, [folders]);

    const openFolderCopy = useCallback(() => {
        if (homeSelectedIds.length !== 1) { toast.info('Select a single folder to copy'); return; }
        const folder = folders.find(f => f.id === homeSelectedIds[0]);
        if (!folder) return;
        setFolderCopySource({ id: folder.id, name: folder.name });
    }, [homeSelectedIds, folders]);

    const commitFolderCopy = useCallback(async (targetFolderId: number | null) => {
        const source = folderCopySource;
        setFolderCopySource(null);
        if (!source || !accountId) return;
        const copyTree = async (sourceId: number, parentId: number | null) => {
            const name = folders.find(f => f.id === sourceId)?.name ?? source.name;
            const created = await invoke<{ id: number }>('cmd_create_folder', { name: `${name} copy` });
            if (parentId !== null) {
                await invoke('cmd_move_folder', { folderId: created.id, newParentId: parentId });
            }
            const result = await invoke<{ files: TelegramFile[] }>('cmd_get_files', { folderId: sourceId });
            const ids = (result?.files ?? []).map(file => file.id);
            if (ids.length) {
                await invoke('cmd_copy_files', {
                    messageIds: ids,
                    sourceFolderId: sourceId,
                    targetFolderId: created.id,
                    ownerId: accountId,
                });
            }
            for (const child of folders.filter(f => f.parent_id === sourceId)) {
                await copyTree(child.id, created.id);
            }
        };
        try {
            toast.info('Copying folder…');
            await copyTree(source.id, targetFolderId);
            await refreshFolders();
            void queryClient.invalidateQueries({ queryKey: ['files'] });
            void queryClient.invalidateQueries({ queryKey: ['driveStats'] });
            setHomeSelectedIds([]);
            toast.success('Folder copied.');
        } catch (error) {
            toast.error(`Copy failed: ${String(error)}`);
        }
    }, [folderCopySource, accountId, folders, refreshFolders, queryClient]);

    const openSelectedCopy = useCallback(() => {
        const files = displayedFiles.filter(file => selectedIds.includes(file.id) && file.type !== 'folder');
        if (!files.length) { toast.info('Select files to copy'); return; }
        setCopyRequest({ files });
    }, [displayedFiles, selectedIds]);

    const commitCopySelected = useCallback(async (targetFolderId: number | null) => {
        const request = copyRequest;
        setCopyRequest(null);
        if (!request) return;
        for (const file of request.files) {
            await handleCopyFile(file, targetFolderId);
        }
        void queryClient.invalidateQueries({ queryKey: ['files'] });
        setSelectedIds([]);
    }, [copyRequest, handleCopyFile, queryClient]);

    const handleBulkShare = useCallback(() => createBulkShares(
        displayedFiles.filter(file => selectedIds.includes(file.id) && file.type !== 'folder'),
        () => setSelectedIds([]),
    ), [createBulkShares, displayedFiles, selectedIds]);

    const handleSelectAll = useCallback(() => {
        if (isCrossFolderView) {
            toast.info('Open a folder to select multiple files safely.');
            return;
        }
        setSelectedIds(displayedFiles.map(f => f.id));
    }, [displayedFiles, isCrossFolderView]);

    const handleKeyboardDelete = useCallback(() => {
        if (selectedIds.length > 0) {
            handleBulkDelete();
        }
    }, [selectedIds, handleBulkDelete]);

    const handleEscape = useCallback(() => {
        lastClickedIndexRef.current = -1;
        setSelectedIds([]);
        setSearchTerm("");
        setPreviewFile(null);
        setPlayingFile(null);
        setPdfFile(null);
        setArchiveViewFile(null);
    }, []);

    const handleFocusSearch = useCallback(() => {
        const searchInput = document.querySelector('input[data-file-search]') as HTMLInputElement;
        if (searchInput) {
            searchInput.focus();
            searchInput.select();
        }
    }, []);

    const handleEnter = useCallback(() => {
        if (selectedIds.length === 1) {
            const selected = displayedFiles.find(f => f.id === selectedIds[0]);
            if (selected) {
                if (selected.type === 'folder') {
                    setActiveFolderId(selected.id);
                } else {
                    handlePreview(selected, displayedFiles);
                }
            }
        }
    }, [selectedIds, displayedFiles, setActiveFolderId]);


    useLayoutEffect(() => {
        lastClickedIndexRef.current = -1;
        setSelectedIds([]);
        setMoveRequest(null);
        setSearchTerm("");
        setPreviewFile(null);
        setPlayingFile(null);
        setPdfFile(null);
        setPreviewContextFiles([]);
        setPreviewContextIndex(-1);
        setArchiveViewFile(null);
    }, [accountId, activeFolderId, activeSmartView]);


    const lastClickedIndexRef = useRef<number>(-1);

    const clearSelection = useCallback(() => {
        lastClickedIndexRef.current = -1;
        setSelectedIds([]);
    }, []);

    const handleFileClick = (e: React.MouseEvent, file: TelegramFile, orderedFiles: TelegramFile[] = []) => {
        e.stopPropagation();
        const filesSource = orderedFiles.length > 0 ? orderedFiles : displayedFiles;
        if (isCrossFolderView) {
            clearSelection();
            if (file.type === 'folder') {
                setActiveSmartView(null);
                setActiveFolderId(file.id);
            } else {
                handlePreview(file, filesSource);
            }
            return;
        }

        if (file.type === 'folder') {
            clearSelection();
            setActiveSmartView(null);
            setActiveFolderId(file.id);
            return;
        }

        const id = file.id;
        const currentIndex = filesSource.findIndex(candidate => sameFile(candidate, file));

        if (e.shiftKey && lastClickedIndexRef.current >= 0) {
            // Shift+Click: range select from last clicked to current
            const start = Math.min(lastClickedIndexRef.current, currentIndex);
            const end = Math.max(lastClickedIndexRef.current, currentIndex);
            const rangeIds = filesSource.slice(start, end + 1).map(f => f.id);
            setSelectedIds(rangeIds);
        } else if (e.metaKey || e.ctrlKey) {
            // Ctrl/Cmd+Click: toggle individual file
            lastClickedIndexRef.current = currentIndex;
            setSelectedIds(ids => ids.includes(id) ? ids.filter(i => i !== id) : [...ids, id]);
        } else {
            // Plain click: select single file
            lastClickedIndexRef.current = currentIndex;
            setSelectedIds([id]);
        }
    }

    const handleToggleSelection = useCallback((id: number) => {
        setSelectedIds(ids => ids.includes(id) ? ids.filter(i => i !== id) : [...ids, id]);
    }, []);

    const handleFileMove = useCallback((file: TelegramFile) => {
        if (!accountId || !captureMutationScope()()) return;
        setMoveRequest({ ownerId: accountId, files: [{ ...file, folder_id: sourceFolder(file, activeFolderId) }] });
    }, [accountId, activeFolderId, captureMutationScope]);

    const handleOpenBulkMove = useCallback(() => {
        if (!accountId || !captureMutationScope()()) return;
        const files = displayedFiles.filter(file => selectedIds.includes(file.id))
            .map(file => ({ ...file, folder_id: sourceFolder(file, activeFolderId) }));
        if (files.length && files.length === selectedIds.length) setMoveRequest({ ownerId: accountId, files });
    }, [accountId, activeFolderId, captureMutationScope, displayedFiles, selectedIds]);

    const handleRename = useCallback((file: TelegramFile) => {
        if (!accountId || !captureMutationScope()()) return;
        setRenameRequest({ ownerId: accountId, file: { ...file, folder_id: sourceFolder(file, activeFolderId) } });
    }, [accountId, activeFolderId, captureMutationScope]);

    const handleRenameSubmit = useCallback(async (newName: string) => {
        if (!renameRequest || renameRequest.ownerId !== accountId || !captureMutationScope()()) return;
        if (!await renameOwnedFile(renameRequest.file, newName)) throw new Error('ACCOUNT_CHANGED');
    }, [accountId, captureMutationScope, renameOwnedFile, renameRequest]);

    const handleKeyboardDownload = useCallback(() => {
        if (selectedIds.length > 0) {
            handleBulkDownload();
        }
    }, [selectedIds, handleBulkDownload]);

    const handleKeyboardShare = useCallback(() => {
        if (selectedIds.length > 0) {
            handleBulkShare();
        }
    }, [selectedIds, handleBulkShare]);

    const handleKeyboardRename = useCallback(() => {
        if (selectedIds.length === 1) {
            const selected = displayedFiles.find(f => f.id === selectedIds[0]);
            if (selected && selected.type !== 'folder') {
                handleRename(selected);
            }
        }
    }, [selectedIds, displayedFiles, handleRename]);

    useKeyboardShortcuts({
        onSelectAll: handleSelectAll,
        onDelete: handleKeyboardDelete,
        onEscape: handleEscape,
        onSearch: handleFocusSearch,
        onEnter: handleEnter,
        onDownload: handleKeyboardDownload,
        onShare: handleKeyboardShare,
        onRename: handleKeyboardRename,
        onShowShortcuts: () => setShowShortcuts(true),
        enabled: !previewFile && !playingFile && !pdfFile && !archiveViewFile
            && !showMoveModal && !showSettings && !showShortcuts && !showHelp && !supporterOfferTrigger && workspaceKeys === null
            && !showRemoteUpload && !shareFile && !bulkShareLinks
            && settings.driveTourSeen
    });

    const handlePreview = (file: TelegramFile, orderedFiles?: TelegramFile[], localPath?: string) => {
        setLocalPreview(localPath ? { key: previewFileKey(file), path: localPath } : null);
        if (localPath && !isMediaFile(file.name) && !isPdfFile(file.name) && !isImageFile(file.name)) {
            setPlayingFile(null); setPreviewFile(null); setPdfFile(null); setArchiveViewFile(null);
            void invoke('cmd_open_file_externally', { path: localPath }).catch(() => toast.error(t('workspace.preview_failed')));
            return;
        }
        const sourceFolderId = sourceFolder(file, activeFolderId);
        const openedOwner = accountId;
        const openedGeneration = fileLoadScopeRef.current.generation;
        if (openedOwner) void invoke('cmd_record_file_opened', {
            ownerId: openedOwner,
            folderId: sourceFolderId,
            messageId: file.id,
            fileName: file.name,
            fileSize: file.size,
            mimeType: file.mime_type ?? null,
            fileExt: file.file_ext ?? null,
            createdAt: file.created_at ?? null,
            encryptionState: file.encryption_state ?? 'plain',
        }).then(() => {
            if (fileLoadScopeRef.current.generation === openedGeneration) return queryClient.invalidateQueries({
                queryKey: ['files', 'recents'], predicate: query => query.queryKey[query.queryKey.length - 1] === openedOwner,
            });
        }).catch(() => {});
        const contextFiles = (localPath ? [file] : orderedFiles || displayedFiles).filter((f) => f.type !== 'folder');
        const contextIndex = contextFiles.findIndex((candidate) => sameFile(candidate, file));

        setPreviewContextFiles(contextFiles);
        setPreviewContextIndex(contextIndex);

        const isMedia = isMediaFile(file.name);
        const isPdf = isPdfFile(file.name);
        const isArchive = isArchiveFile(file.name);

        if (isArchive) {
            setArchiveViewFile(file);
            setPreviewFile(null);
            setPlayingFile(null);
            setPdfFile(null);
        } else if (isMedia) {
            setPlayingFile(file);
            setPreviewFile(null);
            setPdfFile(null);
            setArchiveViewFile(null);
        } else if (isPdf) {
            setPdfFile(file);
            setPreviewFile(null);
            setPlayingFile(null);
            setArchiveViewFile(null);
        } else {
            setPreviewFile(file);
            setPlayingFile(null);
            setPdfFile(null);
            setArchiveViewFile(null);
        }
    };

    const navigatePreview = useCallback((step: 1 | -1) => {
        const next = getAdjacentPreview(
            previewContextFiles,
            previewFile ?? playingFile ?? pdfFile ?? archiveViewFile,
            step,
        );
        if (!next) return;
        const nextFile = next.file;
        setPreviewContextIndex(next.index);

        const isMedia = isMediaFile(nextFile.name);
        const isPdf = isPdfFile(nextFile.name);
        const isArchive = isArchiveFile(nextFile.name);

        if (isArchive) {
            setArchiveViewFile(nextFile);
            setPreviewFile(null);
            setPlayingFile(null);
            setPdfFile(null);
        } else if (isMedia) {
            setPlayingFile(nextFile);
            setPreviewFile(null);
            setPdfFile(null);
            setArchiveViewFile(null);
        } else if (isPdf) {
            setPdfFile(nextFile);
            setPreviewFile(null);
            setPlayingFile(null);
            setArchiveViewFile(null);
        } else {
            setPreviewFile(nextFile);
            setPlayingFile(null);
            setPdfFile(null);
            setArchiveViewFile(null);
        }
    }, [previewContextFiles, previewFile, playingFile, pdfFile, archiveViewFile]);

    const handleNextPreview = useCallback(() => {
        navigatePreview(1);
    }, [navigatePreview]);

    const handlePrevPreview = useCallback(() => {
        navigatePreview(-1);
    }, [navigatePreview]);

    const previewNeighborFiles = useCallback(() => {
        if (previewContextFiles.length === 0) {
            return { nextFile: null as TelegramFile | null, prevFile: null as TelegramFile | null };
        }

        const currentFile = previewFile ?? playingFile ?? pdfFile ?? archiveViewFile;
        if (!currentFile) {
            return { nextFile: null as TelegramFile | null, prevFile: null as TelegramFile | null };
        }

        const currentIdx = previewContextFiles.findIndex((file) => sameFile(file, currentFile));
        if (currentIdx === -1) {
            return { nextFile: null as TelegramFile | null, prevFile: null as TelegramFile | null };
        }

        const nextIdx = (currentIdx + 1) % previewContextFiles.length;
        const prevIdx = (currentIdx - 1 + previewContextFiles.length) % previewContextFiles.length;

        return {
            nextFile: previewContextFiles[nextIdx] || null,
            prevFile: previewContextFiles[prevIdx] || null,
        };
    }, [previewContextFiles, previewFile, playingFile, pdfFile, archiveViewFile]);

    const handleInternalDragStart = (event: DragStartEvent) => {
        const isCurrent = captureMutationScope();
        if (!accountId || !isCurrent() || event.active.data.current?.kind !== 'telegram-files') return;
        const fileIds = event.active.data.current.fileIds;
        if (!Array.isArray(fileIds) || fileIds.length === 0) return;
        const ids = fileIds.filter((id): id is number => typeof id === 'number');
        const files = displayedFiles.filter(file => ids.includes(file.id))
            .map(file => ({ ...file, folder_id: sourceFolder(file, activeFolderId) }));
        if (files.length !== ids.length) return;
        setInternalDrag({ ownerId: accountId, fileIds: ids, files, isCurrent, label: String(event.active.data.current.label || '') });
    };

    const handleInternalDragEnd = async (event: DragEndEvent) => {
        const { active, over } = event;
        const drag = internalDrag;
        setInternalDrag(null);
        if (!over || !captureMutationScope()()) return;

        const activeKind = active.data.current?.kind;
        const overKind = over.data.current?.kind;

        if (activeKind === 'telegram-files') {
            if (!drag || drag.ownerId !== accountId || !drag.isCurrent()) return;
            const fileIds = drag.fileIds;
            const targetFolderId = over.data.current?.folderId;
            const isFolderTarget = overKind === 'sidebar-folder' || overKind === 'content-folder';
            if (isFolderTarget && Array.isArray(fileIds) && (targetFolderId === null || typeof targetFolderId === 'number')) {
                await handleMoveFiles(drag.files, targetFolderId, () => setSelectedIds([]), true);
            }
            return;
        }

        if (activeKind === 'sidebar-folder') {
            const draggedFolderId = active.data.current?.folderId;
            if (typeof draggedFolderId !== 'number') return;

            if (overKind === 'sidebar-group') {
                const groupId = over.data.current?.groupId;
                await handleAssignFolderToGroup(draggedFolderId, typeof groupId === 'number' ? groupId : null);
                return;
            }

            if (overKind === 'sidebar-folder') {
                const overFolderId = over.data.current?.folderId;
                if (typeof overFolderId !== 'number' || draggedFolderId === overFolderId) return;
                const oldIndex = folders.findIndex(folder => folder.id === draggedFolderId);
                const newIndex = folders.findIndex(folder => folder.id === overFolderId);
                if (oldIndex !== -1 && newIndex !== -1) {
                    await handleReorderFolders(arrayMove(folders, oldIndex, newIndex));
                }
            }
            return;
        }

        if (activeKind === 'sidebar-group' && overKind === 'sidebar-group') {
            const draggedGroupId = active.data.current?.groupId;
            const overGroupId = over.data.current?.groupId;
            if (typeof draggedGroupId !== 'number' || typeof overGroupId !== 'number' || draggedGroupId === overGroupId) return;
            const oldIndex = groups.findIndex(group => group.id === draggedGroupId);
            const newIndex = groups.findIndex(group => group.id === overGroupId);
            if (oldIndex !== -1 && newIndex !== -1) {
                await handleUpdateGroupOrder(arrayMove(groups, oldIndex, newIndex));
            }
        }
    };

    const currentFolderName = activeFolderId === null
        ? t('common.saved_messages')
        : folders.find(f => f.id === activeFolderId)?.name || t('common.folders');
    const currentViewName = activeSmartView
        ? ({
            recents: t('common.recents'),
            favorites: t('common.favorites'),
            pinned: t('common.pinned'),
            offline: t('common.offline_files'),
            large: t('common.large_files'),
            old: t('common.old_files'),
            duplicates: t('common.duplicates'),
        } as const)[activeSmartView]
        : currentFolderName;

    // Home / parent / child… breadcrumb, built from the folder hierarchy.
    const breadcrumbPath = useMemo(() => {
        const rootCrumb = { id: null as number | null, name: 'Home' };
        if (activeSmartView || activeFolderId === null) return [rootCrumb];
        const byId = new Map(folders.map(f => [f.id, f]));
        const chain: { id: number; name: string }[] = [];
        let cursor: number | null = activeFolderId;
        const seen = new Set<number>();
        while (cursor != null && !seen.has(cursor) && chain.length < 32) {
            seen.add(cursor);
            const folder = byId.get(cursor);
            if (!folder) break;
            chain.unshift({ id: folder.id, name: folder.name });
            cursor = folder.parent_id ?? null;
        }
        return [rootCrumb, ...chain];
    }, [folders, activeFolderId, activeSmartView]);

    const navigateToPath = useCallback((id: number | null) => {
        setShowTrash(false);
        if (id === null) {
            setShowHome(true);
            setActiveSmartView(null);
            setActiveFolderId(null);
            return;
        }
        setShowHome(false);
        setActiveSmartView(null);
        setActiveFolderId(id);
    }, [setActiveFolderId]);

    const updateActivityFlag = useCallback(async (file: TelegramFile, flag: 'favorite' | 'pinned') => {
        if (!accountId) return;
        const generation = fileLoadScopeRef.current.generation;
        const nextValue = flag === 'favorite' ? !file.is_favorite : !file.is_pinned;
        try {
        await invoke('cmd_set_file_activity_flag', {
            ownerId: accountId,
            folderId: sourceFolder(file, activeFolderId),
            messageId: file.id,
            fileName: file.name,
            fileSize: file.size,
            mimeType: file.mime_type ?? null,
            fileExt: file.file_ext ?? null,
            createdAt: file.created_at ?? null,
            encryptionState: file.encryption_state ?? 'plain',
            flag,
            value: nextValue,
        });
        if (generation !== fileLoadScopeRef.current.generation) return;
        updateFileQueryData(
            queryClient,
            sourceFolder(file, activeFolderId),
            new Set([file.id]),
            current => ({
                ...current,
                [flag === 'favorite' ? 'is_favorite' : 'is_pinned']: nextValue,
            }),
            accountId,
        );
        await queryClient.invalidateQueries({
            queryKey: ['files', flag === 'favorite' ? 'favorites' : 'pinned'],
            predicate: query => query.queryKey[query.queryKey.length - 1] === accountId,
        });
        toast.success(flag === 'favorite'
            ? (nextValue ? i18n.t('navigation_copy.favorite_added') : i18n.t('navigation_copy.favorite_removed'))
            : (nextValue ? i18n.t("common.pinned") : i18n.t('navigation_copy.unpinned')));
        } catch {
            if (generation === fileLoadScopeRef.current.generation) toast.error(t('common.operation_failed'));
        }
    }, [accountId, activeFolderId, queryClient, t]);


    const previewNeighbors = previewNeighborFiles();

    return (
        <DndContext
            key={accountId ?? 'signed-out'}
            sensors={dragSensors}
            collisionDetection={closestCenter}
            onDragStart={handleInternalDragStart}
            onDragCancel={() => setInternalDrag(null)}
            onDragEnd={handleInternalDragEnd}
        >
            <div className="desktop-shell relative flex h-screen w-full overflow-hidden bg-app-canvas">
                <SyncDashboard />

            <ExternalDropBlocker
                currentFolderName={currentViewName}
                enabled={isConnected}
                onFilesDropped={handleDropUpload}
                onUploadClick={handleManualUpload}
            />

            <>
                {showMoveModal && moveRequest && (
                    <LazyFeatureBoundary>
                        <LazyFolderTreePicker
                            folders={folders}
                            title={moveRequest.files.length > 1 ? `Move ${moveRequest.files.length} files` : 'Move to folder'}
                            actionLabel="Move here"
                            kind="move"
                            currentFolderId={sourceFolder(moveRequest.files[0], activeFolderId)}
                            onClose={() => setMoveRequest(current => current === moveRequest ? null : current)}
                            onSelect={targetFolderId => {
                                if (moveRequest.ownerId !== accountId || !captureMutationScope()()) return;
                                void handleMoveFiles(moveRequest.files, targetFolderId, () => {
                                    setSelectedIds([]);
                                    setMoveRequest(current => current === moveRequest ? null : current);
                                    void queryClient.invalidateQueries({ queryKey: ['files'] });
                                    void queryClient.invalidateQueries({ queryKey: ['driveStats'] });
                                });
                            }}
                            key={`move:${moveRequest.ownerId}`}
                        />
                    </LazyFeatureBoundary>
                )}
                {folderMoveTarget && (
                    <LazyFeatureBoundary>
                        <LazyFolderTreePicker
                            folders={folders}
                            title={`Move “${folderMoveTarget.name}”`}
                            actionLabel="Move here"
                            kind="move"
                            sourceFolderId={folderMoveTarget.id}
                            blockedIds={descendantIdsOf(folderMoveTarget.id)}
                            onClose={() => setFolderMoveTarget(null)}
                            onSelect={targetId => void commitFolderMove(targetId)}
                            key={`folder-move:${folderMoveTarget.id}`}
                        />
                    </LazyFeatureBoundary>
                )}
                {folderCopySource && (
                    <LazyFeatureBoundary>
                        <LazyFolderTreePicker
                            folders={folders}
                            title={`Copy “${folderCopySource.name}”`}
                            actionLabel="Copy here"
                            kind="copy"
                            onClose={() => setFolderCopySource(null)}
                            onSelect={targetId => void commitFolderCopy(targetId)}
                            key={`folder-copy:${folderCopySource.id}`}
                        />
                    </LazyFeatureBoundary>
                )}
                {copyRequest && (
                    <LazyFeatureBoundary>
                        <LazyFolderTreePicker
                            folders={folders}
                            title={copyRequest.files.length > 1 ? `Copy ${copyRequest.files.length} files` : 'Copy to folder'}
                            actionLabel="Copy here"
                            kind="copy"
                            currentFolderId={sourceFolder(copyRequest.files[0], activeFolderId)}
                            onClose={() => setCopyRequest(null)}
                            onSelect={targetId => void commitCopySelected(targetId)}
                        />
                    </LazyFeatureBoundary>
                )}
                {showVault && (
                    <LazyFeatureBoundary>
                        <LazyVaultModal
                            onClose={() => setShowVault(false)}
                            onOpenFolder={id => { setShowHome(false); setActiveSmartView(null); setActiveFolderId(id); }}
                            onOpenEncryptionSettings={() => { setSettingsInitialTab('encryption'); setShowSettings(true); }}
                        />
                    </LazyFeatureBoundary>
                )}
                {playingFile && (
                    <LazyFeatureBoundary key={`media:${previewFileKey(playingFile)}`}>
                        <LazyMediaPlayer
                            file={playingFile}
                            onClose={() => setPlayingFile(null)}
                            onPlayFile={file => handlePreview(file, previewContextFiles)}
                            onNext={handleNextPreview}
                            onPrev={handlePrevPreview}
                            currentIndex={previewContextIndex}
                            totalItems={previewContextFiles.length}
                            activeFolderId={sourceFolder(playingFile, activeFolderId)}
                            localPath={localPreview?.key === previewFileKey(playingFile) ? localPreview.path : undefined}
                        />
                    </LazyFeatureBoundary>
                )}
                {pdfFile && (
                    <LazyFeatureBoundary key={`pdf:${previewFileKey(pdfFile)}`}>
                        <LazyPdfViewer
                            file={pdfFile}
                            onClose={() => setPdfFile(null)}
                            onNext={handleNextPreview}
                            onPrev={handlePrevPreview}
                            currentIndex={previewContextIndex}
                            totalItems={previewContextFiles.length}
                            activeFolderId={sourceFolder(pdfFile, activeFolderId)}
                            localPath={localPreview?.key === previewFileKey(pdfFile) ? localPreview.path : undefined}
                        />
                    </LazyFeatureBoundary>
                )}
                {showRemoteUpload && (
                    <LazyFeatureBoundary>
                        <RemoteUploadModal
                            isOpen={showRemoteUpload}
                            onClose={() => setShowRemoteUpload(false)}
                            folders={folders}
                            onUpload={handleUrlUpload}
                            key="remote-upload-modal"
                        />
                    </LazyFeatureBoundary>
                )}
            </>

            <Sidebar
                folders={folders}
                groups={groups}
                activeFolderId={activeFolderId}
                setActiveFolderId={setActiveFolderId}
                onDelete={handleFolderDelete}
                onRename={(id, name) => setRenameFolder({ id, name })}
                onToggleVisibility={async (id, _name, isPublic) => {
                    try {
                        await handleFolderToggleVisibility(id, !isPublic);
                        queryClient.invalidateQueries({ queryKey: ['folders'] });
                    } catch { /* toast handled in hook */ }
                }}
                onExportInvite={async (id, _name) => {
                    try {
                        const info = await handleExportFolderInvite(id);
                        try {
                            await copyToClipboard(info.link);
                            toast.success(i18n.t('navigation_copy.invite_copied', { link: info.link }));
                        } catch (e) {
                            toast.error(i18n.t('navigation_copy.clipboard_error', { error: String(e) }));
                        }
                    } catch { /* backend error already toasted in hook */ }
                }}
                onCreate={handleCreateFolder}
                isSyncing={isSyncing}
                isConnected={isConnected}
                onSync={() => void handleManualSync()}
                onLogout={handleLogout}
                onSettings={() => setShowSettings(true)}
                bandwidth={bandwidth || null}
                onAssignFolderToGroup={handleAssignFolderToGroup}
                onCreateGroup={handleCreateGroup}
                onUpdateGroup={handleUpdateGroup}
                onDeleteGroup={handleDeleteGroup}
                createFolderRequest={createFolderRequest}
                activeSmartView={activeSmartView}
                onSmartViewChange={setActiveSmartView}
                homeActive={showHome}
                onHomeClick={() => { setShowTrash(false); setShowHome(true); setActiveSmartView(null); setActiveFolderId(null); }}
                onVaultClick={() => setShowVault(true)}
            />

            <section aria-label={currentViewName} className="flex min-w-0 flex-1 flex-col">
                <div className="desktop-chrome-row relative justify-between gap-2">
                    <div className="flex min-w-0 items-center gap-2">
                        {showHome && (
                            <>
                                <Sparkles className="h-4 w-4 shrink-0 text-app-accent" />
                                <span className="shrink-0 text-ui font-semibold text-app-text">Drive Overview</span>
                            </>
                        )}
                    </div>
                    {showHome && (
                        <div className="pointer-events-none absolute left-1/2 top-1/2 w-[min(420px,42vw)] -translate-x-1/2 -translate-y-1/2">
                            <div className="pointer-events-auto relative">
                                <Search className="pointer-events-none absolute start-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-app-accent" />
                                <input
                                    value={searchTerm}
                                    onChange={e => { setSearchTerm(e.target.value); if (e.target.value.trim()) setShowHome(false); }}
                                    placeholder="Search files…"
                                    aria-label="Search files"
                                    className="quiet-control h-8 w-full min-w-0 rounded-lg border border-app-border bg-app-surface-sunken/40 ps-8 pe-3 text-ui text-app-text placeholder:text-app-text-tertiary focus:border-app-accent/50 focus:outline-none"
                                />
                            </div>
                        </div>
                    )}
                    <div className="flex items-center gap-1">
                        {showHome && (
                            <button type="button" title="New folder" aria-label="New folder" onClick={() => setCreateFolderRequest(value => value + 1)} className="quiet-control flex h-8 w-8 items-center justify-center text-emerald-400 hover:bg-app-hover">
                                <FolderPlus className="h-4 w-4" />
                            </button>
                        )}
                        {showHome && (
                            <button type="button" title="Upload folder" aria-label="Upload folder" onClick={handleFolderUpload} className="quiet-control flex h-8 w-8 items-center justify-center text-sky-400 hover:bg-app-hover">
                                <FolderUp className="h-4 w-4" />
                            </button>
                        )}
                        {!showHome && (
                            <button type="button" title="Upload folder" aria-label="Upload folder" onClick={handleFolderUpload} className="quiet-control flex h-8 w-8 items-center justify-center text-sky-400 hover:bg-app-hover">
                                <FolderUp className="h-4 w-4" />
                            </button>
                        )}
                        <button type="button" title="Storage" aria-label="Storage" onClick={() => setShowStorageStats(true)} className="quiet-control flex h-8 w-8 items-center justify-center text-violet-400 hover:bg-app-hover">
                            <PieChart className="h-4 w-4" />
                        </button>
                        <button type="button" title="Trash" aria-label="Trash" onClick={() => { setShowHome(false); setShowTrash(v => !v); }} className="quiet-control flex h-8 w-8 items-center justify-center text-red-400 hover:bg-app-hover">
                            <Trash2 className="h-4 w-4" />
                        </button>
                        <button type="button" title={t('workspace.title')} aria-label={t('workspace.title')} onClick={() => setWorkspaceKeys([])} className="quiet-control flex h-8 w-8 items-center justify-center text-app-accent hover:bg-app-hover">
                            <Library className="h-4 w-4" />
                        </button>
                    </div>
                </div>
                {showHome && homeSelectedIds.length > 0 && (
                    <div className="desktop-chrome-row justify-between gap-2 border-b border-app-border-subtle px-3 py-1.5">
                        <span className="text-badge font-medium text-app-text">{homeSelectedIds.length} folder{homeSelectedIds.length === 1 ? '' : 's'} selected</span>
                        <div className="flex items-center gap-1">
                            <button type="button" onClick={selectAllFolders} className="quiet-control flex h-8 items-center gap-1.5 px-2.5 text-ui font-medium text-app-text-secondary hover:bg-app-hover">
                                <CheckSquare className="h-4 w-4" />Select all
                            </button>
                            <button type="button" onClick={() => void bulkDownloadFolders()} className="quiet-control flex h-8 items-center gap-1.5 px-2.5 text-ui font-medium text-emerald-400 hover:bg-app-hover">
                                <Download className="h-4 w-4" />Download
                            </button>
                            <button type="button" onClick={() => void bulkShareFolders()} className="quiet-control flex h-8 items-center gap-1.5 px-2.5 text-ui font-medium text-amber-400 hover:bg-app-hover">
                                <Share2 className="h-4 w-4" />Share
                            </button>
                            <button type="button" onClick={() => void openFolderMove()} className="quiet-control flex h-8 items-center gap-1.5 px-2.5 text-ui font-medium text-sky-400 hover:bg-app-hover">
                                <ArrowRightLeft className="h-4 w-4" />Move
                            </button>
                            <button type="button" onClick={openFolderCopy} className="quiet-control flex h-8 items-center gap-1.5 px-2.5 text-ui font-medium text-teal-400 hover:bg-app-hover">
                                <Copy className="h-4 w-4" />Copy
                            </button>
                            <button type="button" onClick={() => void bulkDeleteFolders()} className="quiet-control flex h-8 items-center gap-1.5 px-2.5 text-ui font-medium text-red-400 hover:bg-app-hover">
                                <Trash2 className="h-4 w-4" />Delete
                            </button>
                            <button type="button" onClick={clearHomeSelection} className="quiet-control flex h-8 items-center gap-1.5 px-2.5 text-ui font-medium text-app-text-secondary hover:bg-app-hover">
                                <X className="h-4 w-4" />Clear
                            </button>
                        </div>
                    </div>
                )}

                {showHome ? (
                    <LazyFeatureBoundary>
                        <LazyHomeDashboard
                            folders={folders}
                            onFolderClick={(id) => { setShowHome(false); setActiveSmartView(null); setActiveFolderId(id); }}
                            onTrashClick={() => { setShowHome(false); setShowTrash(true); }}
                            onStorageStatsClick={() => setShowStorageStats(true)}
                            onRenameFolder={(id, name) => setRenameFolder({ id, name })}
                            onDeleteFolder={handleFolderDelete}
                            onClose={() => setShowHome(false)}
                            selectedIds={homeSelectedIds}
                            onToggleSelect={toggleHomeSelect}
                        />
                    </LazyFeatureBoundary>
                ) : showTrash ? (
                    <LazyFeatureBoundary>
                        <LazyTrashView
                            onClose={() => setShowTrash(false)}
                            onRestored={() => { void queryClient.invalidateQueries({ queryKey: ['folders'] }); }}
                        />
                    </LazyFeatureBoundary>
                ) : (
                    <>
                <TopBar
                    currentFolderName={currentViewName}
                    fileCount={displayedFiles.length}
                    folderCount={folderRows.length}
                    path={breadcrumbPath}
                    onNavigatePath={navigateToPath}
                    selectedIds={selectedIds}
                    onShowMoveModal={handleOpenBulkMove}
                    onBulkDownload={handleBulkDownload}
                    onBulkDelete={handleBulkDelete}
                    onBulkShare={handleBulkShare}
                    onEncryptSelected={handleEncryptSelected}
                    onCopySelected={openSelectedCopy}
                    onDownloadFolder={handleDownloadFolder}
                    onClearSelection={clearSelection}
                    onUploadClick={handleManualUpload}
                    viewMode={viewMode}
                    setViewMode={setViewMode}
                    cardScale={cardScale}
                    onCardScaleChange={setCardScale}
                    sortField={sortField}
                    sortDirection={sortDirection}
                    onSortChange={handleSortChange}
                    searchTerm={searchTerm}
                    onSearchChange={setSearchTerm}
                    searchFilters={searchFilters}
                    onSearchFiltersChange={setSearchFilters}
                    onSettingsClick={() => setShowSettings(true)}
                    onRemoteUploadClick={() => setShowRemoteUpload(true)}
                    onNewFolderClick={() => setCreateFolderRequest((value) => value + 1)}
                    onShowShortcuts={() => setShowShortcuts(true)}
                    onShowHelp={() => setShowHelp(true)}
                />
                {!search.enabled && filteringFiles && (
                    <div className="px-3 pb-0 pt-3">
                        <h2 className="text-ui font-medium text-app-text-secondary">
                            {displayedFiles.length.toLocaleString()} result{displayedFiles.length === 1 ? '' : 's'}{searchTerm.trim() ? <> for <span className="text-app-accent">"{searchTerm}"</span></> : null}
                        </h2>
                    </div>
                )}
                {insightSummary?.scope === fileLoadScope && !insightSummary.complete && <p role="status" className="px-3 text-xs text-app-text-secondary">{t('workspace.search_partial', { count: insightSummary.count })}</p>}
                {search.enabled && (
                    <div className="flex items-center gap-2 px-3 text-ui text-app-text-secondary">
                        {search.error ? <span role="alert">{t('workspace.error_operation')}</span> :
                            <span role="status">{search.busy ? t('common.loading') :
                                !displayedFiles.length && search.complete && !search.more ? t('workspace.no_matches') :
                                t(search.complete ? 'workspace.search_complete' : 'workspace.search_partial', { count: search.total }) + (search.offline ? ' · ' + t('settings.offline') : '')}</span>}
                        {(search.more || search.error) && <button type="button" className="quiet-control" disabled={search.busy} onClick={search.request}>
                            {t(search.error ? 'common.retry' : 'workspace.show_more')}
                        </button>}
                    </div>
                )}
                <FileExplorer
                    key={accountId ?? 'signed-out'}
                    folders={folders}
                    files={filesWithFolders}
                    loading={search.enabled ? search.busy && !displayedFiles.length : isLoading && allFiles.length === 0}
                    error={search.enabled ? null : error}
                    showEmptyFolder={!filteringFiles && !search.enabled}
                    viewMode={viewMode}
                    selectedIds={selectedIds}
                    activeFolderId={activeFolderId}
                    onFileClick={handleFileClick}
                    onDelete={handleDelete}
                    onDownload={(file) => queueDownload(file.id, file.name, sourceFolder(file, activeFolderId), file.size)}
                    onPreview={handlePreview}
                    onManualUpload={handleManualUpload}
                    onFolderUpload={handleFolderUpload}
                    showFolderUpload
                    onToggleSelection={handleToggleSelection}
                    onShare={setShareFile}
                    onRename={handleRename}
                    onFileMove={handleFileMove}
                    onCopy={(file) => setCopyTarget(file)}
                    onProperties={(file) => setPropertiesTarget(file)}
                    cardScale={cardScale}
                    sortField={sortField}
                    sortDirection={sortDirection}
                    onSortChange={handleSortChange}
                    onToggleFavorite={(file) => void updateActivityFlag(file, 'favorite')}
                    onTogglePinned={(file) => void updateActivityFlag(file, 'pinned')}
                    syncProgress={search.enabled ? undefined : folderSyncProgress}
                    selectionDisabled={isCrossFolderView}
                />
                    </>
                )}
            </section>

            {workspaceKeys !== null && <LazyFeatureBoundary><LazyWorkspaceHub folders={folders} initialKeys={workspaceKeys} onClose={() => setWorkspaceKeys(null)} onOpen={(file, orderedFiles, localPath) => handlePreview(file, orderedFiles, localPath)} onFolder={id => { setWorkspaceKeys(null); setActiveSmartView(null); setActiveFolderId(id); }} /></LazyFeatureBoundary>}

            {showStorageStats && (
                <LazyFeatureBoundary>
                    <LazyStorageStatsModal
                        driveStats={driveStatsForStats}
                        folders={folders}
                        onClose={() => setShowStorageStats(false)}
                    />
                </LazyFeatureBoundary>
            )}

            {copyTarget && (
                <LazyFeatureBoundary>
                    <LazyFolderTreePicker
                        folders={folders}
                        title="Copy to folder"
                        actionLabel="Copy here"
                        kind="copy"
                        currentFolderId={sourceFolder(copyTarget, activeFolderId)}
                        onClose={() => setCopyTarget(null)}
                        onSelect={targetId => { const file = copyTarget; setCopyTarget(null); void handleCopyFile(file, targetId).then(() => { void queryClient.invalidateQueries({ queryKey: ['files'] }); }); }}
                    />
                </LazyFeatureBoundary>
            )}

            {propertiesTarget && (
                <LazyFeatureBoundary>
                    <LazyPropertiesModal item={propertiesTarget} onClose={() => setPropertiesTarget(null)} />
                </LazyFeatureBoundary>
            )}

            {previewFile && (
                <LazyFeatureBoundary key={`preview:${previewFileKey(previewFile)}`}>
                    <LazyPreviewModal
                        file={previewFile}
                        onDownload={() => queueDownload(previewFile.id, previewFile.name, sourceFolder(previewFile, activeFolderId), previewFile.size)}
                        activeFolderId={sourceFolder(previewFile, activeFolderId)}
                        localPath={localPreview?.key === previewFileKey(previewFile) ? localPreview.path : undefined}
                        onClose={() => setPreviewFile(null)}
                        onNext={handleNextPreview}
                        onPrev={handlePrevPreview}
                        currentIndex={previewContextIndex}
                        totalItems={previewContextFiles.length}
                        nextFile={previewNeighbors.nextFile}
                        prevFile={previewNeighbors.prevFile}
                    />
                </LazyFeatureBoundary>
            )}

            {archiveViewFile && (
                <LazyFeatureBoundary key={`archive:${previewFileKey(archiveViewFile)}`}>
                    <LazyArchiveViewerModal
                        file={archiveViewFile}
                        activeFolderId={sourceFolder(archiveViewFile, activeFolderId)}
                        folders={folders}
                        onClose={() => setArchiveViewFile(null)}
                        onNext={handleNextPreview}
                        onPrev={handlePrevPreview}
                        currentIndex={previewContextIndex}
                        totalItems={previewContextFiles.length}
                        nextFile={previewNeighbors.nextFile}
                        prevFile={previewNeighbors.prevFile}
                    />
                </LazyFeatureBoundary>
            )}


            <TransferCenter
                openRequest={transferCenterOpenRequest}
                uploads={uploadQueue}
                downloads={downloadQueue}
                folders={folders}
                onClearUploads={clearUploads}
                onCancelUploads={cancelUploads}
                onPauseUploads={pauseUploads}
                onResumeUploads={resumeUploads}
                onCancelUpload={cancelUploadItem}
                onRetryUpload={retryUploadItem}
                onRetryFailedUploads={() => void retryFailedUploads()}
                retryingFailedUploads={retryingFailedUploads}
                onClearDownloads={clearDownloads}
                onCancelDownloads={cancelDownloads}
                onPauseDownloads={pauseDownloads}
                onResumeDownloads={resumeDownloads}
                onCancelDownload={cancelDownloadItem}
                onRetryDownload={retryDownloadItem}
            />

            {settingsModuleRequested.current && (
                <LazyFeatureBoundary>
                    <LazySettingsModal
                        ownerId={accountId}
                        isOpen={showSettings}
                        onClose={() => { setShowSettings(false); setFocusSupporter(false); }}
                        initialTab={settingsInitialTab}
                        focusSupporter={focusSupporter}
                    />
                </LazyFeatureBoundary>
            )}

            {showShortcuts && <KeyboardShortcutsDialog onClose={() => setShowShortcuts(false)} />}

            {settingsLoaded && supporterStatus.state !== 'loading' && !settings.driveTourSeen && (
                <DriveConceptTour
                    onFinish={() => updateSetting('driveTourSeen', true)}
                    onOpenHelp={() => { updateSetting('driveTourSeen', true); setShowHelp(true); }}
                />
            )}

            {showHelp && <LazyFeatureBoundary><LazyHelpCenterDialog onClose={() => setShowHelp(false)} /></LazyFeatureBoundary>}

            {supporterOfferTrigger && (
                <LazyFeatureBoundary><SupporterOfferDialog
                    trigger={supporterOfferTrigger}
                    onClose={() => setSupporterOfferTrigger(null)}
                    onOpenSupporter={() => { setSupporterOfferTrigger(null); setSettingsInitialTab('license'); setFocusSupporter(true); setShowSettings(true); }}
                /></LazyFeatureBoundary>
            )}

            <DesktopAdBanner
                suppressed={
                    uploadQueue.some(item => ['pending', 'uploading', 'downloading', 'encrypting', 'verifying'].includes(item.status))
                    || downloadQueue.some(item => ['pending', 'cooldown', 'downloading', 'decrypting', 'verifying'].includes(item.status))
                    || Boolean(previewFile || playingFile || pdfFile || archiveViewFile || showSettings || showMoveModal || shareFile || showRemoteUpload || showHelp || supporterOfferTrigger || !settings.driveTourSeen)
                }
                onSupport={() => { setSettingsInitialTab('license'); setFocusSupporter(true); setShowSettings(true); }}
                onManualDismiss={() => showSupporterOffer('ad_dismissed')}
            />

            {shareFile && shareOwnerId && (
                <LazyFeatureBoundary>
                    <LazyShareDialog
                        ownerId={shareOwnerId}
                        file={shareFile}
                        onClose={() => setShareFile(null)}
                        folders={folders}
                        activeFolderId={activeFolderId}
                        onOpenSettings={() => { setShareFile(null); setSettingsInitialTab('webdav'); setShowSettings(true); }}
                    />
                </LazyFeatureBoundary>
            )}

            {renameFolder && (
                <LazyFeatureBoundary>
                    <LazyRenameFolderModal
                        folderId={renameFolder.id}
                        currentName={renameFolder.name}
                        onRename={handleFolderRename}
                        onClose={() => setRenameFolder(null)}
                    />
                </LazyFeatureBoundary>
            )}

            {renameFileTarget && (
                <RenameFileModal
                    fileName={renameFileTarget.name}
                    onRename={handleRenameSubmit}
                    onClose={() => setRenameRequest(current => current === renameRequest ? null : current)}
                />
            )}

            {/* Bulk Share Results Modal */}
            {bulkShareLinks && (
                <div
                    className="fixed inset-0 z-[200] flex items-center justify-center bg-black/60 backdrop-blur-sm"
                    onClick={() => setBulkShareLinks(null)}
                >
                    <div
                        className="bg-telegram-surface border border-telegram-border rounded-xl w-[500px] max-h-[70vh] shadow-2xl overflow-hidden flex flex-col animate-in fade-in zoom-in-95 duration-150"
                        onClick={e => e.stopPropagation()}
                    >
                        <div className="p-4 border-b border-telegram-border flex items-center justify-between">
                            <h3 className="text-telegram-text font-medium flex items-center gap-2">
                                <Link className="w-5 h-5 text-telegram-primary" />
                                {i18n.t('common.action_count', { action: i18n.t('files.share_link'), count: bulkShareLinks.length })}
                            </h3>
                            <button onClick={() => setBulkShareLinks(null)} className="text-telegram-subtext hover:text-telegram-text">
                                <X className="w-5 h-5" />
                            </button>
                        </div>

                        {bulkShareLoading ? (
                            <div className="flex flex-col items-center justify-center py-16 space-y-3">
                                <Loader2 className="w-8 h-8 text-telegram-primary animate-spin" />
                                <p className="text-sm text-telegram-subtext">{i18n.t('navigation_copy.generating_links')}</p>
                            </div>
                        ) : (
                            <div className="flex-1 overflow-y-auto p-4 space-y-2 min-h-0">
                                {bulkShareLinks.map(({ file, link }) => {
                                    const isCopied = bulkShareCopied.has(link);
                                    return (
                                        <div
                                            key={file.id}
                                            className="p-3 rounded-lg bg-telegram-hover/30 border border-telegram-border/30 space-y-2"
                                        >
                                            <p className="text-xs font-semibold text-telegram-text truncate">{file.name}</p>
                                            <div className="flex gap-2">
                                                <input
                                                    type="text"
                                                    readOnly
                                                    value={link}
                                                    className="flex-1 bg-telegram-bg border border-telegram-border rounded-lg px-2.5 py-1.5 text-xs text-telegram-text focus:outline-none select-all truncate"
                                                />
                                                <button
                                                    onClick={() => handleCopyBulkLink(link)}
                                                    className={`px-2.5 py-1.5 rounded-lg border flex items-center justify-center transition-all flex-shrink-0 ${
                                                        isCopied
                                                            ? 'bg-emerald-500 border-emerald-500 text-white'
                                                            : 'bg-telegram-hover border-telegram-border text-telegram-text hover:bg-white/10'
                                                    }`}
                                                >
                                                    {isCopied ? <Check className="w-3.5 h-3.5" /> : <Copy className="w-3.5 h-3.5" />}
                                                </button>
                                                {typeof navigator !== 'undefined' && typeof navigator.share === 'function' && (
                                                    <button
                                                        onClick={() => handleNativeShareBulkLink(file, link)}
                                                        className="px-2.5 py-1.5 rounded-lg bg-telegram-primary/20 hover:bg-telegram-primary/30 text-telegram-primary border border-telegram-primary/30 transition-all flex items-center justify-center flex-shrink-0"
                                                    >
                                                        <Share2 className="w-3.5 h-3.5" />
                                                    </button>
                                                )}
                                            </div>
                                        </div>
                                    );
                                })}
                            </div>
                        )}

                        <button
                            onClick={() => setBulkShareLinks(null)}
                            className="w-full px-4 py-2.5 border-t border-telegram-border bg-telegram-hover/20 hover:bg-telegram-hover/40 text-telegram-text text-sm font-medium transition-colors"
                        >
                            {i18n.t("settings.done")}
                        </button>
                    </div>
                </div>
            )}
                <DragOverlay dropAnimation={null}>
                    {internalDrag && internalDrag.ownerId === accountId && (
                        <div className="flex max-w-xs items-center gap-2 rounded-lg border border-app-accent/40 bg-app-surface px-3 py-2 text-sm font-medium text-app-text shadow-2xl">
                            <Files className="h-4 w-4 shrink-0 text-app-accent" />
                            <span className="truncate">{internalDrag.label}</span>
                            {internalDrag.fileIds.length > 1 && (
                                <span className="rounded-full bg-app-accent px-1.5 py-0.5 text-[10px] font-bold text-app-accent-contrast">
                                    {internalDrag.fileIds.length}
                                </span>
                            )}
                        </div>
                    )}
                </DragOverlay>
            </div>
        </DndContext>
    );
}
