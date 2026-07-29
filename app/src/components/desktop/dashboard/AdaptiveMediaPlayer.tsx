import { useEffect, useRef, useState, useCallback, useMemo } from 'react';
import { X, ChevronLeft, ChevronRight, AlertTriangle, Loader2, RefreshCw, StopCircle, Maximize2, Minimize2, Volume2, VolumeX, Volume1, Play, Activity, Trash2, Zap } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { toast } from 'sonner';
import Hls from 'hls.js';
import { TelegramFile, StreamingQuality, TranscodePrepareResult, TranscodeJobPhase, TranscodeCapabilities, QUALITY_LABELS, HLS_QUALITIES } from '../../../types';
import { useAdaptiveStreaming } from '../../../hooks/useAdaptiveStreaming';
import { QualitySelector } from '../../shared/QualitySelector';
import { redactSensitiveText, redactSensitiveValue } from '../../../security/redaction';

interface AdaptiveMediaPlayerProps {
    file: TelegramFile;
    activeFolderId: number | null;
    onClose: () => void;
    onNext?: () => void;
    onPrev?: () => void;
    currentIndex?: number;
    totalItems?: number;
    streamUrl: string;
}

// ── HLS Quality State ────────────────────────────────────────────────

const STREAM_BASE_KEY = '/stream/';

export function AdaptiveMediaPlayer({
    file,
    activeFolderId,
    onClose,
    onNext,
    onPrev,
    currentIndex,
    totalItems,
    streamUrl,
}: AdaptiveMediaPlayerProps) {
    // ── Restart counter for MSE pipeline reinit ──────────────────────
    // Must be declared BEFORE restartStreamUrl useMemo
    const [restartNonce, setRestartNonce] = useState(0);

    // ── fMP4 remux state ────────────────────────────────────────────
    const [fmp4Remuxing, setFmp4Remuxing] = useState(false);
    const [fmp4RemuxError, setFmp4RemuxError] = useState<string | null>(null);
    const [fmp4StreamUrl, setFmp4StreamUrl] = useState<string | null>(null);
    const fmp4RemuxingRef = useRef(false);
    // Generation counter bumped on source change so stale async IIFEs
    // from a previous file don't set state on the current file.
    const remuxGenerationRef = useRef(0);

    // ── Effective stream URL: use fMP4 URL when available ────────────
    const effectiveStreamUrl = fmp4StreamUrl || streamUrl;

    // ── Original MSE streaming (for "original" quality) ──────────────
    // Changing restartNonce forces the hook to reinitialize by altering the streamUrl
    const restartStreamUrl = useMemo(() => {
        if (restartNonce > 0) {
            const sep = effectiveStreamUrl.includes('?') ? '&' : '?';
            return `${effectiveStreamUrl}${sep}_r=${restartNonce}`;
        }
        return effectiveStreamUrl;
    }, [effectiveStreamUrl, restartNonce]);

    // ── Refs for late-bound values needed by progressive handler ─────
    const abortMseRef = useRef<(() => void) | null>(null);
    const logRef = useRef<((msg: string, ...args: unknown[]) => void) | null>(null);
    const transcodeCapsRef = useRef<TranscodeCapabilities | null>(null);

    // ── Handle progressive MP4 detection — trigger fMP4 remux ──────
    const handleProgressiveDetected = useCallback(() => {
        if (fmp4RemuxingRef.current) return;

        // Don't attempt fMP4 remux if FFmpeg is not available — silently
        // fall back to native <video> without showing an error overlay.
        if (!transcodeCapsRef.current?.available) {
            logRef.current?.('Progressive MP4 detected, but FFmpeg unavailable — using native video');
            return;
        }

        fmp4RemuxingRef.current = true;
        setFmp4Remuxing(true);
        setFmp4RemuxError(null);

        // Capture the generation counter so the async IIFE can bail out
        // if the user navigates to a different file before the remux
        // completes (prevents stale errors from leaking onto the wrong file).
        const gen = remuxGenerationRef.current;

        logRef.current?.('Progressive MP4 detected — triggering fMP4 remux...');

        (async () => {
            try {
                const result = await invoke<{ url: string; output_file_key: string; status: string }>(
                    'cmd_prepare_fmp4_stream',
                    {
                        messageId: file.id,
                        folderId: activeFolderId,
                    },
                );

                if (gen !== remuxGenerationRef.current) {
                    logRef.current?.('fMP4 remux: source changed, discarding stale result');
                    fmp4RemuxingRef.current = false;
                    return;
                }

                if (result.status === 'ready') {
                    // Already cached — switch immediately
                    const fullUrl = `${streamBaseRef.current}${result.url}?token=${streamTokenRef.current}`;
                    logRef.current?.('fMP4 already cached, switching to:', fullUrl);
                    abortMseRef.current?.();
                    setFmp4StreamUrl(fullUrl);
                    setFmp4Remuxing(false);
                    fmp4RemuxingRef.current = false;
                    setRestartNonce(n => n + 1);
                    return;
                }

                // status === 'processing' — poll until ready
                logRef.current?.('fMP4 remux started in background, polling status...');
                const fileKey = result.output_file_key;
                const fmp4Url = result.url;

                const poll = async (): Promise<void> => {
                    for (let i = 0; i < 600; i++) { // max 10 minutes
                        await new Promise(r => setTimeout(r, 1000));
                        if (gen !== remuxGenerationRef.current) {
                            logRef.current?.('fMP4 poll: source changed, stopping');
                            fmp4RemuxingRef.current = false;
                            return;
                        }

                        const status = await invoke<{ status: string; error: string | null }>(
                            'cmd_get_fmp4_status',
                            { fileKey },
                        );

                        if (status.status === 'ready') {
                            const fullUrl = `${streamBaseRef.current}${fmp4Url}?token=${streamTokenRef.current}`;
                            logRef.current?.('fMP4 remux complete, switching to:', fullUrl);
                            abortMseRef.current?.();
                            setFmp4StreamUrl(fullUrl);
                            setFmp4Remuxing(false);
                            fmp4RemuxingRef.current = false;
                            setRestartNonce(n => n + 1);
                            return;
                        }

                        if (status.status === 'error') {
                            throw new Error(status.error || 'fMP4 remux failed');
                        }

                        // status === 'processing' — continue polling
                    }
                    throw new Error('fMP4 remux timed out after 10 minutes');
                };

                await poll();
            } catch (e: any) {
                // Don't show error if the user already moved to another file.
                if (gen !== remuxGenerationRef.current) {
                    fmp4RemuxingRef.current = false;
                    return;
                }
                const safeError = redactSensitiveText(String(e));
                logRef.current?.('fMP4 remux failed:', safeError);
                setFmp4RemuxError(safeError);
                setFmp4Remuxing(false);
                fmp4RemuxingRef.current = false;
                // The hook will fall back to native video since we
                // didn't provide a new URL
            }
        })();
    }, [file.id, activeFolderId]);

    // ── progressive callback for useAdaptiveStreaming ────────────────
    const progressiveCallback = useMemo(
        () => handleProgressiveDetected,
        [handleProgressiveDetected],
    );

    const {
        videoRef: mseVideoRef,
        phase: msePhase,
        error: mseError,
        tracks,
        loadProgress,
        currentQuality,
        setQuality,
        adaptiveMode,
        setAdaptiveMode,
        measuredKbps,
        useFallback,
        fallbackUrl,
        abort: abortMse,
    } = useAdaptiveStreaming(restartStreamUrl, file.name, progressiveCallback);

    // ── HLS transcode state ──────────────────────────────────────────
    // playbackMode is the single source of truth: 'original' or 'hls'
    const [playbackMode, setPlaybackMode] = useState<'original' | 'hls'>('original');
    const [hlsQuality, setHlsQuality] = useState<StreamingQuality | null>(null);
    const [hlsPhase, setHlsPhase] = useState<TranscodeJobPhase>('idle');
    const [hlsProgress, setHlsProgress] = useState(0);
    const [hlsError, setHlsError] = useState<string | null>(null);
    const [hlsPlaylistUrl, setHlsPlaylistUrl] = useState<string | null>(null);
    const [transcodeCapabilities, setTranscodeCapabilities] = useState<TranscodeCapabilities | null>(null);
    const [hlsVariantStates, setHlsVariantStates] = useState<Record<string, TranscodeJobPhase>>({});

    const hlsRef = useRef<Hls | null>(null);
    const hlsVideoRef = useRef<HTMLVideoElement>(null);
    const pollTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
    const savedTimeRef = useRef<number>(0);
    const hlsQualityRef = useRef<StreamingQuality | null>(null);
    const streamTokenRef = useRef<string>('');
    const currentJobIdRef = useRef<string | null>(null);
    const streamBaseRef = useRef<string>('');
    const containerRef = useRef<HTMLDivElement>(null);
    const [isFullscreen, setIsFullscreen] = useState(false);
    const isFullscreenRef = useRef(false);
    // HLS video ready flag for retry-safe attach
    const [hlsVideoReady, setHlsVideoReady] = useState(false);
    // Callback ref: fires when HLS video element mounts
    const hlsVideoCallbackRef = useCallback((el: HTMLVideoElement | null) => {
        (hlsVideoRef as React.MutableRefObject<HTMLVideoElement | null>).current = el;
        setHlsVideoReady(!!el);
    }, []);

    // ── Logging helpers ─────────────────────────────────────────────
    const log = useCallback((msg: string, ...args: unknown[]) => {
        console.log(`[AdaptivePlayer] ${msg}`, ...args.map(arg => redactSensitiveValue(arg)));
    }, []);
    // Wire late-bound refs for the progressive handler
    logRef.current = log;

    // ── Volume state ─────────────────────────────────────────────────
    const [volume, setVolume] = useState(1);
    const [isMuted, setIsMuted] = useState(false);
    const [showVolumeSlider, setShowVolumeSlider] = useState(false);
    const volumeBeforeMute = useRef(1);

    // ── Source / playing resolution tracking ──────────────────────────
    const [sourceResolution, setSourceResolution] = useState<{ w: number; h: number } | null>(null);
    const [playingResolution, setPlayingResolution] = useState<{ w: number; h: number } | null>(null);

    // ── Source height for upscale prevention ──────────────────────────
    const sourceHeight = useMemo(() => {
        // From MSE tracks (mp4box metadata)
        const videoTrack = tracks.find(t => t.type === 'video');
        if (videoTrack?.height) return videoTrack.height;
        // From resolution badge (HLS metadata or video element)
        if (sourceResolution?.h) return sourceResolution.h;
        return null;
    }, [tracks, sourceResolution]);

    // ── Debug overlay ───────────────────────────────────────────────
    const [debugOverlay, setDebugOverlay] = useState(() => {
        try { return localStorage.getItem('debug_overlay') === '1'; } catch { return false; }
    });
    const [debugBufferedSecs, setDebugBufferedSecs] = useState(0);

    const toggleDebugOverlay = useCallback(() => {
        setDebugOverlay(prev => {
            const next = !prev;
            try { localStorage.setItem('debug_overlay', next ? '1' : '0'); } catch {}
            return next;
        });
    }, []);

    // Clear transcode cache for current file
    const [clearingCache, setClearingCache] = useState(false);
    const fileKey = `${activeFolderId ?? 0}_${file.id}`;

    const handleClearTranscodeCache = useCallback(async () => {
        setClearingCache(true);
        try {
            const msg = await invoke<string>('cmd_clear_transcode_cache', { fileKey });
            toast.success(msg);
            log('cleared transcode cache for', fileKey);
        } catch (e) {
            toast.error(`Failed to clear cache: ${e}`);
        } finally {
            setClearingCache(false);
        }
    }, [fileKey, log]);

    // Poll buffered seconds
    useEffect(() => {
        if (!debugOverlay) return;
        const interval = setInterval(() => {
            const video = (playbackMode === 'hls' ? hlsVideoRef.current : mseVideoRef.current);
            if (video && video.buffered.length > 0) {
                setDebugBufferedSecs(video.buffered.end(video.buffered.length - 1) - video.currentTime);
            }
        }, 1000);
        return () => clearInterval(interval);
    }, [debugOverlay, playbackMode]);

    // Track resolution from MSE track info
    useEffect(() => {
        const videoTrack = tracks.find(t => t.type === 'video');
        if (videoTrack?.width && videoTrack?.height && !sourceResolution) {
            setSourceResolution({ w: videoTrack.width, h: videoTrack.height });
        }
    }, [tracks, sourceResolution]);

    // Poll active video element for current playing resolution
    useEffect(() => {
        const interval = setInterval(() => {
            const video = (playbackMode === 'hls' ? hlsVideoRef.current : mseVideoRef.current);
            if (video && video.videoWidth > 0 && video.videoHeight > 0) {
                const pw = video.videoWidth;
                const ph = video.videoHeight;
                setPlayingResolution(prev => {
                    if (!prev || prev.w !== pw || prev.h !== ph) return { w: pw, h: ph };
                    return prev;
                });
            }
        }, 2000);
        return () => clearInterval(interval);
    }, [playbackMode]);

    // Sync volume to active video element
    const applyVolume = useCallback((v: number, muted: boolean) => {
        const video = hlsVideoRef.current || mseVideoRef.current;
        if (video) {
            video.volume = muted ? 0 : v;
            video.muted = muted;
        }
    }, []);

    const handleVolumeChange = useCallback((newVolume: number) => {
        const clamped = Math.max(0, Math.min(1, newVolume));
        setVolume(clamped);
        setIsMuted(clamped === 0);
        applyVolume(clamped, clamped === 0);
        if (clamped > 0) volumeBeforeMute.current = clamped;
    }, [applyVolume]);

    // Wire abortMse ref for progressive handler
    abortMseRef.current = abortMse;

    const toggleMute = useCallback(() => {
        if (isMuted) {
            setIsMuted(false);
            setVolume(volumeBeforeMute.current);
            applyVolume(volumeBeforeMute.current, false);
        } else {
            volumeBeforeMute.current = volume || 1;
            setIsMuted(true);
            setVolume(volume);
            applyVolume(volume, true);
        }
    }, [isMuted, volume, applyVolume]);

    // Re-apply volume when video element changes (HLS <-> MSE)
    useEffect(() => {
        const video = hlsVideoRef.current || mseVideoRef.current;
        if (video) {
            video.volume = isMuted ? 0 : volume;
            video.muted = isMuted;
        }
    }, [isMuted, volume, hlsPhase, msePhase]);

    // Extract stream token and base URL once
    useEffect(() => {
        try {
            const url = new URL(streamUrl);
            const token = url.searchParams.get('token');
            if (token) streamTokenRef.current = token;
            // Store origin for constructing HLS URLs
            const streamIdx = streamUrl.indexOf(STREAM_BASE_KEY);
            if (streamIdx !== -1) {
                streamBaseRef.current = streamUrl.substring(0, streamIdx);
            } else {
                streamBaseRef.current = url.origin;
            }
        } catch { /* ignore */ }

        // Reset fMP4 remux state when switching files.
        // Prevents stale error / loading state from a previous progressive
        // MP4 from leaking into the current file (especially fragmented MP4s
        // that never trigger the remux pipeline).
        // Also bump the generation counter so any in-flight async invokes
        // from the previous file discard their results instead of setting
        // state on the new file.
        setFmp4Remuxing(false);
        setFmp4RemuxError(null);
        setFmp4StreamUrl(null);
        fmp4RemuxingRef.current = false;
        remuxGenerationRef.current += 1;
    }, [streamUrl]);

    // ── Fetch transcode capabilities ──────────────────────────────────
    useEffect(() => {
        invoke<TranscodeCapabilities>('cmd_get_transcode_capabilities')
            .then(caps => {
                log('transcodeCapabilities', caps);
                setTranscodeCapabilities(caps);
                transcodeCapsRef.current = caps;
            })
            .catch(() => {
                const fallback = { available: false, variants: [], mode: 'original' as const };
                setTranscodeCapabilities(fallback);
                transcodeCapsRef.current = fallback;
            });
    }, []);

    // ── Poll transcode status ────────────────────────────────────────
    const pollTranscodeStatus = useCallback((jobId: string, quality: StreamingQuality) => {
        if (pollTimerRef.current) clearInterval(pollTimerRef.current);

        pollTimerRef.current = setInterval(async () => {
            try {
                const status = await invoke<{
                    job_id: string;
                    status: string;
                    progress: number;
                    error: string | null;
                    playlist_url: string | null;
                }>('cmd_get_transcode_status', { jobId });

                log('cmd_get_transcode_status result', { jobId, status: status.status, progress: status.progress });

                const q = quality;
                switch (status.status) {
                    case 'caching':
                        setHlsPhase('caching');
                        setHlsProgress(status.progress);
                        break;
                    case 'transcoding':
                        setHlsPhase('transcoding');
                        setHlsProgress(status.progress);
                        break;
                    case 'ready':
                        if (pollTimerRef.current) clearInterval(pollTimerRef.current);
                        setHlsPhase('ready');
                        setHlsProgress(1);
                        if (status.playlist_url) {
                            const fullUrl = `${streamBaseRef.current}${status.playlist_url}?token=${streamTokenRef.current}`;
                            setHlsPlaylistUrl(fullUrl);
                        }
                        setHlsVariantStates(prev => ({ ...prev, [q]: 'ready' }));
                        break;
                    case 'error':
                        if (pollTimerRef.current) clearInterval(pollTimerRef.current);
                        setHlsPhase('failed');
                        setHlsError(status.error || 'Transcode failed');
                        setHlsVariantStates(prev => ({ ...prev, [q]: 'failed' }));
                        break;
                    case 'cancelled':
                        if (pollTimerRef.current) clearInterval(pollTimerRef.current);
                        setHlsPhase('idle');
                        break;
                }
            } catch {
                // Status check failed, will retry
            }
        }, 1000);
    }, []);

    // ── Start HLS transcode for a quality ───────────────────────────
    const startTranscode = useCallback(async (quality: StreamingQuality) => {
        if (quality === 'original' || !HLS_QUALITIES.includes(quality)) return;

        log('startTranscode', { quality, activeFolderId, messageId: file.id });

        // Save current playback time
        const video = mseVideoRef.current || hlsVideoRef.current;
        if (video) savedTimeRef.current = video.currentTime;

        hlsQualityRef.current = quality;
        setPlaybackMode('hls');
        setHlsQuality(quality);
        setHlsPhase('preparing');
        setHlsProgress(0);
        setHlsError(null);
        setHlsPlaylistUrl(null);

        // Abort the original MSE download to save bandwidth
        abortMse();

        // Destroy existing HLS instance
        if (hlsRef.current) {
            hlsRef.current.destroy();
            hlsRef.current = null;
        }

        try {
            // Use activeFolderId instead of file.folder_id so we resolve the correct peer
            const result = await invoke<TranscodePrepareResult>('cmd_prepare_transcoded_stream', {
                messageId: file.id,
                folderId: activeFolderId,
                quality,
            });

            log('cmd_prepare_transcoded_stream result', result);

            if (result.status === 'ready') {
                setHlsPhase('ready');
                setHlsProgress(1);
                setHlsVariantStates(prev => ({ ...prev, [quality]: 'ready' }));
                if (result.playlist_url) {
                    const fullUrl = `${streamBaseRef.current}${result.playlist_url}?token=${streamTokenRef.current}`;
                    log('hlsPlaylistUrl', fullUrl);
                    setHlsPlaylistUrl(fullUrl);
                }
            } else if (result.status === 'error') {
                setHlsPhase('failed');
                setHlsError('Transcode failed to start');
                setHlsVariantStates(prev => ({ ...prev, [quality]: 'failed' }));
            } else {
                // Job started, begin polling
                currentJobIdRef.current = result.job_id;
                setHlsVariantStates(prev => ({ ...prev, [quality]: 'preparing' }));
                pollTranscodeStatus(result.job_id, quality);
            }
        } catch (e: any) {
            const safeError = redactSensitiveText(String(e));
            log('startTranscode error', safeError);
            setHlsPhase('failed');
            setHlsError(safeError);
            setHlsVariantStates(prev => ({ ...prev, [quality]: 'failed' }));
        }
    }, [file.id, activeFolderId, mseVideoRef, pollTranscodeStatus, abortMse, log]);

    // ── Handle quality change ────────────────────────────────────────
    const handleQualityChange = useCallback((quality: StreamingQuality) => {
        log('handleQualityChange', { quality, currentPlaybackMode: playbackMode, transcodeAvailable: transcodeCapabilities?.available });

        // Always update quality first for immediate UI feedback
        setQuality(quality);

        if (quality === 'original') {
            // Clean up HLS if switching back from transcode mode
            if (hlsRef.current) {
                hlsRef.current.destroy();
                hlsRef.current = null;
            }
            if (pollTimerRef.current) {
                clearInterval(pollTimerRef.current);
                pollTimerRef.current = null;
            }
            hlsQualityRef.current = null;
            currentJobIdRef.current = null;
            setPlaybackMode('original');
            setHlsPhase('idle');
            setHlsQuality(null);
            setHlsPlaylistUrl(null);
            setHlsError(null);
            setHlsVideoReady(false);
            // Force re-init of the MSE pipeline by bumping restartNonce.
            // This causes useAdaptiveStreaming to reinitialize with a new streamUrl.
            log('Switching back to original — restarting MSE pipeline');
            setRestartNonce(n => n + 1);
        } else if (transcodeCapabilities?.available) {
            // FFmpeg available — start real transcode for HLS playback
            startTranscode(quality);
        } else {
            // FFmpeg not available — log and fall through to throttle mode
            log('FFmpeg unavailable, using throttle mode for', quality);
        }
        // If FFmpeg not available: setQuality already applied the bandwidth throttle
    }, [setQuality, startTranscode, transcodeCapabilities, playbackMode, log]);

    // ── Fullscreen helpers (DOM/video fullscreen first, Tauri fallback) ──
    const enterFullscreen = useCallback(async () => {
        // Try DOM/video fullscreen first
        let fullscreenSuccess = false;
        try {
            const video = (hlsVideoRef.current || mseVideoRef.current) as HTMLVideoElement | null;
            if (video && typeof video.requestFullscreen === 'function') {
                await video.requestFullscreen({ navigationUI: 'hide' });
                fullscreenSuccess = true;
            } else if (containerRef.current) {
                await containerRef.current.requestFullscreen({ navigationUI: 'hide' });
                fullscreenSuccess = true;
            }
        } catch { /* DOM fullscreen not supported or denied */ }
        // Fall back to Tauri window fullscreen if DOM fullscreen failed
        if (!fullscreenSuccess) {
            try {
                await getCurrentWindow().setFullscreen(true);
            } catch {}
        }
        setIsFullscreen(true);
    }, []);

    const exitFullscreen = useCallback(async () => {
        if (document.fullscreenElement) {
            await document.exitFullscreen().catch(() => {});
        }
        try {
            await getCurrentWindow().setFullscreen(false);
        } catch {}
        setIsFullscreen(false);
    }, []);

    const toggleFullscreen = useCallback(async () => {
        if (isFullscreenRef.current) {
            await exitFullscreen();
        } else {
            await enterFullscreen();
        }
    }, [enterFullscreen, exitFullscreen]);

    // Sync isFullscreen ref (avoids re-registering event listeners on every toggle)
    useEffect(() => {
        isFullscreenRef.current = isFullscreen;
    }, [isFullscreen]);

    // Sync isFullscreen from both Tauri window + DOM fullscreen sources.
    // isFullscreen is true if EITHER source reports fullscreen.
    useEffect(() => {
        let mounted = true;
        let unlistenFn: (() => void) | undefined;

        // Track each source independently so we can OR them
        let tauriFs = false;
        let domFs = false;
        const sync = () => { if (mounted) setIsFullscreen(tauriFs || domFs); };

        getCurrentWindow().onResized(async () => {
            if (!mounted) return;
            try {
                tauriFs = await getCurrentWindow().isFullscreen();
                sync();
            } catch {}
        }).then(fn => { if (mounted) unlistenFn = fn; });

        const onFsChange = () => {
            domFs = !!document.fullscreenElement;
            sync();
        };
        document.addEventListener('fullscreenchange', onFsChange);

        return () => {
            mounted = false;
            unlistenFn?.();
            document.removeEventListener('fullscreenchange', onFsChange);
        };
    }, []);

    // ── Initialize hls.js when playlist is ready ─────────────────────
    useEffect(() => {
        if (playbackMode !== 'hls' || !hlsPlaylistUrl || hlsPhase !== 'ready') return;
        if (!hlsVideoReady) {
            log('hlsVideoRef not ready yet, waiting for callback ref...');
            return;
        }
        const video = hlsVideoRef.current;
        if (!video) return;

        log('Initializing HLS playback', { playlistUrl: hlsPlaylistUrl });

        // Seek to saved position
        const savedTime = savedTimeRef.current;

        // Log HLS metadata once the video loads
        const onHlsMetadata = () => {
            const v = hlsVideoRef.current;
            if (v) {
                console.log('[AdaptivePlayer] HLS metadata', {
                    width: v.videoWidth,
                    height: v.videoHeight,
                    hasSource: Boolean(v.currentSrc),
                });
                // Capture source resolution for the badge
                if (v.videoWidth > 0 && v.videoHeight > 0) {
                    setSourceResolution(prev => prev || { w: v.videoWidth, h: v.videoHeight });
                }
            }
        };

        if (video.canPlayType('application/vnd.apple.mpegurl')) {
            // Native HLS support (Safari)
            video.src = hlsPlaylistUrl;
            video.addEventListener('loadedmetadata', () => {
                onHlsMetadata();
                if (savedTime > 0) video.currentTime = savedTime;
                video.play().catch(() => {});
            }, { once: true });
        } else if (Hls.isSupported()) {
            const token = streamTokenRef.current;
            const hls = new Hls({
                enableWorker: true,
                lowLatencyMode: false,
                backBufferLength: 90,
                // ── Token propagation: append ?token=... to every /hls/ request ──
                // Guard against URLs that already have a token param
                xhrSetup: (xhr, url) => {
                    if (token && url.includes('/hls/') && !/[?&]token=/.test(url)) {
                        const sep = url.includes('?') ? '&' : '?';
                        xhr.open('GET', `${url}${sep}token=${encodeURIComponent(token)}`, true);
                    }
                },
            });
            hlsRef.current = hls;

            hls.loadSource(hlsPlaylistUrl);
            hls.attachMedia(video);

            hls.on(Hls.Events.MANIFEST_PARSED, () => {
                log('HLS MANIFEST_PARSED, seeking to', savedTime);
                onHlsMetadata();
                if (savedTime > 0) video.currentTime = savedTime;
                video.play().catch(() => {});
            });

            hls.on(Hls.Events.ERROR, (_event, data) => {
                if (data.fatal) {
                    const safeDetails = redactSensitiveText(String(data.details));
                    log('HLS fatal error', data.type, safeDetails);
                    console.error('[HLS] Fatal error:', data.type, safeDetails);
                    setHlsError(`HLS playback error: ${safeDetails}`);
                    setHlsPhase('failed');
                    hls.destroy();
                    hlsRef.current = null;
                } else {
                    log('HLS non-fatal error', data.type, data.details);
                }
            });
        } else {
            log('HLS not supported in this browser');
            setHlsError('HLS playback not supported in this browser');
            setHlsPhase('failed');
        }

        return () => {
            // Don't destroy on cleanup, only on explicit switch
        };
    }, [playbackMode, hlsPlaylistUrl, hlsPhase, hlsVideoReady, log]);

    // ── Cancel ongoing transcode ─────────────────────────────────────
    const cancelTranscode = useCallback(async () => {
        log('cancelTranscode');
        const jobId = currentJobIdRef.current;
        if (jobId) {
            try { await invoke('cmd_cancel_transcode', { jobId }); } catch {}
            currentJobIdRef.current = null;
        }
        if (pollTimerRef.current) {
            clearInterval(pollTimerRef.current);
            pollTimerRef.current = null;
        }
        if (hlsRef.current) {
            hlsRef.current.destroy();
            hlsRef.current = null;
        }
        hlsQualityRef.current = null;
        setPlaybackMode('original');
        setHlsPhase('idle');
        setHlsQuality(null);
        setHlsPlaylistUrl(null);
        setHlsError(null);
        setHlsVideoReady(false);
        setQuality('original');
        // Force re-init of the MSE pipeline
        log('cancelTranscode — restarting MSE pipeline');
        setRestartNonce(n => n + 1);
    }, [setQuality, log]);

    // ── Retry failed transcode ───────────────────────────────────────
    const retryTranscode = useCallback(() => {
        if (hlsQuality) startTranscode(hlsQuality);
    }, [hlsQuality, startTranscode]);

    // ── Cleanup ──────────────────────────────────────────────────────
    useEffect(() => {
        return () => {
            if (hlsRef.current) {
                hlsRef.current.destroy();
                hlsRef.current = null;
            }
            if (pollTimerRef.current) {
                clearInterval(pollTimerRef.current);
                pollTimerRef.current = null;
            }
        };
    }, []);

    // ── Keyboard shortcuts ───────────────────────────────────────────
    useEffect(() => {
        const handleKeyDown = (e: KeyboardEvent) => {
            const target = e.target as HTMLElement;
            if (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable) return;
            const key = e.key.toLowerCase();
            if (e.key === 'ArrowRight' || key === 'l') { e.preventDefault(); onNext?.(); }
            else if (e.key === 'ArrowLeft' || key === 'j') { e.preventDefault(); onPrev?.(); }
            else if (e.key === 'Escape') {
                e.preventDefault();
                // Exit fullscreen first, then close
                if (isFullscreenRef.current) {
                    toggleFullscreen();
                } else {
                    onClose();
                }
            }
            else if (key === 'f') { e.preventDefault(); toggleFullscreen(); }
            else if (key === 'm') { e.preventDefault(); toggleMute(); }
            else if (key === 'd') { e.preventDefault(); toggleDebugOverlay(); }
            else if (e.key === ' ') {
                e.preventDefault();
                const video = hlsVideoRef.current || mseVideoRef.current;
                if (video) {
                    video.paused ? video.play().catch(() => {}) : video.pause();
                }
            }
        };
        window.addEventListener('keydown', handleKeyDown);
        return () => window.removeEventListener('keydown', handleKeyDown);
    }, [onClose, onNext, onPrev, toggleFullscreen, toggleMute, toggleDebugOverlay]);

    // ── Determine current display state ──────────────────────────────
    const isHlsMode = playbackMode === 'hls';
    const hasVideoTrack = tracks.some(t => t.type === 'video');
    const isMseLoading = msePhase === 'loading' || msePhase === 'initializing';
    const isHlsLoading = hlsPhase === 'preparing' || hlsPhase === 'caching' || hlsPhase === 'transcoding';
    const displayPhase: string = isHlsMode ? hlsPhase : (isMseLoading ? 'loading' : msePhase);
    const displayError: string | null = isHlsMode ? hlsError : mseError;
    const showOriginalVideo = !isHlsMode && !useFallback;

    // Build the effective quality label
    const effectiveQuality: StreamingQuality = isHlsMode ? (hlsQuality || 'original') : currentQuality;

    return (
        <div
            className={`fixed inset-0 z-[200] bg-black/90 animate-in fade-in duration-200 ${isFullscreen ? 'p-0' : 'flex items-center justify-center p-4 backdrop-blur-md'}`}
            onClick={onClose}
        >
            <div ref={containerRef} className={`relative ${isFullscreen ? 'fixed inset-0 w-screen h-screen max-w-none' : 'w-full max-w-6xl flex flex-col items-center'}`} onClick={e => e.stopPropagation()}>
                {/* Nav buttons */}
                <button onClick={onPrev} className={`absolute left-2 top-1/2 -translate-y-1/2 p-2 text-white/50 hover:text-white bg-white/10 hover:bg-white/20 rounded-full transition-all z-10 ${isFullscreen ? 'left-4' : ''}`} title="Previous (ArrowLeft / J)">
                    <ChevronLeft className="w-6 h-6" />
                </button>
                <button onClick={onNext} className={`absolute right-2 top-1/2 -translate-y-1/2 p-2 text-white/50 hover:text-white bg-white/10 hover:bg-white/20 rounded-full transition-all z-10 ${isFullscreen ? 'right-4' : ''}`} title="Next (ArrowRight / L)">
                    <ChevronRight className="w-6 h-6" />
                </button>
                <div className={`absolute z-30 flex items-center gap-2 ${isFullscreen ? 'top-4 right-4' : '-top-12 right-0'}`}>
                    <button
                        onClick={toggleFullscreen}
                        className="w-10 h-10 flex items-center justify-center text-white/50 hover:text-white bg-white/10 hover:bg-white/20 rounded-full transition-all"
                        title={isFullscreen ? 'Exit fullscreen (F)' : 'Fullscreen (F)'}
                    >
                        {isFullscreen ? <Minimize2 className="w-5 h-5" /> : <Maximize2 className="w-5 h-5" />}
                    </button>
                    <button
                        onClick={onClose}
                        className="w-10 h-10 flex items-center justify-center text-white/50 hover:text-white bg-white/10 hover:bg-white/20 rounded-full transition-all"
                        title="Close (Esc)"
                    >
                        <X className="w-5 h-5" />
                    </button>
                </div>

                {/* Video container */}
                <div className={`bg-black overflow-hidden flex items-center justify-center relative ${isFullscreen ? 'w-full h-full rounded-none shadow-none ring-0' : 'w-full aspect-video rounded-xl shadow-2xl ring-1 ring-white/10'}`}>
                    {/* fMP4 remux loading overlay */}
                    {fmp4Remuxing && (
                        <div className="flex flex-col items-center gap-4 text-white absolute inset-0 bg-black/80 z-10">
                            <Zap className="w-10 h-10 text-telegram-primary animate-pulse" />
                            <div className="flex flex-col items-center gap-1">
                                <p className="text-sm font-medium">Converting to streaming format...</p>
                                <p className="text-[11px] text-white/30 mt-1">
                                    Remuxing MP4 for optimal playback. This only happens once per file.
                                </p>
                            </div>
                        </div>
                    )}

                    {/* fMP4 remux error */}
                    {fmp4RemuxError && !fmp4Remuxing && (
                        <div className="flex flex-col items-center gap-3 text-white absolute inset-0 bg-black/80 z-10">
                            <AlertTriangle className="w-10 h-10 text-amber-400" />
                            <p className="text-sm text-amber-400 font-medium">Streaming conversion failed</p>
                            <p className="text-xs text-white/40 text-center max-w-md">
                                {fmp4RemuxError}
                            </p>
                            <p className="text-[11px] text-white/20">Falling back to native video player...</p>
                        </div>
                    )}

                    {/* Error display */}
                    {(displayPhase === 'error' || displayPhase === 'failed') && (
                        <div className="flex flex-col items-center gap-3 text-white px-8">
                            <AlertTriangle className="w-10 h-10 text-red-400" />
                            <p className="text-sm text-red-400 font-medium">Playback Error</p>
                            <p className="text-xs text-white/40 text-center max-w-md">{displayError || 'Unknown error'}</p>
                            {isHlsMode && (
                                <button onClick={retryTranscode} className="mt-2 flex items-center gap-2 px-3 py-1.5 bg-white/10 hover:bg-white/20 rounded-lg text-xs font-medium transition-colors">
                                    <RefreshCw className="w-3.5 h-3.5" />
                                    Retry
                                </button>
                            )}
                        </div>
                    )}

                    {/* Loading overlay */}
                    {isHlsLoading && (
                        <div className="flex flex-col items-center gap-4 text-white absolute inset-0 bg-black/80 z-10">
                            <Loader2 className="w-10 h-10 text-telegram-primary animate-spin" />
                            <div className="flex flex-col items-center gap-1">
                                <p className="text-sm font-medium">
                                    {hlsPhase === 'preparing' ? `Preparing ${hlsQuality}...` :
                                     hlsPhase === 'caching' ? 'Downloading source...' :
                                     hlsPhase === 'transcoding' ? `Transcoding to ${hlsQuality}...` : ''}
                                </p>
                                {hlsProgress > 0 && (
                                    <div className="flex items-center gap-2">
                                        <div className="w-32 h-1.5 bg-white/10 rounded-full overflow-hidden">
                                            <div className="h-full bg-telegram-primary rounded-full transition-all duration-300" style={{ width: `${Math.round(hlsProgress * 100)}%` }} />
                                        </div>
                                        <span className="text-[11px] text-white/40">{Math.round(hlsProgress * 100)}%</span>
                                    </div>
                                )}
                                {hlsPhase === 'preparing' && <p className="text-[11px] text-white/30 mt-1">Starting transcode job...</p>}
                            </div>
                            <button
                                onClick={cancelTranscode}
                                className="mt-3 flex items-center gap-1.5 px-3 py-1.5 bg-white/10 hover:bg-red-500/20 text-white/60 hover:text-red-400 rounded-lg text-xs font-medium transition-all border border-white/10 hover:border-red-500/30"
                                title="Cancel transcode"
                            >
                                <StopCircle className="w-3.5 h-3.5" />
                                Cancel
                            </button>
                        </div>
                    )}

                    {/* MSE loading overlay (original mode) */}
                    {showOriginalVideo && isMseLoading && (
                        <div className="flex flex-col items-center gap-4 text-white absolute inset-0 bg-black/80 z-10">
                            <Loader2 className="w-10 h-10 text-telegram-primary animate-spin" />
                            <div className="flex flex-col items-center gap-1">
                                <p className="text-sm font-medium">Loading video</p>
                                {loadProgress > 0 && (
                                    <div className="flex items-center gap-2">
                                        <div className="w-32 h-1.5 bg-white/10 rounded-full overflow-hidden">
                                            <div className="h-full bg-telegram-primary rounded-full transition-all duration-300" style={{ width: `${loadProgress}%` }} />
                                        </div>
                                        <span className="text-[11px] text-white/40">{loadProgress}%</span>
                                    </div>
                                )}
                                <p className="text-[11px] text-white/30 mt-1">
                                    {msePhase === 'initializing' ? 'Initializing decoder...' : 'Parsing video metadata...'}
                                </p>
                            </div>
                        </div>
                    )}

                    {/* Fallback: native <video> (non-MP4 or no MSE support) */}
                    {useFallback && !isHlsMode && (
                        <video src={fallbackUrl} controls controlsList="nodownload" autoPlay className="w-full h-full object-contain" />
                    )}

                    {/* HLS video element — rendered as soon as HLS mode is active so attachMedia works */}
                    {isHlsMode && (
                        <video
                            ref={hlsVideoCallbackRef}
                            controls={hlsPhase === 'ready'}
                            controlsList="nodownload"
                            autoPlay
                            className={`w-full h-full object-contain ${hlsPhase === 'ready' ? 'opacity-100' : 'opacity-0 absolute inset-0 pointer-events-none'}`}
                        />
                    )}

                    {/* MSE video element (original mode, hidden during loading/error/HLS) */}
                    {showOriginalVideo && (
                        <video
                            ref={mseVideoRef}
                            controls
                            controlsList="nodownload"
                            autoPlay
                            className={`w-full h-full object-contain ${(isMseLoading || msePhase === 'error') ? 'opacity-0' : 'opacity-100'}`}
                        />
                    )}
                </div>

                {/* Quality overlay badge on video */}
                {(displayPhase === 'playing' || displayPhase === 'ready') && (
                    <div className={`absolute ${isFullscreen ? 'bottom-16 left-4' : 'top-3 right-3'} z-20 flex items-center gap-2`}>
                        {/* Quality / mode badge */}
                        <div className="px-2.5 py-1 rounded-full bg-black/60 backdrop-blur-sm border border-white/10 text-xs font-medium text-white/90 shadow-lg pointer-events-none">
                            {isHlsMode && hlsQuality ? QUALITY_LABELS[hlsQuality] : `${effectiveQuality === 'original' ? 'Original' : QUALITY_LABELS[effectiveQuality]}${measuredKbps > 0 && effectiveQuality !== 'original' ? ` · ${(measuredKbps / 1000).toFixed(0)}k` : ''}`}
                        </div>
                        {/* Resolution badge */}
                        {(sourceResolution || playingResolution) && (
                            <div className="px-2.5 py-1 rounded-full bg-black/60 backdrop-blur-sm border border-white/10 text-[10px] font-medium text-white/70 shadow-lg pointer-events-none flex items-center gap-1.5">
                                {sourceResolution && (
                                    <span>Source: {sourceResolution.w}×{sourceResolution.h}</span>
                                )}
                                {playingResolution && (!sourceResolution || playingResolution.w !== sourceResolution.w || playingResolution.h !== sourceResolution.h) && (
                                    <span>· Playing: {playingResolution.w}×{playingResolution.h}</span>
                                )}
                            </div>
                        )}
                    </div>
                )}

                {/* Debug overlay (lower-left, toggle with D key) */}
                {debugOverlay && (
                    <div className={`absolute z-40 pointer-events-none ${isFullscreen ? 'bottom-20 left-4' : 'bottom-4 left-4'}`}>
                        <div className="px-3 py-2 rounded-lg bg-black/80 backdrop-blur-sm border border-white/10 text-[10px] font-mono text-white/80 shadow-xl space-y-1">
                            <div className="flex items-center gap-2 text-[11px] font-semibold text-white/60 mb-0.5">
                                <Activity className="w-3 h-3" />
                                Debug
                            </div>
                            <div className="flex justify-between gap-4">
                                <span className="text-white/40">Speed</span>
                                <span>{isHlsMode ? '—' : measuredKbps > 999 ? `${(measuredKbps / 1000).toFixed(1)} Mbps` : `${Math.round(measuredKbps)} Kbps`}</span>
                            </div>
                            <div className="flex justify-between gap-4">
                                <span className="text-white/40">Cap</span>
                                <span>{effectiveQuality === 'original' ? 'Unlimited' : `${QUALITY_LABELS[effectiveQuality]}${!transcodeCapabilities?.available ? ' (throttle)' : ''}`}</span>
                            </div>
                            <div className="flex justify-between gap-4">
                                <span className="text-white/40">Buffered</span>
                                <span>{debugBufferedSecs.toFixed(1)}s</span>
                            </div>
                            <div className="flex justify-between gap-4">
                                <span className="text-white/40">Mode</span>
                                <span className={isHlsMode ? 'text-emerald-400' : effectiveQuality !== 'original' && !transcodeCapabilities?.available ? 'text-amber-400' : 'text-white/60'}>
                                    {isHlsMode ? 'HLS' : effectiveQuality !== 'original' && !transcodeCapabilities?.available ? 'Bandwidth capped' : 'Original'}
                                </span>
                            </div>
                            {(sourceResolution || playingResolution) && (
                                <div className="flex justify-between gap-4">
                                    <span className="text-white/40">Size</span>
                                    <span>
                                        {playingResolution ? `${playingResolution.w}×${playingResolution.h}` : ''}
                                        {playingResolution && sourceResolution && (playingResolution.w !== sourceResolution.w || playingResolution.h !== sourceResolution.h)
                                            ? ` (src: ${sourceResolution.w}×${sourceResolution.h})`
                                            : sourceResolution ? ` ${sourceResolution.w}×${sourceResolution.h}` : ''}
                                    </span>
                                </div>
                            )}
                            {/* Clear transcode cache button */}
                            <div className="pt-1.5 mt-1 border-t border-white/5">
                                <button
                                    onClick={handleClearTranscodeCache}
                                    disabled={clearingCache}
                                    className="pointer-events-auto flex items-center gap-1.5 text-[9px] text-red-400/60 hover:text-red-400 transition-colors disabled:opacity-50"
                                    title={`Clear all transcoded HLS variants for ${fileKey}`}
                                >
                                    <Trash2 className="w-3 h-3" />
                                    {clearingCache ? 'Clearing...' : 'Clear Transcodes'}
                                </button>
                            </div>
                        </div>
                    </div>
                )}

                {/* Fullscreen overlay toolbar */}
                {isFullscreen && (displayPhase !== 'error' && displayPhase !== 'failed') && (
                    <div className="absolute bottom-0 left-0 right-0 z-30 bg-gradient-to-t from-black/90 via-black/60 to-transparent p-4 pt-12 pointer-events-none">
                        <div className="flex items-center justify-between pointer-events-auto">
                            <div className="flex items-center gap-3">
                                {/* Play/Pause */}
                                <button
                                    onClick={() => {
                                        const video = hlsVideoRef.current || mseVideoRef.current;
                                        if (video) video.paused ? video.play().catch(() => {}) : video.pause();
                                    }}
                                    className="p-2 text-white/80 hover:text-white hover:bg-white/10 rounded-full transition-all"
                                    title="Play/Pause (Space)"
                                >
                                    <Play className="w-5 h-5" />
                                </button>

                                {/* Volume */}
                                <div className="flex items-center gap-1.5"
                                    onMouseEnter={() => setShowVolumeSlider(true)}
                                    onMouseLeave={() => setShowVolumeSlider(false)}
                                >
                                    <button onClick={toggleMute} className="p-1.5 text-white/60 hover:text-white rounded-full hover:bg-white/10" title={isMuted ? 'Unmute' : 'Mute'}>
                                        {isMuted || volume === 0 ? <VolumeX className="w-4 h-4" /> : volume < 0.5 ? <Volume1 className="w-4 h-4" /> : <Volume2 className="w-4 h-4" />}
                                    </button>
                                    <div className={`overflow-hidden transition-all duration-200 ${showVolumeSlider ? 'w-20 opacity-100' : 'w-0 opacity-0'}`}>
                                        <input
                                            type="range"
                                            min={0} max={1} step={0.05}
                                            value={isMuted ? 0 : volume}
                                            onChange={(e) => handleVolumeChange(parseFloat(e.target.value))}
                                            className="w-20 h-1 appearance-none bg-white/20 rounded-full cursor-pointer [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:w-3 [&::-webkit-slider-thumb]:h-3 [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-white"
                                        />
                                    </div>
                                </div>

                                {/* File name */}
                                <span className="text-sm text-white/80 truncate max-w-[200px]">{file.name}</span>
                            </div>

                            <div className="flex items-center gap-2">
                                {/* Quality selector in fullscreen */}
                                {!useFallback && (
                                    <QualitySelector
                                        currentQuality={effectiveQuality}
                                        onChange={handleQualityChange}
                                        adaptiveMode={adaptiveMode}
                                        onToggleAdaptive={() => setAdaptiveMode(!adaptiveMode)}
                                        measuredSpeedKbps={measuredKbps}
                                        transcodeCapabilities={transcodeCapabilities}
                                        variantStates={hlsVariantStates}
                                        sourceHeight={sourceHeight}
                                    />
                                )}

                                {/* Exit fullscreen */}
                                <button
                                    onClick={exitFullscreen}
                                    className="w-10 h-10 flex items-center justify-center text-white/60 hover:text-white hover:bg-white/10 rounded-full transition-all"
                                    title="Exit fullscreen (F)"
                                >
                                    <Minimize2 className="w-5 h-5" />
                                </button>

                                {/* Close */}
                                <button
                                    onClick={onClose}
                                    className="w-10 h-10 flex items-center justify-center text-white/60 hover:text-white hover:bg-white/10 rounded-full transition-all"
                                    title="Close (Esc)"
                                >
                                    <X className="w-5 h-5" />
                                </button>
                            </div>
                        </div>
                    </div>
                )}

                {/* Bottom bar: file info + volume + quality selector (non-fullscreen) */}
                {!isFullscreen && <div className="mt-3 w-full flex items-center justify-between px-1 gap-3">
                    <div className="text-left min-w-0 flex-1">
                        <h3 className="text-sm font-medium text-white truncate max-w-md">{file.name}</h3>
                        <p className="text-[11px] text-white/40 flex items-center gap-2">
                            {isHlsMode && hlsQuality && (
                                <span className="text-telegram-primary">{QUALITY_LABELS[hlsQuality]}</span>
                            )}
                            {hasVideoTrack && !isHlsMode && tracks.find(t => t.type === 'video') && (
                                <span>
                                    {tracks.find(t => t.type === 'video')?.width}p

                                </span>
                            )}
                            {typeof currentIndex === 'number' && typeof totalItems === 'number' && totalItems > 0 && (
                                <span>{currentIndex + 1}/{totalItems}</span>
                            )}
                        </p>
                    </div>

                    {/* Mode indicator + Quality selector */}
                    <div className="flex items-center gap-2">
                        {/* Playback mode badge */}
                        <span className={`text-[10px] font-medium px-1.5 py-0.5 rounded border ${
                            isHlsMode
                                ? 'text-emerald-400 bg-emerald-500/10 border-emerald-500/20'
                                : effectiveQuality !== 'original' && !transcodeCapabilities?.available
                                    ? 'text-amber-400 bg-amber-500/10 border-amber-500/20'
                                    : 'text-white/40 bg-white/5 border-white/10'
                        }`}>
                            {isHlsMode ? 'HLS' : effectiveQuality !== 'original' && !transcodeCapabilities?.available ? 'Throttled' : 'Original'}
                        </span>
                    {!useFallback && displayPhase !== 'error' && displayPhase !== 'failed' && (
                        <QualitySelector
                            currentQuality={effectiveQuality}
                            onChange={handleQualityChange}
                            adaptiveMode={adaptiveMode}
                            onToggleAdaptive={() => setAdaptiveMode(!adaptiveMode)}
                            measuredSpeedKbps={measuredKbps}
                            transcodeCapabilities={transcodeCapabilities}
                            variantStates={hlsVariantStates}
                            sourceHeight={sourceHeight}
                        />
                    )}
                    </div>
                </div>}

                {/* Keyboard shortcut hints */}
                {!isFullscreen && <div className="mt-2 flex items-center gap-4 text-[10px] text-white/25 select-none">
                    <span className="flex items-center gap-1">
                        <kbd className="px-1 py-0.5 rounded bg-white/10 text-white/40 text-[9px] font-mono">← →</kbd> Navigate
                    </span>
                    <span className="flex items-center gap-1">
                        <kbd className="px-1 py-0.5 rounded bg-white/10 text-white/40 text-[9px] font-mono">Space</kbd> Play/Pause
                    </span>
                    <span className="flex items-center gap-1">
                        <kbd className="px-1 py-0.5 rounded bg-white/10 text-white/40 text-[9px] font-mono">F</kbd> Fullscreen
                    </span>
                    <span className="flex items-center gap-1">
                        <kbd className="px-1 py-0.5 rounded bg-white/10 text-white/40 text-[9px] font-mono">Esc</kbd> Close
                    </span>
                    <span className="flex items-center gap-1">
                        <kbd className="px-1 py-0.5 rounded bg-white/10 text-white/40 text-[9px] font-mono">M</kbd> Mute
                    </span>
                    <span className="flex items-center gap-1">
                        <kbd className="px-1 py-0.5 rounded bg-white/10 text-white/40 text-[9px] font-mono">D</kbd> Debug
                    </span>
                </div>}
            </div>
        </div>
    );
}
