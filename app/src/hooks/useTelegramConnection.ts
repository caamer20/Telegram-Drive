import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Store } from '@tauri-apps/plugin-store';
import { useQueryClient } from '@tanstack/react-query';
import { toast } from 'sonner';
import { useConfirm } from '../context/ConfirmContext';
import { TelegramFolder } from '../types';

export function useTelegramConnection(onLogoutParent: () => void) {
    const queryClient = useQueryClient();
    const { confirm } = useConfirm();

    const [folders, setFolders] = useState<TelegramFolder[]>([]);
    const [activeFolderId, setActiveFolderId] = useState<number | null>(null);
    const [store, setStore] = useState<Store | null>(null);
    const [isSyncing, setIsSyncing] = useState(false);

    // Load store and folders on mount
    useEffect(() => {
        const initStore = async () => {
            try {
                let _store = await Store.load('config.json');
                const checkId = await _store.get<string>('api_id');
                if (!checkId) {
                    _store = await Store.load('settings.json');
                }
                setStore(_store);

                const savedFolders = await _store.get<TelegramFolder[]>('folders');
                if (savedFolders) setFolders(savedFolders);

                const apiIdStr = await _store.get<string>('api_id');
                if (apiIdStr) {
                    try {
                        const apiId = parseInt(apiIdStr as string);
                        console.log("Connecting to Telegram with ID:", apiId);
                        await invoke('cmd_connect', { apiId });
                        queryClient.invalidateQueries({ queryKey: ['files'] });
                    } catch (e) {
                        console.error("Failed to connect:", e);
                        // Using native confirm here as hooks might not be ready or reliable during initial mount race
                        const shouldRetry = window.confirm("Failed to connect to Telegram. Retry?\n\nError: " + e);
                        if (shouldRetry) {
                            window.location.reload();
                        } else {
                            if (_store) {
                                await _store.delete('api_id');
                                await _store.save();
                            }
                            onLogoutParent();
                        }
                    }
                } else {
                    onLogoutParent();
                }

            } catch (e) {
                console.error("Failed to load store", e);
            }
        };
        initStore();
    }, [queryClient, onLogoutParent]);

    const handleLogout = async () => {
        if (!await confirm({ title: "Sign Out", message: "Are you sure you want to sign out? This will disconnect your active session.", confirmText: "Sign Out", variant: 'danger' })) return;

        try {
            await invoke('cmd_logout');
            await invoke('cmd_clean_cache');
            if (store) {
                await store.delete('api_id');
                await store.delete('api_hash');
                await store.delete('folders');
                await store.save();
            }
            onLogoutParent();
        } catch (e) {
            console.error("Logout failed:", e);
            toast.error("Error signing out: " + e);
            onLogoutParent();
        }
    };

    const handleSyncFolders = async () => {
        if (!store) return;
        setIsSyncing(true);
        try {
            const foundFolders = await invoke<TelegramFolder[]>('cmd_scan_folders');
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
                toast.success(`Scan complete. Found ${added} new folders.`);
            } else {
                toast.info("Scan complete. No new folders found.");
            }
        } catch (e) {
            console.error("Sync failed:", e);
            toast.error("Sync failed: " + e);
        } finally {
            setIsSyncing(false);
        }
    };

    const handleCreateFolder = async (name: string) => {
        if (!store) return;
        try {
            const newFolder = await invoke<TelegramFolder>('cmd_create_folder', { name });
            const updated = [...folders, newFolder];
            setFolders(updated);
            await store.set('folders', updated);
            await store.save();
            toast.success(`Folder "${name}" created.`);
        } catch (e) {
            toast.error("Failed to create folder: " + e);
            throw e;
        }
    };

    const handleFolderDelete = async (folderId: number, folderName: string) => {
        if (!await confirm({
            title: "Delete Folder",
            message: `Are you sure you want to delete "${folderName}"?\nThis will delete the channel on Telegram.`,
            confirmText: "Delete",
            variant: 'danger'
        })) return;

        try {
            await invoke('cmd_delete_folder', { folderId });
            const updated = folders.filter(f => f.id !== folderId);
            setFolders(updated);
            if (store) {
                await store.set('folders', updated);
                await store.save();
            }
            if (activeFolderId === folderId) setActiveFolderId(null);
            toast.success(`Folder "${folderName}" deleted.`);
        } catch (e: any) {
            const errStr = String(e);
            if (errStr.includes("not found")) {
                if (await confirm({
                    title: "Folder Not Found",
                    message: `Folder "${folderName}" not found on Telegram (it may have been deleted externally).\nRemove from this app?`,
                    confirmText: "Remove",
                    variant: 'info'
                })) {
                    const updated = folders.filter(f => f.id !== folderId);
                    setFolders(updated);
                    if (store) {
                        await store.set('folders', updated);
                        await store.save();
                    }
                    if (activeFolderId === folderId) setActiveFolderId(null);
                }
            } else {
                toast.error(`Failed to delete folder: ${e}`);
            }
        }
    };

    return {
        store,
        folders,
        activeFolderId,
        setActiveFolderId,
        isSyncing,
        handleLogout,
        handleSyncFolders,
        handleCreateFolder,
        handleFolderDelete
    };
}
