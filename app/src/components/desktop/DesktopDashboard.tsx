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

import { TelegramFile, BandwidthStats, type SmartView, type StorageInsightResult } from '../../types';
import { formatBytes, isMediaFile, isPdfFile, isArchiveFile, isImageFile, copyToClipboard } from '../../utils';

// Components
import { Sidebar } from './dashboard/Sidebar';
import { TopBar } from './dashboard/TopBar';
import { FileExplorer, type SortDirection, type SortField } from './dashboard/FileExplorer';
import { TransferCenter } from './dashboard/TransferCenter';
import { MoveToFolderModal } from './dashboard/MoveToFolderModal';
import { ExternalDropBlocker } from './dashboard/ExternalDropBlocker';
import type { SettingsTab } from './dashboard/SettingsModal';
import { RenameFolderModal } from './dashboard/RenameFolderModal';
import { RenameFileModal } from './dashboard/RenameFileModal';
import { DesktopAdBanner } from './dashboard/DesktopAdBanner';
import { RemoteUploadModal } from './dashboard/RemoteUploadModal';
import { KeyboardShortcutsDialog } from './dashboard/KeyboardShortcutsDialog';
import { DriveConceptTour } from './dashboard/DriveConceptTour';
import { LazyFeatureBoundary } from '../shared/LazyFeatureBoundary';
import { SyncDashboard } from './sync/SyncDashboard';
import { Files, Link, Copy, Check, X, Loader2, Share2 } from 'lucide-react';

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

const LazyShareDialog = lazy(() => import('./dashboard/ShareDialog').then(module => ({ default: module.ShareDialog })));
const LazyPreviewModal = lazy(() => import('./dashboard/PreviewModal').then((module) => ({ default: module.PreviewModal })));
const LazyMediaPlayer = lazy(() => import('./dashboard/MediaPlayer').then((module) => ({ default: module.MediaPlayer })));
const LazyPdfViewer = lazy(() => import('./dashboard/PdfViewer').then((module) => ({ default: module.PdfViewer })));
const LazyArchiveViewerModal = lazy(() => import('./dashboard/ArchiveViewerModal').then((module) => ({ default: module.ArchiveViewerModal })));
const LazySettingsModal = lazy(() => import('./dashboard/SettingsModal').then((module) => ({ default: module.SettingsModal })));
const SupporterOfferDialog = lazy(() => import('../shared/SupporterOfferDialog').then(module => ({ default: module.SupporterOfferDialog })));
const LazyHelpCenterDialog = lazy(() => import('./dashboard/HelpCenterDialog').then((module) => ({ default: module.HelpCenterDialog })));
const LazyWorkspaceHub = lazy(() => import('../workspace/WorkspaceHub').then(module => ({ default: module.WorkspaceHub })));

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
        return () => { closed = true; stop?.(); };
    }, [accountId, queryClient]);

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
    const { results: searchResults, isSearching, reply: searchReply, error: searchError } = useGlobalFileSearch(searchTerm, searchFilters, accountId);
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
    const {
        shareFile, shareOwnerId, setShareFile, bulkShareLinks, bulkShareLoading, bulkShareCopied,
        setBulkShareLinks, createBulkShares, handleCopyBulkLink, handleNativeShareBulkLink,
    } = useFileSharing(accountId, activeFolderId);
    const [previewContextFiles, setPreviewContextFiles] = useState<TelegramFile[]>([]);
    const [previewContextIndex, setPreviewContextIndex] = useState(-1);
    const [renameFolder, setRenameFolder] = useState<{ id: number; name: string } | null>(null);
    const [renameRequest, setRenameRequest] = useState<{ ownerId: string; file: TelegramFile } | null>(null);
    const renameFileTarget = renameRequest?.ownerId === accountId ? renameRequest.file : null;
    const moveFileTarget = showMoveModal && moveRequest?.files.length === 1 ? moveRequest.files[0] : null;
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

    const displayedFiles = useMemo(() => {
        if (searchFilters.scope === 'all') return searchResults;
        return filterAndRankFiles(allFiles, searchTerm, searchFilters);
    }, [allFiles, searchResults, searchTerm, searchFilters]);
    const isCrossFolderView = activeSmartView !== null
        || searchFilters.scope === 'all';
    useEffect(() => { if (isCrossFolderView) setSelectedIds([]); }, [isCrossFolderView]);

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
        handleMoveFiles, handleRenameFile: renameOwnedFile, handleDownloadFolder

    } = useFileOperations(activeFolderId, selectedIds, setSelectedIds, displayedFiles, queueBulkDownload, accountId);

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
                    <MoveToFolderModal
                        folders={folders}
                        fileName={moveFileTarget?.name}
                        onClose={() => setMoveRequest(current => current === moveRequest ? null : current)}
                        onSelect={targetFolderId => {
                            if (moveRequest.ownerId !== accountId || !captureMutationScope()()) return;
                            void handleMoveFiles(moveRequest.files, targetFolderId, () => {
                                setSelectedIds([]);
                                setMoveRequest(current => current === moveRequest ? null : current);
                            });
                        }}
                        activeFolderId={sourceFolder(moveRequest.files[0], activeFolderId)}
                        key={`move:${moveRequest.ownerId}`}
                    />
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
                    <RemoteUploadModal
                        isOpen={showRemoteUpload}
                        onClose={() => setShowRemoteUpload(false)}
                        folders={folders}
                        onUpload={handleUrlUpload}
                        key="remote-upload-modal"
                    />
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
                bandwidth={bandwidth || null}
                onAssignFolderToGroup={handleAssignFolderToGroup}
                onCreateGroup={handleCreateGroup}
                onUpdateGroup={handleUpdateGroup}
                onDeleteGroup={handleDeleteGroup}
                createFolderRequest={createFolderRequest}
                activeSmartView={activeSmartView}
                onSmartViewChange={setActiveSmartView}
            />

            <section aria-label={currentViewName} className="flex min-w-0 flex-1 flex-col">
                <div className="desktop-chrome-row justify-end"><button type="button" onClick={() => setWorkspaceKeys([])} className="quiet-control flex h-8 items-center gap-2 px-3 text-ui font-medium text-app-accent hover:bg-app-hover"><Files className="h-4 w-4" />{t('workspace.title')}</button></div>
                <TopBar
                    currentFolderName={currentViewName}
                    fileCount={displayedFiles.length}
                    selectedIds={selectedIds}
                    onShowMoveModal={handleOpenBulkMove}
                    onBulkDownload={handleBulkDownload}
                    onBulkDelete={handleBulkDelete}
                    onBulkShare={handleBulkShare}
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
                {(searchTerm.trim().length > 0 || searchFilters.type !== 'all' || searchFilters.size !== 'any' || searchFilters.date !== 'any') && (
                    <div className="px-3 pb-0 pt-3">
                        <h2 className="text-ui font-medium text-app-text-secondary">
                            {displayedFiles.length.toLocaleString()} result{displayedFiles.length === 1 ? '' : 's'}{searchTerm.trim() ? <> for <span className="text-app-accent">"{searchTerm}"</span></> : null}
                        </h2>
                    </div>
                )}
                {insightSummary?.scope === fileLoadScope && !insightSummary.complete && <p role="status" className="px-3 text-xs text-app-text-secondary">{t('workspace.search_partial', { count: insightSummary.count })}</p>}
                {searchError && <p role="alert" className="px-3 text-xs text-app-danger">{t('workspace.error_operation')}</p>}
                {searchReply && <p role="status" className="px-3 text-xs text-app-text-secondary">{t(searchReply.complete ? 'workspace.search_complete' : 'workspace.search_partial', { count: searchReply.total })}{searchReply.offline ? ` · ${t('settings.offline')}` : ''}</p>}
                <FileExplorer
                    key={accountId ?? 'signed-out'}
                    folders={folders}
                    files={displayedFiles}
                    loading={(isLoading && allFiles.length === 0) || isSearching}
                    error={searchError ? new Error(t('workspace.error_operation')) : error}
                    viewMode={viewMode}
                    selectedIds={selectedIds}
                    activeFolderId={activeFolderId}
                    onFileClick={handleFileClick}
                    onDelete={handleDelete}
                    onDownload={(file) => queueDownload(file.id, file.name, sourceFolder(file, activeFolderId), file.size)}
                    onPreview={handlePreview}
                    onManualUpload={handleManualUpload}
                    onFolderUpload={handleFolderUpload}
                    showFolderUpload={settings.zipFolders}
                    onToggleSelection={handleToggleSelection}
                    onShare={setShareFile}
                    onRename={handleRename}
                    onFileMove={handleFileMove}
                    cardScale={cardScale}
                    sortField={sortField}
                    sortDirection={sortDirection}
                    onSortChange={handleSortChange}
                    onToggleFavorite={(file) => void updateActivityFlag(file, 'favorite')}
                    onTogglePinned={(file) => void updateActivityFlag(file, 'pinned')}
                    syncProgress={folderSyncProgress}
                    selectionDisabled={isCrossFolderView}
                />
            </section>

            {workspaceKeys !== null && <LazyFeatureBoundary><LazyWorkspaceHub folders={folders} initialKeys={workspaceKeys} onClose={() => setWorkspaceKeys(null)} onOpen={(file, orderedFiles, localPath) => handlePreview(file, orderedFiles, localPath)} onFolder={id => { setWorkspaceKeys(null); setActiveSmartView(null); setActiveFolderId(id); }} /></LazyFeatureBoundary>}

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
                <RenameFolderModal
                    folderId={renameFolder.id}
                    currentName={renameFolder.name}
                    onRename={handleFolderRename}
                    onClose={() => setRenameFolder(null)}
                />
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
