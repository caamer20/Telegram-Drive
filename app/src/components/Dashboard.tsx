import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { HardDrive, Folder, File, LayoutGrid, Plus, Eye, X, LogOut, RefreshCw } from 'lucide-react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Store } from '@tauri-apps/plugin-store';
import { open } from '@tauri-apps/plugin-dialog';

interface QueueItem {
    id: string;
    path: string;
    folderId: number | null;
    status: 'pending' | 'uploading' | 'success' | 'error';
    error?: string;
}

export function Dashboard({ onLogout }: { onLogout: () => void }) {
    const queryClient = useQueryClient();

    const [folders, setFolders] = useState<any[]>([]);
    const [uploadQueue, setUploadQueue] = useState<QueueItem[]>([]);
    const [processing, setProcessing] = useState(false);
    const [activeFolderId, setActiveFolderId] = useState<number | null>(null);
    const [store, setStore] = useState<Store | null>(null);
    const [showNewFolderInput, setShowNewFolderInput] = useState(false);
    const [newFolderName, setNewFolderName] = useState("");
    const [previewFile, setPreviewFile] = useState<any>(null);
    const [isSyncing, setIsSyncing] = useState(false);

    // Logout Handler
    const handleLogout = async () => {
        if (!confirm("Are you sure you want to sign out? This will disconnect your session.")) return;
        try {
            // 1. Backend logout
            await invoke('cmd_logout');
            // Clean cache
            await invoke('cmd_clean_cache');

            // 2. Clear Local Store
            if (store) {
                await store.delete('api_id');
                await store.delete('api_hash');
                await store.delete('folders');
                await store.save();
            }

            // 3. Trigger parent logout
            onLogout();
        } catch (e) {
            console.error("Logout failed:", e);
            alert("Error signing out: " + e);
            // Force logout anyway?
            onLogout();
        }
    };

    const handleSyncFolders = async () => {
        if (!store) return;
        setIsSyncing(true);
        try {
            const foundFolders = await invoke<any[]>('cmd_scan_folders');
            // Merge with existing avoiding duplicates
            const merged = [...folders];
            let added = 0;
            for (const f of foundFolders) {
                if (!merged.find(existing => existing.id === f.id)) {
                    merged.push(f);
                    added++;
                }
            }
            if (added > 0) {
                setFolders(merged);
                await store.set('folders', merged);
                await store.save();
                alert(`Scan complete. Found ${added} previously hidden folders.`);
            } else {
                alert("Scan complete. No new folders found.");
            }
        } catch (e) {
            console.error("Sync failed:", e);
            alert("Sync failed: " + e);
        } finally {
            setIsSyncing(false);
        }
    };

    // Load store and folders on mount
    useEffect(() => {
        const initStore = async () => {
            try {
                // Determine which file to use. AuthWizard uses config.json, we used settings.json?
                // Let's check config.json first as that is where Auth puts it.
                // Actually to fix the bug, we should look at 'config.json' if 'settings.json' is empty or switch entirely.
                // For now, let's load 'config.json' which is what AuthWizard writes to. 
                // Wait, previous code said `Store.load('settings.json')`. 
                // If the user's app is working, `settings.json` must be it.
                // But `AuthWizard` writes to `config.json`. This implies the user might have manually copied or successful login writes to settings?
                // Let's assume we need to align with AuthWizard.
                // I will try to load 'config.json'.
                let _store = await Store.load('config.json');

                // If empty try settings.json (migration or legacy)
                const checkId = await _store.get<string>('api_id');
                if (!checkId) {
                    _store = await Store.load('settings.json');
                }

                setStore(_store);

                const savedFolders = await _store.get<{ id: number, name: string }[]>('folders');
                if (savedFolders) setFolders(savedFolders);

                // Re-connect
                const apiIdStr = await _store.get<string>('api_id');

                if (apiIdStr) {
                    try {
                        const apiId = parseInt(apiIdStr as string);
                        console.log("Connecting to Telegram with ID:", apiId);
                        await invoke('cmd_connect', { apiId });
                        // Now we can fetch files
                        queryClient.invalidateQueries({ queryKey: ['files'] });
                    } catch (e) {
                        console.error("Failed to connect:", e);
                        // If we can't connect, should we logout?
                        // "if it can't reconnect for whatever reason we should be returned to the login screen"
                        const shouldRetry = confirm("Failed to connect to Telegram. Retry?\n\nError: " + e);
                        if (shouldRetry) {
                            window.location.reload();
                        } else {
                            // Force logout logic (without backend call if connect failed)
                            if (_store) {
                                await _store.delete('api_id'); // Clear so valid login is required
                                await _store.save();
                            }
                            onLogout();
                        }
                    }
                } else {
                    // No ID found, should logout
                    onLogout();
                }

            } catch (e) {
                console.error("Failed to load store", e);
            }
        };
        initStore();
    }, [queryClient]);

    // Queue Processor
    useEffect(() => {
        if (processing) return;

        const nextItem = uploadQueue.find(i => i.status === 'pending');
        if (nextItem) {
            processItem(nextItem);
        }
    }, [uploadQueue, processing]);

    const processItem = async (item: QueueItem) => {
        setProcessing(true);
        // Update status to uploading
        setUploadQueue(q => q.map(i => i.id === item.id ? { ...i, status: 'uploading' } : i));

        try {
            await invoke('cmd_upload_file', { path: item.path, folderId: item.folderId });
            setUploadQueue(q => q.map(i => i.id === item.id ? { ...i, status: 'success' } : i));
            // Refresh if current folder matches target
            if (activeFolderId === item.folderId) {
                queryClient.invalidateQueries({ queryKey: ['files', activeFolderId] });
            }
        } catch (e) {
            setUploadQueue(q => q.map(i => i.id === item.id ? { ...i, status: 'error', error: String(e) } : i));
        } finally {
            setProcessing(false);
        }
    };

    const log = (msg: string) => {
        invoke('cmd_log', { message: msg });
        console.log(msg);
    };

    const handleManualUpload = async () => {
        log("Manual upload button clicked");
        try {
            log("Attempting to open file dialog...");
            const selected = await open({
                multiple: true,
                directory: false,
            });

            if (selected) {
                const paths = Array.isArray(selected) ? selected : [selected];
                log(`Queuing ${paths.length} files for upload`);

                const newItems: QueueItem[] = paths.map(path => ({
                    id: Math.random().toString(36).substr(2, 9),
                    path,
                    folderId: activeFolderId,
                    status: 'pending'
                }));

                setUploadQueue(prev => [...prev, ...newItems]);
            }
        } catch (e) {
            log(`Upload Error: ${e}`);
            console.error("Manual upload failed:", e);
        }
    };

    // Listen for file drops
    useEffect(() => {
        const unlistenPromise = getCurrentWindow().listen('tauri://file-drop', async (event: any) => {
            const paths = event.payload as string[];
            if (paths && paths.length > 0) {
                // Add to queue
                const newItems = paths.map(path => ({
                    id: Math.random().toString(36).substr(2, 9),
                    path,
                    folderId: activeFolderId,
                    status: 'pending' as const
                }));
                setUploadQueue(prev => [...prev, ...newItems]);
            }
        });



        return () => {
            unlistenPromise.then(unlisten => unlisten());

        };
    }, [queryClient, activeFolderId]);


    const handleCreateFolder = async () => {
        if (!newFolderName.trim() || !store) return;

        try {
            // Backend Create
            const newFolder = await invoke<{ id: number, name: string, parent_id: any }>('cmd_create_folder', { name: newFolderName });

            const updated = [...folders, newFolder];
            setFolders(updated);
            await store.set('folders', updated);
            await store.save();

            setNewFolderName("");
            setShowNewFolderInput(false);
        } catch (e) {
            alert("Failed to create folder on Telegram: " + e);
        }
    };

    const handleFolderDelete = async (folderId: number, folderName: string) => {
        if (!confirm(`Are you sure you want to delete the folder "${folderName}"? This will delete the channel on Telegram.`)) return;

        try {
            await invoke('cmd_delete_folder', { folderId });

            // Update local state
            const updated = folders.filter(f => f.id !== folderId);
            setFolders(updated);
            if (store) {
                await store.set('folders', updated);
                await store.save();
            }
            if (activeFolderId === folderId) setActiveFolderId(null);

        } catch (e: any) {
            const errStr = String(e); // Ensure string
            if (errStr.includes("not found")) {
                if (confirm(`Folder "${folderName}" not found on Telegram (it may have been deleted externally). Remove from this app?`)) {
                    // Update local state despite backend error
                    const updated = folders.filter(f => f.id !== folderId);
                    setFolders(updated);
                    if (store) {
                        await store.set('folders', updated);
                        await store.save();
                    }
                    if (activeFolderId === folderId) setActiveFolderId(null);
                }
            } else {
                alert(`Failed to delete folder: ${e}`);
            }
        }
    };

    // Fetch files from Backend (filtered by folderId on backend)
    const { data: displayedFiles = [], isLoading, error } = useQuery({
        queryKey: ['files', activeFolderId],
        queryFn: () => invoke<any[]>('cmd_get_files', { folderId: activeFolderId }).then(res => res.map(f => ({
            ...f,
            sizeStr: formatBytes(f.size),
            type: f.icon_type
        }))),
    });

    const { data: bandwidth } = useQuery({
        queryKey: ['bandwidth'],
        queryFn: () => invoke<{ up_bytes: number, down_bytes: number }>('cmd_get_bandwidth'),
        refetchInterval: 5000
    });

    const [viewMode, setViewMode] = useState<'grid' | 'list'>('grid');
    const [selectedIds, setSelectedIds] = useState<number[]>([]);
    const [showMoveModal, setShowMoveModal] = useState(false);

    // Clear selection and modal when changing active folder
    useEffect(() => {
        setSelectedIds([]);
        setShowMoveModal(false);
    }, [activeFolderId]);

    // File Action Handlers
    const handleDelete = async (id: number) => {
        if (!confirm("Are you sure you want to delete this file?")) return;
        try {
            await invoke('cmd_delete_file', { messageId: id, folderId: activeFolderId });
            queryClient.invalidateQueries({ queryKey: ['files', activeFolderId] });
        } catch (e) {
            alert(`Delete failed: ${e}`);
        }
    }

    const handleBulkDelete = async () => {
        if (selectedIds.length === 0) return;
        if (!confirm(`Are you sure you want to delete ${selectedIds.length} files?`)) return;

        for (const id of selectedIds) {
            try {
                await invoke('cmd_delete_file', { messageId: id, folderId: activeFolderId });
            } catch (e) {
                console.error(`Failed to delete ${id}`, e);
            }
        }
        setSelectedIds([]);
        queryClient.invalidateQueries({ queryKey: ['files', activeFolderId] });
    }

    const handleDownload = async (id: number, name: string) => {
        try {
            const savePath = await import('@tauri-apps/plugin-dialog').then(d => d.save({
                defaultPath: name,
            }));
            if (!savePath) return;

            log(`Downloading ${id} to ${savePath}`);
            await invoke('cmd_download_file', { messageId: id, savePath, folderId: activeFolderId });
        } catch (e) {
            alert(`Download failed: ${e}`);
        }
    }

    const handleBulkDownload = async () => {
        if (selectedIds.length === 0) return;

        try {
            const dirPath = await import('@tauri-apps/plugin-dialog').then(d => d.open({
                directory: true,
                multiple: false,
                title: "Select Download Destination Folder"
            }));

            if (!dirPath) return;

            let successCount = 0;
            const targetFiles = displayedFiles.filter((f: any) => selectedIds.includes(f.id));

            for (const file of targetFiles) {
                const filePath = `${dirPath}/${file.name}`;
                try {
                    await invoke('cmd_download_file', { messageId: file.id, savePath: filePath, folderId: activeFolderId });
                    successCount++;
                } catch (e) {
                    console.error(`Failed to download ${file.name}`, e);
                }
            }
            alert(`Downloaded ${successCount} files.`);
            setSelectedIds([]);
        } catch (e) {
            alert(`Bulk download failed: ${e}`);
        }
    }

    const handleBulkMove = async (targetFolderId: number | null) => {
        if (selectedIds.length === 0) return;

        try {
            await invoke('cmd_move_files', {
                messageIds: selectedIds,
                sourceFolderId: activeFolderId,
                targetFolderId: targetFolderId
            });
            alert(`Moved ${selectedIds.length} files.`);
            queryClient.invalidateQueries({ queryKey: ['files', activeFolderId] });
            setShowMoveModal(false);
            setSelectedIds([]);
        } catch (e) {
            console.error(`Failed to move files`, e);
            alert(`Failed to move files: ${e}`);
        }
    };

    const handleDownloadFolder = async () => {
        if (displayedFiles.length === 0) {
            alert("Folder is empty.");
            return;
        }

        try {
            const dirPath = await import('@tauri-apps/plugin-dialog').then(d => d.open({
                directory: true,
                multiple: false,
                title: "Download Folder To..."
            }));

            if (!dirPath) return;

            let successCount = 0;
            for (const file of displayedFiles) {
                const filePath = `${dirPath}/${file.name}`;
                try {
                    await invoke('cmd_download_file', { messageId: file.id, savePath: filePath, folderId: activeFolderId });
                    successCount++;
                } catch (e) {
                    console.error(`Failed to download ${file.name}`, e);
                }
            }
            alert(`Folder Download Complete: ${successCount} files.`);
        } catch (e) {
            alert("Error downloading folder: " + e);
        }
    }

    const handleFileClick = (e: React.MouseEvent, id: number) => {
        e.stopPropagation();
        if (e.metaKey || e.ctrlKey) {
            if (selectedIds.includes(id)) {
                setSelectedIds(selectedIds.filter(i => i !== id));
            } else {
                setSelectedIds([...selectedIds, id]);
            }
        } else {
            // Toggle selection logic or preview? 
            // If selecting multiple, click clears?
            // User requested "i select 3 files". Click usually selects. Double click previews.
            // Let's make single click select ONLY if selection mode is active or just select one.
            // Current standard: Click selects/deselects if checkboxes exist.
            // If we just set [id], we lose multi. 
            // Let's simple toggle if ctrl pressed, else select ONE.
            setSelectedIds([id]);
        }
    }

    // Drag and Drop (Move) Logic
    const handleDropOnFolder = async (e: React.DragEvent, targetFolderId: number | null) => {
        e.preventDefault();
        e.stopPropagation();

        if (activeFolderId === targetFolderId) return;

        const fileIdStr = e.dataTransfer.getData("application/x-telegram-file-id");
        if (fileIdStr) {
            const fileId = parseInt(fileIdStr);
            try {
                // Determine if we are dragging a selection or a single file
                // If dragged file is in selectedIds, move all selected.
                // Else move just this one.
                const idsToMove = selectedIds.includes(fileId) ? selectedIds : [fileId];

                await invoke('cmd_move_files', {
                    messageIds: idsToMove,
                    sourceFolderId: activeFolderId,
                    targetFolderId: targetFolderId
                });
                queryClient.invalidateQueries({ queryKey: ['files', activeFolderId] });
                // If we moved selection, clear it
                if (selectedIds.includes(fileId)) setSelectedIds([]);
            } catch (e) {
                alert(`Failed to move file(s): ${e}`);
            }
        }
    }

    function formatBytes(bytes: number, decimals = 2) {
        if (!+bytes) return '0 Bytes';
        const k = 1024;
        const dm = decimals < 0 ? 0 : decimals;
        const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB'];
        const i = Math.floor(Math.log(bytes) / Math.log(k));
        return `${parseFloat((bytes / Math.pow(k, i)).toFixed(dm))} ${sizes[i]}`;
    }

    const currentFolderName = activeFolderId === null
        ? "Saved Messages"
        : folders.find(f => f.id === activeFolderId)?.name || "Folder";

    return (
        <div
            className="flex h-screen w-full overflow-hidden bg-telegram-bg relative"
            onClick={() => setSelectedIds([])} // Click background to clear
            onDragOver={(e) => {
                e.preventDefault();
            }}
            onDrop={(e) => {
                e.preventDefault();
                // Check if internal move or external file drop
                if (!e.dataTransfer.types.includes("application/x-telegram-file-id")) {
                    // External file drop logic (system file)
                    // ... existing file drop logic via tauri events ...
                    // Actually tauri://file-drop is a window event, not div event. 
                    // So we keep the listener in useEffect.
                }
            }}
        >
            {/* Context Menu Hooks handled inside FileCard */}

            {/* Move Modal */}
            <AnimatePresence>
                {showMoveModal && (
                    <MoveToFolderModal
                        folders={folders}
                        onClose={() => setShowMoveModal(false)}
                        onSelect={handleBulkMove}
                        activeFolderId={activeFolderId}
                    />
                )}
            </AnimatePresence>

            {/* Drag Overlay */}


            {/* Sidebar */}
            <aside className="w-64 bg-telegram-surface border-r border-white/5 flex flex-col" onClick={e => e.stopPropagation()}>
                <div className="p-4 flex items-center gap-2">
                    <img src="/logo.svg" className="w-8 h-8 drop-shadow-lg" alt="Logo" />
                    <span className="font-bold text-lg text-white tracking-tight">Telegram Drive</span>
                </div>

                <nav className="flex-1 px-2 py-4 space-y-1">
                    <SidebarItem
                        icon={HardDrive}
                        label="Saved Messages"
                        active={activeFolderId === null}
                        onClick={() => setActiveFolderId(null)}
                        onDrop={(e: any) => handleDropOnFolder(e, null)}
                    />
                    {folders.map(folder => (
                        <SidebarItem
                            key={folder.id}
                            icon={Folder}
                            label={folder.name}
                            active={activeFolderId === folder.id}
                            onClick={() => setActiveFolderId(folder.id)}
                            onDrop={(e: any) => handleDropOnFolder(e, folder.id)}
                            onDelete={() => handleFolderDelete(folder.id, folder.name)}
                        />
                    ))}

                    {/* Add Folder Button */}
                    {showNewFolderInput ? (
                        <div className="px-3 py-2">
                            <input
                                autoFocus
                                type="text"
                                className="w-full bg-white/10 rounded px-2 py-1 text-sm text-white focus:outline-none focus:ring-1 focus:ring-telegram-primary"
                                placeholder="Folder Name"
                                value={newFolderName}
                                onChange={e => setNewFolderName(e.target.value)}
                                onKeyDown={e => e.key === 'Enter' && handleCreateFolder()}
                                onBlur={() => !newFolderName && setShowNewFolderInput(false)}
                            />
                        </div>
                    ) : (
                        <button
                            onClick={() => setShowNewFolderInput(true)}
                            className="w-full flex items-center gap-3 px-3 py-2 rounded-lg text-sm font-medium text-telegram-subtext hover:bg-white/5 hover:text-white transition-colors border border-dashed border-white/10 mt-2"
                        >
                            <Plus className="w-4 h-4" />
                            Create Folder
                        </button>
                    )}
                </nav>

                <div className="p-4 border-t border-white/5">
                    <div className="flex items-center gap-2 text-telegram-subtext text-xs">
                        <div className="w-2 h-2 rounded-full bg-green-500 animate-pulse"></div>
                        <span>Connected to Telegram</span>
                    </div>

                    <div className="flex gap-2 mt-4">
                        <button
                            onClick={handleSyncFolders}
                            disabled={isSyncing}
                            className={`flex-1 flex items-center justify-center gap-2 px-3 py-2 text-xs font-medium text-blue-300 bg-blue-500/10 hover:bg-blue-500/20 rounded-lg transition-colors ${isSyncing ? 'opacity-50 cursor-not-allowed' : ''}`}
                            title="Scan for existing folders"
                        >
                            <RefreshCw className={`w-3 h-3 ${isSyncing ? 'animate-spin' : ''}`} />
                            {isSyncing ? 'Syncing...' : 'Sync'}
                        </button>
                        <button
                            onClick={handleLogout}
                            className="flex-1 flex items-center justify-center gap-2 px-3 py-2 text-xs font-medium text-red-300 bg-red-500/10 hover:bg-red-500/20 rounded-lg transition-colors"
                            title="Sign Out"
                        >
                            <LogOut className="w-3 h-3" />
                            Logout
                        </button>
                    </div>

                    {bandwidth && (
                        <div className="mt-3 text-xs text-telegram-subtext space-y-1">
                            <div className="flex justify-between">
                                <span>Used Today:</span>
                            </div>
                            <div className="w-full bg-white/10 rounded-full h-1.5 overflow-hidden">
                                <div className="bg-telegram-primary h-full rounded-full" style={{ width: `${Math.min(((bandwidth.up_bytes + bandwidth.down_bytes) / (250 * 1024 * 1024 * 1024)) * 100, 100)}%` }}></div>
                            </div>
                            <div className="flex justify-between text-[10px] opacity-70">
                                <span>{formatBytes(bandwidth.up_bytes + bandwidth.down_bytes)}</span>
                                <span>250 GB</span>
                            </div>
                        </div>
                    )}
                </div>

            </aside>

            {/* Main Content */}
            <main className="flex-1 flex flex-col" onClick={(e) => {
                // Background click clears selection
                if (e.target === e.currentTarget) setSelectedIds([]);
            }}>
                {/* Top Bar */}
                <header className="h-14 border-b border-white/5 flex items-center px-4 justify-between bg-telegram-bg/50 backdrop-blur-md" onClick={e => e.stopPropagation()}>
                    <div className="flex items-center gap-4">
                        <div className="flex items-center text-sm breadcrumbs text-telegram-subtext select-none">
                            <span className="hover:text-white cursor-pointer transition-colors">Start</span>
                            <span className="mx-2">/</span>
                            <span className="text-white font-medium">{currentFolderName}</span>
                        </div>
                    </div>

                    <div className="flex items-center gap-2">
                        {selectedIds.length > 0 && (
                            <div className="flex items-center gap-2 mr-4 animate-in fade-in slide-in-from-top-2">
                                <span className="text-xs text-telegram-subtext mr-2">{selectedIds.length} Selected</span>
                                <button onClick={() => setShowMoveModal(true)} className="px-3 py-1.5 bg-telegram-primary/20 hover:bg-telegram-primary/30 text-telegram-primary rounded-md text-xs transition font-medium">Move to...</button>
                                <button onClick={handleBulkDownload} className="px-3 py-1.5 bg-white/5 hover:bg-white/10 rounded-md text-xs text-white transition">Download Selected</button>
                                <button onClick={handleBulkDelete} className="px-3 py-1.5 bg-red-500/10 hover:bg-red-500/20 text-red-400 rounded-md text-xs transition">Delete</button>
                            </div>
                        )}

                        <button onClick={handleDownloadFolder} className="p-2 hover:bg-white/5 rounded-md text-telegram-subtext hover:text-white transition group relative" title="Download Folder">
                            <HardDrive className="w-5 h-5" />
                        </button>

                        <button
                            onClick={() => setViewMode(viewMode === 'grid' ? 'list' : 'grid')}
                            className="p-2 hover:bg-white/5 rounded-md text-telegram-subtext hover:text-white transition relative group"
                            title="Toggle Layout"
                        >
                            <LayoutGrid className="w-5 h-5" />
                            <span className="absolute -bottom-8 left-1/2 -translate-x-1/2 text-[10px] bg-black px-2 py-1 rounded opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none whitespace-nowrap z-50">
                                {viewMode === 'grid' ? 'Switch to List' : 'Switch to Grid'}
                            </span>
                        </button>
                    </div>
                </header >

                {/* File View */}
                < div className="flex-1 p-6 overflow-auto custom-scrollbar" onClick={(e) => {
                    if (e.target === e.currentTarget) setSelectedIds([]);
                }}>
                    {
                        isLoading ? (
                            <div className="flex justify-center items-center h-full text-telegram-subtext flex-col gap-4" >
                                <div className="w-8 h-8 border-4 border-telegram-primary border-t-transparent rounded-full animate-spin"></div>
                                Loading your files...
                            </div>
                        ) : error ? (
                            <div className="flex justify-center items-center h-full text-red-400">Error loading files</div>
                        ) : (
                            <>
                                {viewMode === 'grid' ? (
                                    <div className="grid grid-cols-2 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-4 animate-in fade-in duration-300">
                                        {displayedFiles.map((file: any) => (
                                            <FileCard
                                                key={file.id}
                                                file={file}
                                                isSelected={selectedIds.includes(file.id)}
                                                onClick={(e: React.MouseEvent) => handleFileClick(e, file.id)}
                                                onDelete={() => handleDelete(file.id)}
                                                onDownload={() => handleDownload(file.id, file.name)}
                                                onPreview={() => setPreviewFile(file)}
                                            />
                                        ))}

                                        {/* Upload Button */}
                                        <button
                                            onClick={(e) => { e.stopPropagation(); handleManualUpload(); }}
                                            className="aspect-[4/3] border-2 border-dashed border-white/10 rounded-xl flex flex-col items-center justify-center text-telegram-subtext hover:border-telegram-primary hover:text-telegram-primary transition-all group"
                                        >
                                            <Plus className="w-8 h-8 mb-2 group-hover:scale-110 transition-transform" />
                                            <span className="text-sm font-medium">Upload File</span>
                                        </button>
                                    </div>
                                ) : (
                                    <div className="flex flex-col gap-1 w-full animate-in fade-in duration-300">
                                        {/* List Header */}
                                        <div className="grid grid-cols-[2rem_2fr_6rem_8rem] gap-4 px-4 py-2 text-xs font-semibold text-telegram-subtext border-b border-white/5 mb-2 select-none items-center">
                                            <div className="text-center">#</div>
                                            <div>Name</div>
                                            <div className="text-right">Size</div>
                                            <div className="text-right">Date</div>
                                        </div>

                                        {displayedFiles.map((file: any) => (
                                            <div
                                                key={file.id}
                                                onClick={(e) => handleFileClick(e, file.id)}
                                                onContextMenu={(e) => {
                                                    e.preventDefault();
                                                    if (!selectedIds.includes(file.id)) handleFileClick(e, file.id);
                                                }}
                                                draggable
                                                onDragStart={(e) => {
                                                    e.dataTransfer.setData("application/x-telegram-file-id", file.id.toString());
                                                }}
                                                className={`group grid grid-cols-[2rem_2fr_6rem_8rem] gap-4 items-center px-4 py-3 rounded-lg cursor-pointer border border-transparent transition-all hover:bg-white/5 ${selectedIds.includes(file.id) ? 'bg-telegram-primary/10 border-telegram-primary/20' : ''}`}
                                            >
                                                <div className="text-telegram-primary flex justify-center">
                                                    {file.type === 'folder' ? <Folder className="w-5 h-5" /> : <File className="w-5 h-5" />}
                                                </div>
                                                <div className="truncate text-sm text-white font-medium relative pr-8">
                                                    {file.name}
                                                    {/* List Actions (visible on hover or select) - Absolute positioned in name cell or end */}
                                                    <div className="absolute right-0 top-1/2 -translate-y-1/2 opacity-0 group-hover:opacity-100 flex items-center bg-[#1c1c1c] shadow-lg rounded px-1">
                                                        <button onClick={(e) => { e.stopPropagation(); setPreviewFile(file) }} className="p-1 hover:text-white text-telegram-subtext" title="Preview"><Eye className="w-4 h-4" /></button>
                                                        <button onClick={(e) => { e.stopPropagation(); handleDownload(file.id, file.name) }} className="p-1 hover:text-white text-telegram-subtext" title="Download"><HardDrive className="w-4 h-4" /></button>
                                                        <button onClick={(e) => { e.stopPropagation(); handleDelete(file.id) }} className="p-1 hover:text-red-400 text-telegram-subtext" title="Delete"><Plus className="w-4 h-4 rotate-45" /></button>
                                                    </div>
                                                </div>
                                                <div className="text-right text-xs text-telegram-subtext truncate">{file.sizeStr}</div>
                                                <div className="text-right text-xs text-telegram-subtext font-mono opacity-50 truncate">{file.created_at || '-'}</div>
                                            </div>
                                        ))}

                                        {/* List View Add Button - simpler */}
                                        {activeFolderId === null && (
                                            <button
                                                onClick={(e) => { e.stopPropagation(); handleManualUpload(); }}
                                                className="flex items-center gap-4 px-4 py-3 rounded-lg cursor-pointer border border-dashed border-white/10 text-telegram-subtext hover:text-white hover:bg-white/5 mt-2"
                                            >
                                                <div className="w-5 h-5 flex items-center justify-center"><Plus className="w-4 h-4" /></div>
                                                <span className="text-sm font-medium">Upload File...</span>
                                            </button>
                                        )}
                                    </div>
                                )}

                                {displayedFiles.length === 0 && (
                                    <div className="flex flex-col items-center justify-center text-telegram-subtext py-20 pointer-events-none w-full">
                                        <Folder className="w-16 h-16 mb-4 opacity-20" />
                                        <p>This folder is empty</p>
                                        <p className="text-xs opacity-50 mt-1">Drag files from Saved Messages to here</p>
                                    </div>
                                )}
                            </>
                        )}
                </div >
            </main >

            {/* Preview Modal */}
            {
                previewFile && (
                    <PreviewModal
                        file={previewFile}
                        activeFolderId={activeFolderId}
                        onClose={() => setPreviewFile(null)}
                    />
                )
            }

            {/* Upload Queue Widget */}
            {
                uploadQueue.length > 0 && (
                    <div className="fixed bottom-4 right-4 w-80 bg-telegram-surface border border-white/10 rounded-xl shadow-2xl overflow-hidden z-[100]">
                        <div className="p-3 border-b border-white/5 bg-white/5 flex justify-between items-center">
                            <h4 className="text-sm font-medium text-white">Uploads</h4>
                            <button onClick={() => setUploadQueue(q => q.filter(i => i.status !== 'success'))} className="text-xs text-telegram-primary hover:text-white transition-colors">Clear Finished</button>
                        </div>
                        <div className="max-h-60 overflow-y-auto p-2 space-y-2">
                            {uploadQueue.map(item => (
                                <div key={item.id} className="flex items-center gap-3 text-sm p-2 bg-black/20 rounded">
                                    <div className="truncate flex-1 text-white/80" title={item.path}>{item.path.split(/[/\\]/).pop()}</div>
                                    <div className="text-xs shrink-0">
                                        {item.status === 'pending' && <span className="text-yellow-500">Queue</span>}
                                        {item.status === 'uploading' && <span className="text-blue-400 animate-pulse">Uploading...</span>}
                                        {item.status === 'success' && <span className="text-green-400">Done</span>}
                                        {item.status === 'error' && <span className="text-red-400" title={item.error}>Error</span>}
                                    </div>
                                </div>
                            ))}
                        </div>
                    </div>
                )
            }
        </div >
    );
}

function SidebarItem({ icon: Icon, label, active = false, onClick, onDrop, onDelete }: any) {
    const [isOver, setIsOver] = useState(false);

    return (
        <button
            onClick={onClick}
            onDragOver={(e) => {
                e.preventDefault();
                setIsOver(true);
            }}
            onDragLeave={() => setIsOver(false)}
            onDrop={(e) => {
                setIsOver(false);
                if (onDrop) onDrop(e);
            }}
            onContextMenu={(e) => {
                if (onDelete) {
                    e.preventDefault();
                    onDelete();
                }
            }}
            className={`group w-full flex items-center gap-3 px-3 py-2 rounded-lg text-sm font-medium transition-colors ${active ? 'bg-telegram-primary/10 text-telegram-primary' : isOver ? 'bg-telegram-primary/20 text-white' : 'text-telegram-subtext hover:bg-white/5 hover:text-white'}`}
        >
            <Icon className="w-4 h-4" />
            <span className="flex-1 text-left truncate">{label}</span>
            {onDelete && (
                <div onClick={(e) => { e.stopPropagation(); onDelete(); }} className="opacity-0 group-hover:opacity-100 p-1 hover:text-red-400">
                    <Plus className="w-3 h-3 rotate-45" />
                </div>
            )}
        </button>
    )
}

function FileCard({ file, onDelete, onDownload, onPreview, isSelected, onClick }: any) {
    const isFolder = file.type === 'folder';
    const [showMenu, setShowMenu] = useState(false);

    // Handle Right Click
    const handleContextMenu = (e: React.MouseEvent) => {
        if (isFolder) return;
        e.preventDefault();
        e.stopPropagation();

        // If not selected, select it? Or just show menu?
        // Usually right click selects item if not selected.
        if (!isSelected && onClick) onClick(e);

        setShowMenu(true);
    };

    // Close menu when clicking elsewhere
    useEffect(() => {
        const close = () => setShowMenu(false);
        if (showMenu) window.addEventListener('click', close);
        return () => window.removeEventListener('click', close);
    }, [showMenu]);

    return (
        <div
            className="relative"
            onContextMenu={handleContextMenu}
            onClick={onClick}
        >
            <motion.div
                layout
                draggable={!isFolder}
                onDragStart={(e: any) => {
                    e.dataTransfer.setData("application/x-telegram-file-id", file.id.toString());
                }}
                whileHover={{ y: -4 }}
                className={`group cursor-pointer aspect-[4/3] bg-telegram-surface rounded-xl p-4 flex flex-col justify-between border hover:shadow-[0_4px_20px_rgba(0,0,0,0.4)] transition-all relative overflow-hidden ${isSelected ? 'border-telegram-primary bg-telegram-primary/5 ring-1 ring-telegram-primary' : 'border-white/5 hover:border-telegram-primary/50'}`}
            >
                {/* Selection Checkmark */}
                <div className={`absolute top-2 left-2 w-5 h-5 rounded-full border flex items-center justify-center transition-all ${isSelected ? 'bg-telegram-primary border-telegram-primary' : 'border-white/20 bg-black/20 opacity-0 group-hover:opacity-100'}`}>
                    {isSelected && <div className="w-1.5 h-1.5 bg-black rounded-full" />}
                </div>

                <div className={`absolute top-0 right-0 p-2 opacity-0 group-hover:opacity-100 transition-opacity duration-300`}>
                    <div className="w-1.5 h-1.5 rounded-full bg-telegram-primary shadow-[0_0_8px_rgba(255,174,0,0.8)]"></div>
                </div>

                <div className={`${isFolder ? 'text-telegram-primary' : 'text-telegram-secondary'}`}>
                    {isFolder ? <Folder className="w-10 h-10" /> : <File className="w-10 h-10" />}
                </div>
                <div>
                    <h3 className="text-sm font-medium text-white truncate w-full" title={file.name}>{file.name}</h3>
                    <p className="text-xs text-telegram-subtext mt-1">{file.sizeStr} {file.size}</p>
                </div>
                {/* Quick actions on hover */}
                <div className="absolute top-2 right-2 opacity-0 group-hover:opacity-100 transition-opacity flex gap-1">
                    <button onClick={(e) => { e.stopPropagation(); if (onPreview) onPreview() }} className="p-1 bg-black/50 rounded-full hover:bg-telegram-primary hover:text-white text-white/70">
                        <Eye className="w-3 h-3" />
                    </button>
                </div>
            </motion.div>

            {/* Custom Context Menu - Outside overflow-hidden */}
            {showMenu && (
                <div className="absolute top-2 right-2 bg-[#2d2d2d] border border-white/10 rounded-lg shadow-xl z-50 flex flex-col w-32 overflow-hidden animate-in fade-in zoom-in-95 duration-100">
                    <button onClick={(e) => { e.stopPropagation(); if (onPreview) onPreview(); }} className="px-3 py-2 text-left text-xs text-white hover:bg-white/10 flex items-center gap-2">
                        <Eye className="w-3 h-3" /> Preview
                    </button>
                    <button onClick={(e) => { e.stopPropagation(); onDownload(); }} className="px-3 py-2 text-left text-xs text-white hover:bg-white/10 flex items-center gap-2">
                        Download
                    </button>
                    <div className="h-px bg-white/10 my-0"></div>
                    <button onClick={(e) => { e.stopPropagation(); onDelete(); }} className="px-3 py-2 text-left text-xs text-red-400 hover:bg-white/10 flex items-center gap-2">
                        Delete
                    </button>
                </div>
            )}
        </div>
    )
}

function MoveToFolderModal({ folders, onClose, onSelect, activeFolderId }: any) {
    return (
        <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/50 backdrop-blur-sm" onClick={onClose}>
            <div className="bg-[#1c1c1c] border border-white/10 rounded-xl w-80 shadow-2xl overflow-hidden flex flex-col max-h-[80vh]" onClick={e => e.stopPropagation()}>
                <div className="p-4 border-b border-white/10 flex justify-between items-center">
                    <h3 className="text-white font-medium">Move to Folder</h3>
                    <button onClick={onClose} className="text-telegram-subtext hover:text-white"><Plus className="w-5 h-5 rotate-45" /></button>
                </div>
                <div className="flex-1 overflow-y-auto p-2 space-y-1">
                    {activeFolderId !== null && (
                        <button
                            onClick={() => onSelect(null)}
                            className="w-full flex items-center gap-3 px-3 py-3 rounded-lg text-sm text-left text-white hover:bg-white/5 transition-colors"
                        >
                            <div className="w-8 h-8 rounded bg-telegram-primary/20 flex items-center justify-center text-telegram-primary">
                                <HardDrive className="w-4 h-4" />
                            </div>
                            <span className="font-medium">Saved Messages</span>
                        </button>
                    )}

                    {folders.map((f: any) => {
                        if (f.id === activeFolderId) return null;
                        return (
                            <button
                                key={f.id}
                                onClick={() => onSelect(f.id)}
                                className="w-full flex items-center gap-3 px-3 py-3 rounded-lg text-sm text-left text-white hover:bg-white/5 transition-colors"
                            >
                                <div className="w-8 h-8 rounded bg-white/10 flex items-center justify-center text-white">
                                    <Folder className="w-4 h-4" />
                                </div>
                                <span className="font-medium">{f.name}</span>
                            </button>
                        )
                    })}

                    {folders.length === 0 && activeFolderId === null && (
                        <div className="p-4 text-center text-xs text-telegram-subtext">No other folders available. Create one first!</div>
                    )}
                </div>
            </div>
        </div>
    )
}

function PreviewModal({ file, onClose, activeFolderId }: any) {
    const [src, setSrc] = useState<string | null>(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);

    useEffect(() => {
        const load = async () => {
            setLoading(true);
            setError(null);
            try {
                const path = await invoke<string>('cmd_get_preview', {
                    messageId: file.id,
                    folderId: activeFolderId
                });
                if (path) {
                    if (path.startsWith('data:')) {
                        setSrc(path);
                    } else {
                        setSrc(convertFileSrc(path));
                    }
                } else {
                    setError("Preview not available");
                }
            } catch (e) {
                setError(String(e));
            } finally {
                setLoading(false);
            }
        };
        load();
    }, [file, activeFolderId]);



    return (
        <div className="fixed inset-0 z-[150] bg-black/90 flex items-center justify-center p-4 backdrop-blur-sm" onClick={onClose}>
            <div className="relative max-w-5xl w-full max-h-screen flex flex-col items-center justify-center" onClick={e => e.stopPropagation()}>
                <button onClick={onClose} className="absolute -top-12 right-0 text-white hover:text-gray-300 p-2">
                    <X className="w-8 h-8" />
                </button>

                {loading && (
                    <div className="flex flex-col items-center gap-4 text-white">
                        <div className="w-10 h-10 border-4 border-telegram-primary border-t-transparent rounded-full animate-spin"></div>
                        <p>Loading preview...</p>
                        <p className="text-xs text-white/50">Downloading from Telegram...</p>
                    </div>
                )}

                {error && (
                    <div className="text-red-400 bg-white/10 p-4 rounded-lg border border-red-500/20">
                        <p className="font-bold">Preview Error</p>
                        <p className="text-sm">{error}</p>
                    </div>
                )}

                {!loading && !error && src && (
                    <div className="flex flex-col items-center">
                        {['jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp', 'svg', 'heic', 'heif'].some(ext => file.name.toLowerCase().endsWith(ext)) ? (
                            <img src={src.startsWith('data:') ? src : `${src}?t=${Date.now()}`} className="max-w-full max-h-[85vh] object-contain rounded-lg shadow-2xl bg-black" alt="Preview" />
                        ) : ['mp4', 'webm', 'ogg', 'mov'].some(ext => file.name.toLowerCase().endsWith(ext)) ? (
                            <video src={src} controls className="max-w-full max-h-[85vh] rounded-lg shadow-2xl bg-black" />
                        ) : (
                            <div className="bg-[#1c1c1c] p-8 rounded-xl text-center border border-white/10 shadow-2xl">
                                <File className="w-16 h-16 text-telegram-primary mx-auto mb-4" />
                                <h3 className="text-xl text-white font-medium mb-2">{file.name}</h3>
                                <p className="text-gray-400 mb-6">Preview not supported in app.</p>
                                <p className="text-xs text-gray-500">File type: {file.name.split('.').pop()}</p>
                            </div>
                        )}
                    </div>
                )}


                <div className="absolute bottom-[-3rem] text-white text-sm opacity-50">
                    {file.name}
                </div>
            </div>
        </div>
    );
}
