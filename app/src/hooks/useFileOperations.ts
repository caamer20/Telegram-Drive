import { invoke } from '@tauri-apps/api/core';
import { useQueryClient } from '@tanstack/react-query';
import { toast } from 'sonner';
import { useConfirm } from '../context/ConfirmContext';
import { TelegramFile } from '../types';

export function useFileOperations(
    activeFolderId: number | null,
    selectedIds: number[],
    setSelectedIds: (ids: number[]) => void,
    displayedFiles: TelegramFile[]
) {
    const queryClient = useQueryClient();
    const { confirm } = useConfirm();

    const handleDelete = async (id: number) => {
        if (!await confirm({ title: "Delete File", message: "Are you sure you want to delete this file?", confirmText: "Delete", variant: 'danger' })) return;
        try {
            await invoke('cmd_delete_file', { messageId: id, folderId: activeFolderId });
            queryClient.invalidateQueries({ queryKey: ['files', activeFolderId] });
            toast.success("File deleted");
        } catch (e) {
            toast.error(`Delete failed: ${e}`);
        }
    }

    const handleBulkDelete = async () => {
        if (selectedIds.length === 0) return;
        if (!await confirm({ title: "Delete Files", message: `Are you sure you want to delete ${selectedIds.length} files?`, confirmText: "Delete All", variant: 'danger' })) return;

        let success = 0;
        let fail = 0;
        for (const id of selectedIds) {
            try {
                await invoke('cmd_delete_file', { messageId: id, folderId: activeFolderId });
                success++;
            } catch {
                fail++;
            }
        }
        setSelectedIds([]);
        queryClient.invalidateQueries({ queryKey: ['files', activeFolderId] });
        if (success > 0) toast.success(`Deleted ${success} files.`);
        if (fail > 0) toast.error(`Failed to delete ${fail} files.`);
    }

    const handleDownload = async (id: number, name: string) => {
        try {
            const savePath = await import('@tauri-apps/plugin-dialog').then(d => d.save({
                defaultPath: name,
            }));
            if (!savePath) return;
            toast.info(`Download started: ${name}`);
            await invoke('cmd_download_file', { messageId: id, savePath, folderId: activeFolderId });
            toast.success(`Download complete: ${name}`);
        } catch (e) {
            toast.error(`Download failed: ${e}`);
        }
    }

    const handleBulkDownload = async () => {
        if (selectedIds.length === 0) return;
        try {
            const dirPath = await import('@tauri-apps/plugin-dialog').then(d => d.open({
                directory: true, multiple: false, title: "Select Download Destination"
            }));
            if (!dirPath) return;
            let successCount = 0;
            const targetFiles = displayedFiles.filter((f) => selectedIds.includes(f.id));
            toast.info(`Starting batch download of ${targetFiles.length} files...`);

            for (const file of targetFiles) {
                const filePath = `${dirPath}/${file.name}`;
                try {
                    await invoke('cmd_download_file', { messageId: file.id, savePath: filePath, folderId: activeFolderId });
                    successCount++;
                } catch (e) { }
            }
            toast.success(`Downloaded ${successCount} files.`);
            setSelectedIds([]);
        } catch (e) {
            toast.error(`Bulk download failed: ${e}`);
        }
    }

    const handleBulkMove = async (targetFolderId: number | null, onSuccess?: () => void) => {
        if (selectedIds.length === 0) return;
        try {
            await invoke('cmd_move_files', {
                messageIds: selectedIds,
                sourceFolderId: activeFolderId,
                targetFolderId: targetFolderId
            });
            toast.success(`Moved ${selectedIds.length} files.`);
            queryClient.invalidateQueries({ queryKey: ['files', activeFolderId] });
            setSelectedIds([]);
            if (onSuccess) onSuccess();
        } catch {
            toast.error('Failed to move files');
        }
    };

    const handleDownloadFolder = async () => {
        if (displayedFiles.length === 0) {
            toast.info("Folder is empty.");
            return;
        }
        try {
            const dirPath = await import('@tauri-apps/plugin-dialog').then(d => d.open({
                directory: true, multiple: false, title: "Download Folder To..."
            }));
            if (!dirPath) return;
            let successCount = 0;
            toast.info(`Downloading folder contents (${displayedFiles.length} files)...`);
            for (const file of displayedFiles) {
                const filePath = `${dirPath}/${file.name}`;
                try {
                    await invoke('cmd_download_file', { messageId: file.id, savePath: filePath, folderId: activeFolderId });
                    successCount++;
                } catch (e) { }
            }
            toast.success(`Folder Download Complete: ${successCount} files.`);
        } catch (e) {
            toast.error("Error: " + e);
        }
    }

    return {
        handleDelete,
        handleBulkDelete,
        handleDownload,
        handleBulkDownload,
        handleBulkMove,
        handleDownloadFolder,
        handleGlobalSearch: async (query: string) => {
            try {
                return await invoke<TelegramFile[]>('cmd_search_global', { query });
            } catch {
                return [];
            }
        }
    };
}
