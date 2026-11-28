import i18n from '../i18n';
import { searchLocal } from '../services/localSearch';
import { sourceFolder } from '../services/fileIdentity';
import { useCallback, useRef } from 'react';
import { useActionScope } from './useActionScope';
import { showFileDialogFallback, pickWithFallback, sanitizeFilename } from '../utils';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { useQueryClient } from '@tanstack/react-query';
import { toast } from 'sonner';
import { useConfirm } from '../context/ConfirmContext';
import { TelegramFile } from '../types';
import { updateFileQueryData, invalidateOwnedFileQueries } from '../services/fileListRefresh';
import { userFacingError } from '../services/userFacingError';
import { useTranslation } from 'react-i18next';

export function useFileOperations(
    activeFolderId: number | null,
    selectedIds: number[],
    setSelectedIds: (ids: number[]) => void,
    displayedFiles: TelegramFile[],
    queueBulkDownload?: (files: TelegramFile[], folderId: number | null) => void,
    ownerId: string | null = null,
) {
    const queryClient = useQueryClient();
    const { confirm } = useConfirm();
    const { t } = useTranslation();
    const capture = useActionScope(ownerId);

    // Callbacks read current selection data without being recreated for every click.
    const selectedIdsRef = useRef(selectedIds);
    selectedIdsRef.current = selectedIds;
    const displayedFilesRef = useRef(displayedFiles);
    displayedFilesRef.current = displayedFiles;

    const handleDelete = useCallback(async (target: TelegramFile | number) => {
        const isCurrent = capture();
        if (!ownerId || !isCurrent()) return;
        // Resolve the original target before confirmation. Reading current
        // lists afterwards can reinterpret the same message ID in another account.
        const file = typeof target === 'number'
            ? displayedFilesRef.current.find(candidate => candidate.id === target)
            : target;
        if (!file) { toast.error(t('common.operation_failed')); return; }
        const id = file.id;
        const sourceFolderId = sourceFolder(file, activeFolderId);
        if (!await confirm({ title: t('operations.delete_file'), message: t('operations.delete_file_question'), confirmText: i18n.t("files.delete"), variant: 'danger' }) || !isCurrent()) return;
        try {
            await invoke('cmd_move_file_to_trash', { messageId: id, folderId: sourceFolderId });
            if (!isCurrent()) return;
            updateFileQueryData(queryClient, sourceFolderId, new Set([id]), () => null, ownerId);
            void invalidateOwnedFileQueries(queryClient, ownerId);
            toast.success(t('operations.file_deleted'));
        } catch (error) {
            if (isCurrent()) toast.error(userFacingError(error, t));
        }
    }, [activeFolderId, capture, confirm, ownerId, queryClient, t]);

    const handleBulkDelete = useCallback(async () => {
        const isCurrent = capture();
        if (!ownerId || !isCurrent()) return;
        const ids = [...selectedIdsRef.current];
        if (ids.length === 0) return;
        const targets = ids.map(id => displayedFilesRef.current.find(file => file.id === id))
            .filter((file): file is TelegramFile => Boolean(file))
            .map(file => ({ id: file.id, folderId: sourceFolder(file, activeFolderId) }));
        if (targets.length !== ids.length) { toast.error(t('common.operation_failed')); return; }
        if (!await confirm({ title: t('operations.delete_files'), message: t('operations.delete_files_question',{count:ids.length}), confirmText: t('operations.delete_all'), variant: 'danger' }) || !isCurrent()) return;
        let success = 0;
        let fail = 0;
        const deletedByFolder = new Map<number | null, number[]>();
        for (const target of targets) {
            if (!isCurrent()) return;
            try {
                await invoke('cmd_move_file_to_trash', { messageId: target.id, folderId: target.folderId });
                if (!isCurrent()) return;
                success++;
                deletedByFolder.set(target.folderId, [...(deletedByFolder.get(target.folderId) ?? []), target.id]);
            } catch {
                if (!isCurrent()) return;
                fail++;
            }
        }
        if (!isCurrent()) return;
        setSelectedIds([]);
        for (const [folderId, deletedIds] of deletedByFolder) {
            updateFileQueryData(queryClient, folderId, new Set(deletedIds), () => null, ownerId);
        }
        void invalidateOwnedFileQueries(queryClient, ownerId);
        if (success > 0) toast.success(t('operations.deleted_count',{count:success}));
        if (fail > 0) toast.error(t('operations.delete_failed_count',{count:fail}));
    }, [activeFolderId, capture, confirm, ownerId, queryClient, setSelectedIds, t]);

    const handleRenameFile = useCallback(async (file: TelegramFile, newName: string) => {
        const isCurrent = capture();
        if (!ownerId || !isCurrent()) return false;
        const id = file.id;
        const folderId = sourceFolder(file, activeFolderId);
        try {
            await invoke('cmd_rename_file', { ownerId, messageId: id, folderId, newName });
            if (!isCurrent()) return false;
            updateFileQueryData(queryClient, folderId, new Set([id]), item => ({ ...item, name: newName }), ownerId);
            void invalidateOwnedFileQueries(queryClient, ownerId);
            toast.success(t('operations.renamed',{name:newName}));
            return true;
        } catch (error) {
            if (isCurrent()) toast.error(userFacingError(error, t));
            return false;
        }
    }, [activeFolderId, capture, ownerId, queryClient, t]);

    const handleMoveFiles = useCallback(async (files: TelegramFile[], targetFolderId: number | null, onSuccess?: () => void, confirmLarge = false) => {
        const isCurrent = capture();
        if (!ownerId || !isCurrent() || !files.length) return false;
        const ids = [...new Set(files.map(file => file.id))];
        const sourceFolders = new Set(files.map(file => sourceFolder(file, activeFolderId)));
        if (sourceFolders.size !== 1) { toast.info(t('operations.move_one_source')); return false; }
        const sourceFolderId = sourceFolders.values().next().value!;
        if (sourceFolderId === targetFolderId) { toast.info(t('operations.already_in_folder')); return false; }
        if (confirmLarge && ids.length >= 10) {
            const accepted = await confirm({ title: t('operations.bulk_move'), message: t('operations.move_question',{count:ids.length}), confirmText: t('operations.move_action',{count:ids.length}), variant: 'info' });
            if (!accepted || !isCurrent()) return false;
        }
        try {
            await invoke('cmd_move_files', { ownerId, messageIds: ids, sourceFolderId, targetFolderId });
            if (!isCurrent()) return false;
            updateFileQueryData(queryClient, sourceFolderId, new Set(ids), () => null, ownerId);
            void invalidateOwnedFileQueries(queryClient, ownerId);
            toast.success(t('operations.moved_count',{count:ids.length}));
            onSuccess?.();
            return true;
        } catch (error) {
            if (isCurrent()) toast.error(userFacingError(error, t));
            return false;
        }
    }, [activeFolderId, capture, confirm, ownerId, queryClient, t]);

    const handleCopyFile = useCallback(async (file: TelegramFile, targetFolderId: number | null, onSuccess?: () => void) => {
        const isCurrent = capture();
        if (!ownerId || !isCurrent()) return false;
        const sourceFolderId = sourceFolder(file, activeFolderId);
        if (sourceFolderId === targetFolderId) { toast.info(t('operations.already_in_folder')); return false; }
        try {
            await invoke('cmd_copy_files', { ownerId, messageIds: [file.id], sourceFolderId, targetFolderId });
            if (!isCurrent()) return false;
            toast.success('Copied to folder');
            onSuccess?.();
            return true;
        } catch (error) {
            if (isCurrent()) toast.error(userFacingError(error, t));
            return false;
        }
    }, [activeFolderId, capture, ownerId, t]);

    const handleBulkDownload = useCallback(async () => {
        const ids = selectedIdsRef.current;
        if (ids.length === 0) return;
        const currentFiles = displayedFilesRef.current;
        const targetFiles = currentFiles.filter((f) => ids.includes(f.id));
        if (targetFiles.length === 0) return;
        if (queueBulkDownload) {
            queueBulkDownload(targetFiles, activeFolderId);
            setSelectedIds([]);
            return;
        }
        const downloadToDir = async (dirPath: string) => {
            let successCount = 0;
            const sep = dirPath.includes('\\') ? '\\' : '/';
            for (const file of targetFiles) {
                const sanitizedName = sanitizeFilename(file.name);
                const filePath = dirPath.endsWith(sep) ? `${dirPath}${sanitizedName}` : `${dirPath}${sep}${sanitizedName}`;
                try {
                    await invoke('cmd_download_file', { req: { message_id: file.id, save_path: filePath, folder_id: sourceFolder(file, activeFolderId) } });
                    successCount++;
                } catch { }
            }
            toast.success(t('operations.downloaded_count',{count:successCount}));
            setSelectedIds([]);
        };
        try {
            const dirPath = await pickWithFallback(
                () => open({ directory: true, multiple: false, title: i18n.t('common.download_destination') }),
                () => handleBulkDownload(),
                {
                    errorTitle: 'Folder picker failed',
                    onBrowserPicker: async () => {
                        const paths = await showFileDialogFallback({ directory: true, multiple: false });
                        if (paths.length === 0) return null;
                        const sep = paths[0].includes('\\') ? '\\' : '/';
                        return paths[0].substring(0, paths[0].lastIndexOf(sep));
                    },
                },
            );
            if (!dirPath) return;
            await downloadToDir(dirPath);
        } catch (e) {
            toast.error(t('operations.bulk_download_failed',{error:String(e)}));
        }
    }, [activeFolderId, setSelectedIds, queueBulkDownload, t]);

    const handleBulkMove = useCallback(async (targetFolderId: number | null, onSuccess?: () => void) => {
        const ids = [...selectedIdsRef.current];
        const files = displayedFilesRef.current.filter(file => ids.includes(file.id));
        if (files.length !== ids.length) return;
        await handleMoveFiles(files, targetFolderId, () => { setSelectedIds([]); onSuccess?.(); });
    }, [handleMoveFiles, setSelectedIds]);

    const handleDownloadFolder = useCallback(async () => {
        const files = displayedFilesRef.current;
        if (files.length === 0) {
            toast.info(t('operations.folder_empty'));
            return;
        }
        if (queueBulkDownload) {
            queueBulkDownload(files, activeFolderId);
            return;
        }
        const downloadToDir = async (dirPath: string) => {
            let successCount = 0;
            toast.info(t('operations.folder_downloading',{count:files.length}));
            const sep = dirPath.includes('\\') ? '\\' : '/';
            for (const file of files) {
                const sanitizedName = sanitizeFilename(file.name);
                const filePath = dirPath.endsWith(sep) ? `${dirPath}${sanitizedName}` : `${dirPath}${sep}${sanitizedName}`;
                try {
                    await invoke('cmd_download_file', { req: { message_id: file.id, save_path: filePath, folder_id: sourceFolder(file, activeFolderId) } });
                    successCount++;
                } catch { }
            }
            toast.success(t('operations.folder_download_complete',{count:successCount}));
        };
        try {
            const dirPath = await pickWithFallback(
                () => open({
                    directory: true, multiple: false, title: i18n.t('common.download_folder_to')
                }),
                () => handleDownloadFolder(),
                {
                    errorTitle: 'Folder picker failed',
                    onBrowserPicker: async () => {
                        const paths = await showFileDialogFallback({ directory: true, multiple: false });
                        if (paths.length === 0) return null;
                        const sep = paths[0].includes('\\') ? '\\' : '/';
                        return paths[0].substring(0, paths[0].lastIndexOf(sep));
                    },
                },
            );
            if (!dirPath) return;
            await downloadToDir(dirPath);
        } catch (e) {
            toast.error(userFacingError(e, t));
        }
    }, [activeFolderId, queueBulkDownload, t]);

    const handleGlobalSearch = useCallback(async (query: string) => {
        const isCurrent = capture();
        if (!ownerId || !isCurrent()) return [];
        try {
            const result = await searchLocal(ownerId, { query }, isCurrent);
            return isCurrent() ? result.files : [];
        } catch {
            return [];
        }
    }, [capture, ownerId]);

    return {
        handleDelete,
        handleBulkDelete,
        handleBulkDownload,
        handleBulkMove,
        handleMoveFiles,
        handleCopyFile,
        handleRenameFile,
        handleDownloadFolder,
        handleGlobalSearch,
    };
}
