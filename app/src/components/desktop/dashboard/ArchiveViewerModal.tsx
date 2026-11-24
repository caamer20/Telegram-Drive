import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { useQueryClient } from '@tanstack/react-query';
import { X, Folder, File, Archive, Loader2, AlertTriangle, FileArchive, Download, ChevronDown, HardDrive, Zap, Square, CheckCircle, XCircle } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { ArchiveEntry, TelegramFile, TelegramFolder } from '../../../types';
import { formatBytes } from '../../../utils';
import { toast } from 'sonner';
import { useModalFocus } from '../../../hooks/useModalFocus';
import { userFacingError } from '../../../services/userFacingError';
import { invalidateFolderFileQueries } from '../../../services/fileListRefresh';
import i18n from '../../../i18n';

interface ArchiveViewerModalProps {
    file: TelegramFile;
    activeFolderId?: number | null;
    folders: TelegramFolder[];
    onClose: () => void;
    onNext?: () => void;
    onPrev?: () => void;
    currentIndex?: number;
    totalItems?: number;
    nextFile?: TelegramFile | null;
    prevFile?: TelegramFile | null;
}

interface ExtractedFile {
    temp_path: string;
    filename: string;
    size: number;
}

export function ArchiveViewerModal({
    file,
    activeFolderId,
    folders,
    onClose,
    onNext,
    onPrev,
    currentIndex = 0,
    totalItems = 0,
    nextFile,
    prevFile,
}: ArchiveViewerModalProps) {
    const queryClient = useQueryClient();
    const [entries, setEntries] = useState<ArchiveEntry[] | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [loading, setLoading] = useState(true);
    const { t } = useTranslation();
    const panelRef = useRef<HTMLDivElement>(null);
    useModalFocus(panelRef, onClose);

    // Debounce cache invalidations so rapid successive extracts don't
    // flood React Query with individual refetch requests.
    const invalidationPending = useRef<Set<number | null>>(new Set());
    const invalidationTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
    const mountedRef = useRef(true);
    const debouncedInvalidate = (folderId: number | null) => {
        // A per-entry upload may finish just after the viewer closes. Its result
        // still needs to reach the folder even though no debounce remains mounted.
        if (!mountedRef.current) {
            void invalidateFolderFileQueries(queryClient, folderId);
            return;
        }
        invalidationPending.current.add(folderId);
        if (invalidationTimer.current) clearTimeout(invalidationTimer.current);
        invalidationTimer.current = setTimeout(() => {
            const ids = invalidationPending.current;
            invalidationPending.current = new Set();
            for (const id of ids) {
                void invalidateFolderFileQueries(queryClient, id);
            }
            invalidationTimer.current = null;
        }, 300);
    };
    // Clean up on unmount — flush pending invalidations, cancel running extract-all
    useEffect(() => {
        mountedRef.current = true;
        return () => {
            mountedRef.current = false;
            if (invalidationTimer.current) clearTimeout(invalidationTimer.current);
            const ids = invalidationPending.current;
            if (ids.size > 0) {
                invalidationPending.current = new Set();
                for (const id of ids) {
                    void invalidateFolderFileQueries(queryClient, id);
                }
            }
            // Cancel any running extract-all so orphaned uploads don't continue
            extractAllCancelledRef.current = true;
            extractAllProgressUnlisten.current?.();
            extractAllProgressUnlisten.current = null;
            if (extractAllCurrentTransferRef.current) {
                invoke('cmd_cancel_transfer', { transferId: extractAllCurrentTransferRef.current }).catch(() => {});
            }
        };
    }, []);

    // Extract-all state
    const [extractAllBusy, setExtractAllBusy] = useState(false);
    const [extractAllDone, setExtractAllDone] = useState(0);
    const [extractAllCancelled, setExtractAllCancelled] = useState(false);
    const [extractAllTargetFolderId, setExtractAllTargetFolderId] = useState<number | null>(activeFolderId ?? null);
    const [extractAllFolderMenuOpen, setExtractAllFolderMenuOpen] = useState(false);
    const extractAllMenuRef = useRef<HTMLDivElement | null>(null);
    const extractAllCancelledRef = useRef(false);
    const extractAllCurrentTransferRef = useRef<string>('');
    const extractAllProgressUnlisten = useRef<UnlistenFn | null>(null);
    const extractAllEntryStatuses = useRef<Map<number, 'done' | 'failed'>>(new Map());

    // Per-file progress during extract-all
    const [extractAllProgress, setExtractAllProgress] = useState<UploadProgress | null>(null);
    const [extractAllCurrentEntryIndex, setExtractAllCurrentEntryIndex] = useState<number | null>(null);

    // Close extract-all folder menu on click outside
    useEffect(() => {
        if (!extractAllFolderMenuOpen) return;
        const fn = (e: MouseEvent) => {
            if (extractAllMenuRef.current && !extractAllMenuRef.current.contains(e.target as Node)) {
                setExtractAllFolderMenuOpen(false);
            }
        };
        document.addEventListener('mousedown', fn);
        return () => document.removeEventListener('mousedown', fn);
    }, [extractAllFolderMenuOpen]);

    const extractAllTargetName = extractAllTargetFolderId === null
        ? t('common.saved_messages')
        : folders.find(f => f.id === extractAllTargetFolderId)?.name ?? 'Current Folder';

    const handleExtractAll = async () => {
        if (extractAllBusy || !entries) return;
        const fileEntries = entries.filter(e => !e.is_dir);
        if (fileEntries.length === 0) return;

        setExtractAllBusy(true);
        setExtractAllDone(0);
        setExtractAllCancelled(false);
        setExtractAllProgress(null);
        setExtractAllCurrentEntryIndex(null);
        setExtractAllFolderMenuOpen(false);
        extractAllCancelledRef.current = false;
        extractAllEntryStatuses.current = new Map();

        let done = 0;
        let failed = 0;
        let wasCancelled = false;

        let batchId: number | undefined;
        try {
            // Subscribe to upload-progress once for the entire batch
            const unlisten = await listen<UploadProgress>('upload-progress', (event) => {
                if (event.payload.id === extractAllCurrentTransferRef.current) {
                    setExtractAllProgress({
                        id: event.payload.id,
                        percent: event.payload.percent,
                        uploaded_bytes: event.payload.uploaded_bytes,
                        total_bytes: event.payload.total_bytes,
                        speed_bytes_per_sec: event.payload.speed_bytes_per_sec,
                    });
                }
            });
            extractAllProgressUnlisten.current = unlisten;

            if (!extractAllCancelledRef.current) {
                batchId = await invoke<number>('cmd_open_archive_batch', { messageId: file.id, folderId: activeFolderId ?? null });
            }
            for (let i = 0; i < entries.length; i++) {
                if (extractAllCancelledRef.current) {
                    wasCancelled = true;
                    break;
                }

                const entry = entries[i];
                if (entry.is_dir) continue;

                const transferId = `arch-extract-all-${Date.now()}-${i}`;
                extractAllCurrentTransferRef.current = transferId;
                setExtractAllCurrentEntryIndex(i);
                setExtractAllProgress(null);
                let extracted: ExtractedFile | null = null;

                try {
                    // Step 1: Extract
                    extracted = await invoke<ExtractedFile>('cmd_extract_archive_batch_entry', { batchId, entryIndex: i });

                    if (extractAllCancelledRef.current) { wasCancelled = true; break; }

                    // Step 2: Upload
                    await invoke('initiate_upload', {
                        path: extracted.temp_path,
                        folderId: extractAllTargetFolderId,
                        transferId,
                    });

                    done++;
                    extractAllEntryStatuses.current.set(i, 'done');
                } catch (e) {
                    const msg = String(e);
                    if (msg.includes('cancelled') || msg.includes('canceled')) {
                        wasCancelled = true;
                        break;
                    }
                    failed++;
                    extractAllEntryStatuses.current.set(i, 'failed');
                    console.error(`Extract-all failed for "${entry.filename}":`, msg);
                } finally {
                    // Clean up temp file
                    if (extracted?.temp_path) {
                        try { await invoke('cmd_delete_temp_zip', { path: extracted.temp_path }); } catch {}
                    }
                }

                setExtractAllDone(done);
            }
        } catch (reason) {
            wasCancelled = extractAllCancelledRef.current;
            if (!wasCancelled) {
                failed = fileEntries.length - done;
                toast.error(userFacingError(reason, t));
            }
        } finally {
            if (batchId !== undefined) {
                await invoke('cmd_close_archive_batch', { batchId }).catch(() => {
                    if (mountedRef.current) toast.error(t('common.operation_failed'));
                });
            }
            setExtractAllBusy(false);
            extractAllCurrentTransferRef.current = '';
            setExtractAllProgress(null);
            setExtractAllCurrentEntryIndex(null);
            extractAllProgressUnlisten.current?.();
            extractAllProgressUnlisten.current = null;
        }

        // A completed transfer may need a folder refresh after the viewer closes.
        debouncedInvalidate(extractAllTargetFolderId);
        if (!mountedRef.current) return;

        // Summary toast
        const destName = extractAllTargetFolderId === null ? t('common.saved_messages') : extractAllTargetName;
        if (wasCancelled) {
            if (done > 0) {
                toast.info(`${done} extracted, cancelled — to ${destName}`);
            } else {
                toast.info('Extract All cancelled');
            }
        } else if (failed === 0) {
            toast.success(`${done} file${done !== 1 ? 's' : ''} extracted to ${destName}`);
        } else {
            toast.warning(`${done} extracted, ${failed} failed — to ${destName}`);
        }
    };

    const handleCancelExtractAll = () => {
        if (extractAllCancelledRef.current) return;
        extractAllCancelledRef.current = true;
        setExtractAllCancelled(true);
        // Cancel the currently in-flight transfer
        if (extractAllCurrentTransferRef.current) {
            invoke('cmd_cancel_transfer', { transferId: extractAllCurrentTransferRef.current }).catch(() => {});
        }
    };

    const lastFileId = useRef<number | null>(null);

    useEffect(() => {
        let cancelled = false;

        async function load() {
            setLoading(true);
            setError(null);
            setEntries(null);

            try {
                const result = await invoke<ArchiveEntry[]>('cmd_list_archive_contents', {
                    messageId: file.id,
                    folderId: activeFolderId ?? null,
                });
                if (!cancelled) {
                    const archiveChanged = lastFileId.current !== file.id;
                    lastFileId.current = file.id;
                    setEntries(result);
                    if (archiveChanged) extractAllEntryStatuses.current = new Map();
                }
            } catch (e) {
                if (!cancelled) {
                    setError(userFacingError(e, t));
                }
            } finally {
                if (!cancelled) {
                    setLoading(false);
                }
            }
        }

        load();
        return () => { cancelled = true; };
    }, [file.id, activeFolderId]);

    // Keyboard: Escape to close, arrow keys to navigate
    useEffect(() => {
        const handleKeyDown = (e: KeyboardEvent) => {
            if (e.key === 'ArrowRight' && onNext) {
                onNext();
            } else if (e.key === 'ArrowLeft' && onPrev) {
                onPrev();
            }
        };
        window.addEventListener('keydown', handleKeyDown);
        return () => window.removeEventListener('keydown', handleKeyDown);
    }, [onClose, onNext, onPrev]);

    const totalSize = entries?.reduce((sum, e) => sum + (e.is_dir ? 0 : e.size), 0) ?? 0;
    const fileCount = entries?.filter(e => !e.is_dir).length ?? 0;
    const dirCount = entries?.filter(e => e.is_dir).length ?? 0;

    return (
        <div
            className="fixed inset-0 z-[200] flex items-center justify-center bg-app-overlay p-4 backdrop-blur-sm"
            onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}
        >
            {/* Arrow navigation buttons */}
            {prevFile && onPrev && (
                <button
                    onClick={(e) => { e.stopPropagation(); onPrev(); }}
                    className="viewer-navigation absolute start-4 top-1/2 z-[210] -translate-y-1/2"
                    aria-label={i18n.t('viewer.previous_file')}
                >
                    <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><polyline points="15 18 9 12 15 6"></polyline></svg>
                </button>
            )}
            {nextFile && onNext && (
                <button
                    onClick={(e) => { e.stopPropagation(); onNext(); }}
                    className="viewer-navigation absolute end-4 top-1/2 z-[210] -translate-y-1/2"
                    aria-label={i18n.t('viewer.next_file')}
                >
                    <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><polyline points="9 18 15 12 9 6"></polyline></svg>
                </button>
            )}

            <div
                ref={panelRef}
                role="dialog"
                aria-modal="true"
                aria-label={`Archive contents: ${file.name}`}
                tabIndex={-1}
                className="quiet-raised flex max-h-[82vh] w-[min(640px,calc(100vw-2rem))] flex-col overflow-hidden animate-in fade-in zoom-in-95 duration-150"
                onClick={e => e.stopPropagation()}
            >
                {/* Header */}
                <div className="flex min-h-12 shrink-0 items-center justify-between border-b border-app-border-subtle px-3 py-2">
                    <div className="flex min-w-0 items-center gap-2.5">
                        <FileArchive className="h-5 w-5 shrink-0 text-app-accent" />
                        <div className="min-w-0">
                            <h3 className="truncate text-ui font-medium text-app-text" title={file.name}>{file.name}</h3>
                            <p className="text-badge text-app-text-tertiary">{file.sizeStr}</p>
                        </div>
                    </div>
                    <div className="flex items-center gap-2 shrink-0">
                        {totalItems > 1 && (
                            <span className="me-1 text-badge tabular-nums text-app-text-tertiary">
                                {currentIndex + 1} / {totalItems}
                            </span>
                        )}
                        <button
                            onClick={onClose}
                            className="quiet-control flex h-8 w-8 items-center justify-center text-app-text-secondary hover:text-app-text"
                            aria-label={i18n.t('viewer.close_archive')}
                        >
                            <X className="h-4 w-4" />
                        </button>
                    </div>
                </div>

                {/* Stats bar */}
                {entries && !loading && !error && (
                    <div className="shrink-0 border-b border-app-border-subtle bg-app-surface-sunken/20 px-3 py-1.5">
                        <div className="flex items-center justify-between gap-2">
                            <div className="flex items-center gap-3 text-badge text-app-text-secondary">
                                <span className="flex items-center gap-1">
                                    <File className="w-3 h-3" />
                                    {fileCount} file{fileCount !== 1 ? 's' : ''}
                                </span>
                                {dirCount > 0 && (
                                    <span className="flex items-center gap-1">
                                        <Folder className="w-3 h-3" />
                                        {dirCount} folder{dirCount !== 1 ? 's' : ''}
                                    </span>
                                )}
                                <span>{formatBytes(totalSize)} total</span>
                            </div>

                            {/* Extract All / Cancel button */}
                            {fileCount > 0 && (
                                <div className="flex items-center gap-0.5 relative" ref={extractAllMenuRef}>
                                    {extractAllBusy && !extractAllCancelled && (
                                        <span className="text-[10px] text-telegram-primary mr-1 tabular-nums">
                                            {extractAllDone}/{fileCount}
                                        </span>
                                    )}

                                    {extractAllBusy ? (
                                        /* Cancel button during operation */
                                        <button
                                            onClick={handleCancelExtractAll}
                                            disabled={extractAllCancelled}
                                            className="quiet-control flex h-7 items-center gap-1 bg-app-danger/10 px-2 text-badge font-medium text-app-danger disabled:opacity-40"
                                            title="Cancel Extract All"
                                        >
                                            <Square className="w-3 h-3" />
                                            <span>{extractAllCancelled ? 'Stopping...' : `Cancel (${extractAllDone}/${fileCount})`}</span>
                                        </button>
                                    ) : (
                                        /* Extract All button + folder dropdown (idle) */
                                        <>
                                            <button
                                                onClick={handleExtractAll}
                                                className="quiet-control flex h-7 items-center gap-1 rounded-e-none bg-app-success/10 px-2 text-badge font-medium text-app-success"
                                                title={`Extract all ${fileCount} files to ${extractAllTargetName}`}
                                            >
                                                <Zap className="w-3 h-3" />
                                                <span>{t('archive.extract_all')} ({fileCount})</span>
                                            </button>
                                            <button
                                                onClick={(e) => { e.stopPropagation(); setExtractAllFolderMenuOpen(o => !o); }}
                                                className="quiet-control flex h-7 items-center rounded-s-none border-s border-app-success/20 bg-app-success/10 px-1.5 text-app-success"
                                                title={`Bulk target: ${extractAllTargetName}`}
                                            >
                                                <ChevronDown className={`w-2.5 h-2.5 transition-transform ${extractAllFolderMenuOpen ? 'rotate-180' : ''}`} />
                                            </button>
                                        </>
                                    )}

                                    {/* Bulk folder dropdown */}
                                    {extractAllFolderMenuOpen && (
                                        <div
                                            className="quiet-menu absolute end-0 top-full z-[220] mt-1 w-48 overflow-hidden p-1 animate-in fade-in slide-in-from-top-1 duration-100"
                                            onClick={e => e.stopPropagation()}
                                        >
                                            <div className="border-b border-app-border-subtle px-2 py-1.5">
                                                <p className="text-badge font-medium text-app-text-tertiary">{t('archive.extract_all_to')}</p>
                                            </div>
                                            <div className="max-h-48 overflow-y-auto py-1">
                                                <button
                                                    onClick={() => { setExtractAllTargetFolderId(null); setExtractAllFolderMenuOpen(false); }}
                                                    className={`quiet-menu-item gap-2 text-start ${extractAllTargetFolderId === null ? 'bg-app-success/10 text-app-success' : 'text-app-text'}`}
                                                >
                                                    <HardDrive className="w-3.5 h-3.5 shrink-0" />
                                                    <span className="truncate">{t('common.saved_messages')}</span>
                                                    {extractAllTargetFolderId === null && (
                                                        <span className="ms-auto h-1.5 w-1.5 shrink-0 rounded-full bg-app-success" />
                                                    )}
                                                </button>
                                                {folders.map(f => (
                                                    <button
                                                        key={f.id}
                                                        onClick={() => { setExtractAllTargetFolderId(f.id); setExtractAllFolderMenuOpen(false); }}
                                                        className={`quiet-menu-item gap-2 text-start ${extractAllTargetFolderId === f.id ? 'bg-app-success/10 text-app-success' : 'text-app-text'}`}
                                                    >
                                                        <Folder className="w-3.5 h-3.5 shrink-0" />
                                                        <span className="truncate">{f.name}</span>
                                                        {extractAllTargetFolderId === f.id && (
                                                            <span className="ms-auto h-1.5 w-1.5 shrink-0 rounded-full bg-app-success" />
                                                        )}
                                                    </button>
                                                ))}
                                            </div>
                                        </div>
                                    )}
                                </div>
                            )}
                        </div>
                    </div>
                )}

                {/* Content */}
                <div className="flex-1 overflow-y-auto min-h-0">
                    {/* Loading */}
                    {loading && (
                        <div className="flex flex-col items-center justify-center space-y-3 py-14">
                            <Loader2 className="h-6 w-6 animate-spin text-app-accent" />
                            <p className="text-metadata text-app-text-secondary">{t('common.loading')}</p>
                        </div>
                    )}

                    {/* Error */}
                    {error && !loading && (
                        <div className="flex flex-col items-center justify-center py-16 space-y-3 px-6">
                            <AlertTriangle className="w-10 h-10 text-amber-500" />
                            <p className="text-sm text-center text-telegram-text font-medium">
                                {t('archive.failed_read')}
                            </p>
                            <p className="text-xs text-center text-telegram-subtext max-w-sm break-words">
                                {error}
                            </p>
                        </div>
                    )}

                    {/* Empty */}
                    {entries && entries.length === 0 && !loading && !error && (
                        <div className="flex flex-col items-center justify-center py-16 space-y-3">
                            <Archive className="w-10 h-10 text-telegram-subtext/50" />
                            <p className="text-sm text-telegram-subtext">{t('archive.empty_archive')}</p>
                        </div>
                    )}

                    {/* File list */}
                    {entries && entries.length > 0 && (
                        <div className="p-1.5">
                            {entries.map((entry, i) => (
                                <div
                                    key={`${entry.filename}-${i}`}
                                    className="group flex min-h-10 items-center gap-2.5 rounded-control px-2.5 py-1.5 hover:bg-app-hover"
                                >
                                    {/* Icon */}
                                    <div className="shrink-0">
                                        {entry.is_dir ? (
                                            <Folder className="h-4 w-4 text-app-accent" />
                                        ) : (
                                            <File className="h-4 w-4 text-app-text-tertiary" />
                                        )}
                                    </div>

                                    {/* Name */}
                                    <div className="min-w-0 flex-1">
                                        <p className="truncate text-ui text-app-text" title={entry.filename}>
                                            {entry.is_dir ? entry.filename.replace(/\/$/, '') : entry.filename}
                                        </p>
                                        {/* Per-file status during Extract All */}
                                        {extractAllBusy && extractAllCurrentEntryIndex === i ? (
                                            extractAllProgress ? (
                                                /* Upload progress bar */
                                                <div className="flex items-center gap-1.5 mt-1">
                                                    <div className="flex-1 h-1 bg-telegram-hover rounded-full overflow-hidden max-w-[160px]">
                                                        <div
                                                            className="h-full bg-emerald-500 rounded-full transition-all duration-300 ease-out"
                                                            style={{ width: `${Math.min(extractAllProgress.percent, 99)}%` }}
                                                        />
                                                    </div>
                                                    <span className="text-[10px] text-emerald-600 font-mono tabular-nums shrink-0">
                                                        {Math.min(extractAllProgress.percent, 99)}%
                                                    </span>
                                                </div>
                                            ) : (
                                                /* Extracting spinner */
                                                <div className="flex items-center gap-1.5 mt-1">
                                                    <Loader2 className="w-2.5 h-2.5 text-telegram-subtext animate-spin" />
                                                    <span className="text-[10px] text-telegram-subtext">{t('archive.extracting')}</span>
                                                </div>
                                            )
                                        ) : extractAllEntryStatuses.current.get(i) === 'done' ? (
                                            <div className="flex items-center gap-1 mt-1">
                                                <CheckCircle className="w-3 h-3 text-emerald-500" />
                                                <span className="text-[10px] text-emerald-600">Extracted</span>
                                            </div>
                                        ) : extractAllEntryStatuses.current.get(i) === 'failed' ? (
                                            <div className="flex items-center gap-1 mt-1">
                                                <XCircle className="w-3 h-3 text-red-500" />
                                                <span className="text-[10px] text-red-500">{i18n.t("settings.desktop_notify_failed")}</span>
                                            </div>
                                        ) : null}
                                    </div>

                                    {/* Extract button (files only) */}
                                    {!entry.is_dir && (
                                        <ExtractButton
                                            file={file}
                                            activeFolderId={activeFolderId}
                                            folders={folders}
                                            entryIndex={i}
                                            entryName={entry.filename}
                                            entrySize={entry.size}
                                            disabled={extractAllBusy}
                                            onCacheInvalidate={debouncedInvalidate}
                                        />
                                    )}

                                    {/* Sizes */}
                                    <div className="flex shrink-0 flex-col items-end text-end">
                                        {!entry.is_dir ? (
                                            <>
                                                <p className="font-mono text-metadata text-app-text">
                                                    {formatBytes(entry.size)}
                                                </p>
                                                {entry.compressed_size !== entry.size && (
                                                    <p className="text-[10px] text-telegram-subtext/60 font-mono">
                                                        {formatBytes(entry.compressed_size)} compressed
                                                    </p>
                                                )}
                                            </>
                                        ) : (
                                            <p className="text-[10px] text-telegram-subtext/60">folder</p>
                                        )}
                                    </div>
                                </div>
                            ))}
                        </div>
                    )}
                </div>

                {/* Footer */}
                <div className="shrink-0 border-t border-app-border-subtle bg-app-surface-sunken/15 p-2">
                    <button
                        onClick={onClose}
                        className="quiet-control h-8 w-full border border-app-border px-4 text-ui font-medium text-app-text-secondary hover:text-app-text"
                    >
                        {t('common.close')}
                    </button>
                </div>
            </div>
        </div>
    );
}

// ── Extract & Upload button per archive entry ────────────────────────────

interface ExtractButtonProps {
    file: TelegramFile;
    activeFolderId?: number | null;
    folders: TelegramFolder[];
    entryIndex: number;
    entryName: string;
    entrySize: number;
    disabled?: boolean;
    onCacheInvalidate: (folderId: number | null) => void;
}

interface UploadProgress {
    id: string;
    percent: number;
    uploaded_bytes: number;
    total_bytes: number;
    speed_bytes_per_sec: number;
}

function ExtractButton({ file, activeFolderId, folders, entryIndex, entryName, entrySize, disabled: parentDisabled, onCacheInvalidate }: ExtractButtonProps) {
    const [extracting, setExtracting] = useState(false);
    const [uploading, setUploading] = useState(false);
    const [progress, setProgress] = useState<UploadProgress | null>(null);
    const [targetFolderId, setTargetFolderId] = useState<number | null>(activeFolderId ?? null);
    const [folderMenuOpen, setFolderMenuOpen] = useState(false);
    const unlistenRef = useRef<UnlistenFn | null>(null);
    const menuRef = useRef<HTMLDivElement | null>(null);
    const { t } = useTranslation();

    // Close folder menu on click outside
    useEffect(() => {
        if (!folderMenuOpen) return;
        const handleClickOutside = (e: MouseEvent) => {
            if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
                setFolderMenuOpen(false);
            }
        };
        document.addEventListener('mousedown', handleClickOutside);
        return () => document.removeEventListener('mousedown', handleClickOutside);
    }, [folderMenuOpen]);

    // Clean up listener on unmount
    useEffect(() => {
        return () => {
            unlistenRef.current?.();
        };
    }, []);

    const targetFolderName = targetFolderId === null
        ? t('common.saved_messages')
        : folders.find(f => f.id === targetFolderId)?.name ?? 'Current Folder';

    const handleExtract = async () => {
        if (extracting || uploading || parentDisabled) return;
        setExtracting(true);
        setProgress(null);
        setFolderMenuOpen(false);

        const transferId = `arch-extract-${Date.now()}-${entryIndex}`;
        let extracted: ExtractedFile | null = null;

        // Subscribe to upload progress BEFORE starting the upload
        const unlisten = await listen<UploadProgress>('upload-progress', (event) => {
            if (event.payload.id === transferId) {
                setProgress({
                    id: event.payload.id,
                    percent: event.payload.percent,
                    uploaded_bytes: event.payload.uploaded_bytes,
                    total_bytes: event.payload.total_bytes,
                    speed_bytes_per_sec: event.payload.speed_bytes_per_sec,
                });
            }
        });
        unlistenRef.current = unlisten;

        try {
            // Step 1: Extract
            extracted = await invoke<ExtractedFile>('cmd_extract_archive_entry', {
                messageId: file.id,
                folderId: activeFolderId ?? null,
                entryIndex,
            });

            setExtracting(false);
            setUploading(true);

            // Step 2: Upload to the selected target folder
            await invoke('initiate_upload', {
                path: extracted.temp_path,
                folderId: targetFolderId,
                transferId,
            });

            const destName = targetFolderId === null ? t('common.saved_messages') : targetFolderName;
            toast.success(`"${entryName}" extracted to ${destName}`);

            // Refresh the file list for the target folder (debounced)
            onCacheInvalidate(targetFolderId);
        } catch (e) {
            const msg = String(e);
            if (msg.includes('cancelled') || msg.includes('canceled')) {
                toast.info('Extract cancelled');
            } else {
                toast.error(`Failed: ${msg}`);
            }
        } finally {
            unlistenRef.current?.();
            unlistenRef.current = null;
            setExtracting(false);
            setUploading(false);
            setProgress(null);
            // Always clean up the temp file
            if (extracted?.temp_path) {
                try {
                    await invoke('cmd_delete_temp_zip', { path: extracted.temp_path });
                } catch {
                    // best-effort cleanup; OS will reclaim temp eventually
                }
            }
        }
    };

    const isBusy = extracting || uploading;
    const label = extracting ? t('archive.extracting') : 'Uploading...';
    const progressPct = progress ? Math.min(progress.percent, 99) : 0;
    const showProgress = uploading && progress;

    return (
        <div className="me-2 flex shrink-0 flex-col items-end gap-0.5" ref={menuRef}>
            <div className="flex items-center gap-0.5">
                <button
                    onClick={handleExtract}
                    disabled={isBusy}
                    className="quiet-control flex h-7 items-center gap-1 rounded-e-none bg-app-selected px-2 text-badge font-medium text-app-accent opacity-0 group-focus-within:opacity-100 group-hover:opacity-100 disabled:opacity-50"
                    title={`Extract "${entryName}" (${formatBytes(entrySize)}) to ${targetFolderName}`}
                >
                    {isBusy ? (
                        <Loader2 className="w-3 h-3 animate-spin" />
                    ) : (
                        <Download className="w-3 h-3" />
                    )}
                    <span>{isBusy ? label : t('archive.extract')}</span>
                </button>

                {/* Folder selector trigger */}
                <button
                    onClick={(e) => { e.stopPropagation(); setFolderMenuOpen(o => !o); }}
                    disabled={isBusy}
                    className="quiet-control flex h-7 items-center rounded-s-none border-s border-app-accent/20 bg-app-selected px-1.5 text-app-accent opacity-0 group-focus-within:opacity-100 group-hover:opacity-100 disabled:opacity-50"
                    title={`Target: ${targetFolderName}`}
                >
                    <ChevronDown className={`w-2.5 h-2.5 transition-transform ${folderMenuOpen ? 'rotate-180' : ''}`} />
                </button>
            </div>

            {/* Folder dropdown */}
            {folderMenuOpen && (
                <div
                    className="quiet-menu absolute end-2 z-[220] mt-8 w-48 overflow-hidden p-1 animate-in fade-in slide-in-from-top-1 duration-100"
                    onClick={e => e.stopPropagation()}
                >
                    <div className="border-b border-app-border-subtle px-2 py-1.5">
                        <p className="text-badge font-medium text-app-text-tertiary">{t('archive.extract_to')}</p>
                    </div>
                    <div className="max-h-48 overflow-y-auto py-1">
                        {/* Saved Messages */}
                        <button
                            onClick={() => { setTargetFolderId(null); setFolderMenuOpen(false); }}
                            className={`quiet-menu-item gap-2 text-start ${targetFolderId === null ? 'bg-app-selected text-app-accent' : 'text-app-text'}`}
                        >
                            <HardDrive className="w-3.5 h-3.5 shrink-0" />
                            <span className="truncate">{t('common.saved_messages')}</span>
                            {targetFolderId === null && (
                                <span className="ms-auto h-1.5 w-1.5 shrink-0 rounded-full bg-app-accent" />
                            )}
                        </button>
                        {/* Folders */}
                        {folders.map(f => (
                            <button
                                key={f.id}
                                onClick={() => { setTargetFolderId(f.id); setFolderMenuOpen(false); }}
                                className={`quiet-menu-item gap-2 text-start ${targetFolderId === f.id ? 'bg-app-selected text-app-accent' : 'text-app-text'}`}
                            >
                                <Folder className="w-3.5 h-3.5 shrink-0" />
                                <span className="truncate">{f.name}</span>
                                {targetFolderId === f.id && (
                                    <span className="ms-auto h-1.5 w-1.5 shrink-0 rounded-full bg-app-accent" />
                                )}
                            </button>
                        ))}
                    </div>
                </div>
            )}

            {/* Inline progress bar — always visible during active upload */}
            {showProgress && (
                <div className={`w-full flex items-center gap-1.5 transition-all ${uploading ? 'opacity-100' : 'opacity-0 group-hover:opacity-100'}`}>
                    <div className="h-1 flex-1 overflow-hidden rounded-full bg-app-border">
                        <div
                            className="h-full rounded-full bg-app-accent transition-all duration-300 ease-out"
                            style={{ width: `${progressPct}%` }}
                        />
                    </div>
                    <span className="shrink-0 font-mono text-badge tabular-nums text-app-text-tertiary">
                        {progressPct}%
                    </span>
                </div>
            )}
        </div>
    );
}
