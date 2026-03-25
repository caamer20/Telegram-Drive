import { useState, useEffect, useRef } from 'react';
import { X, File, ChevronLeft, ChevronRight } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { convertFileSrc } from '@tauri-apps/api/core';
import { TelegramFile } from '../../types';

const PREVIEW_CACHE_TTL_MS = 5 * 60 * 1000;
const PREVIEW_CACHE_MAX_ITEMS = 8;

type PreviewCacheValue = {
    src: string;
    cachedAt: number;
};

const previewCache = new Map<string, PreviewCacheValue>();
const pendingPrefetch = new Set<string>();

const getPreviewCacheKey = (fileId: number, folderId: number | null) => `${folderId ?? 'home'}:${fileId}`;

const touchPreviewCache = (key: string, value: PreviewCacheValue) => {
    if (previewCache.has(key)) previewCache.delete(key);
    previewCache.set(key, value);

    while (previewCache.size > PREVIEW_CACHE_MAX_ITEMS) {
        const oldestKey = previewCache.keys().next().value;
        if (!oldestKey) break;
        previewCache.delete(oldestKey);
    }
};

const getCachedPreview = (key: string): string | null => {
    const value = previewCache.get(key);
    if (!value) return null;

    if (Date.now() - value.cachedAt > PREVIEW_CACHE_TTL_MS) {
        previewCache.delete(key);
        return null;
    }

    touchPreviewCache(key, value);
    return value.src;
};

const rememberPreview = (key: string, src: string) => {
    touchPreviewCache(key, { src, cachedAt: Date.now() });
};

const forgetPreview = (key: string) => {
    previewCache.delete(key);
};

const isSafeToPrefetch = (name: string) => {
    const lower = name.toLowerCase();
    return ['jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp', 'svg', 'heic', 'heif']
        .some(ext => lower.endsWith(ext));
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
}

export function PreviewModal({ file, onClose, onNext, onPrev, currentIndex, totalItems, nextFile, prevFile, activeFolderId }: PreviewModalProps) {
    const [src, setSrc] = useState<string | null>(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);
    const [reloadNonce, setReloadNonce] = useState(0);
    const [retryCount, setRetryCount] = useState(0);
    const latestRequestRef = useRef(0);

    useEffect(() => {
        setRetryCount(0);
        setReloadNonce(0);
    }, [file.id, activeFolderId]);

    useEffect(() => {
        const load = async () => {
            const key = getPreviewCacheKey(file.id, activeFolderId);
            const shouldBypassCache = reloadNonce > 0;
            const requestId = ++latestRequestRef.current;
            const cachedSrc = shouldBypassCache ? null : getCachedPreview(key);

            if (cachedSrc) {
                if (requestId !== latestRequestRef.current) return;
                setSrc(cachedSrc);
                setLoading(false);
                setError(null);
                return;
            }

            setLoading(true);
            setError(null);
            try {
                const path = await invoke<string>('cmd_get_preview', {
                    messageId: file.id,
                    folderId: activeFolderId
                });
                if (requestId !== latestRequestRef.current) return;

                if (path) {
                    if (path.startsWith('data:')) {
                        setSrc(path);
                        rememberPreview(key, path);
                    } else {
                        const converted = convertFileSrc(path);
                        setSrc(converted);
                        rememberPreview(key, converted);
                    }
                } else {
                    setError("Preview not available");
                }
            } catch (e) {
                if (requestId !== latestRequestRef.current) return;
                setError(String(e));
            } finally {
                if (requestId !== latestRequestRef.current) return;
                setLoading(false);
            }
        };
        load();
    }, [file, activeFolderId, reloadNonce]);

    useEffect(() => {
        const candidates = [nextFile, prevFile].filter((f): f is TelegramFile => !!f && isSafeToPrefetch(f.name));

        candidates.forEach((candidate) => {
            const key = getPreviewCacheKey(candidate.id, activeFolderId);
            if (getCachedPreview(key) || pendingPrefetch.has(key)) return;

            pendingPrefetch.add(key);
            invoke<string>('cmd_get_preview', {
                messageId: candidate.id,
                folderId: activeFolderId
            }).then((path) => {
                if (!path) return;
                const normalized = path.startsWith('data:') ? path : convertFileSrc(path);
                rememberPreview(key, normalized);
            }).catch(() => {
                // Ignore prefetch errors, main preview flow will handle user-visible failures.
            }).finally(() => {
                pendingPrefetch.delete(key);
            });
        });
    }, [nextFile, prevFile, activeFolderId]);

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
            }
        };

        window.addEventListener('keydown', handleKeyDown);
        return () => window.removeEventListener('keydown', handleKeyDown);
    }, [onClose, onNext, onPrev]);

    return (
        <div className="fixed inset-0 z-[150] bg-black/90 flex items-center justify-center p-4 backdrop-blur-sm" onClick={onClose}>
            <div className="relative max-w-5xl w-full max-h-screen flex flex-col items-center justify-center" onClick={e => e.stopPropagation()}>
                <button
                    onClick={onPrev}
                    className="absolute left-2 top-1/2 -translate-y-1/2 p-2 bg-black/60 hover:bg-black/80 rounded-full transition-colors"
                    style={{ color: '#ffffff' }}
                    title="Previous (ArrowLeft / J)"
                >
                    <ChevronLeft className="w-6 h-6" />
                </button>

                <button
                    onClick={onNext}
                    className="absolute right-2 top-1/2 -translate-y-1/2 p-2 bg-black/60 hover:bg-black/80 rounded-full transition-colors"
                    style={{ color: '#ffffff' }}
                    title="Next (ArrowRight / L)"
                >
                    <ChevronRight className="w-6 h-6" />
                </button>

                <button
                    onClick={onClose}
                    className="absolute -top-12 right-0 p-2 bg-black/60 hover:bg-black/80 rounded-full transition-colors"
                    style={{ color: '#ffffff' }}
                >
                    <X className="w-6 h-6" />
                </button>

                {loading && (
                    <div className="flex flex-col items-center gap-4 text-white">
                        <div className="w-10 h-10 border-4 border-telegram-primary border-t-transparent rounded-full animate-spin"></div>
                        <p>Loading preview...</p>
                        <p className="text-xs text-white/50">Downloading from Telegram...</p>
                    </div>
                )}

                {error && (
                    <div className="text-red-400 bg-white/10 p-4 rounded-lg border border-red-500/20">
                        <p className="font-bold">Preview Error</p>
                        <p className="text-sm">{error}</p>
                    </div>
                )}

                {!loading && !error && src && (
                    <div className="flex flex-col items-center">
                        {['jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp', 'svg', 'heic', 'heif'].some(ext => file.name.toLowerCase().endsWith(ext)) ? (
                            <img
                                src={src}
                                className="max-w-full max-h-[85vh] object-contain rounded-lg shadow-2xl bg-black"
                                alt="Preview"
                                onError={() => {
                                    const key = getPreviewCacheKey(file.id, activeFolderId);
                                    forgetPreview(key);

                                    if (retryCount < 1) {
                                        setRetryCount((prev) => prev + 1);
                                        setReloadNonce((prev) => prev + 1);
                                        return;
                                    }

                                    setError('Failed to render image preview');
                                }}
                            />
                        ) : ['mp4', 'webm', 'ogg', 'mov'].some(ext => file.name.toLowerCase().endsWith(ext)) ? (
                            <video src={src} controls className="max-w-full max-h-[85vh] rounded-lg shadow-2xl bg-black" />
                        ) : (
                            <div className="bg-[#1c1c1c] p-8 rounded-xl text-center border border-white/10 shadow-2xl">
                                <File className="w-16 h-16 text-telegram-primary mx-auto mb-4" />
                                <h3 className="text-xl text-white font-medium mb-2">{file.name}</h3>
                                <p className="text-gray-400 mb-6">Preview not supported in app.</p>
                                <p className="text-xs text-gray-500">File type: {file.name.split('.').pop()}</p>
                            </div>
                        )}
                    </div>
                )}

                <div className="absolute bottom-[-3rem] text-white text-sm opacity-50">
                    {file.name}
                    {typeof currentIndex === 'number' && typeof totalItems === 'number' && totalItems > 0 && (
                        <span className="ml-3">{currentIndex + 1}/{totalItems} • Arrow/J-L</span>
                    )}
                </div>
            </div>
        </div>
    );
}
