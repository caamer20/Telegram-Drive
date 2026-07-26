import React, { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { MsAccountInfo, OneDriveItem } from '../../types';
import {
    Cloud,
    Folder,
    HardDrive,
    Send,
    LogOut,
    LogIn,
    ChevronRight,
    FolderTree,
    Check,
    FolderCheck,
} from 'lucide-react';

interface SetupSectionProps {
    msAccount: MsAccountInfo | null;
    loading: boolean;
    sourceFolderPath: string;
    telegramDestName: string;
    localDir: string;
    onConnectMs: (clientId?: string, tenant?: string) => void;
    onDisconnectMs: () => void;
    onListOneDriveFolders: (parentId?: string) => Promise<OneDriveItem[]>;
    onSetSourceFolder: (folderId: string, folderPath: string) => void;
    onSetTelegramDest: (destId: number | null, destName: string) => void;
    onSetLocalDir: (localDir: string) => void;
}

export const SetupSection: React.FC<SetupSectionProps> = ({
    msAccount,
    loading,
    sourceFolderPath,
    telegramDestName,
    localDir,
    onConnectMs,
    onDisconnectMs,
    onListOneDriveFolders,
    onSetSourceFolder,
    onSetTelegramDest,
    onSetLocalDir,
}) => {
    const { t } = useTranslation();
    const [tenant, setTenant] = useState<string>('common');

    const [treeItems, setTreeItems] = useState<OneDriveItem[]>([]);
    const [currentParentId, setCurrentParentId] = useState<string | undefined>(undefined);
    const [pathHistory, setPathHistory] = useState<{ id?: string; name: string }[]>([
        { name: 'OneDrive Root' },
    ]);

    // Load OneDrive items when MS connected or parent changes
    useEffect(() => {
        if (msAccount) {
            onListOneDriveFolders(currentParentId).then((items) => {
                setTreeItems(items.filter((i) => i.item_type === 'folder'));
            });
        } else {
            setTreeItems([]);
        }
    }, [msAccount, currentParentId, onListOneDriveFolders]);

    const handleSelectFolder = (item: OneDriveItem) => {
        const fullPath = pathHistory
            .map((p) => p.name)
            .concat(item.name)
            .join('/')
            .replace('OneDrive Root/', '/');
        onSetSourceFolder(item.id, fullPath);
    };

    const handleNavigateInto = (item: OneDriveItem) => {
        setCurrentParentId(item.id);
        setPathHistory((prev) => [...prev, { id: item.id, name: item.name }]);
    };

    const handleBreadcrumbClick = (index: number) => {
        const newHistory = pathHistory.slice(0, index + 1);
        setPathHistory(newHistory);
        setCurrentParentId(newHistory[newHistory.length - 1].id);
    };

    const handleSelectLocalDir = async () => {
        try {
            const selected = await openDialog({
                directory: true,
                multiple: false,
                title: t('migration.select_local_dir', 'Select Local Working Directory'),
            });
            if (selected && typeof selected === 'string') {
                onSetLocalDir(selected);
            }
        } catch (e) {
            console.error('Failed to open folder picker:', e);
        }
    };

    return (
        <div className="space-y-6">
            {/* Step 1: Microsoft Account Connection */}
            <div className="bg-slate-900/60 rounded-xl border border-slate-800 p-5 space-y-4">
                <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
                    <div className="flex items-center gap-3">
                        <div className="p-2.5 rounded-lg bg-blue-500/10 text-blue-400 border border-blue-500/20">
                            <Cloud className="w-5 h-5" />
                        </div>
                        <div>
                            <h3 className="font-semibold text-slate-100 text-sm">
                                {t('migration.step1_ms_account', '1. Microsoft Account')}
                            </h3>
                            <p className="text-xs text-slate-400">
                                {msAccount
                                    ? t('migration.connected_status', 'Connected to OneDrive')
                                    : t('migration.connect_prompt', 'Connect your Microsoft account to browse OneDrive')}
                            </p>
                        </div>
                    </div>

                    {msAccount ? (
                        <div className="flex items-center gap-3">
                            <div className="text-right hidden sm:block">
                                <p className="text-xs font-semibold text-slate-200">{msAccount.account_name}</p>
                                <p className="text-[11px] text-slate-400">{msAccount.account_email}</p>
                            </div>
                            <button
                                onClick={onDisconnectMs}
                                disabled={loading}
                                className="inline-flex items-center gap-1.5 px-3 py-1.5 bg-slate-800 hover:bg-rose-950/40 text-slate-300 hover:text-rose-300 border border-slate-700 rounded-lg text-xs font-medium transition-colors"
                            >
                                <LogOut className="w-3.5 h-3.5" />
                                {t('migration.btn_disconnect', 'Disconnect')}
                            </button>
                        </div>
                    ) : (
                        <div className="flex flex-col sm:flex-row items-stretch sm:items-center gap-2">
                            <select
                                value={tenant}
                                onChange={(e) => setTenant(e.target.value)}
                                className="px-2.5 py-1.5 bg-slate-950 border border-slate-800 rounded-lg text-xs text-slate-200 focus:outline-none focus:border-blue-500"
                            >
                                <option value="common">Common (Multi-tenant)</option>
                                <option value="consumers">Consumers (Personal MS)</option>
                                <option value="organizations">Organizations (Work/School)</option>
                            </select>
                            <button
                                onClick={() => onConnectMs(undefined, tenant)}
                                disabled={loading}
                                className="inline-flex items-center justify-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-500 text-white rounded-lg text-xs font-semibold shadow-lg transition-all shrink-0"
                            >
                                <LogIn className="w-4 h-4" />
                                {t('migration.btn_connect_ms', 'Connect Microsoft')}
                            </button>
                        </div>
                    )}

                </div>
            </div>

            {/* Step 2: OneDrive Source Folder Picker (Tree View) */}
            {msAccount && (
                <div className="bg-slate-900/60 rounded-xl border border-slate-800 p-5 space-y-4">
                    <div className="flex items-center justify-between">
                        <div className="flex items-center gap-3">
                            <div className="p-2.5 rounded-lg bg-indigo-500/10 text-indigo-400 border border-indigo-500/20">
                                <FolderTree className="w-5 h-5" />
                            </div>
                            <div>
                                <h3 className="font-semibold text-slate-100 text-sm">
                                    {t('migration.step2_source_folder', '2. OneDrive Source Folder')}
                                </h3>
                                <p className="text-xs text-slate-400">
                                    {sourceFolderPath
                                        ? t('migration.selected_source', 'Selected: {{path}}', { path: sourceFolderPath })
                                        : t('migration.select_source_prompt', 'Browse and select a folder to migrate')}
                                </p>
                            </div>
                        </div>

                        {sourceFolderPath && (
                            <span className="inline-flex items-center gap-1.5 px-3 py-1 bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 rounded-full text-xs font-medium">
                                <FolderCheck className="w-3.5 h-3.5" />
                                {t('migration.selected', 'Selected')}
                            </span>
                        )}
                    </div>

                    {/* Breadcrumbs */}
                    <div className="flex items-center gap-1 text-xs text-slate-400 overflow-x-auto pb-1">
                        {pathHistory.map((item, idx) => (
                            <React.Fragment key={idx}>
                                {idx > 0 && <ChevronRight className="w-3 h-3 text-slate-600 flex-shrink-0" />}
                                <button
                                    onClick={() => handleBreadcrumbClick(idx)}
                                    className={`hover:text-blue-400 font-medium truncate max-w-[150px] ${
                                        idx === pathHistory.length - 1 ? 'text-slate-200' : 'text-slate-400'
                                    }`}
                                >
                                    {item.name}
                                </button>
                            </React.Fragment>
                        ))}
                    </div>

                    {/* Folder List Tree */}
                    <div className="bg-slate-950/80 rounded-lg border border-slate-800/80 divide-y divide-slate-800/50 max-h-56 overflow-y-auto custom-scrollbar">
                        {treeItems.length === 0 ? (
                            <div className="p-4 text-center text-xs text-slate-500">
                                {t('migration.empty_folder', 'No subfolders found in this directory')}
                            </div>
                        ) : (
                            treeItems.map((item) => (
                                <div
                                    key={item.id}
                                    className="flex items-center justify-between px-4 py-2.5 hover:bg-slate-800/40 transition-colors"
                                >
                                    <div
                                        onClick={() => handleNavigateInto(item)}
                                        className="flex items-center gap-2.5 cursor-pointer flex-1 min-w-0"
                                    >
                                        <Folder className="w-4 h-4 text-indigo-400 flex-shrink-0" />
                                        <span className="text-xs font-medium text-slate-200 truncate">
                                            {item.name}
                                        </span>
                                        {item.child_count != null && (
                                            <span className="text-[10px] text-slate-500 bg-slate-900 px-1.5 py-0.5 rounded">
                                                {item.child_count} items
                                            </span>
                                        )}
                                    </div>

                                    <button
                                        onClick={() => handleSelectFolder(item)}
                                        className="inline-flex items-center gap-1 px-2.5 py-1 bg-slate-800 hover:bg-indigo-600 text-slate-300 hover:text-white rounded text-xs font-medium transition-colors ml-2"
                                    >
                                        <Check className="w-3 h-3" />
                                        {t('migration.btn_select', 'Select')}
                                    </button>
                                </div>
                            ))
                        )}
                    </div>
                </div>
            )}

            {/* Step 3: Telegram Destination & Local Directory */}
            {msAccount && (
                <div className="bg-slate-900/60 rounded-xl border border-slate-800 p-5 space-y-5">
                    <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                        {/* Telegram Destination */}
                        <div className="p-4 bg-slate-950/60 rounded-lg border border-slate-800/80 space-y-2">
                            <div className="flex items-center gap-2 text-xs font-semibold text-slate-200">
                                <Send className="w-4 h-4 text-blue-400" />
                                {t('migration.step3_telegram_dest', 'Telegram Destination')}
                            </div>
                            <p className="text-xs text-slate-400">
                                {telegramDestName || t('migration.saved_messages', 'Saved Messages (Default)')}
                            </p>
                            <button
                                onClick={() => onSetTelegramDest(null, 'Saved Messages')}
                                className="px-3 py-1.5 bg-slate-800 hover:bg-slate-700 text-slate-200 rounded text-xs font-medium transition-colors"
                            >
                                {t('migration.use_saved_messages', 'Use Saved Messages')}
                            </button>
                        </div>

                        {/* Local Working Directory */}
                        <div className="p-4 bg-slate-950/60 rounded-lg border border-slate-800/80 space-y-2">
                            <div className="flex items-center gap-2 text-xs font-semibold text-slate-200">
                                <HardDrive className="w-4 h-4 text-emerald-400" />
                                {t('migration.step4_local_dir', 'Local Working Directory')}
                            </div>
                            <p className="text-xs text-slate-400 truncate" title={localDir || ''}>
                                {localDir || t('migration.no_local_dir', 'No local directory selected')}
                            </p>
                            <button
                                onClick={handleSelectLocalDir}
                                className="px-3 py-1.5 bg-slate-800 hover:bg-slate-700 text-slate-200 rounded text-xs font-medium transition-colors"
                            >
                                {t('migration.btn_browse_local', 'Browse Directory...')}
                            </button>
                        </div>
                    </div>
                </div>
            )}
        </div>
    );
};
