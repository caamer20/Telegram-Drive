import { useState } from 'react';
import { HardDrive, Folder, Plus, RefreshCw, LogOut } from 'lucide-react';
import { SidebarItem } from './SidebarItem';
import { BandwidthWidget } from './BandwidthWidget';
import { TelegramFolder, BandwidthStats } from '../../types';

interface SidebarProps {
    folders: TelegramFolder[];
    activeFolderId: number | null;
    setActiveFolderId: (id: number | null) => void;
    onDrop: (e: React.DragEvent, folderId: number | null) => void;
    onDelete: (id: number, name: string) => void;
    onCreate: (name: string) => Promise<void>;
    isSyncing: boolean;
    isConnected: boolean;
    onSync: () => void;
    onLogout: () => void;
    bandwidth: BandwidthStats | null;
}

export function Sidebar({
    folders, activeFolderId, setActiveFolderId, onDrop, onDelete, onCreate,
    isSyncing, isConnected, onSync, onLogout, bandwidth
}: SidebarProps) {
    const [showNewFolderInput, setShowNewFolderInput] = useState(false);
    const [newFolderName, setNewFolderName] = useState("");

    const submitCreate = async () => {
        if (!newFolderName.trim()) return;
        try {
            await onCreate(newFolderName);
            setNewFolderName("");
            setShowNewFolderInput(false);
        } catch {
            // handled by parent
        }
    }

    return (
        <aside className="w-64 bg-telegram-surface border-r border-telegram-border flex flex-col" onClick={e => e.stopPropagation()}>
            <div className="p-4 flex items-center gap-2">
                <img src="/logo.svg" className="w-8 h-8 drop-shadow-lg" alt="Logo" />
                <span className="font-bold text-lg text-telegram-text tracking-tight">Telegram Drive</span>
            </div>

            <nav className="flex-1 px-2 py-4 space-y-1">
                <SidebarItem
                    icon={HardDrive}
                    label="Saved Messages"
                    active={activeFolderId === null}
                    onClick={() => setActiveFolderId(null)}
                    onDrop={(e: React.DragEvent) => onDrop(e, null)}
                    folderId={null}
                />
                {folders.map(folder => (
                    <SidebarItem
                        key={folder.id}
                        icon={Folder}
                        label={folder.name}
                        active={activeFolderId === folder.id}
                        onClick={() => setActiveFolderId(folder.id)}
                        onDrop={(e: React.DragEvent) => onDrop(e, folder.id)}
                        onDelete={() => onDelete(folder.id, folder.name)}
                        folderId={folder.id}
                    />
                ))}


                {showNewFolderInput ? (
                    <div className="px-3 py-2">
                        <input
                            autoFocus
                            type="text"
                            className="w-full bg-white/10 rounded px-2 py-1 text-sm text-white focus:outline-none focus:ring-1 focus:ring-telegram-primary"
                            placeholder="Folder Name"
                            value={newFolderName}
                            onChange={e => setNewFolderName(e.target.value)}
                            onKeyDown={e => e.key === 'Enter' && submitCreate()}
                            onBlur={() => !newFolderName && setShowNewFolderInput(false)}
                        />
                    </div>
                ) : (
                    <button
                        onClick={() => setShowNewFolderInput(true)}
                        className="w-full flex items-center gap-3 px-3 py-2 rounded-lg text-sm font-medium text-telegram-subtext hover:bg-telegram-hover hover:text-telegram-text transition-colors border border-dashed border-telegram-border mt-2"
                    >
                        <Plus className="w-4 h-4" />
                        Create Folder
                    </button>
                )}
            </nav>

            <div className="p-4 border-t border-telegram-border">
                <div className="flex items-center gap-2 text-telegram-subtext text-xs">
                    <div className={`w-2 h-2 rounded-full ${isConnected ? 'bg-green-500 animate-pulse' : 'bg-red-500'}`}></div>
                    <span>{isConnected ? 'Connected to Telegram' : 'Disconnected from Telegram'}</span>
                </div>

                <div className="flex gap-2 mt-4">
                    <button
                        onClick={onSync}
                        disabled={isSyncing}
                        className={`flex-1 flex items-center justify-center gap-2 px-3 py-2 text-xs font-medium text-blue-500 hover:text-blue-600 bg-blue-500/10 hover:bg-blue-500/20 rounded-lg transition-colors ${isSyncing ? 'opacity-50 cursor-not-allowed' : ''}`}
                        title="Scan for existing folders"
                    >
                        <RefreshCw className={`w-3 h-3 ${isSyncing ? 'animate-spin' : ''}`} />
                        {isSyncing ? 'Syncing...' : 'Sync'}
                    </button>
                    <button
                        onClick={onLogout}
                        className="flex-1 flex items-center justify-center gap-2 px-3 py-2 text-xs font-medium text-red-500 hover:text-red-600 bg-red-500/10 hover:bg-red-500/20 rounded-lg transition-colors"
                        title="Sign Out"
                    >
                        <LogOut className="w-3 h-3" />
                        Logout
                    </button>
                </div>

                {bandwidth && <BandwidthWidget bandwidth={bandwidth} />}
            </div>

        </aside>
    )
}
