import { useState, useCallback, useMemo, useEffect } from 'react';
import { Folder, Download, Menu, LogOut, RefreshCw, UploadCloud, MoreVertical, Trash2, Pencil, Globe, Shield, ChevronDown } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-shell';
import { useQuery } from '@tanstack/react-query';
import { toast } from 'sonner';
import { BottomNavBar } from './BottomNavBar';
import { TouchFileList } from './TouchFileList';
import { ThemeToggle } from '../shared/ThemeToggle';
import AdsterraBanner from '../shared/AdsterraBanner';
import { ActionPopover, ActionItem } from './ActionPopover';
import { usePlatform } from '../../hooks/usePlatform';
import { useTelegramConnection } from '../../hooks/useTelegramConnection';
import { useFileUpload } from '../../hooks/useFileUpload';
import { useFileDownload } from '../../hooks/useFileDownload';
import { useFileOperations } from '../../hooks/useFileOperations';
import { formatBytes, isMediaFile, isPdfFile, isImageFile } from '../../utils';
import { MediaPlayer } from '../desktop/dashboard/MediaPlayer';
import { PdfViewer } from '../desktop/dashboard/PdfViewer';
import { PreviewModal } from '../desktop/dashboard/PreviewModal';
import { useTheme } from '../../context/ThemeContext';
import { TelegramFile, TelegramFolder } from '../../types';
import { useSettings } from '../../context/SettingsContext';
import { version as appVersion } from '../../../package.json';

export default function MobileDashboard({ onLogout }: { onLogout?: () => void }) {
  const [activeTab, setActiveTab] = useState<'files' | 'downloads' | 'settings'>('files');
  const [isSidebarOpen, setIsSidebarOpen] = useState(false);
  const { isAndroid } = usePlatform();
  const { theme } = useTheme();
  const { settings, updateSetting } = useSettings();

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
          secret: settings.proxySecret,
        });
      } catch {
        // best-effort sync
      }
    };
    applyProxy();
  }, [
    settings.proxyEnabled, settings.proxyType, settings.proxyHost,
    settings.proxyPort, settings.proxyUsername, settings.proxyPassword,
    settings.proxySecret,
  ]);

  const logoutHandler = useMemo(() => onLogout || (() => {}), [onLogout]);

  const {
    store, folders, activeFolderId, setActiveFolderId, isSyncing, isConnected,
    handleLogout, handleSyncFolders, handleCreateFolder, handleFolderDelete,
    handleFolderRename
  } = useTelegramConnection(logoutHandler);

  const { handleManualUpload } = useFileUpload(activeFolderId, store);
  const { queueDownload } = useFileDownload(store);

  const [playingFile, setPlayingFile] = useState<TelegramFile | null>(null);
  const [pdfFile, setPdfFile] = useState<TelegramFile | null>(null);
  const [previewFile, setPreviewFile] = useState<TelegramFile | null>(null);

  const adVisible = !playingFile && !pdfFile && !previewFile;

  // Real files loader
  const { data: allFiles = [], isLoading } = useQuery({
    queryKey: ['files', activeFolderId],
    queryFn: () => invoke<any[]>('cmd_get_files', { folderId: activeFolderId }).then(res => res.map(f => ({
      ...f,
      sizeStr: formatBytes(f.size),
      type: f.icon_type || (f.name.endsWith('/') ? 'folder' : 'file')
    }))),
    enabled: !!store,
  });

  const [selectedIds, setSelectedIds] = useState<number[]>([]);
  const [fileRenames, setFileRenames] = useState<Map<number, string>>(new Map());
  const { handleDelete: handleDeleteOp, handleBulkDelete, handleBulkDownload, handleBulkMove } = useFileOperations(activeFolderId, selectedIds, setSelectedIds, allFiles);

  const activeFolder = activeFolderId === null
    ? 'Saved Messages'
    : folders.find(f => f.id === activeFolderId)?.name || 'Unknown Channel';

  // Folder action menu state (replaces swipe-to-reveal)
  const [folderActionMenu, setFolderActionMenu] = useState<TelegramFolder | null>(null);

  const buildFolderActions = useCallback((folder: TelegramFolder): ActionItem[] => [
    {
      label: 'Rename',
      icon: <Pencil className="w-4 h-4" />,
      onClick: () => handleFolderRename(folder.id, folder.name),
    },
    {
      label: 'Delete',
      icon: <Trash2 className="w-4 h-4" />,
      onClick: () => handleFolderDelete(folder.id, folder.name),
      destructive: true,
    },
  ], [handleFolderRename, handleFolderDelete]);

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

  const handlePreview = useCallback((file: TelegramFile) => {
    if (isMediaFile(file.name)) {
      setPlayingFile(file);
    } else if (isPdfFile(file.name)) {
      setPdfFile(file);
    } else if (isImageFile(file.name)) {
      setPreviewFile(file);
    } else {
      toast.info(`Preview not supported for ${file.name}`);
    }
  }, []);

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
              isLoading={isLoading}
              onDownload={handleDownload}
              onDelete={handleDeleteFile}
              onPreview={handlePreview}
              onRename={handleRenameFile}
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
              <h3 className="text-sm font-bold text-telegram-primary tracking-wide uppercase text-[10px]">Preferences</h3>
              <div className="flex items-center justify-between py-2 border-b border-telegram-border/20">
                <div>
                  <p className="text-xs font-medium">Zip Folders Before Upload</p>
                  <p className="text-[10px] text-telegram-subtext">Compress folders into .zip before uploading</p>
                </div>
                <button
                  onClick={() => updateSetting('zipFolders', !settings.zipFolders)}
                  className={`relative w-11 h-6 rounded-full transition-colors duration-200 flex-shrink-0 ${settings.zipFolders ? 'bg-telegram-primary' : 'bg-telegram-border'}`}
                >
                  <span className={`absolute top-0.5 left-0.5 w-5 h-5 rounded-full bg-white shadow transition-transform duration-200 ${settings.zipFolders ? 'translate-x-5' : 'translate-x-0'}`} />
                </button>
              </div>

              <div className="flex items-center justify-between py-2 border-b border-telegram-border/20">
                <div>
                  <p className="text-xs font-medium">Upload Videos as Media</p>
                  <p className="text-[10px] text-telegram-subtext">Send videos as streamable media instead of files</p>
                </div>
                <button
                  onClick={() => updateSetting('uploadVideoAsMedia', !settings.uploadVideoAsMedia)}
                  className={`relative w-11 h-6 rounded-full transition-colors duration-200 flex-shrink-0 ${settings.uploadVideoAsMedia ? 'bg-telegram-primary' : 'bg-telegram-border'}`}
                >
                  <span className={`absolute top-0.5 left-0.5 w-5 h-5 rounded-full bg-white shadow transition-transform duration-200 ${settings.uploadVideoAsMedia ? 'translate-x-5' : 'translate-x-0'}`} />
                </button>
              </div>

              <div className="flex items-center justify-between py-2">
                <div>
                  <p className="text-xs font-medium">Delete File After Upload</p>
                  <p className="text-[10px] text-telegram-subtext">Automatically delete local files once uploaded</p>
                </div>
                <button
                  onClick={() => updateSetting('deleteAfterUpload', !settings.deleteAfterUpload)}
                  className={`relative w-11 h-6 rounded-full transition-colors duration-200 flex-shrink-0 ${settings.deleteAfterUpload ? 'bg-telegram-primary' : 'bg-telegram-border'}`}
                >
                  <span className={`absolute top-0.5 left-0.5 w-5 h-5 rounded-full bg-white shadow transition-transform duration-200 ${settings.deleteAfterUpload ? 'translate-x-5' : 'translate-x-0'}`} />
                </button>
              </div>
            </div>

            {/* Proxy Configuration */}
            <div className="p-4 rounded-2xl bg-telegram-hover/20 border border-telegram-border/30 space-y-4">
              <h3 className="text-sm font-bold text-telegram-primary tracking-wide uppercase text-[10px] flex items-center gap-1.5">
                <Shield className="w-3 h-3" />
                Proxy
              </h3>

              {/* Enable Proxy Toggle */}
              <div className="flex items-center justify-between py-2 border-b border-telegram-border/20">
                <div>
                  <p className="text-xs font-medium">Enable Proxy</p>
                  <p className="text-[10px] text-telegram-subtext">Route traffic through a proxy server</p>
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
                  <p className="text-xs font-medium">Proxy Type</p>
                  <p className="text-[10px] text-telegram-subtext">SOCKS5 or MTProto</p>
                </div>
                <div className="relative">
                  <select
                    value={settings.proxyType}
                    onChange={e => updateSetting('proxyType', e.target.value as 'socks5' | 'mtproto')}
                    className="appearance-none bg-telegram-bg border border-telegram-border rounded-lg pl-2.5 pr-7 py-1.5 text-xs text-telegram-text focus:outline-none focus:border-telegram-primary/50 transition cursor-pointer"
                  >
                    <option value="socks5">SOCKS5</option>
                    <option value="mtproto">MTProto</option>
                  </select>
                  <ChevronDown className="w-3.5 h-3.5 text-telegram-subtext absolute right-2 top-1/2 -translate-y-1/2 pointer-events-none" />
                </div>
              </div>

              {/* Host */}
              <div className="flex items-center justify-between py-2 border-b border-telegram-border/20">
                <div>
                  <p className="text-xs font-medium">Host</p>
                  <p className="text-[10px] text-telegram-subtext">Proxy server address</p>
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
                  <p className="text-xs font-medium">Port</p>
                  <p className="text-[10px] text-telegram-subtext">1–65535</p>
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
                      <p className="text-xs font-medium">Username</p>
                      <p className="text-[10px] text-telegram-subtext">Optional</p>
                    </div>
                    <input
                      type="text"
                      placeholder="Optional"
                      value={settings.proxyUsername}
                      onChange={e => updateSetting('proxyUsername', e.target.value)}
                      className="w-32 bg-telegram-bg border border-telegram-border rounded-lg px-2 py-1.5 text-xs text-telegram-text text-right focus:outline-none focus:border-telegram-primary/50 transition placeholder:text-telegram-subtext/40"
                    />
                  </div>
                  <div className="flex items-center justify-between py-2">
                    <div>
                      <p className="text-xs font-medium">Password</p>
                      <p className="text-[10px] text-telegram-subtext">Optional</p>
                    </div>
                    <input
                      type="password"
                      placeholder="Optional"
                      value={settings.proxyPassword}
                      onChange={e => updateSetting('proxyPassword', e.target.value)}
                      className="w-32 bg-telegram-bg border border-telegram-border rounded-lg px-2 py-1.5 text-xs text-telegram-text text-right focus:outline-none focus:border-telegram-primary/50 transition placeholder:text-telegram-subtext/40"
                    />
                  </div>
                </>
              )}

              {/* MTProto secret */}
              {settings.proxyType === 'mtproto' && (
                <div className="flex items-center justify-between py-2">
                  <div>
                    <p className="text-xs font-medium">Secret</p>
                    <p className="text-[10px] text-telegram-subtext">MTProto proxy secret key</p>
                  </div>
                  <input
                    type="password"
                    placeholder="Required"
                    value={settings.proxySecret}
                    onChange={e => updateSetting('proxySecret', e.target.value)}
                    className="w-32 bg-telegram-bg border border-telegram-border rounded-lg px-2 py-1.5 text-xs text-telegram-text text-right focus:outline-none focus:border-telegram-primary/50 transition placeholder:text-telegram-subtext/40"
                  />
                </div>
              )}

              {/* Info note */}
              <div className="p-2.5 rounded-lg bg-yellow-500/5 border border-yellow-500/10">
                <p className="text-[10px] text-yellow-400/70 leading-relaxed">
                  ⚠️ Proxy changes require reconnecting.
                </p>
              </div>
            </div>

            <div className="p-4 rounded-2xl bg-telegram-hover/20 border border-telegram-border/30 space-y-4">
              <h3 className="text-sm font-bold text-telegram-primary tracking-wide uppercase text-[10px]">About</h3>
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
                    onClick={(e) => { e.preventDefault(); open('https://www.cameronamer.com'); }}
                    className="flex items-center justify-center gap-1.5 text-[11px] text-telegram-primary hover:text-telegram-primary/80 transition-colors cursor-pointer"
                  >
                    <Globe className="w-3 h-3" />
                    www.cameronamer.com
                  </button>

                  <button
                    onClick={(e) => { e.preventDefault(); open('https://github.com/caamer20/telegram-drive'); }}
                    className="flex items-center justify-center gap-1.5 text-[11px] text-telegram-primary hover:text-telegram-primary/80 transition-colors cursor-pointer"
                  >
                    <svg className="w-3 h-3" viewBox="0 0 24 24" fill="currentColor">
                      <path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z"/>
                    </svg>
                    github.com/caamer20/telegram-drive
                  </button>
                </div>

                <p className="text-[10px] text-telegram-subtext/60 leading-relaxed text-center px-2">
                  Turn your Telegram account into unlimited, secure cloud storage.
                  Open-source and free forever.
                </p>
              </div>
            </div>

            <button onClick={handleLogout} className="w-full flex items-center justify-center gap-2 py-3 rounded-2xl bg-red-500/10 hover:bg-red-500/20 text-red-400 border border-red-500/20 font-semibold text-xs active:scale-98 transition-all duration-200">
              <LogOut className="w-4 h-4" />
              Log Out
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

          {folders.map(folder => (
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
                <span className="truncate block max-w-[150px]">{folder.name}</span>
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
          ))}
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
    </div>
  );
}
