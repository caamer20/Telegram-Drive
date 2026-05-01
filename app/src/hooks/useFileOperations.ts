import { invoke } from '@tauri-apps/api/core';
import { useQueryClient } from '@tanstack/react-query';
import { toast } from 'sonner';
import { useConfirm } from '../context/ConfirmContext';
import { DriveMode, TelegramFile } from '../types';

export function useFileOperations(
    driveMode: DriveMode,
    activeFolderId: number | null,
    selectedIds: number[],
    setSelectedIds: (ids: number[]) => void,
    displayedFiles: TelegramFile[]
) {
    const queryClient = useQueryClient();
    const { confirm } = useConfirm();

    const selectedItems = () => displayedFiles.filter((file) => selectedIds.includes(file.id));
    const selectedFiles = () => selectedItems().filter((file) => file.type !== 'folder');
    const selectedFolders = () => selectedItems().filter((file) => file.type === 'folder');

    const handleDelete = async (id: number) => {
        if (!await confirm({ title: "Delete File", message: "Are you sure you want to delete this file?", confirmText: "Delete", variant: 'danger' })) return;
        try {
            await invoke(driveMode === 'vault' ? 'cmd_vault_delete_file' : 'cmd_delete_file', { messageId: id, folderId: activeFolderId });
            queryClient.invalidateQueries({ queryKey: ['files', driveMode, activeFolderId] });
            toast.success("File deleted");
        } catch (e) {
            toast.error(`Delete failed: ${e}`);
        }
    }

    const handleBulkDelete = async () => {
        if (selectedIds.length === 0) return;
        const files = selectedFiles();
        const folders = selectedFolders();
        if (files.length === 0) {
            if (folders.length > 0) toast.info("Use the folder delete action to remove folders.");
            return;
        }
        if (!await confirm({ title: "Delete Files", message: `Are you sure you want to delete ${files.length} files?`, confirmText: "Delete All", variant: 'danger' })) return;

        let success = 0;
        let fail = 0;
        for (const file of files) {
            try {
                await invoke(driveMode === 'vault' ? 'cmd_vault_delete_file' : 'cmd_delete_file', { messageId: file.id, folderId: activeFolderId });
                success++;
            } catch {
                fail++;
            }
        }
        setSelectedIds([]);
        queryClient.invalidateQueries({ queryKey: ['files', driveMode, activeFolderId] });
        if (success > 0) toast.success(`Deleted ${success} files.`);
        if (fail > 0) toast.error(`Failed to delete ${fail} files.`);
        if (folders.length > 0) toast.info("Skipped selected folders. Use the folder delete action to remove folders.");
    }

    const handleDownload = async (id: number, name: string) => {
        try {
            const savePath = await import('@tauri-apps/plugin-dialog').then(d => d.save({
                defaultPath: name,
            }));
            if (!savePath) return;
            toast.info(`Download started: ${name}`);
            await invoke(driveMode === 'vault' ? 'cmd_vault_download_file' : 'cmd_download_file', { messageId: id, savePath, folderId: activeFolderId });
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
            const targetFiles = selectedFiles();
            if (targetFiles.length === 0) {
                toast.info("Select files to download.");
                return;
            }
            toast.info(`Starting batch download of ${targetFiles.length} files...`);

            for (const file of targetFiles) {
                const filePath = `${dirPath}/${file.name}`;
                try {
                    await invoke(driveMode === 'vault' ? 'cmd_vault_download_file' : 'cmd_download_file', { messageId: file.id, savePath: filePath, folderId: activeFolderId });
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
        const fileIds = selectedFiles().map((file) => file.id);
        if (fileIds.length === 0) {
            toast.info("Select files to move.");
            return;
        }
        try {
            await invoke(driveMode === 'vault' ? 'cmd_vault_move_files' : 'cmd_move_files', {
                messageIds: fileIds,
                sourceFolderId: activeFolderId,
                targetFolderId: targetFolderId
            });
            toast.success(`Moved ${fileIds.length} files.`);
            queryClient.invalidateQueries({ queryKey: ['files', driveMode, activeFolderId] });
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
            const files = displayedFiles.filter((file) => file.type !== 'folder');
            toast.info(`Downloading folder contents (${files.length} files)...`);
            for (const file of files) {
                const filePath = `${dirPath}/${file.name}`;
                try {
                    await invoke(driveMode === 'vault' ? 'cmd_vault_download_file' : 'cmd_download_file', { messageId: file.id, savePath: filePath, folderId: activeFolderId });
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
                return await invoke<TelegramFile[]>(driveMode === 'vault' ? 'cmd_vault_search_global' : 'cmd_search_global', { query });
            } catch {
                return [];
            }
        }
    };
}
