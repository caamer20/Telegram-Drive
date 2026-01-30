import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { open } from '@tauri-apps/plugin-dialog';
import { useQueryClient } from '@tanstack/react-query';
import { toast } from 'sonner';
import { QueueItem } from '../types';

export function useFileUpload(activeFolderId: number | null) {
    const queryClient = useQueryClient();
    const [uploadQueue, setUploadQueue] = useState<QueueItem[]>([]);
    const [processing, setProcessing] = useState(false);

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
        setUploadQueue(q => q.map(i => i.id === item.id ? { ...i, status: 'uploading' } : i));
        try {
            await invoke('cmd_upload_file', { path: item.path, folderId: item.folderId });
            setUploadQueue(q => q.map(i => i.id === item.id ? { ...i, status: 'success' } : i));
            if (activeFolderId === item.folderId) {
                queryClient.invalidateQueries({ queryKey: ['files', activeFolderId] });
            }
        } catch (e) {
            setUploadQueue(q => q.map(i => i.id === item.id ? { ...i, status: 'error', error: String(e) } : i));
            toast.error(`Upload failed for ${item.path.split('/').pop()}: ${e}`);
        } finally {
            setProcessing(false);
        }
    };

    const handleManualUpload = async () => {
        try {
            const selected = await open({ multiple: true, directory: false });
            if (selected) {
                const paths = Array.isArray(selected) ? selected : [selected];
                const newItems: QueueItem[] = paths.map((path: string) => ({
                    id: Math.random().toString(36).substr(2, 9),
                    path,
                    folderId: activeFolderId,
                    status: 'pending'
                }));
                setUploadQueue(prev => [...prev, ...newItems]);
                toast.info(`Queued ${paths.length} files for upload`);
            }
        } catch (e) {
            console.error("Manual upload failed:", e);
            toast.error("Failed to open file dialog");
        }
    };

    // File Drop
    useEffect(() => {
        const unlistenPromise = getCurrentWindow().listen('tauri://file-drop', async (event: any) => {
            const paths = event.payload as string[];
            if (paths && paths.length > 0) {
                const newItems = paths.map(path => ({
                    id: Math.random().toString(36).substr(2, 9),
                    path,
                    folderId: activeFolderId,
                    status: 'pending' as const
                }));
                setUploadQueue(prev => [...prev, ...newItems]);
                toast.info(`Queued ${paths.length} dropped files`);
            }
        });
        return () => { unlistenPromise.then(unlisten => unlisten()); };
    }, [queryClient, activeFolderId]);

    return {
        uploadQueue,
        setUploadQueue,
        handleManualUpload
    };
}
