import { useModalFocus } from '../../../hooks/useModalFocus';
import {useTranslation} from 'react-i18next';
import { useEffect, useState, useRef } from 'react';
import { X, ChevronLeft, ChevronRight, ZoomIn, ZoomOut, Maximize } from 'lucide-react';
import { invoke, convertFileSrc } from '@tauri-apps/api/core';
// Use the legacy build — the modern build uses Map.getOrInsertComputed()
// which isn't available in Tauri's WebKit WebView
import * as pdfjsLib from 'pdfjs-dist/legacy/build/pdf.mjs';
import { TelegramFile } from '../../../types';
import { isAndroidPlatform } from '../../../utils';

// Use Vite's ?url suffix to get a properly bundled asset URL for the worker.
// The asset is copied as-is, so ship the upstream minified worker.
import workerUrl from 'pdfjs-dist/legacy/build/pdf.worker.min.mjs?url';
import i18n from '../../../i18n';
import { toast } from 'sonner';
pdfjsLib.GlobalWorkerOptions.workerSrc = workerUrl;

const PDF_CACHE_LIMIT_BYTES = 25 * 1024 * 1024;
const PAGE_PREFETCH_ROOT_MARGIN = '1000px 0px';

interface StreamInfo {
    token: string;
    base_url: string;
    operation_token?: string | null;
}

interface PdfViewerProps {
    file: TelegramFile;
    onClose: () => void;
    onNext?: () => void;
    onPrev?: () => void;
    currentIndex?: number;
    totalItems?: number;
    activeFolderId: number | null;
    localPath?: string;
}

export function PdfViewer({ file, onClose, onNext, onPrev, currentIndex, totalItems, activeFolderId, localPath }: PdfViewerProps) {
    useTranslation();
    const viewerRef = useRef<HTMLDivElement>(null);
    useModalFocus(viewerRef, onClose);
    const [pdf, setPdf] = useState<pdfjsLib.PDFDocumentProxy | null>(null);
    const [numPages, setNumPages] = useState<number>(0);
    const [scale, setScale] = useState<number>(1.2);
    const [loading, setLoading] = useState<boolean>(true);
    const [error, setError] = useState<string | null>(null);
    const [openingExternal, setOpeningExternal] = useState<boolean>(false);
    const containerRef = useRef<HTMLDivElement>(null);
    const pdfRef = useRef<pdfjsLib.PDFDocumentProxy | null>(null);

    const handleOpenExternally = async (e: React.MouseEvent) => {
        e.stopPropagation();
        setOpeningExternal(true);
        try {
            const path = localPath ?? await invoke<string>('cmd_get_preview', {
                messageId: file.id,
                folderId: activeFolderId
            });
            if (path) {
                await invoke('cmd_open_file_externally', { path });
            } else {
                toast.error(i18n.t('common.operation_failed'));
            }
        } catch (err) {
            console.error("Failed to open externally:", err);
            toast.error(i18n.t('common.operation_failed'));
        } finally {
            setOpeningExternal(false);
        }
    };

    useEffect(() => {
        let cancelled = false;
        let activeLoadingTask: pdfjsLib.PDFDocumentLoadingTask | null = null;
        setLoading(true);
        setError(null);
        setPdf(null);
        setNumPages(0);

        const finishPdfLoad = (pdfDoc: pdfjsLib.PDFDocumentProxy) => {
            if (cancelled) {
                void pdfDoc.loadingTask.destroy();
                return;
            }
            if (pdfRef.current) void pdfRef.current.loadingTask.destroy();
            pdfRef.current = pdfDoc;
            setPdf(pdfDoc);
            setNumPages(pdfDoc.numPages);
            setLoading(false);
        };

        const loadStream = async () => {
            if (cancelled) return;
            let streamInfo: StreamInfo;
            try {
                streamInfo = await invoke<StreamInfo>('cmd_get_stream_info');
                if (!streamInfo?.base_url || !streamInfo.token) throw new Error('Missing stream information');
            } catch {
                if (!cancelled) {
                    setError(i18n.t('viewer.stream_failed'));
                    setLoading(false);
                }
                return;
            }
            if (cancelled) return;
            const folderIdParam = activeFolderId !== null ? activeFolderId.toString() : 'home';
            const credential = streamInfo.operation_token
                ? `&credential=${encodeURIComponent(streamInfo.operation_token)}`
                : '';
            const streamUrl = `${streamInfo.base_url}/stream/${folderIdParam}/${file.id}?token=${encodeURIComponent(streamInfo.token)}${credential}`;
            activeLoadingTask = pdfjsLib.getDocument({ url: streamUrl });
            activeLoadingTask.promise.then(finishPdfLoad, (err) => {
                if (cancelled) return;
                console.error("Error loading PDF stream:", err);
                setError(i18n.t('viewer.pdf_document_failed'));
                setLoading(false);
            });
        };

        const loadLocalFile = (filePath: string) => {
            activeLoadingTask = pdfjsLib.getDocument({
                url: convertFileSrc(filePath),
                disableRange: true,
                disableStream: true,
                disableAutoFetch: true,
            });
            activeLoadingTask.promise.then(finishPdfLoad, (err) => {
                if (cancelled) return;
                if (localPath) {
                    setError(i18n.t('workspace.preview_failed'));
                    setLoading(false);
                    return;
                }
                if (!isAndroidPlatform) {
                    console.warn("Cached PDF could not be rendered; falling back to streaming:", err);
                    loadStream();
                    return;
                }
                console.error("Error loading PDF via cache URL, falling back to external opener:", err);
                invoke('cmd_open_file_externally', { path: filePath })
                    .then(() => {
                        if (!cancelled) onClose();
                    })
                    .catch((externalError) => {
                        if (!cancelled) {
                            setError(i18n.t('viewer.pdf_native_failed', { error: String(externalError) }));
                            setLoading(false);
                        }
                    });
            });
        };

        if (localPath) {
            loadLocalFile(localPath);
            return () => { cancelled = true; void activeLoadingTask?.destroy(); };
        }

        // Small PDFs are loaded from the bounded plaintext preview cache so a
        // recently viewed document remains available offline. Large desktop
        // PDFs retain range-based streaming to avoid filling the cache.
        const shouldCache = isAndroidPlatform || file.size <= PDF_CACHE_LIMIT_BYTES;
        if (shouldCache) {
            invoke<string>('cmd_get_preview', {
                messageId: file.id,
                folderId: activeFolderId,
            }).then((filePath) => {
                if (cancelled) return;
                if (filePath) loadLocalFile(filePath);
                else if (!isAndroidPlatform) loadStream();
                else {
                    setError(i18n.t('viewer.pdf_path_failed'));
                    setLoading(false);
                }
            }).catch((err) => {
                if (cancelled) return;
                if (!isAndroidPlatform) {
                    console.warn("Could not cache PDF; falling back to streaming:", err);
                    loadStream();
                } else {
                    console.error("Error invoking PDF preview command:", err);
                    setError(i18n.t('viewer.pdf_load_failed'));
                    setLoading(false);
                }
            });
        } else {
            loadStream();
        }

        return () => {
            cancelled = true;
            void activeLoadingTask?.destroy();
        };
    }, [activeFolderId, file.id, file.size, localPath]);

    useEffect(() => {
        return () => {
            if (pdfRef.current) {
                void pdfRef.current.loadingTask.destroy();
                pdfRef.current = null;
            }
        };
    }, []);

    // Keyboard shortcuts
    useEffect(() => {
        const handleKeyDown = (e: KeyboardEvent) => {
            const target = e.target as HTMLElement;
            if (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable) {
                return;
            }

            const key = e.key.toLowerCase();

            if (e.key === 'ArrowRight' || key === 'l') {
                e.preventDefault();
                onNext?.();
                return;
            }

            if (e.key === 'ArrowLeft' || key === 'j') {
                e.preventDefault();
                onPrev?.();
                return;
            }

            if (e.key === 'Escape') {
                e.preventDefault();
                onClose();
                return;
            }

            if (e.key === '=' || key === '+') {
                e.preventDefault();
                setScale(s => Math.min(s + 0.2, 3));
            }

            if (e.key === '-') {
                e.preventDefault();
                setScale(s => Math.max(s - 0.2, 0.5));
            }
        };

        window.addEventListener('keydown', handleKeyDown);
        return () => window.removeEventListener('keydown', handleKeyDown);
    }, [onClose, onNext, onPrev]);

    const handleZoomIn = (e: React.MouseEvent) => {
        e.stopPropagation();
        setScale(s => Math.min(s + 0.2, 3));
    };

    const handleZoomOut = (e: React.MouseEvent) => {
        e.stopPropagation();
        setScale(s => Math.max(s - 0.2, 0.5));
    };

    const handleFitWidth = (e: React.MouseEvent) => {
        e.stopPropagation();
        setScale(1.2);
    };

    return (
        <div ref={viewerRef} role="dialog" aria-modal="true" aria-label={file.name} className="viewer-overlay fixed inset-0 z-[200] flex flex-col p-4 animate-in fade-in duration-150" onClick={onClose}>
            {/* Header / Controls */}
            <div className="pointer-events-none absolute inset-x-0 top-0 z-50 flex items-center justify-between px-4 pt-[calc(1rem+env(safe-area-inset-top))] md:px-6">
                <div className="viewer-toolbar pointer-events-auto px-2">
                    <h3 className="max-w-[120px] truncate px-1 text-metadata font-medium text-white sm:max-w-xs" title={file.name}>{file.name}</h3>
                    <button
                        onClick={handleOpenExternally}
                        disabled={openingExternal}
                        className="quiet-control h-7 rounded-control bg-app-accent px-2 text-badge font-semibold text-app-accent-contrast disabled:opacity-50"
                        title={i18n.t('viewer.open_native')}
                    >
                        {openingExternal ? i18n.t('viewer.opening') : i18n.t('viewer.open_native')}
                    </button>
                </div>

                <div className="pointer-events-auto flex items-center gap-2">
                    <div className="viewer-toolbar">
                        <button onClick={handleZoomOut} className="viewer-control" title={i18n.t('common.zoom_out_shortcut')} aria-label={i18n.t("common.zoom_out")}>
                            <ZoomOut className="w-4 h-4" />
                        </button>
                        <span className="min-w-[3rem] text-center text-badge font-medium tabular-nums text-white/85">{Math.round(scale * 100)}%</span>
                        <button onClick={handleZoomIn} className="viewer-control" title={i18n.t('common.zoom_in_shortcut')} aria-label={i18n.t("common.zoom_in")}>
                            <ZoomIn className="w-4 h-4" />
                        </button>
                        <div className="mx-0.5 h-4 w-px bg-white/15"></div>
                        <button onClick={handleFitWidth} className="viewer-control" title={i18n.t('viewer.fit_width')} aria-label={i18n.t('viewer.fit_width')}>
                            <Maximize className="w-4 h-4" />
                        </button>
                    </div>

                    <button
                        onClick={onClose}
                        className="viewer-navigation"
                        title={i18n.t('viewer.close_pdf')}
                        aria-label={i18n.t('viewer.close_pdf')}
                    >
                        <X className="w-5 h-5" />
                    </button>
                </div>
            </div>

            {/* Navigation Buttons */}
            <button
                onClick={(e) => { e.stopPropagation(); onPrev?.(); }}
                className="viewer-navigation absolute start-4 top-1/2 z-10 -translate-y-1/2"
                title={i18n.t('viewer.previous_shortcut')}
                aria-label={i18n.t('viewer.previous_file')}
            >
                <ChevronLeft className="h-5 w-5 rtl:rotate-180" />
            </button>

            <button
                onClick={(e) => { e.stopPropagation(); onNext?.(); }}
                className="viewer-navigation absolute end-4 top-1/2 z-10 -translate-y-1/2"
                title={i18n.t('viewer.next_shortcut')}
                aria-label={i18n.t('viewer.next_file')}
            >
                <ChevronRight className="h-5 w-5 rtl:rotate-180" />
            </button>

            {/* Scrollable Document Container */}
            <div
                ref={containerRef}
                className="flex-1 w-full overflow-auto custom-scrollbar flex flex-col items-center pt-[calc(5rem+env(safe-area-inset-top))] pb-24 relative"
                onClick={(e) => e.stopPropagation()}
            >
                {loading && (
                    <div role="status" className="absolute inset-0 flex flex-1 flex-col items-center justify-center text-white">
                        <div className="mb-3 h-6 w-6 animate-spin rounded-full border-2 border-white/20 border-t-app-accent"></div>
                        <p className="text-ui font-medium">{i18n.t('viewer.loading_document')}</p>
                        <p className="mt-1 text-badge text-white/45">{i18n.t('viewer.downloading_telegram')}</p>
                    </div>
                )}

                {error && (
                    <div role="alert" className="viewer-panel mt-20 flex max-w-md flex-col items-center justify-center border-app-danger/30 bg-app-danger/10 p-5 text-center text-white">
                        <p className="mb-1 text-ui font-semibold text-app-danger">{i18n.t('viewer.unable_pdf')}</p>
                        <p className="mb-5 text-metadata leading-relaxed text-white/60">{error}</p>
                        <button
                            onClick={handleOpenExternally}
                            disabled={openingExternal}
                            className="quiet-control pointer-events-auto h-9 rounded-control bg-app-accent px-4 text-ui font-semibold text-app-accent-contrast disabled:opacity-50"
                        >
                            {openingExternal ? i18n.t('viewer.opening') : i18n.t('viewer.open_external')}
                        </button>
                    </div>
                )}

                {pdf && numPages > 0 && (
                    <div className="flex flex-col gap-4 w-full items-center">
                        {Array.from({ length: numPages }, (_, index) => (
                            <PdfPage
                                key={`${file.id}_page_${index + 1}`}
                                pageNumber={index + 1}
                                pdf={pdf}
                                scale={scale}
                            />
                        ))}
                    </div>
                )}
            </div>

            {/* Footer Navigation Info */}
            <div className="viewer-toolbar pointer-events-none absolute bottom-4 start-1/2 -translate-x-1/2 px-3 py-1.5 text-metadata text-white/50 rtl:translate-x-1/2">
                {typeof currentIndex === 'number' && typeof totalItems === 'number' && totalItems > 0 && (
                    <span className="me-3 border-e border-white/20 pe-3">{i18n.t("settings.video_upload_file")} {currentIndex + 1} of {totalItems}</span>
                )}
                <span>{numPages} {numPages === 1 ? 'page' : 'pages'}</span>
            </div>
        </div>
    );
}

// Individual Page Component — lazy-loaded via IntersectionObserver
function PdfPage({ pageNumber, pdf, scale }: { pageNumber: number; pdf: pdfjsLib.PDFDocumentProxy; scale: number }) {
    const canvasRef = useRef<HTMLCanvasElement>(null);
    const renderTaskRef = useRef<ReturnType<pdfjsLib.PDFPageProxy['render']> | null>(null);
    const [isVisible, setIsVisible] = useState(false);
    const containerRef = useRef<HTMLDivElement>(null);
    const [page, setPage] = useState<pdfjsLib.PDFPageProxy | null>(null);

    useEffect(() => {
        const el = containerRef.current;
        if (!el) return;

        const observer = new IntersectionObserver(
            (entries) => {
                setIsVisible(entries[0].isIntersecting);
            },
            { rootMargin: PAGE_PREFETCH_ROOT_MARGIN }
        );

        observer.observe(el);
        return () => observer.disconnect();
    }, []);

    // Clear/unload canvas and page structure when not visible to release memory
    useEffect(() => {
        if (!isVisible) {
            setPage(null);
            if (canvasRef.current) {
                const canvas = canvasRef.current;
                canvas.width = 0;
                canvas.height = 0;
            }
        }
    }, [isVisible]);

    // Fetch the PDF page object when visible
    useEffect(() => {
        if (!isVisible || !pdf) return;

        let cancelled = false;
        pdf.getPage(pageNumber).then(loadedPage => {
            if (!cancelled) {
                setPage(loadedPage);
            }
        }).catch(err => console.error(`Error loading page ${pageNumber}:`, err));

        return () => {
            cancelled = true;
        };
    }, [isVisible, pdf, pageNumber]);

    const [text, setText] = useState('');
    useEffect(() => {
        let current = true;
        setText('');
        if (page) {
            void page.getTextContent().then(content => {
                if (current) setText(content.items.map(item => 'str' in item ? item.str : '').join(' '));
            }).catch(() => { /* A scanned or unreadable text layer still has a visual preview. */ });
        }
        return () => { current = false; };
    }, [page]);

    // Render the page to canvas — re-runs when page loads or scale changes
    useEffect(() => {
        if (!page || !canvasRef.current || !isVisible) return;

        const viewport = page.getViewport({ scale });
        const canvas = canvasRef.current;
        const context = canvas.getContext('2d');

        if (!context) return;

        // Cancel any in-flight render before starting a new one
        if (renderTaskRef.current) {
            renderTaskRef.current.cancel();
            renderTaskRef.current = null;
        }

        // Size canvas and clear before render to avoid stale frame flash
        canvas.height = viewport.height;
        canvas.width = viewport.width;
        context.clearRect(0, 0, viewport.width, viewport.height);

        const renderTask = page.render({
            canvasContext: context,
            viewport: viewport,
            canvas: canvas,
        });
        renderTaskRef.current = renderTask;

        renderTask.promise.catch((err) => {
            // RenderingCancelledException is expected during zoom — ignore it
            if (err?.name !== 'RenderingCancelledException') {
                console.error(`Render error on page ${pageNumber}:`, err);
            }
        });

        return () => {
            renderTask.cancel();
            renderTaskRef.current = null;
        };
    }, [page, scale, isVisible, pageNumber]);

    // Estimated dimensions for the placeholder before page loads (US Letter @ 96 DPI)
    const estimatedHeight = 1056 * scale;
    const estimatedWidth = 816 * scale;

    return (
        <div
            ref={containerRef}
            role="region"
            aria-label={i18n.t('syncPreview.page', { current: pageNumber, total: pdf.numPages })}
            tabIndex={0}
            className="relative my-2 flex flex-col items-center overflow-hidden rounded-control bg-white/5 shadow-[0_8px_28px_rgba(0,0,0,0.32)]"
            style={{
                minHeight: !page ? `${estimatedHeight}px` : undefined,
                minWidth: !page ? `${estimatedWidth}px` : undefined,
            }}
        >
            <span className="sr-only">{text}</span>
            <canvas aria-hidden="true" ref={canvasRef} className="max-w-full h-auto bg-white" />

            {!page && isVisible && (
                <div className="absolute inset-0 flex items-center justify-center pointer-events-none text-white/30">
                    <div className="w-8 h-8 border-2 border-white/20 border-t-white/60 rounded-full animate-spin"></div>
                </div>
            )}
        </div>
    );
}
