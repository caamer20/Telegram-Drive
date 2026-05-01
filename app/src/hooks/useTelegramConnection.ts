import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Store } from '@tauri-apps/plugin-store';
import { useQueryClient } from '@tanstack/react-query';
import { toast } from 'sonner';
import { useConfirm } from '../context/ConfirmContext';
import { DriveMode, TelegramFolder } from '../types';
import { useNetworkStatus } from './useNetworkStatus';

export function useTelegramConnection(driveMode: DriveMode, onLogoutParent: () => void) {
    const queryClient = useQueryClient();
    const { confirm } = useConfirm();

    const [folders, setFolders] = useState<TelegramFolder[]>([]);
    const [activeFolderId, setActiveFolderId] = useState<number | null>(null);
    const [store, setStore] = useState<Store | null>(null);
    const [isSyncing, setIsSyncing] = useState(false);
    const [isConnected, setIsConnected] = useState(true);


    const networkIsOnline = useNetworkStatus();


    useEffect(() => {
        const initStore = async () => {
            try {
                let _store = await Store.load('config.json');
                const checkId = await _store.get<string>('api_id');
                if (!checkId) {
                    _store = await Store.load('settings.json');
                }
                setStore(_store);

                if (driveMode === 'plain') {
                    const savedFolders = await _store.get<TelegramFolder[]>('folders');
                    if (savedFolders) setFolders(savedFolders);
                }


                const activeFolderKey = driveMode === 'vault' ? 'vaultActiveFolderId' : 'activeFolderId';
                const savedActiveFolderId = await _store.get<number | null>(activeFolderKey);
                if (savedActiveFolderId !== undefined) setActiveFolderId(savedActiveFolderId);

                const apiIdStr = await _store.get<string>('api_id');
                if (apiIdStr) {
                    try {
                        const apiId = parseInt(apiIdStr as string);
                        await invoke('cmd_connect', { apiId });
                        if (driveMode === 'vault') {
                            const vaultFolders = await invoke<TelegramFolder[]>('cmd_vault_scan_folders').catch(() => []);
                            setFolders(vaultFolders);
                        }
                        setIsConnected(true);
                        queryClient.invalidateQueries({ queryKey: ['files'] });
                    } catch {
                        const shouldRetry = window.confirm("Failed to connect to Telegram. Retry?");
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

            } catch {
                // store not available
            }
        };
        initStore();
    }, [queryClient, onLogoutParent, driveMode]);


    useEffect(() => {
        setIsConnected(networkIsOnline);
    }, [networkIsOnline]);


    const isNetworkError = (error: string): boolean => {
        const keywords = ['timeout', 'connection', 'network', 'socket', 'disconnected', 'EOF', 'ECONNREFUSED', 'overflow'];
        return keywords.some(k => error.toLowerCase().includes(k.toLowerCase()));
    };

    const forceLogout = async () => {
        setIsConnected(false);
        try {
            await invoke('cmd_clean_cache').catch(() => { });
            await invoke('cmd_vault_lock').catch(() => { });
            if (store) {
                await store.delete('api_id');
                await store.delete('api_hash');
                if (driveMode === 'plain') {
                    await store.delete('folders');
                }
                await store.save();
            }
        } catch {
            // best effort cleanup
        }
        toast.error("Connection lost. Please log in again.");
        onLogoutParent();
    };


    const handleLogout = async () => {
        if (!await confirm({ title: "Sign Out", message: "Are you sure you want to sign out? This will disconnect your active session.", confirmText: "Sign Out", variant: 'danger' })) return;

        try {
            await invoke('cmd_logout');
            await invoke('cmd_clean_cache');
            await invoke('cmd_vault_lock').catch(() => { });
            if (store) {
                await store.delete('api_id');
                await store.delete('api_hash');
                if (driveMode === 'plain') {
                    await store.delete('folders');
                }
                await store.save();
            }
            onLogoutParent();
        } catch {
            toast.error("Error signing out");
            onLogoutParent();
        }
    };

    const handleSyncFolders = async () => {
        if (!store) return;
        setIsSyncing(true);
        try {
            const foundFolders = await invoke<TelegramFolder[]>(
                driveMode === 'vault' ? 'cmd_vault_scan_folders' : 'cmd_scan_folders'
            );
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
                if (driveMode === 'plain') {
                    await store.set('folders', merged);
                    await store.save();
                }
                toast.success(`Scan complete. Found ${added} new folders.`);
            } else {
                toast.info("Scan complete. No new folders found.");
            }
        } catch {
            toast.error("Sync failed");
        } finally {
            setIsSyncing(false);
        }
    };

    const handleCreateFolder = async (name: string) => {
        if (!store) return;
        try {
            const newFolder = await invoke<TelegramFolder>(
                driveMode === 'vault' ? 'cmd_vault_create_folder' : 'cmd_create_folder',
                { name }
            );
            const updated = [...folders, newFolder];
            setFolders(updated);
            if (driveMode === 'plain') {
                await store.set('folders', updated);
                await store.save();
            }
            toast.success(`Folder "${name}" created.`);
        } catch (e) {
            toast.error("Failed to create folder: " + e);
            throw e;
        }
    };

    const handleFolderDelete = async (folderId: number, folderName: string) => {
        if (!await confirm({
            title: "Delete Folder",
            message: driveMode === 'vault'
                ? `Are you sure you want to delete "${folderName}"?\nThe folder must be empty.`
                : `Are you sure you want to delete "${folderName}"?\nThis will delete the channel on Telegram.`,
            confirmText: "Delete",
            variant: 'danger'
        })) return;

        try {
            await invoke(driveMode === 'vault' ? 'cmd_vault_delete_folder' : 'cmd_delete_folder', { folderId });
            const updated = folders.filter(f => f.id !== folderId);
            setFolders(updated);
            if (store && driveMode === 'plain') {
                await store.set('folders', updated);
                await store.save();
            }
            if (activeFolderId === folderId) setActiveFolderId(null);
            toast.success(`Folder "${folderName}" deleted.`);
        } catch (e: unknown) {
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
                    if (store && driveMode === 'plain') {
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


    const handleSetActiveFolderId = async (id: number | null) => {
        setActiveFolderId(id);
        if (store) {
            await store.set(driveMode === 'vault' ? 'vaultActiveFolderId' : 'activeFolderId', id);
            await store.save();
        }
    };

    return {
        store,
        folders,
        activeFolderId,
        setActiveFolderId: handleSetActiveFolderId,
        isSyncing,
        isConnected,
        handleLogout,
        handleSyncFolders,
        handleCreateFolder,
        handleFolderDelete,
        isNetworkError,
        forceLogout
    };
}
