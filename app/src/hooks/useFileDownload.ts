import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { save, open } from '@tauri-apps/plugin-dialog';
import { toast } from 'sonner';
import { DownloadItem, TelegramFile } from '../types';

export function useFileDownload() {
    const [downloadQueue, setDownloadQueue] = useState<DownloadItem[]>([]);
    const [processing, setProcessing] = useState(false);

    // Queue Processor
    useEffect(() => {
        if (processing) return;
        const nextItem = downloadQueue.find(i => i.status === 'pending');
        if (nextItem) {
            processItem(nextItem);
        }
    }, [downloadQueue, processing]);

    const processItem = async (item: DownloadItem) => {
        setProcessing(true);
        setDownloadQueue(q => q.map(i => i.id === item.id ? { ...i, status: 'downloading' } : i));

        try {
            const savePath = await save({ defaultPath: item.filename });
            if (!savePath) {
                // User cancelled
                setDownloadQueue(q => q.filter(i => i.id !== item.id));
                setProcessing(false);
                return;
            }

            await invoke('cmd_download_file', {
                messageId: item.messageId,
                savePath,
                folderId: item.folderId
            });

            setDownloadQueue(q => q.map(i => i.id === item.id ? { ...i, status: 'success' } : i));
            toast.success(`Downloaded: ${item.filename}`);
        } catch (e) {
            setDownloadQueue(q => q.map(i => i.id === item.id ? { ...i, status: 'error', error: String(e) } : i));
            toast.error(`Download failed: ${item.filename}`);
        } finally {
            setProcessing(false);
        }
    };

    const queueDownload = (messageId: number, filename: string, folderId: number | null) => {
        const newItem: DownloadItem = {
            id: Math.random().toString(36).substr(2, 9),
            messageId,
            filename,
            folderId,
            status: 'pending'
        };
        setDownloadQueue(prev => [...prev, newItem]);
    };

    const queueBulkDownload = async (files: TelegramFile[], folderId: number | null) => {
        const dirPath = await open({
            directory: true,
            multiple: false,
            title: "Select Download Destination"
        });
        if (!dirPath) return;

        // For bulk, we don't prompt for each file - we use the directory
        for (const file of files) {
            const newItem: DownloadItem = {
                id: Math.random().toString(36).substr(2, 9),
                messageId: file.id,
                filename: file.name,
                folderId,
                status: 'pending'
            };
            setDownloadQueue(prev => [...prev, newItem]);
        }

        toast.info(`Queued ${files.length} files for download`);
    };

    const clearFinished = () => {
        setDownloadQueue(q => q.filter(i => i.status !== 'success'));
    };

    return {
        downloadQueue,
        queueDownload,
        queueBulkDownload,
        clearFinished
    };
}
