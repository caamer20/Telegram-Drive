import { useModalFocus } from '../../../hooks/useModalFocus';
import { useCallback, useEffect, useRef, useState } from 'react';
import { ChevronLeft, ChevronRight, File, Maximize, Scan, X, ZoomIn, ZoomOut } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { listen } from '@tauri-apps/api/event';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { TelegramFile } from '../../../types';
import { isImageFile } from '../../../utils';
import { isAndroidPlatform } from '../../../utils/platform';
import { useSettings } from '../../../context/SettingsContext';
import { userFacingError } from '../../../services/userFacingError';
import {
    forgetPreview,
    forgetThumbnail,
    getCachedPreview,
    getCachedThumbnail,
    loadPreview,
    loadLocalPreview,
    loadThumbnail,
} from '../../../services/imagePreviewCache';
import i18n from '../../../i18n';

const MAX_PREFETCH_BYTES = 25 * 1024 * 1024;
const MIN_IMAGE_ZOOM = 0.25;
const MAX_IMAGE_ZOOM = 16;
const IMAGE_ZOOM_STEP = 1.25;

type Point = { x: number; y: number };
type ImageTransform = { zoom: number; pan: Point };

function distance(a: Point, b: Point): number {
    return Math.hypot(b.x - a.x, b.y - a.y);
}

function midpoint(a: Point, b: Point): Point {
    return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
}

type PreviewProgress = {
    message_id: number;
    folder_id: number | null;
    downloaded_bytes: number;
    total_bytes: number;
    percent: number;
};

interface PreviewModalProps {
    file: TelegramFile;
    onClose: () => void;
    onNext?: () => void;
    onPrev?: () => void;
    currentIndex?: number;
    totalItems?: number;
    nextFile?: TelegramFile | null;
    prevFile?: TelegramFile | null;
    activeFolderId: number | null;
    localPath?: string;
    onDownload?: () => void;
}

export function PreviewModal({
    file,
    onClose,
    onNext,
    onPrev,
    currentIndex,
    totalItems,
    nextFile,
    activeFolderId,
    localPath,
    onDownload,
}: PreviewModalProps) {
    const viewerRef = useRef<HTMLDivElement>(null);
    useModalFocus(viewerRef, onClose);
    const { t } = useTranslation();
    const { settings } = useSettings();
    const [thumbnailSrc, setThumbnailSrc] = useState<string | null>(null);
    const [fullSrc, setFullSrc] = useState<string | null>(null);
    const [fullReady, setFullReady] = useState(false);
    const [loading, setLoading] = useState(true);
    const [progress, setProgress] = useState(0);
    const [error, setError] = useState<string | null>(null);
    const latestRequestRef = useRef(0);
    const currentFileIdRef = useRef(file.id);
    currentFileIdRef.current = file.id;
    const imagePreview = isImageFile(file.name);
    const heicPreview = !isAndroidPlatform && /\.hei[cf]$/i.test(file.name);
    const imageViewportRef = useRef<HTMLDivElement>(null);
    const fullImageRef = useRef<HTMLImageElement>(null);
    const [imageTransform, setImageTransform] = useState<ImageTransform>({ zoom: 1, pan: { x: 0, y: 0 } });
    const imageTransformRef = useRef(imageTransform);
    const [imageInteracting, setImageInteracting] = useState(false);
    const activePointersRef = useRef(new Map<number, Point>());
    const pointerGestureRef = useRef<
        | { type: 'pan'; start: Point; startPan: Point }
        | { type: 'pinch'; startDistance: number; startZoom: number; startCenter: Point; startPan: Point }
        | null
    >(null);
    const pointerStartRef = useRef<{ point: Point; time: number } | null>(null);
    const gestureHadMultiplePointersRef = useRef(false);
    const lastTouchTapRef = useRef<{ point: Point; time: number } | null>(null);

    const commitImageTransform = useCallback((next: ImageTransform) => {
        imageTransformRef.current = next;
        setImageTransform(next);
    }, []);

    const clampPan = useCallback((pan: Point, zoom: number): Point => {
        const viewport = imageViewportRef.current;
        const image = fullImageRef.current;
        if (!viewport || !image) return zoom <= 1 ? { x: 0, y: 0 } : pan;

        const maxX = Math.max(0, (image.clientWidth * zoom - viewport.clientWidth) / 2);
        const maxY = Math.max(0, (image.clientHeight * zoom - viewport.clientHeight) / 2);
        return {
            x: Math.max(-maxX, Math.min(maxX, pan.x)),
            y: Math.max(-maxY, Math.min(maxY, pan.y)),
        };
    }, []);

    const zoomImageTo = useCallback((requestedZoom: number, focalPoint?: Point) => {
        const viewport = imageViewportRef.current;
        const current = imageTransformRef.current;
        const zoom = Math.max(MIN_IMAGE_ZOOM, Math.min(MAX_IMAGE_ZOOM, requestedZoom));
        let pan = current.pan;

        if (viewport && focalPoint && current.zoom > 0) {
            const bounds = viewport.getBoundingClientRect();
            const focalOffset = {
                x: focalPoint.x - (bounds.left + bounds.width / 2),
                y: focalPoint.y - (bounds.top + bounds.height / 2),
            };
            const ratio = zoom / current.zoom;
            pan = {
                x: focalOffset.x - (focalOffset.x - current.pan.x) * ratio,
                y: focalOffset.y - (focalOffset.y - current.pan.y) * ratio,
            };
        }

        commitImageTransform({ zoom, pan: clampPan(pan, zoom) });
    }, [clampPan, commitImageTransform]);

    const fitImage = useCallback(() => {
        commitImageTransform({ zoom: 1, pan: { x: 0, y: 0 } });
    }, [commitImageTransform]);

    const showActualImageSize = useCallback(() => {
        const image = fullImageRef.current;
        if (!image || image.clientWidth <= 0 || image.clientHeight <= 0) return;
        const actualSizeZoom = Math.max(
            image.naturalWidth / image.clientWidth,
            image.naturalHeight / image.clientHeight,
            1,
        );
        zoomImageTo(actualSizeZoom);
    }, [zoomImageTo]);

    const panImageBy = useCallback((x: number, y: number) => {
        const current = imageTransformRef.current;
        const pan = clampPan({ x: current.pan.x + x, y: current.pan.y + y }, current.zoom);
        commitImageTransform({ ...current, pan });
    }, [clampPan, commitImageTransform]);

    const toggleImageZoom = useCallback((point?: Point) => {
        if (imageTransformRef.current.zoom > 1.05) fitImage();
        else zoomImageTo(2, point);
    }, [fitImage, zoomImageTo]);

    useEffect(() => {
        if (localPath) return;
        let disposed = false;
        let unlisten: (() => void) | undefined;

        listen<PreviewProgress>('preview-progress', ({ payload }) => {
            if (
                payload.message_id === file.id
                && (payload.folder_id ?? null) === activeFolderId
            ) {
                setProgress(payload.percent);
            }
        }).then((stopListening) => {
            if (disposed) stopListening();
            else unlisten = stopListening;
        }).catch(() => {
            // Progress is an enhancement; preview loading remains fully functional without it.
        });

        return () => {
            disposed = true;
            unlisten?.();
        };
    }, [file.id, activeFolderId, localPath]);

    useEffect(() => {
        const requestId = ++latestRequestRef.current;
        if (localPath && !heicPreview) {
            setThumbnailSrc(null); setFullSrc(convertFileSrc(localPath));
            setFullReady(false); setLoading(imagePreview); setProgress(100); setError(null);
            fitImage(); activePointersRef.current.clear(); pointerGestureRef.current = null; setImageInteracting(false);
            return;
        }
        const cachedPreview = localPath ? null : getCachedPreview(file.id, activeFolderId);
        const cachedThumbnail = imagePreview
            ? getCachedThumbnail(file.id, activeFolderId)
            : null;

        setThumbnailSrc(cachedThumbnail);
        setFullSrc(cachedPreview);
        setFullReady(false);
        setLoading(true);
        setProgress(cachedPreview ? 100 : 0);
        setError(null);
        fitImage();
        activePointersRef.current.clear();
        pointerGestureRef.current = null;
        setImageInteracting(false);

        if (imagePreview && !cachedThumbnail) {
            loadThumbnail(file.id, activeFolderId).then((src) => {
                if (requestId === latestRequestRef.current && src) {
                    setThumbnailSrc(src);
                }
            }).catch(() => {
                // The full-resolution preview can still load without a thumbnail.
            });
        }

        const preview = localPath
            ? loadLocalPreview(file.id, activeFolderId, localPath)
            : loadPreview(file.id, activeFolderId);
        preview.then((src) => {
            if (requestId !== latestRequestRef.current) return;
            if (!src) {
                setError(t('workspace.preview_failed'));
                setLoading(false);
                return;
            }
            setFullSrc(src);
            if (!imagePreview) setLoading(false);
        }).catch((loadError) => {
            if (requestId !== latestRequestRef.current) return;
            setThumbnailSrc(null);
            setError(heicPreview ? t('workspace.preview_failed') : userFacingError(loadError, t));
            setLoading(false);
        });
    }, [file.id, file.name, activeFolderId, imagePreview, heicPreview, fitImage, localPath]);

    // Prefetch only the likely next image, after the current one is fully decoded and
    // the browser is idle. Avoid speculative downloads when a bandwidth cap is active.
    useEffect(() => {
        if (localPath) return;
        if (!fullReady || !nextFile || !isImageFile(nextFile.name)) return;
        if (nextFile.size > MAX_PREFETCH_BYTES) return;
        if (settings.bandwidthLimitDownKBs > 0 || settings.bandwidth_schedule) return;
        const connection = (navigator as Navigator & { connection?: { saveData?: boolean } }).connection;
        if (connection?.saveData) return;

        const idleWindow = window as Window & {
            requestIdleCallback?: (callback: () => void, options?: { timeout: number }) => number;
            cancelIdleCallback?: (id: number) => void;
        };
        let idleId: number | undefined;
        const timerId = window.setTimeout(() => {
            if (getCachedPreview(nextFile.id, activeFolderId)) return;
            if (idleWindow.requestIdleCallback) {
                idleId = idleWindow.requestIdleCallback(
                    () => { void loadPreview(nextFile.id, activeFolderId).catch(() => {}); },
                    { timeout: 1500 },
                );
            } else {
                void loadPreview(nextFile.id, activeFolderId).catch(() => {});
            }
        }, 500);

        return () => {
            window.clearTimeout(timerId);
            if (idleId !== undefined) idleWindow.cancelIdleCallback?.(idleId);
        };
    }, [fullReady, nextFile, activeFolderId, settings.bandwidth_schedule, settings.bandwidthLimitDownKBs, localPath]);

    useEffect(() => {
        if (!fullReady) return;
        const viewport = imageViewportRef.current;
        if (!viewport) return;

        const keepTransformInBounds = () => {
            const current = imageTransformRef.current;
            commitImageTransform({ ...current, pan: clampPan(current.pan, current.zoom) });
        };
        const observer = typeof ResizeObserver !== 'undefined'
            ? new ResizeObserver(keepTransformInBounds)
            : null;
        observer?.observe(viewport);
        window.addEventListener('resize', keepTransformInBounds);
        return () => {
            observer?.disconnect();
            window.removeEventListener('resize', keepTransformInBounds);
        };
    }, [fullReady, clampPan, commitImageTransform]);

    const handleImageWheel = useCallback((event: React.WheelEvent<HTMLDivElement>) => {
        if (!fullReady) return;
        event.preventDefault();
        const factor = Math.exp(-event.deltaY * 0.002);
        zoomImageTo(imageTransformRef.current.zoom * factor, { x: event.clientX, y: event.clientY });
    }, [fullReady, zoomImageTo]);

    const handleImagePointerDown = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
        if (!fullReady || event.button !== 0) return;
        event.preventDefault();
        const point = { x: event.clientX, y: event.clientY };
        activePointersRef.current.set(event.pointerId, point);
        try { event.currentTarget.setPointerCapture(event.pointerId); } catch { /* unsupported WebView */ }

        if (activePointersRef.current.size === 1) {
            pointerStartRef.current = { point, time: Date.now() };
            gestureHadMultiplePointersRef.current = false;
            pointerGestureRef.current = {
                type: 'pan',
                start: point,
                startPan: imageTransformRef.current.pan,
            };
        } else {
            gestureHadMultiplePointersRef.current = true;
            const [first, second] = [...activePointersRef.current.values()];
            pointerGestureRef.current = {
                type: 'pinch',
                startDistance: Math.max(1, distance(first, second)),
                startZoom: imageTransformRef.current.zoom,
                startCenter: midpoint(first, second),
                startPan: imageTransformRef.current.pan,
            };
        }
        setImageInteracting(true);
    }, [fullReady]);

    const handleImagePointerMove = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
        if (!activePointersRef.current.has(event.pointerId)) return;
        event.preventDefault();
        activePointersRef.current.set(event.pointerId, { x: event.clientX, y: event.clientY });
        const gesture = pointerGestureRef.current;
        if (!gesture) return;

        if (activePointersRef.current.size >= 2 && gesture.type === 'pinch') {
            const [first, second] = [...activePointersRef.current.values()];
            const center = midpoint(first, second);
            const zoom = Math.max(
                MIN_IMAGE_ZOOM,
                Math.min(MAX_IMAGE_ZOOM, gesture.startZoom * distance(first, second) / gesture.startDistance),
            );
            const viewport = imageViewportRef.current;
            let pan = gesture.startPan;
            if (viewport) {
                const bounds = viewport.getBoundingClientRect();
                const startFocal = {
                    x: gesture.startCenter.x - (bounds.left + bounds.width / 2),
                    y: gesture.startCenter.y - (bounds.top + bounds.height / 2),
                };
                const ratio = zoom / gesture.startZoom;
                pan = {
                    x: startFocal.x - (startFocal.x - gesture.startPan.x) * ratio + center.x - gesture.startCenter.x,
                    y: startFocal.y - (startFocal.y - gesture.startPan.y) * ratio + center.y - gesture.startCenter.y,
                };
            }
            commitImageTransform({ zoom, pan: clampPan(pan, zoom) });
        } else if (activePointersRef.current.size === 1 && gesture.type === 'pan') {
            const current = [...activePointersRef.current.values()][0];
            const pan = {
                x: gesture.startPan.x + current.x - gesture.start.x,
                y: gesture.startPan.y + current.y - gesture.start.y,
            };
            const zoom = imageTransformRef.current.zoom;
            commitImageTransform({ zoom, pan: clampPan(pan, zoom) });
        }
    }, [clampPan, commitImageTransform]);

    const finishImagePointer = useCallback((event: React.PointerEvent<HTMLDivElement>, allowTap: boolean) => {
        const endPoint = { x: event.clientX, y: event.clientY };
        const pointerStart = pointerStartRef.current;
        const wasSingleTouch = event.pointerType === 'touch' && !gestureHadMultiplePointersRef.current;
        try { event.currentTarget.releasePointerCapture(event.pointerId); } catch { /* unsupported WebView */ }
        activePointersRef.current.delete(event.pointerId);

        if (allowTap && wasSingleTouch && pointerStart
            && Date.now() - pointerStart.time < 350
            && distance(pointerStart.point, endPoint) < 12) {
            const previousTap = lastTouchTapRef.current;
            if (previousTap && Date.now() - previousTap.time < 325 && distance(previousTap.point, endPoint) < 32) {
                toggleImageZoom(endPoint);
                lastTouchTapRef.current = null;
            } else {
                lastTouchTapRef.current = { point: endPoint, time: Date.now() };
            }
        }

        if (activePointersRef.current.size === 1) {
            const remaining = [...activePointersRef.current.values()][0];
            pointerGestureRef.current = {
                type: 'pan',
                start: remaining,
                startPan: imageTransformRef.current.pan,
            };
        } else if (activePointersRef.current.size === 0) {
            pointerGestureRef.current = null;
            pointerStartRef.current = null;
            gestureHadMultiplePointersRef.current = false;
            setImageInteracting(false);
        }
    }, [toggleImageZoom]);

    useEffect(() => {
        const handleKeyDown = (event: KeyboardEvent) => {
            const target = event.target as HTMLElement;
            if (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable) {
                return;
            }

            const key = event.key.toLowerCase();
            if (imagePreview && (event.key === '+' || event.key === '=')) {
                event.preventDefault();
                zoomImageTo(imageTransformRef.current.zoom * IMAGE_ZOOM_STEP);
            } else if (imagePreview && event.key === '-') {
                event.preventDefault();
                zoomImageTo(imageTransformRef.current.zoom / IMAGE_ZOOM_STEP);
            } else if (imagePreview && key === '0') {
                event.preventDefault();
                fitImage();
            } else if (imagePreview && key === '1') {
                event.preventDefault();
                showActualImageSize();
            } else if (imagePreview && imageTransformRef.current.zoom > 1.05 && event.key.startsWith('Arrow')) {
                event.preventDefault();
                const amount = event.shiftKey ? 160 : 64;
                if (event.key === 'ArrowRight') panImageBy(-amount, 0);
                else if (event.key === 'ArrowLeft') panImageBy(amount, 0);
                else if (event.key === 'ArrowDown') panImageBy(0, -amount);
                else if (event.key === 'ArrowUp') panImageBy(0, amount);
            } else if (event.key === 'ArrowRight' || key === 'l') {
                event.preventDefault();
                onNext?.();
            } else if (event.key === 'ArrowLeft' || key === 'j') {
                event.preventDefault();
                onPrev?.();
            } else if (event.key === 'Escape') {
                event.preventDefault();
                onClose();
            }
        };

        window.addEventListener('keydown', handleKeyDown);
        return () => window.removeEventListener('keydown', handleKeyDown);
    }, [onClose, onNext, onPrev, imagePreview, zoomImageTo, fitImage, showActualImageSize, panImageBy]);

    return (
        <div ref={viewerRef} role="dialog" aria-modal="true" aria-label={file.name} className="viewer-overlay fixed inset-0 z-[150] flex items-center justify-center p-4" onClick={onClose}>
            <div className="relative flex max-h-screen w-full max-w-5xl flex-col items-center justify-center" onClick={(event) => event.stopPropagation()}>
                <button
                    onClick={onPrev}
                    disabled={!onPrev}
                    className="viewer-navigation absolute start-2 top-1/2 z-20 -translate-y-1/2 disabled:pointer-events-none disabled:opacity-0"
                    title={i18n.t('viewer.previous_shortcut')}
                    aria-label={i18n.t('viewer.previous_file')}
                >
                    <ChevronLeft className="h-5 w-5 rtl:rotate-180" />
                </button>

                <button
                    onClick={onNext}
                    disabled={!onNext}
                    className="viewer-navigation absolute end-2 top-1/2 z-20 -translate-y-1/2 disabled:pointer-events-none disabled:opacity-0"
                    title={i18n.t('viewer.next_shortcut')}
                    aria-label={i18n.t('viewer.next_file')}
                >
                    <ChevronRight className="h-5 w-5 rtl:rotate-180" />
                </button>

                <button
                    onClick={onClose}
                    className="viewer-control absolute -top-10 end-0 z-20 border border-white/10 bg-black/55"
                    title={i18n.t("common.close")}
                    aria-label={i18n.t('viewer.close_preview')}
                >
                    <X className="h-4 w-4" />
                </button>

                {error && (
                    <div className="viewer-panel max-w-md border-app-danger/25 bg-app-danger/10 p-4 text-app-danger">
                        <p className="text-ui font-semibold">{i18n.t('viewer.preview_error')}</p>
                        <p className="mt-1 text-metadata leading-relaxed">{error}</p>
                    </div>
                )}

                {!error && imagePreview && (
                    <div
                        ref={imageViewportRef}
                        className={`viewer-panel relative flex h-[85vh] w-full items-center justify-center ${imageTransform.zoom > 1 ? (imageInteracting ? 'cursor-grabbing' : 'cursor-grab') : 'cursor-zoom-in'}`}
                        style={{ touchAction: 'none', overscrollBehavior: 'contain' }}
                        onWheel={handleImageWheel}
                        onPointerDown={handleImagePointerDown}
                        onPointerMove={handleImagePointerMove}
                        onPointerUp={(event) => finishImagePointer(event, true)}
                        onPointerCancel={(event) => finishImagePointer(event, false)}
                        onDoubleClick={(event) => toggleImageZoom({ x: event.clientX, y: event.clientY })}
                    >
                        {thumbnailSrc && !fullReady && (
                            <img
                                src={thumbnailSrc}
                                decoding="async"
                                className="pointer-events-none max-h-full max-w-full scale-[1.01] select-none bg-black object-contain blur-[2px]"
                                alt=""
                                aria-hidden="true"
                                draggable={false}
                                onError={() => {
                                    if (!localPath) forgetThumbnail(file.id, activeFolderId);
                                    setThumbnailSrc(null);
                                }}
                            />
                        )}

                        {fullSrc && (
                            <img
                                ref={fullImageRef}
                                src={fullSrc}
                                decoding="async"
                                draggable={false}
                                className={`pointer-events-none absolute inset-0 m-auto max-h-full max-w-full select-none bg-black object-contain ${fullReady ? 'opacity-100' : 'opacity-0'}`}
                                style={{
                                    transform: `translate3d(${imageTransform.pan.x}px, ${imageTransform.pan.y}px, 0) scale(${imageTransform.zoom})`,
                                    transformOrigin: 'center center',
                                    transition: imageInteracting ? 'none' : 'transform 160ms ease-out, opacity 200ms',
                                    willChange: 'transform',
                                }}
                                alt={file.name}
                                onLoad={(event) => {
                                    const image = event.currentTarget;
                                    const loadedFileId = file.id;
                                    const reveal = () => {
                                        if (currentFileIdRef.current !== loadedFileId) return;
                                        setFullReady(true);
                                        setLoading(false);
                                        setProgress(100);
                                    };
                                    if (typeof image.decode === 'function') {
                                        void image.decode().catch(() => {}).finally(reveal);
                                    } else {
                                        reveal();
                                    }
                                }}
                                onError={() => {
                                    if (!localPath) forgetPreview(file.id, activeFolderId);
                                    setError(i18n.t('viewer.image_render_failed'));
                                    setLoading(false);
                                }}
                            />
                        )}

                        {fullReady && (
                            <div
                                className="image-viewer-toolbar viewer-toolbar absolute start-1/2 top-3 z-30 -translate-x-1/2 rtl:translate-x-1/2"
                                onPointerDown={(event) => event.stopPropagation()}
                                onDoubleClick={(event) => event.stopPropagation()}
                                onWheel={(event) => event.stopPropagation()}
                            >
                                <button
                                    type="button"
                                    className="viewer-control disabled:opacity-35"
                                    onClick={() => zoomImageTo(imageTransformRef.current.zoom / IMAGE_ZOOM_STEP)}
                                    disabled={imageTransform.zoom <= MIN_IMAGE_ZOOM + 0.001}
                                    title={t('common.zoom_out_shortcut')}
                                    aria-label={t('common.zoom_out')}
                                >
                                    <ZoomOut className="h-4 w-4" />
                                </button>
                                <button
                                    type="button"
                                    className="min-w-14 rounded-md px-1.5 text-[11px] tabular-nums text-white/80 hover:bg-white/10"
                                    onClick={fitImage}
                                    title={t('common.fit_image_shortcut')}
                                    aria-label={`${t('common.current_zoom', { percent: Math.round(imageTransform.zoom * 100) })}. ${t('common.fit_image')}`}
                                >
                                    {Math.round(imageTransform.zoom * 100)}%
                                </button>
                                <button
                                    type="button"
                                    className="viewer-control disabled:opacity-35"
                                    onClick={() => zoomImageTo(imageTransformRef.current.zoom * IMAGE_ZOOM_STEP)}
                                    disabled={imageTransform.zoom >= MAX_IMAGE_ZOOM - 0.001}
                                    title={t('common.zoom_in_shortcut')}
                                    aria-label={t('common.zoom_in')}
                                >
                                    <ZoomIn className="h-4 w-4" />
                                </button>
                                <span className="mx-0.5 h-5 w-px bg-white/10" aria-hidden="true" />
                                <button
                                    type="button"
                                    className="viewer-control"
                                    onClick={fitImage}
                                    title={t('common.fit_image_shortcut')}
                                    aria-label={t('common.fit_image')}
                                >
                                    <Maximize className="h-4 w-4" />
                                </button>
                                <button
                                    type="button"
                                    className="viewer-control"
                                    onClick={showActualImageSize}
                                    title={t('common.actual_size_shortcut')}
                                    aria-label={t('common.actual_size')}
                                >
                                    <Scan className="h-4 w-4" />
                                </button>
                            </div>
                        )}

                        {loading && (
                            <div className={`viewer-toolbar absolute flex-col gap-2 px-4 py-3 text-white ${thumbnailSrc ? 'bottom-4' : 'start-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 rtl:translate-x-1/2'}`}>
                                <div className="h-5 w-5 animate-spin rounded-full border-2 border-white/25 border-t-app-accent" />
                                <p className="text-metadata">{i18n.t('viewer.loading_preview')}</p>
                                {progress > 0 && (
                                    <div className="h-1 w-32 overflow-hidden rounded-full bg-white/15" aria-label={`${progress}%`}>
                                        <div className="h-full rounded-full bg-telegram-primary transition-[width] duration-200" style={{ width: `${progress}%` }} />
                                    </div>
                                )}
                            </div>
                        )}
                    </div>
                )}

                {!error && !imagePreview && !loading && fullSrc && (
                    <div className="viewer-panel max-w-md p-6 text-center text-white">
                        <File className="mx-auto mb-3 h-10 w-10 text-app-accent" />
                        <h3 className="truncate text-app-title font-medium" title={file.name}>{file.name}</h3>
                        <p className="mt-2 text-ui text-white/60">{i18n.t('viewer.preview_unsupported')}</p>
                        <p className="mt-4 text-badge text-white/40">{i18n.t('viewer.file_type')} {file.name.split('.').pop()}</p>
                    </div>
                )}

                {heicPreview && <div className="viewer-toolbar absolute -top-12 flex gap-2 text-white">
                    <button type="button" className="viewer-control" onClick={() => {
                        void (localPath ? Promise.resolve(localPath) : invoke<string>('cmd_get_preview', {messageId: file.id, folderId: activeFolderId}))
                            .then(path => invoke('cmd_open_file_externally', {path})).catch(error => setError(userFacingError(error, t)));
                    }}>{t('viewer.open_external')}</button>
                    {onDownload && <button type="button" className="viewer-control" onClick={onDownload}>{t('files.download')}</button>}
                </div>}
                <div className="viewer-toolbar absolute -bottom-11 max-w-[min(80vw,40rem)] px-3 py-1.5 text-metadata text-white/70">
                    <span className="min-w-0 truncate" title={file.name}>{file.name}</span>
                    {typeof currentIndex === 'number' && typeof totalItems === 'number' && totalItems > 0 && (
                        <span className="ms-2 shrink-0 tabular-nums text-white/45">{currentIndex + 1}/{totalItems}</span>
                    )}
                </div>
            </div>
        </div>
    );
}
