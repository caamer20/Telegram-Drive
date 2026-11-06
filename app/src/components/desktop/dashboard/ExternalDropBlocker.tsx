import { useEffect, useRef, useState } from 'react';
import { useModalFocus } from '../../../hooks/useModalFocus';
import { Upload } from 'lucide-react';
import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { useTranslation } from 'react-i18next';

import type { DropUploadResult } from '../../../types';
import { DragDropOverlay } from './DragDropOverlay';

interface ExternalDropBlockerProps {
    currentFolderName: string;
    enabled?: boolean;
    onFilesDropped: (paths: string[]) => Promise<DropUploadResult> | DropUploadResult;
    onUploadClick?: () => void;
}

/**
 * Handles files dragged from the operating-system file manager. Tauri's native
 * event is the source of truth because browser File objects do not expose
 * portable absolute paths.
 */
export function ExternalDropBlocker({
    currentFolderName,
    enabled = true,
    onFilesDropped,
    onUploadClick,
}: ExternalDropBlockerProps) {
    const { t } = useTranslation();
    const [isDragging, setIsDragging] = useState(false);
    const [detectedCount, setDetectedCount] = useState(0);
    const [showFallback, setShowFallback] = useState(false);
    const fallbackRef = useRef<HTMLDivElement>(null);
    useModalFocus(fallbackRef, () => setShowFallback(false), showFallback && enabled);
    const onFilesDroppedRef = useRef(onFilesDropped);
    const lastDropRef = useRef<{ signature: string; timestamp: number } | null>(null);

    onFilesDroppedRef.current = onFilesDropped;

    useEffect(() => {
        if (!enabled) {
            setShowFallback(false);
            setIsDragging(false);
            setDetectedCount(0);
            return;
        }

        // Browser-only development cannot turn dropped File objects into paths
        // the Rust upload command can read. Preserve the gesture and direct the
        // developer to the normal file picker instead.
        if (!isTauri()) {
            const handleDragEnter = (event: DragEvent) => {
                if (!event.dataTransfer?.types.includes('Files')) return;
                event.preventDefault();
                setDetectedCount(event.dataTransfer.items.length || event.dataTransfer.files.length);
                setIsDragging(true);
            };
            const handleDragOver = (event: DragEvent) => {
                if (!event.dataTransfer?.types.includes('Files')) return;
                event.preventDefault();
                event.dataTransfer.dropEffect = 'copy';
            };
            const handleDragLeave = (event: DragEvent) => {
                if (event.relatedTarget !== null) return;
                setIsDragging(false);
                setDetectedCount(0);
            };
            const handleDrop = (event: DragEvent) => {
                if (!event.dataTransfer?.types.includes('Files')) return;
                event.preventDefault();
                setIsDragging(false);
                setDetectedCount(0);
                setShowFallback(true);
            };

            document.addEventListener('dragenter', handleDragEnter, true);
            document.addEventListener('dragover', handleDragOver, true);
            document.addEventListener('dragleave', handleDragLeave, true);
            document.addEventListener('drop', handleDrop, true);
            return () => {
                document.removeEventListener('dragenter', handleDragEnter, true);
                document.removeEventListener('dragover', handleDragOver, true);
                document.removeEventListener('dragleave', handleDragLeave, true);
                document.removeEventListener('drop', handleDrop, true);
            };
        }

        let disposed = false;
        let unlisten: (() => void) | undefined;

        getCurrentWebview().onDragDropEvent(async (event) => {
            if (disposed) return;

            switch (event.payload.type) {
                case 'enter':
                    setDetectedCount(event.payload.paths.length);
                    setIsDragging(event.payload.paths.length > 0);
                    break;
                case 'over':
                    break;
                case 'leave':
                    setIsDragging(false);
                    setDetectedCount(0);
                    break;
                case 'drop': {
                    setIsDragging(false);
                    setDetectedCount(0);
                    const paths = event.payload.paths.filter(path => path.trim().length > 0);
                    if (paths.length === 0) return;

                    // Guard against duplicate native delivery during webview/window
                    // transitions without suppressing a later intentional re-drop.
                    const signature = [...paths].sort().join('\u0000');
                    const now = Date.now();
                    const lastDrop = lastDropRef.current;
                    if (lastDrop && lastDrop.signature === signature && now - lastDrop.timestamp < 750) {
                        return;
                    }
                    lastDropRef.current = { signature, timestamp: now };

                    try {
                        await onFilesDroppedRef.current(paths);
                    } catch (error) {
                        console.error('[ExternalDropBlocker] Failed to queue dropped files:', error);
                    }
                    break;
                }
            }
        }).then(listener => {
            if (disposed) listener();
            else unlisten = listener;
        }).catch(error => {
            console.error('[ExternalDropBlocker] Failed to register native drop listener:', error);
            if (!disposed) setShowFallback(true);
        });

        return () => {
            disposed = true;
            unlisten?.();
        };
    }, [enabled]);

    return (
        <>
            <>
                {isDragging && (
                    <DragDropOverlay
                        currentFolderName={currentFolderName}
                        fileCount={detectedCount}
                    />
                )}
            </>

            <>
                {enabled && showFallback && (
                    <div
                        className="fixed inset-0 z-[200] flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm"
                        onClick={() => setShowFallback(false)}
                    >
                        <div
                            ref={fallbackRef} role="dialog" aria-modal="true" aria-labelledby="drop-fallback-title" tabIndex={-1} data-ui-motion
                            className="ui-enter glass max-w-md rounded-2xl border border-telegram-border bg-telegram-surface p-8 shadow-2xl"
                            onClick={event => event.stopPropagation()}
                        >
                            <div className="flex flex-col items-center gap-4 text-center">
                                <div className="flex h-16 w-16 items-center justify-center rounded-full bg-telegram-primary/20">
                                    <Upload className="h-8 w-8 text-telegram-primary" />
                                </div>
                                <div>
                                    <h3 id="drop-fallback-title" className="mb-2 text-lg font-semibold text-telegram-text">
                                        {t('files.drop_not_available')}
                                    </h3>
                                    <p className="text-sm text-telegram-subtext">
                                        {t('files.drop_browser_help')}
                                    </p>
                                </div>
                                <div className="flex gap-3">
                                    <button
                                        onClick={() => setShowFallback(false)}
                                        className="quiet-control px-4 py-2 text-sm text-telegram-text"
                                    >
                                        {t('common.cancel')}
                                    </button>
                                    <button
                                        onClick={() => {
                                            setShowFallback(false);
                                            onUploadClick?.();
                                        }}
                                        className="rounded-lg bg-telegram-primary px-6 py-2 text-sm font-medium text-app-accent-contrast hover:bg-telegram-primary/90"
                                    >
                                        {t('files.open_upload_dialog')}
                                    </button>
                                </div>
                            </div>
                        </div>
                    </div>
                )}
            </>
        </>
    );
}
