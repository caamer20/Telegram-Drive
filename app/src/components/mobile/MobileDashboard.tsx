import { useState, useCallback, useMemo, useEffect, useRef } from 'react';
import { Folder, Download, Menu, LogOut, RefreshCw, UploadCloud, MoreVertical, Trash2, Pencil, Globe, Shield, Lock, ChevronDown, Share2, Link, Copy, Check, X, Loader2, Wifi, Activity, Zap, Eye, EyeOff } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import { onOpenUrl } from '@tauri-apps/plugin-deep-link';
import { listen } from '@tauri-apps/api/event';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { toast } from 'sonner';
import { BottomNavBar } from './BottomNavBar';
import { TouchFileList } from './TouchFileList';
import { ThemeToggle } from '../shared/ThemeToggle';
import AdsterraBanner from '../shared/AdsterraBanner';
import { ActionPopover, ActionItem } from './ActionPopover';
import { ShareDialog } from '../desktop/dashboard/ShareDialog';
import { RenameFolderSheet } from './RenameFolderSheet';
import { usePlatform } from '../../hooks/usePlatform';
import { useTelegramConnection } from '../../hooks/useTelegramConnection';
import { useFileUpload } from '../../hooks/useFileUpload';
import { useFileDownload } from '../../hooks/useFileDownload';
import { useFileOperations } from '../../hooks/useFileOperations';
import { formatBytes, isMediaFile, isVideoFile, isPdfFile, isImageFile, nativeShareOrCopy, copyToClipboard } from '../../utils';
import { MediaPlayer } from '../desktop/dashboard/MediaPlayer';
import { PdfViewer } from '../desktop/dashboard/PdfViewer';
import { PreviewModal } from '../desktop/dashboard/PreviewModal';
import { useTheme } from '../../context/ThemeContext';
import { TelegramFile, TelegramFolder, ShareInfo, BandwidthStats } from '../../types';
import { useSettings } from '../../context/SettingsContext';
import { version as appVersion } from '../../../package.json';
import { LANGUAGES } from '../../i18n/languages';
import { useTranslation } from 'react-i18next';
import {
  loadNativeResumePosition,
  cleanupNativePlayerForLogout,
  nativePlayerErrorMessage,
  nativePlayerInvocationMessage,
  NativePlayerLaunchGuard,
  NATIVE_PLAYER_BUILD_MARKER,
  openNativePlayerWithStartupRetry,
  saveNativeResumePosition,
  shouldShowReturnedNativeError,
  shouldUseNativePlayer,
  takePendingNativePlayerRestore,
} from '../../native-player';

export default function MobileDashboard({ onLogout }: { onLogout?: () => void }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [activeTab, setActiveTab] = useState<'files' | 'downloads' | 'settings'>('files');
  const [isSidebarOpen, setIsSidebarOpen] = useState(false);
  const { isAndroid } = usePlatform();
  const { theme } = useTheme();
  const { settings, updateSetting } = useSettings();

  useEffect(() => {
    console.info('[NativePlayer] rollout mode:', NATIVE_PLAYER_BUILD_MARKER);
  }, []);

  // ── Android deep-link listener (https://t.me/ links) ──────────────────
  useEffect(() => {
    if (!isAndroid) return;
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        unlisten = await onOpenUrl((urls) => {
          if (urls.length > 0) {
            const url = urls[0];
            toast.success(`Telegram link received: ${url}`, { duration: 5000 });
          }
        });
      } catch (e) {
        console.warn('[DeepLink] Failed to register listener:', e);
      }
    })();
    return () => { unlisten?.(); };
  }, [isAndroid]);

  // ── Android share-received listener (warm start) ──────────────────────
  useEffect(() => {
    if (!isAndroid) return;
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        unlisten = await listen<{ count: number }>('share-received', (event) => {
          const count = event.payload?.count ?? 0;
          if (count > 0) {
            const label = count === 1 ? '1 file' : `${count} files`;
            toast.success(`${label} received! Ready to upload.`, { duration: 4000 });
          }
        });
      } catch (e) {
        console.warn('[Share] Failed to register listener:', e);
      }
    })();
    return () => { unlisten?.(); };
  }, [isAndroid]);

  // ── Android cold-start share check ────────────────────────────────────
  useEffect(() => {
    if (!isAndroid) return;
    (async () => {
      try {
        const count = await invoke<number>('cmd_get_pending_share_count');
        if (count > 0) {
          const label = count === 1 ? '1 file' : `${count} files`;
          toast.success(`${label} received! Ready to upload.`, { duration: 4000 });
        }
      } catch (e) {
        // Best-effort; JNI cache may not be ready on very early mount
        console.warn('[Share] Cold-start check failed (may be expected):', e);
      }
    })();
  }, [isAndroid]);

  // Sync proxy settings to backend whenever they change
  useEffect(() => {
    const applyProxy = async () => {
      try {
        await invoke('cmd_apply_proxy_settings', {
          enabled: settings.proxyEnabled,
          proxyType: settings.proxyType,
          host: settings.proxyHost,
          port: settings.proxyPort,
          username: settings.proxyUsername,
          password: settings.proxyPassword,
        });
      } catch {
        // best-effort sync
      }
    };
    applyProxy();
  }, [
    settings.proxyEnabled, settings.proxyType, settings.proxyHost,
    settings.proxyPort, settings.proxyUsername, settings.proxyPassword,
  ]);

  const logoutHandler = useMemo(() => async () => {
    if (isAndroid) await cleanupNativePlayerForLogout();
    onLogout?.();
  }, [isAndroid, onLogout]);

  const {
    store, folders, activeFolderId, setActiveFolderId, isSyncing, isConnected,
    handleLogout, handleSyncFolders, handleCreateFolder, handleFolderDelete,
    handleFolderRename, handleFolderToggleVisibility, handleExportFolderInvite
  } = useTelegramConnection(logoutHandler);

  const { handleManualUpload } = useFileUpload(activeFolderId, store);
  const { queueDownload, queueBulkDownload } = useFileDownload(store);

  const [playingFile, setPlayingFile] = useState<TelegramFile | null>(null);
  const nativePlayerGuard = useRef(new NativePlayerLaunchGuard());
  const [pdfFile, setPdfFile] = useState<TelegramFile | null>(null);
  const [previewFile, setPreviewFile] = useState<TelegramFile | null>(null);
  const [shareFile, setShareFile] = useState<TelegramFile | null>(null);
  const [bulkShareLinks, setBulkShareLinks] = useState<Array<{ file: TelegramFile; link: string }> | null>(null);
  const [bulkShareLoading, setBulkShareLoading] = useState(false);
  const [bulkShareCopied, setBulkShareCopied] = useState<Set<string>>(new Set());
  const [uploadingCacheFiles, setUploadingCacheFiles] = useState<Set<string>>(new Set());
  const transferIdCounter = useRef(0);

  useEffect(() => {
    if (!isAndroid || !isConnected) return;
    let cancelled = false;
    void takePendingNativePlayerRestore()
      .then(async source => {
        if (cancelled || !source) return;
        const result = await nativePlayerGuard.current.open(
          source,
          candidate => openNativePlayerWithStartupRetry(candidate),
        );
        if (!result || cancelled) return;
        saveNativeResumePosition(source.folderId, source.messageId, result);
        if (result.error && shouldShowReturnedNativeError(result)) {
          toast.error(nativePlayerErrorMessage(result.error));
        }
      })
      .catch(error => {
        if (!cancelled) {
          toast.error(`Could not restore native playback: ${nativePlayerInvocationMessage(error)}`);
        }
      });
    return () => { cancelled = true; };
  }, [isAndroid, isConnected]);

  // ── Connection diagnostics state ──────────────────────────────────────
  const [checkingLatency, setCheckingLatency] = useState(false);
  const [latencyMs, setLatencyMs] = useState<number | null>(null);

  const { data: bandwidth } = useQuery({
    queryKey: ['bandwidth'],
    queryFn: () => invoke<BandwidthStats>('cmd_get_bandwidth'),
    refetchInterval: activeTab === 'settings' ? 5000 : false,
  });

  const handleCheckLatency = useCallback(async () => {
    setCheckingLatency(true);
    setLatencyMs(null);
    try {
      const ms = await invoke<number>('cmd_check_latency');
      setLatencyMs(ms);
      if (ms >= 0) {
        const emoji = ms < 100 ? '🟢' : ms < 250 ? '🟡' : '🔴';
        toast.success(`${emoji} Ping: ${ms}ms to Telegram DC`);
      } else {
        toast.error('Unable to reach Telegram servers');
      }
    } catch (e) {
      console.warn('Ping check failed:', e);
      toast.error('Unable to reach Telegram servers');
      setLatencyMs(-1);
    } finally {
      setCheckingLatency(false);
    }
  }, []);

  const adVisible = !playingFile && !pdfFile && !previewFile && !shareFile && !bulkShareLinks;

  // ── Android cached shared files ───────────────────────────────────────
  interface CachedFileEntry {
    uri: string;
    cached_path: string;
    file_name: string;
    file_size: number;
  }

  const { data: cachedFiles = [], refetch: refetchCachedFiles } = useQuery({
    queryKey: ['cached-files'],
    queryFn: () => invoke<CachedFileEntry[]>('cmd_list_cached_files'),
    enabled: isAndroid,
    refetchInterval: isAndroid ? 5000 : false, // poll while app is open (lightweight)
  });

  const handleUploadCachedFile = useCallback(async (entry: CachedFileEntry) => {
    const tid = `cache-upload-${++transferIdCounter.current}-${Date.now()}`;
    setUploadingCacheFiles(prev => new Set(prev).add(entry.cached_path));
    try {
      await invoke<string>('cmd_upload_file', {
        path: entry.cached_path,
        folderId: activeFolderId,
        transferId: tid,
      });
      toast.success(`Uploaded: ${entry.file_name}`);
      // Refresh the list to remove the uploaded entry
      refetchCachedFiles();
    } catch (e) {
      toast.error(`Upload failed: ${e}`);
    } finally {
      setUploadingCacheFiles(prev => {
        const next = new Set(prev);
        next.delete(entry.cached_path);
        return next;
      });
    }
  }, [activeFolderId, refetchCachedFiles]);

  const handleClearCachedFiles = useCallback(async () => {
    try {
      await Promise.all(cachedFiles.map(entry =>
        invoke('cmd_remove_cached_path', { uri: entry.uri }).catch(() => {})
      ));
      refetchCachedFiles();
      toast.success('Shared files cleared');
    } catch (e) {
      toast.error(`Failed to clear: ${e}`);
    }
  }, [cachedFiles, refetchCachedFiles]);

  // Real files loader
  const { data: allFiles = [], isLoading } = useQuery({
    queryKey: ['files', activeFolderId],
    queryFn: async () => {
      let accumulatedFiles: any[] = [];
      queryClient.setQueryData(['files', activeFolderId], []);

      const unlisten = await listen<any>('folder-load-chunk', (event) => {
        const payload = event.payload;
        if (payload.folderId === activeFolderId) {
          const newChunk = payload.files.map((f: any) => ({
            ...f,
            sizeStr: formatBytes(f.size),
            type: f.icon_type || (f.name.endsWith('/') ? 'folder' : 'file')
          }));
          accumulatedFiles = [...accumulatedFiles, ...newChunk];
          queryClient.setQueryData(['files', activeFolderId], accumulatedFiles);
        }
      });

      try {
        await invoke('cmd_get_files', { folderId: activeFolderId });
        return accumulatedFiles;
      } finally {
        unlisten();
      }
    },
    enabled: !!store,
  });

  const [selectedIds, setSelectedIds] = useState<number[]>([]);
  const [fileRenames, setFileRenames] = useState<Map<number, string>>(new Map());
  const { handleDelete: handleDeleteOp, handleBulkDelete, handleBulkDownload, handleBulkMove } = useFileOperations(activeFolderId, selectedIds, setSelectedIds, allFiles, queueBulkDownload);

  const activeFolder = activeFolderId === null
    ? 'Saved Messages'
    : folders.find(f => f.id === activeFolderId)?.name || 'Unknown Channel';

  // Folder action menu state (replaces swipe-to-reveal)
  const [folderActionMenu, setFolderActionMenu] = useState<TelegramFolder | null>(null);
  const [renameFolder, setRenameFolder] = useState<{ id: number; name: string } | null>(null);

  const handleFolderVisibilityToggle = useCallback(async (folder: TelegramFolder) => {
    const isPublic = folder.is_public || !!folder.username;
    if (isPublic) {
      // Make private
      try {
        await handleFolderToggleVisibility(folder.id, false);
      } catch { /* error already toasted */ }
    } else {
      // Make public — prompt for optional username
      const defaultUsername = folder.name.toLowerCase().replace(/[^a-z0-9_]/g, '').slice(0, 30);
      const username = prompt(`Make "${folder.name}" public. Enter a username (leave empty for auto-generated):`, defaultUsername)?.trim();
      if (username === undefined) return; // cancelled
      try {
        await handleFolderToggleVisibility(folder.id, true, username || undefined);
      } catch { /* error already toasted */ }
    }
  }, [handleFolderToggleVisibility]);

  const handleFolderShareInvite = useCallback(async (folder: TelegramFolder) => {
    try {
      const info = await handleExportFolderInvite(folder.id);
      try {
        await copyToClipboard(info.link);
        toast.success(`Invite link copied: ${info.link}`);
      } catch (e) {
        toast.error(`Failed to copy to clipboard: ${e}`);
      }
    } catch { /* backend error already toasted in hook */ }
  }, [handleExportFolderInvite]);

  const buildFolderActions = useCallback((folder: TelegramFolder): ActionItem[] => {
    const isPublic = folder.is_public || !!folder.username;
    return [
      {
        label: 'Rename',
        icon: <Pencil className="w-4 h-4" />,
        onClick: () => {
          setFolderActionMenu(null);
          setRenameFolder({ id: folder.id, name: folder.name });
        },
      },
      {
        label: isPublic ? 'Make Private' : 'Make Public',
        icon: isPublic ? <EyeOff className="w-4 h-4" /> : <Eye className="w-4 h-4" />,
        onClick: () => handleFolderVisibilityToggle(folder),
      },
      {
        label: 'Copy Invite Link',
        icon: <Link className="w-4 h-4" />,
        onClick: () => handleFolderShareInvite(folder),
      },
      {
        label: 'Delete',
        icon: <Trash2 className="w-4 h-4" />,
        onClick: () => handleFolderDelete(folder.id, folder.name),
        destructive: true,
      },
    ];
  }, [handleFolderDelete, handleFolderVisibilityToggle, handleFolderShareInvite]);

  const handleSelectAll = useCallback(() => {
    if (selectedIds.length === allFiles.length) {
      setSelectedIds([]);
    } else {
      setSelectedIds(allFiles.map(f => f.id));
    }
  }, [selectedIds.length, allFiles]);

  const handleClearSelection = useCallback(() => setSelectedIds([]), []);

  const handleToggleSelection = useCallback((id: number) => {
    setSelectedIds(prev => prev.includes(id) ? prev.filter(i => i !== id) : [...prev, id]);
  }, []);

  const handleDownload = useCallback((file: TelegramFile) => {
    queueDownload(file.id, file.name, activeFolderId);
  }, [queueDownload, activeFolderId]);

  const handleDeleteFile = useCallback((file: TelegramFile) => {
    handleDeleteOp(file.id);
  }, [handleDeleteOp]);

  const handlePreview = useCallback(async (file: TelegramFile) => {
    if (shouldUseNativePlayer(isAndroid, file.name) && isVideoFile(file.name)) {
      try {
        const source = {
          folderId: activeFolderId,
          messageId: file.id,
          title: file.name.slice(0, 256),
          fileName: file.name.slice(0, 512),
          startPositionMs: loadNativeResumePosition(activeFolderId, file.id),
          autoplay: true,
        };
        const result = await nativePlayerGuard.current.open(
          source,
          candidate => openNativePlayerWithStartupRetry(candidate),
        );
        if (result) {
          saveNativeResumePosition(activeFolderId, file.id, result);
          if (result.error && shouldShowReturnedNativeError(result)) {
            toast.error(nativePlayerErrorMessage(result.error));
          }
        }
      } catch (error) {
        toast.error(`Native playback failed: ${nativePlayerInvocationMessage(error)}`);
      }
      return;
    }
    if (isMediaFile(file.name)) {
      setPlayingFile(file);
    } else if (isPdfFile(file.name)) {
      setPdfFile(file);
    } else if (isImageFile(file.name)) {
      setPreviewFile(file);
    } else {
      toast.info(`Preview not supported for ${file.name}`);
    }
  }, [activeFolderId, isAndroid]);

  const handleRenameFile = useCallback((file: TelegramFile) => {
    const currentName = fileRenames.get(file.id) || file.name;
    const newName = prompt(`Rename "${currentName}":`, currentName);
    if (!newName || !newName.trim() || newName.trim() === currentName) return;
    setFileRenames(prev => {
      const next = new Map(prev);
      next.set(file.id, newName.trim());
      return next;
    });
    toast.success(`Renamed to "${newName.trim()}"`);
  }, [fileRenames]);

  // Bulk share: generate links for all selected non-folder files
  const handleBulkShare = useCallback(async () => {
    const shareFiles = allFiles.filter(f => selectedIds.includes(f.id) && f.type !== 'folder');
    if (shareFiles.length === 0) {
      toast.info('No shareable files selected (folders cannot be shared)');
      return;
    }
    // Open modal immediately with spinner
    setBulkShareLinks([]);
    setBulkShareLoading(true);
    setBulkShareCopied(new Set());
    try {
      const results = await Promise.all(
        shareFiles.map(async (file) => {
          try {
            const info = await invoke<ShareInfo>('cmd_create_share', {
              folderId: null,
              messageId: file.id,
              fileName: file.name,
              fileSize: file.size,
              password: null,
              expiryHours: 24, // default 1 day
            });
            return { file, link: info.link };
          } catch (e) {
            toast.error(`Failed to share ${file.name}: ${e}`);
            return null;
          }
        })
      );
      const valid = results.filter((r): r is { file: TelegramFile; link: string } => r !== null);
      if (valid.length > 0) {
        setBulkShareLinks(valid);
        setSelectedIds([]); // Clear selection after successful bulk share
      } else {
        setBulkShareLinks(null);
        toast.error('Failed to generate any share links');
      }
    } finally {
      setBulkShareLoading(false);
    }
  }, [allFiles, selectedIds]);

  const handleCopyBulkLink = useCallback((link: string) => {
    navigator.clipboard.writeText(link);
    setBulkShareCopied(prev => new Set(prev).add(link));
    setTimeout(() => setBulkShareCopied(prev => {
      const next = new Set(prev);
      next.delete(link);
      return next;
    }), 2000);
  }, []);

  const handleNativeShareBulkLink = useCallback((file: TelegramFile, link: string) => {
    nativeShareOrCopy(file.name, file.sizeStr, link, () => {
      handleCopyBulkLink(link);
    });
  }, [handleCopyBulkLink]);

  // ── Copy Telegram native t.me link ────────────────────────────────────
  const handleCopyTelegramLink = useCallback((file: TelegramFile) => {
    const folder = folders.find(f => f.id === file.folder_id) || folders.find(f => f.id === activeFolderId);
    const username = folder?.username || (folder as any)?.chat?.username || (folder as any)?.channel?.username;
    if (!username) {
      toast.error('Only available for public channels');
      return;
    }
    const url = `https://t.me/${username}/${file.id}`;
    navigator.clipboard.writeText(url).then(() => {
      toast.success('Telegram link copied');
    }).catch(() => {
      toast.error('Failed to copy link');
    });
  }, [folders, activeFolderId]);

  const displayFiles = useMemo(() => {
    if (fileRenames.size === 0) return allFiles;
    return allFiles.map(f =>
      fileRenames.has(f.id) ? { ...f, name: fileRenames.get(f.id)! } : f
    );
  }, [allFiles, fileRenames]);

  return (
    <div className="absolute inset-0 flex flex-col bg-telegram-bg text-telegram-text overflow-hidden select-none font-sans">
      {/* Premium Gradient Top Header */}
      <header className="flex items-center justify-between px-5 pb-4 pt-[calc(1rem+env(safe-area-inset-top,24px))] bg-gradient-to-r from-telegram-hover/40 to-telegram-bg border-b border-telegram-border/60 shadow-lg backdrop-blur-md sticky top-0 z-40">
        <div className="flex items-center gap-3">
          <img src="/logo.svg" className="w-8 h-8 drop-shadow-lg" alt="Logo" />
          <div>
            <h1 className={`text-base font-bold tracking-tight ${theme === 'light' ? 'text-[#1c1c1e]' : 'bg-gradient-to-r from-white to-telegram-subtext bg-clip-text text-transparent'}`}>Telegram Drive</h1>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <ThemeToggle />
          <button
            onClick={() => setIsSidebarOpen(true)}
            className="p-2 rounded-xl bg-telegram-hover/30 hover:bg-telegram-hover/60 border border-telegram-border/40 text-telegram-subtext transition-all duration-300"
          >
            <Menu className="w-5 h-5" />
          </button>
        </div>
      </header>

      {/* Main Viewport Container */}
      <main className="flex-1 overflow-y-auto px-4 py-3 space-y-4 pb-40 scroll-smooth">
        {activeTab === 'files' && (
          <div className="space-y-4">
            {/* Folder Header Breadcrumb */}
            <div className="flex items-center justify-between bg-telegram-hover/20 p-3 rounded-2xl border border-telegram-border/30">
              <div className="flex items-center gap-2.5">
                <Folder className="w-5 h-5 text-telegram-primary" />
                <span className="text-sm font-semibold truncate max-w-[150px]">{activeFolder}</span>
              </div>
              <div className="flex items-center gap-2">
                <button
                  onClick={handleManualUpload}
                  className="flex items-center gap-1.5 px-3 py-1.5 rounded-xl text-xs font-semibold bg-telegram-primary text-black hover:bg-telegram-primary/95 border border-telegram-primary/10 active:scale-95 transition-all duration-200"
                >
                  <UploadCloud className="w-3.5 h-3.5" />
                  Upload
                </button>
                <button
                  onClick={handleSyncFolders}
                  disabled={isSyncing}
                  className="flex items-center gap-1.5 px-3 py-1.5 rounded-xl text-xs font-semibold bg-telegram-primary/15 text-telegram-primary border border-telegram-primary/10 active:scale-95 transition-all duration-200 disabled:opacity-50"
                >
                  <RefreshCw className={`w-3.5 h-3.5 ${isSyncing ? 'animate-spin' : ''}`} />
                  Sync
                </button>
              </div>
            </div>

            {/* Dynamic Real File List */}
            <TouchFileList
              files={displayFiles}
              isLoading={isLoading && allFiles.length === 0}
              onDownload={handleDownload}
              onDelete={handleDeleteFile}
              onPreview={handlePreview}
              onRename={handleRenameFile}
              onShare={setShareFile}
              onCopyTelegramLink={handleCopyTelegramLink}
              onBulkShare={handleBulkShare}
              selectedIds={selectedIds}
              onToggleSelection={handleToggleSelection}
              onSelectAll={handleSelectAll}
              onClearSelection={handleClearSelection}
              onBulkDelete={handleBulkDelete}
              onBulkDownload={handleBulkDownload}
              onBulkMove={handleBulkMove}
              folders={folders}
              activeFolderId={activeFolderId}
            />
          </div>
        )}

        {activeTab === 'downloads' && (
          <div className="flex flex-col items-center justify-center h-[60vh] space-y-3 text-center px-6">
            <div className="p-4 rounded-full bg-telegram-primary/10 text-telegram-primary border border-telegram-primary/20">
              <Download className="w-8 h-8 animate-bounce" />
            </div>
            <h3 className="text-base font-bold">Transfers Queue</h3>
            <p className="text-xs text-telegram-subtext max-w-xs leading-relaxed">
              Downloads and uploads are safely queued and managed in the background.
            </p>
          </div>
        )}

        {activeTab === 'settings' && (
          <div className="space-y-4">
            <div className="p-4 rounded-2xl bg-telegram-hover/20 border border-telegram-border/30 space-y-4">
              <h3 className="text-sm font-bold text-telegram-primary tracking-wide uppercase text-[10px]">{t('common.preferences')}</h3>
              <div className="flex items-center justify-between py-2 border-b border-telegram-border/20">
                <div>
                  <p className="text-xs font-medium">{t('settings.zip_before_upload')}</p>
                  <p className="text-[10px] text-telegram-subtext">{t('settings.zip_folders_desc')}</p>
                </div>
                <button
                  onClick={() => updateSetting('zipFolders', !settings.zipFolders)}
                  className={`relative w-11 h-6 rounded-full transition-colors duration-200 flex-shrink-0 ${settings.zipFolders ? 'bg-telegram-primary' : 'bg-telegram-border'}`}
                >
                  <span className={`absolute top-0.5 left-0.5 w-5 h-5 rounded-full bg-white shadow transition-transform duration-200 ${settings.zipFolders ? 'translate-x-5' : 'translate-x-0'}`} />
                </button>
              </div>

              <div className="flex items-center justify-between py-2">
                <div>
                  <p className="text-xs font-medium">{t('common.language')}</p>
                  <p className="text-[10px] text-telegram-subtext">{t('settings.select_app_language')}</p>
                </div>
                <div className="relative">
                  <select
                    value={settings.language}
                    onChange={e => updateSetting('language', e.target.value as any)}
                    className="appearance-none bg-telegram-bg border border-telegram-border rounded-lg pl-2.5 pr-7 py-1.5 text-xs text-telegram-text focus:outline-none focus:border-telegram-primary/50 transition cursor-pointer"
                  >
                    {LANGUAGES.map(lang => (
                      <option key={lang.code} value={lang.code}>
                        {lang.nativeLabel}
                      </option>
                    ))}
                  </select>
                  <ChevronDown className="w-3.5 h-3.5 text-telegram-subtext absolute right-2 top-1/2 -translate-y-1/2 pointer-events-none" />
                </div>
              </div>
            </div>

            {/* Connection Diagnostics */}
            <div className="p-4 rounded-2xl bg-telegram-hover/20 border border-telegram-border/30 space-y-4">
              <h3 className="text-sm font-bold text-telegram-primary tracking-wide uppercase text-[10px] flex items-center gap-1.5">
                <Wifi className="w-3 h-3" />
                {t('settings.connection_diagnostics')}
              </h3>

              {/* Connection status indicator */}
              <div className="flex items-center justify-between py-2 border-b border-telegram-border/20">
                <div className="flex items-center gap-2">
                  <Activity className="w-3.5 h-3.5 text-telegram-subtext" />
                  <p className="text-xs font-medium">{t('common.status')}</p>
                </div>
                <div className="flex items-center gap-1.5">
                  <span className={`w-2 h-2 rounded-full ${isConnected ? 'bg-green-500 animate-pulse' : 'bg-red-500'}`} />
                  <span className={`text-xs font-semibold ${isConnected ? 'text-green-400' : 'text-red-400'}`}>
                    {isConnected ? t('common.connected_telegram') : t('settings.offline')}
                  </span>
                </div>
              </div>

              {/* Ping test */}
              <div className="flex items-center justify-between py-2 border-b border-telegram-border/20">
                <div>
                  <p className="text-xs font-medium">{t('common.ping')}</p>
                  <p className="text-[10px] text-telegram-subtext">
                    {latencyMs !== null
                      ? latencyMs >= 0
                        ? `${latencyMs}ms`
                        : t('settings.offline')
                      : t('settings.not_tested')}
                  </p>
                </div>
                <button
                  onClick={handleCheckLatency}
                  disabled={checkingLatency}
                  className="flex items-center gap-1.5 px-3 py-1.5 rounded-xl text-xs font-semibold bg-telegram-primary/15 text-telegram-primary hover:bg-telegram-primary/25 border border-telegram-primary/20 active:scale-95 transition-all duration-200 disabled:opacity-50"
                >
                  {checkingLatency ? (
                    <>
                      <div className="w-3 h-3 border-2 border-telegram-primary/30 border-t-telegram-primary rounded-full animate-spin" />
                      {t('settings.testing')}
                    </>
                  ) : (
                    <>
                      <Zap className="w-3 h-3" />
                      {t('settings.check_ping')}
                    </>
                  )}
                </button>
              </div>

              {/* Latency quality bar */}
              {latencyMs !== null && latencyMs >= 0 && (
                <div className="flex items-center gap-2 py-1">
                  <div className="flex-1 h-1.5 rounded-full bg-telegram-border/30 overflow-hidden">
                    <div
                      className={`h-full rounded-full transition-all duration-500 ${latencyMs < 100 ? 'bg-green-500' : latencyMs < 250 ? 'bg-yellow-500' : 'bg-red-500'}`}
                      style={{ width: `${Math.min(100, Math.max(5, (500 - latencyMs) / 5))}%` }}
                    />
                  </div>
                  <span className={`text-[10px] font-semibold ${latencyMs < 100 ? 'text-green-400' : latencyMs < 250 ? 'text-yellow-400' : 'text-red-400'}`}>
                    {latencyMs < 100 ? t('settings.excellent') : latencyMs < 250 ? t('settings.good') : t('settings.slow')}
                  </span>
                </div>
              )}

              {/* Bandwidth stats */}
              {bandwidth && (
                <div className="flex items-center justify-between py-2">
                  <div>
                    <p className="text-xs font-medium">{t('common.usage')}</p>
                    <p className="text-[10px] text-telegram-subtext">{t('settings.up_down_since_connected')}</p>
                  </div>
                  <div className="text-right">
                    <p className="text-[11px] font-mono font-semibold text-telegram-text">
                      <span className="text-emerald-400">↑ {formatBytes(bandwidth.up_bytes)}</span>
                      {' · '}
                      <span className="text-blue-400">↓ {formatBytes(bandwidth.down_bytes)}</span>
                    </p>
                  </div>
                </div>
              )}
            </div>

            {/* Proxy Configuration */}
            <div className="p-4 rounded-2xl bg-telegram-hover/20 border border-telegram-border/30 space-y-4">
              <h3 className="text-sm font-bold text-telegram-primary tracking-wide uppercase text-[10px] flex items-center gap-1.5">
                <Shield className="w-3 h-3" />
                {t('common.proxy')}
              </h3>

              {/* Enable Proxy Toggle */}
              <div className="flex items-center justify-between py-2 border-b border-telegram-border/20">
                <div>
                  <p className="text-xs font-medium">{t('common.enable_proxy')}</p>
                  <p className="text-[10px] text-telegram-subtext">{t('settings.enable_proxy_desc')}</p>
                </div>
                <button
                  onClick={() => updateSetting('proxyEnabled', !settings.proxyEnabled)}
                  className={`relative w-11 h-6 rounded-full transition-colors duration-200 flex-shrink-0 ${settings.proxyEnabled ? 'bg-telegram-primary' : 'bg-telegram-border'}`}
                >
                  <span className={`absolute top-0.5 left-0.5 w-5 h-5 rounded-full bg-white shadow transition-transform duration-200 ${settings.proxyEnabled ? 'translate-x-5' : 'translate-x-0'}`} />
                </button>
              </div>

              {/* Proxy Type */}
              <div className="flex items-center justify-between py-2 border-b border-telegram-border/20">
                <div>
                  <p className="text-xs font-medium">{t('common.proxy_type')}</p>
                  <p className="text-[10px] text-telegram-subtext">{t('settings.socks5_desc_mobile')}</p>
                </div>
                <div className="relative">
                  <select
                    value={settings.proxyType}
                    onChange={e => updateSetting('proxyType', e.target.value as 'socks5')}
                    className="appearance-none bg-telegram-bg border border-telegram-border rounded-lg pl-2.5 pr-7 py-1.5 text-xs text-telegram-text focus:outline-none focus:border-telegram-primary/50 transition cursor-pointer"
                  >
                    <option value="socks5">SOCKS5</option>
                  </select>
                  <ChevronDown className="w-3.5 h-3.5 text-telegram-subtext absolute right-2 top-1/2 -translate-y-1/2 pointer-events-none" />
                </div>
              </div>

              {/* Host */}
              <div className="flex items-center justify-between py-2 border-b border-telegram-border/20">
                <div>
                  <p className="text-xs font-medium">{t('common.host')}</p>
                  <p className="text-[10px] text-telegram-subtext">{t('settings.host_desc')}</p>
                </div>
                <input
                  type="text"
                  placeholder="127.0.0.1"
                  value={settings.proxyHost}
                  onChange={e => updateSetting('proxyHost', e.target.value)}
                  className="w-32 bg-telegram-bg border border-telegram-border rounded-lg px-2 py-1.5 text-xs text-telegram-text text-right focus:outline-none focus:border-telegram-primary/50 transition placeholder:text-telegram-subtext/40"
                />
              </div>

              {/* Port */}
              <div className="flex items-center justify-between py-2 border-b border-telegram-border/20">
                <div>
                  <p className="text-xs font-medium">{t('common.port')}</p>
                  <p className="text-[10px] text-telegram-subtext">{t('settings.port_desc')}</p>
                </div>
                <input
                  type="number"
                  min="1"
                  max="65535"
                  value={settings.proxyPort}
                  onChange={e => updateSetting('proxyPort', Math.max(1, Math.min(65535, parseInt(e.target.value) || 1080)))}
                  className="w-20 bg-telegram-bg border border-telegram-border rounded-lg px-2 py-1.5 text-xs text-telegram-text text-center focus:outline-none focus:border-telegram-primary/50 transition"
                />
              </div>

              {/* SOCKS5 auth fields */}
              {settings.proxyType === 'socks5' && (
                <>
                  <div className="flex items-center justify-between py-2 border-b border-telegram-border/20">
                    <div>
                      <p className="text-xs font-medium">{t('common.username')}</p>
                      <p className="text-[10px] text-telegram-subtext">{t('settings.optional')}</p>
                    </div>
                    <input
                      type="text"
                      placeholder={t('settings.optional')}
                      value={settings.proxyUsername}
                      onChange={e => updateSetting('proxyUsername', e.target.value)}
                      className="w-32 bg-telegram-bg border border-telegram-border rounded-lg px-2 py-1.5 text-xs text-telegram-text text-right focus:outline-none focus:border-telegram-primary/50 transition placeholder:text-telegram-subtext/40"
                    />
                  </div>
                  <div className="flex items-center justify-between py-2">
                    <div>
                      <p className="text-xs font-medium">{t('common.password')}</p>
                      <p className="text-[10px] text-telegram-subtext">{t('settings.optional')}</p>
                    </div>
                    <input
                      type="password"
                      placeholder={t('settings.optional')}
                      value={settings.proxyPassword}
                      onChange={e => updateSetting('proxyPassword', e.target.value)}
                      className="w-32 bg-telegram-bg border border-telegram-border rounded-lg px-2 py-1.5 text-xs text-telegram-text text-right focus:outline-none focus:border-telegram-primary/50 transition placeholder:text-telegram-subtext/40"
                    />
                  </div>
                </>
              )}

              {/* Info note */}
              <div className="p-2.5 rounded-lg bg-yellow-500/5 border border-yellow-500/10">
                <p className="text-[10px] text-yellow-400/70 leading-relaxed">
                  {t('settings.proxy_reconnect_note')}
                </p>
              </div>
            </div>

            {/* Shared Files (Android only) */}
            {isAndroid && cachedFiles.length > 0 && (
              <div className="p-4 rounded-2xl bg-telegram-hover/20 border border-telegram-border/30 space-y-4">
                <h3 className="text-sm font-bold text-telegram-primary tracking-wide uppercase text-[10px] flex items-center gap-1.5">
                  <Share2 className="w-3 h-3" />
                  {t('settings.shared_files', { count: cachedFiles.length })}
                </h3>
                <div className="space-y-2">
                  {cachedFiles.map((entry) => {
                    const isUploading = uploadingCacheFiles.has(entry.cached_path);
                    return (
                      <div
                        key={entry.cached_path}
                        className="flex items-center justify-between p-3 rounded-xl bg-telegram-bg/50 border border-telegram-border/30"
                      >
                        <div className="min-w-0 flex-1 mr-2">
                          <p className="text-xs font-semibold text-telegram-text truncate">{entry.file_name}</p>
                          <p className="text-[10px] text-telegram-subtext/60 font-mono">{formatBytes(entry.file_size)}</p>
                        </div>
                        <button
                          onClick={() => handleUploadCachedFile(entry)}
                          disabled={isUploading || !isConnected}
                          className="flex items-center gap-1.5 px-3 py-1.5 rounded-xl text-xs font-semibold bg-telegram-primary text-black hover:bg-telegram-primary/95 border border-telegram-primary/10 active:scale-95 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed flex-shrink-0"
                        >
                          {isUploading ? (
                            <>
                              <div className="w-3 h-3 border-2 border-black/30 border-t-black rounded-full animate-spin" />
                              {t('settings.uploading')}
                            </>
                          ) : (
                            <>
                              <UploadCloud className="w-3 h-3" />
                              {t('common.upload')}
                            </>
                          )}
                        </button>
                      </div>
                    );
                  })}
                </div>
                <button
                  onClick={handleClearCachedFiles}
                  className="w-full text-center text-[10px] text-red-400/60 hover:text-red-400 transition-colors py-1"
                >
                  {t('settings.clear_shared_files')}
                </button>
              </div>
            )}

            <div className="p-4 rounded-2xl bg-telegram-hover/20 border border-telegram-border/30 space-y-4">
              <h3 className="text-sm font-bold text-telegram-primary tracking-wide uppercase text-[10px]">{t('common.about')}</h3>
              <div className="flex flex-col items-center py-3 space-y-4">
                <img src="/logo.svg" className="w-14 h-14 drop-shadow-lg" alt="Telegram Drive Logo" />
                <div className="text-center">
                  <p className="text-sm font-bold text-telegram-text">Telegram Drive</p>
                  <p className="text-[11px] text-telegram-subtext mt-0.5">v{appVersion}</p>
                </div>

                <div className="w-10 h-px bg-telegram-border" />

                <div className="text-center space-y-2.5">
                  <p className="text-xs font-semibold text-telegram-text">Cameron Amer</p>

                  <button
                    onClick={(e) => { e.preventDefault(); openUrl('https://www.cameronamer.com'); }}
                    className="flex items-center justify-center gap-1.5 text-[11px] text-telegram-primary hover:text-telegram-primary/80 transition-colors cursor-pointer"
                  >
                    <Globe className="w-3 h-3" />
                    www.cameronamer.com
                  </button>

                  <button
                    onClick={(e) => { e.preventDefault(); openUrl('https://github.com/caamer20/telegram-drive'); }}
                    className="flex items-center justify-center gap-1.5 text-[11px] text-telegram-primary hover:text-telegram-primary/80 transition-colors cursor-pointer"
                  >
                    <svg className="w-3 h-3" viewBox="0 0 24 24" fill="currentColor">
                      <path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z"/>
                    </svg>
                    github.com/caamer20/telegram-drive
                  </button>
                </div>

                <p className="text-[10px] text-telegram-subtext/60 leading-relaxed text-center px-2">
                  {t('settings.tagline')}
                </p>
              </div>
            </div>

            <button onClick={handleLogout} className="w-full flex items-center justify-center gap-2 py-3 rounded-2xl bg-red-500/10 hover:bg-red-500/20 text-red-400 border border-red-500/20 font-semibold text-xs active:scale-98 transition-all duration-200">
              <LogOut className="w-4 h-4" />
              {t('common.logout')}
            </button>
          </div>
        )}
      </main>

      {/* Slide-out Sidebar Drawer Overlay */}
      {isSidebarOpen && (
        <div
          className="fixed inset-0 bg-black/60 z-[100] backdrop-blur-sm transition-opacity duration-300"
          onClick={() => setIsSidebarOpen(false)}
        />
      )}

      {/* Slide-out Sidebar Drawer Panel */}
      <div
        className={`fixed top-0 left-0 bottom-0 w-[280px] bg-telegram-surface border-r border-telegram-border/60 z-[110] shadow-2xl flex flex-col pt-[calc(1rem+env(safe-area-inset-top,24px))] pb-28 transition-transform duration-300 ease-out transform ${isSidebarOpen ? 'translate-x-0' : '-translate-x-full'
          }`}
        onClick={e => e.stopPropagation()}
      >
        <div className="p-4 flex items-center justify-between border-b border-telegram-border/30">
          <div className="flex items-center gap-2">
            <img src="/logo.svg" className="w-8 h-8 drop-shadow-lg" alt="Logo" />
            <span className="font-bold text-base text-telegram-text tracking-tight">Telegram Drive</span>
          </div>
          <button
            onClick={() => setIsSidebarOpen(false)}
            className="p-1 rounded-lg bg-telegram-hover/30 hover:bg-telegram-hover/60 text-telegram-subtext text-xs"
          >
            ✕
          </button>
        </div>

        {/* Scrollable Folder List */}
        <nav className="flex-1 px-3 py-4 space-y-1.5 overflow-y-auto min-h-0">
          <button
            onClick={() => {
              setActiveFolderId(null);
              setIsSidebarOpen(false);
            }}
            className={`w-full flex items-center justify-between px-3.5 py-2.5 rounded-xl text-xs font-semibold transition-all duration-200 ${activeFolderId === null
                ? 'bg-telegram-primary/15 text-telegram-primary border border-telegram-primary/15'
                : 'text-telegram-subtext hover:bg-telegram-hover/40 hover:text-telegram-text border border-transparent'
              }`}
          >
            <span>Saved Messages</span>
          </button>

          {folders.map(folder => {
            const isPublic = folder.is_public || !!folder.username;
            return (
            <div key={folder.id} className="flex items-center gap-1">
              <button
                onClick={() => {
                  setActiveFolderId(folder.id);
                  setIsSidebarOpen(false);
                }}
                className={`flex-1 text-left px-3.5 py-2.5 rounded-xl text-xs font-semibold transition-all duration-200 ${
                  activeFolderId === folder.id
                    ? 'bg-telegram-primary/15 text-telegram-primary border border-telegram-primary/15'
                    : 'text-telegram-subtext hover:bg-telegram-hover/40 hover:text-telegram-text border border-transparent'
                }`}
              >
                <span className="flex items-center gap-1.5 max-w-[150px]">
                  <span className="truncate">{folder.name}</span>
                  {isPublic ? (
                    <Globe className="w-3 h-3 text-emerald-400 flex-shrink-0" />
                  ) : (
                    <Lock className="w-3 h-3 text-amber-400/60 flex-shrink-0" />
                  )}
                </span>
              </button>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  setFolderActionMenu(folder);
                }}
                className="flex-shrink-0 p-2 rounded-xl hover:bg-telegram-hover/40 active:bg-telegram-hover/60 text-telegram-subtext/60 hover:text-telegram-subtext transition-all duration-200"
                aria-label="Folder actions"
              >
                <MoreVertical className="w-3.5 h-3.5" />
              </button>
            </div>
            );
          })}
        </nav>

        {/* Action Panel & Connection Status */}
        <div className="px-4 py-3 border-t border-telegram-border/30 space-y-3">
          <button
            onClick={async () => {
              const name = prompt("Enter folder name:");
              if (name && name.trim()) {
                await handleCreateFolder(name.trim());
              }
            }}
            className="w-full flex items-center justify-center gap-2 py-2.5 rounded-xl text-xs font-bold text-telegram-subtext hover:text-telegram-text border border-dashed border-telegram-border/60 hover:bg-telegram-hover/20 transition-all duration-200"
          >
            + Create Folder
          </button>
          <div className="flex items-center gap-2 text-telegram-subtext text-[10px] font-semibold uppercase tracking-wider">
            <span className={`w-1.5 h-1.5 rounded-full ${isConnected ? 'bg-green-500 animate-pulse' : 'bg-red-500'}`} />
            <span>{isConnected ? 'Connected' : 'Offline'}</span>
          </div>
        </div>
      </div>

      {/* Folder action popover (replaces swipe-to-reveal) */}
      {folderActionMenu && (
        <ActionPopover
          title={folderActionMenu.name}
          actions={buildFolderActions(folderActionMenu)}
          onClose={() => setFolderActionMenu(null)}
        />
      )}

      {/* Rename folder bottom sheet */}
      {renameFolder && (
        <RenameFolderSheet
          folderId={renameFolder.id}
          currentName={renameFolder.name}
          onRename={handleFolderRename}
          onClose={() => setRenameFolder(null)}
        />
      )}

      {/* Floating Bottom Nav Bar */}
      <BottomNavBar activeTab={activeTab} setActiveTab={setActiveTab} isAndroid={isAndroid} />

      {/* Adsterra Banner (Android only) — z-[60] keeps it above the BottomNavBar (z-50).
           Positioned at bottom-[144px] to sit cleanly above the nav bar (~60px tall, at bottom-20=80px). */}
      <div className="fixed bottom-[144px] left-0 right-0 z-[60]">
        <AdsterraBanner visible={adVisible} />
      </div>

      {/* Previews Overlays (Media, PDF & Images) */}
      {playingFile && (
        <div className="fixed inset-0 z-[100] bg-black/90">
          <MediaPlayer
            key={playingFile.id}
            file={playingFile}
            onClose={() => setPlayingFile(null)}
            activeFolderId={activeFolderId}
          />
        </div>
      )}
      {pdfFile && (
        <div className="fixed inset-0 z-[100] bg-telegram-bg">
          <PdfViewer
            file={pdfFile}
            onClose={() => setPdfFile(null)}
            activeFolderId={activeFolderId}
          />
        </div>
      )}
      {previewFile && (
        <PreviewModal
          file={previewFile}
          activeFolderId={activeFolderId}
          onClose={() => setPreviewFile(null)}
        />
      )}

      {shareFile && (
        <ShareDialog
          file={shareFile}
          onClose={() => setShareFile(null)}
        />
      )}

      {/* Bulk Share Results Modal */}
      {bulkShareLinks && (
        <div
          className="fixed inset-0 z-[150] flex items-end justify-center bg-black/50 backdrop-blur-sm"
          onClick={() => setBulkShareLinks(null)}
        >
          <div
            className="w-full max-w-lg bg-[#1c1c1e] border border-white/10 rounded-t-3xl p-5 pb-8 shadow-2xl animate-in slide-in-from-bottom duration-300 max-h-[70vh] flex flex-col"
            onClick={e => e.stopPropagation()}
          >
            {/* Drag handle */}
            <div className="flex justify-center mb-4">
              <div className="w-10 h-1 rounded-full bg-white/20" />
            </div>

            <div className="flex items-center justify-between mb-4">
              <h3 className="text-sm font-bold text-white flex items-center gap-2">
                <Link className="w-4 h-4 text-telegram-primary" />
                {bulkShareLinks.length} Share Link{bulkShareLinks.length !== 1 ? 's' : ''}
              </h3>
              <button
                onClick={() => setBulkShareLinks(null)}
                className="p-1.5 rounded-lg hover:bg-white/10 text-telegram-subtext"
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            {bulkShareLoading ? (
              <div className="flex flex-col items-center justify-center py-12 space-y-3">
                <Loader2 className="w-8 h-8 text-telegram-primary animate-spin" />
                <p className="text-xs text-telegram-subtext">Generating share links...</p>
              </div>
            ) : (
              <div className="flex-1 overflow-y-auto space-y-2 min-h-0">
                {bulkShareLinks.map(({ file, link }) => {
                  const isCopied = bulkShareCopied.has(link);
                  return (
                    <div
                      key={file.id}
                      className="p-3 rounded-xl bg-white/5 border border-white/5 space-y-2"
                    >
                      <p className="text-xs font-semibold text-white truncate">{file.name}</p>
                      <div className="flex gap-2">
                        <input
                          type="text"
                          readOnly
                          value={link}
                          className="flex-1 bg-black/30 border border-white/10 rounded-lg px-2.5 py-1.5 text-[11px] text-telegram-subtext focus:outline-none select-all truncate"
                        />
                        <button
                          onClick={() => handleCopyBulkLink(link)}
                          className={`px-2.5 py-1.5 rounded-lg flex items-center justify-center transition-all flex-shrink-0 ${
                            isCopied
                              ? 'bg-emerald-500 border-emerald-500 text-white'
                              : 'bg-white/10 border border-white/10 text-telegram-subtext hover:bg-white/20'
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
              className="w-full mt-3 flex items-center justify-center gap-2 px-4 py-3 rounded-2xl text-sm font-semibold bg-white/5 text-telegram-subtext hover:bg-white/10 border border-white/5 transition-all duration-200 active:scale-[0.98] flex-shrink-0"
            >
              Done
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
