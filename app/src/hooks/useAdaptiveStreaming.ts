import { useRef, useState, useEffect, useCallback } from 'react';
import { createFile, MP4BoxBuffer } from 'mp4box';
import type { ISOFile } from 'mp4box';
import {
    StreamingQuality,
    VideoTrackInfo,
    QUALITY_THROTTLE_MAP,
    ADAPTIVE_THRESHOLDS,
} from '../types';
import { useStreamingSettings } from './useStreamingSettings';
import { getCachedMoov, setCachedMoov, extractCacheKey } from './moovCache';
import { redactSensitiveValue } from '../security/redaction';

// ── Types ────────────────────────────────────────────────────────────

interface Mp4Track {
    id: number;
    type: string;
    codec: string;
    timescale: number;
    duration: number;
    bitrate: number;
    video?: { width: number; height: number };
    audio?: { sample_rate: number; channel_count: number };
}

interface Mp4MovieInfo {
    tracks: Mp4Track[];
    duration: number;
    timescale: number;
    isFragmented: boolean;
    isProgressive: boolean;
    hasIOD: boolean;
    brands: string[];
}

export type PlayerPhase =
    | 'initializing'
    | 'loading'
    | 'ready'
    | 'playing'
    | 'seeking'
    | 'ended'
    | 'error';

interface PlayerState {
    phase: PlayerPhase;
    error: string | null;
    tracks: VideoTrackInfo[];
    loadProgress: number;
    measuredKbps: number;
}

const FALLBACK_EXTENSIONS = ['webm', 'ogg', 'mov', 'mkv', 'avi'];
const SPEED_WINDOW_MS = 3000;
const SPEED_CHECK_INTERVAL_MS = 2000;
const SEEK_DEBOUNCE_MS = 300;
const MOOV_DISCOVERY_BYTES = 131072;   // 128KB — covers ftyp + moov for most files
const MOOV_RETRY_BYTES = 524288;       // 512KB — retry with larger range before tail lookup
const MOOV_TAIL_BYTES = 524288;        // 512KB — tail fetch for moov-at-end files
const MOOV_FALLBACK_TIMEOUT_MS = 3000;
const PROGRESSIVE_CACHE_WARM_BYTES = 3145728; // 3MB — pre-warm HTTP cache for native <video> moov discovery

// ── Pre-warm browser HTTP cache for progressive MP4 moov tail ─────
// When a progressive (non-fragmented) MP4 is detected and we fall back to
// native <video>, the native player must make a Range request to the end
// of the file to find the moov atom. This round-trip through the backend
// to Telegram is slow, causing buffering delays on large files.
//
// By pre-fetching the tail ourselves before triggering the fallback, we
// populate the browser's HTTP cache. When the native player requests the
// same range moments later, it's a cache hit and playback initializes
// almost instantly.
function warmProgressiveMoovCache(streamUrl: string, fileSize: number): void {
    const warmBytes = Math.min(PROGRESSIVE_CACHE_WARM_BYTES, fileSize);
    const tailStart = Math.max(0, fileSize - warmBytes);

    console.log('[AdaptiveStreaming] 🔥 warmProgressiveMoovCache: pre-fetching last', warmBytes, 'bytes from offset', tailStart);

    fetch(streamUrl, {
        headers: { Range: `bytes=${tailStart}-` },
        // Fire-and-forget: we don't need the result — only need the
        // browser's HTTP cache to be populated for the native <video>
        // element that will mount moments later.
        cache: 'default',
    }).then(() => {
        console.log('[AdaptiveStreaming] 🔥 warmProgressiveMoovCache: cache warmed successfully');
    }).catch(() => {
        // Best-effort: failure is non-critical
    });
}

// ── Extract just the moov atom from raw MP4 bytes ──────────────────
// Scans for the 'moov' box fourcc, validates its size, and returns
// the isolated box data with its absolute file offset. Used for
// moov-at-end files to avoid feeding non-contiguous mdat fragments
// that cause mp4box parsing errors on Windows WebView2.
function extractMoovAtom(
    data: ArrayBuffer,
    dataOffset: number,
): { moovData: ArrayBuffer; moovOffset: number } | null {
    const view = new DataView(data);
    let offset = 0;
    while (offset + 12 <= data.byteLength) {
        const boxSize = view.getUint32(offset); // big-endian u32
        // Skip zeros and impossibly small boxes
        if (boxSize === 0 || boxSize < 8) {
            offset += 4;
            continue;
        }
        const fourcc = String.fromCharCode(
            view.getUint8(offset + 4),
            view.getUint8(offset + 5),
            view.getUint8(offset + 6),
            view.getUint8(offset + 7),
        );
        if (fourcc === 'moov') {
            // Validate: box size must fit within buffer (don't extract partial moov)
            if (offset + boxSize > data.byteLength) {
                console.warn('[AdaptiveStreaming] 📦 extractMoovAtom: partial moov box (size', boxSize, '> remaining', data.byteLength - offset, '), skipping');
                return null;
            }
            // Sanity: box size should not exceed 64MB (unrealistic for moov)
            if (boxSize > 64 * 1024 * 1024) {
                console.warn('[AdaptiveStreaming] 📦 extractMoovAtom: moov box implausibly large (', boxSize, 'bytes), skipping');
                return null;
            }
            // Verify the first child box is mvhd (Movie Header) to rule out
            // false positives where 'moov' bytes appear in media data
            const childStart = offset + 8;
            if (childStart + 8 <= data.byteLength) {
                const childFourcc = String.fromCharCode(
                    view.getUint8(childStart + 4),
                    view.getUint8(childStart + 5),
                    view.getUint8(childStart + 6),
                    view.getUint8(childStart + 7),
                );
                if (childFourcc !== 'mvhd') {
                    // False positive — 'moov' bytes inside media data.
                    // Advance by boxSize (validated ≥8 above) to stay box-aligned.
                    offset += boxSize;
                    continue;
                }
            }
            const moovData = data.slice(offset, offset + boxSize);
            return { moovData, moovOffset: dataOffset + offset };
        }
        // Advance by box size for well-formed boxes, otherwise step by 4
        if (boxSize >= 8 && offset + boxSize <= data.byteLength) {
            offset += boxSize;
        } else {
            offset += 4;
        }
    }
    return null;
}

function isMp4File(name: string): boolean {
    return name.toLowerCase().endsWith('.mp4');
}

function shouldUseFallback(name: string): boolean {
    const lower = name.toLowerCase();
    return FALLBACK_EXTENSIONS.some(ext => lower.endsWith(`.${ext}`));
}

function mseSupported(): boolean {
    return typeof MediaSource !== 'undefined' && MediaSource.isTypeSupported('video/mp4; codecs="avc1.42E01E,mp4a.40.2"');
}

export interface UseAdaptiveStreamingResult {
    videoRef: React.RefObject<HTMLVideoElement | null>;
    phase: PlayerPhase;
    error: string | null;
    tracks: VideoTrackInfo[];
    loadProgress: number;
    currentQuality: StreamingQuality;
    setQuality: (q: StreamingQuality) => void;
    adaptiveMode: boolean;
    setAdaptiveMode: (enabled: boolean) => void;
    measuredKbps: number;
    seek: (time: number) => void;
    useFallback: boolean;
    fallbackUrl: string;
    abort: () => void;
}

export function useAdaptiveStreaming(
    streamUrl: string,
    fileName: string,
    onProgressiveDetected?: () => void,
): UseAdaptiveStreamingResult {
    // ── Ref-based mutable state ──────────────────────────────────────
    const mp4boxRef = useRef<ISOFile | null>(null);
    const mediaSourceRef = useRef<MediaSource | null>(null);
    const sourceBuffersRef = useRef<{ [trackId: number]: SourceBuffer }>({});
    const abortRef = useRef<AbortController | null>(null);
    const discoveryAbortRef = useRef<AbortController | null>(null);
    const fetchOffsetRef = useRef(0);
    const totalFetchedRef = useRef(0);
    const throttleBpsRef = useRef(0);
    const playerPhaseRef = useRef<PlayerPhase>('initializing');
    const isSeekingRef = useRef(false);
    const pendingSeekTimeRef = useRef<number | null>(null);
    const speedSamplesRef = useRef<{ time: number; bytes: number }[]>([]);
    const fileSizeRef = useRef<number>(0);
    const appendQueuesRef = useRef<{ [trackId: number]: ArrayBuffer[] }>({});
    const lastSampleNumRef = useRef<number>(0);
    const tracksRef = useRef<VideoTrackInfo[]>([]);
    const onReadyCalledRef = useRef(false);
    const moovEndOffsetRef = useRef(0);
    // Original discovery bytes — fed to the fresh playback file so it
    // boots with real source MP4 data, not generated fMP4 init segment.
    const discoveryPrefixRef = useRef<ArrayBuffer | null>(null);
    const discoveryNextOffsetRef = useRef(0);
    // Tail data for moov-at-end files (stored separately with correct offset)
    const discoverySuffixRef = useRef<ArrayBuffer | null>(null);
    const discoverySuffixOffsetRef = useRef(0);
    // Flag set synchronously by initSegments when initializeSegmentation crashes,
    // allowing the onReady caller to bail out before calling startDownload.
    const segmentationFailedRef = useRef(false);

    // ── React state (UI-relevant only) ───────────────────────────────
    const videoRef = useRef<HTMLVideoElement>(null);
    const [state, setState] = useState<PlayerState>({
        phase: 'initializing',
        error: null,
        tracks: [],
        loadProgress: 0,
        measuredKbps: 0,
    });

    const { settings, setQuality, setAdaptiveMode } = useStreamingSettings();

    const needsFallback = !isMp4File(fileName) || shouldUseFallback(fileName) || !mseSupported();
    const [useFallback] = useState(needsFallback);
    // Dynamic fallback: if MSE pipeline fails to init, switch to native <video>
    const [dynamicFallback, setDynamicFallback] = useState(false);
    const effectiveUseFallback = useFallback || dynamicFallback;

    useEffect(() => {
        const kbps = QUALITY_THROTTLE_MAP[settings.quality];
        const bps = kbps > 0 ? kbps * 1024 : 0;
        throttleBpsRef.current = bps;
        console.log('[AdaptiveStreaming] throttle updated', {
            quality: settings.quality,
            throttleBps: bps,
            throttleKbps: kbps,
        });
    }, [settings.quality]);

    // ── Abort helpers ────────────────────────────────────────────────
    const abortFetch = useCallback(() => {
        if (abortRef.current) {
            abortRef.current.abort();
            abortRef.current = null;
        }
    }, []);

    const abortDiscovery = useCallback(() => {
        if (discoveryAbortRef.current) {
            discoveryAbortRef.current.abort();
            discoveryAbortRef.current = null;
        }
    }, []);

    // ── SourceBuffer append queue ────────────────────────────────────
    const drainAppendQueue = useCallback((trackId: number) => {
        const sb = sourceBuffersRef.current[trackId];
        if (!sb || sb.updating) return;
        const queue = appendQueuesRef.current[trackId];
        if (!queue || queue.length === 0) return;
        try {
            sb.appendBuffer(queue.shift()!);
        } catch (e: any) {
            if (e.name === 'QuotaExceededError') {
                sb.addEventListener('updateend', () => drainAppendQueue(trackId), { once: true });
            } else {
                console.warn(`[AdaptiveStreaming] appendBuffer error for track ${trackId}:`, redactSensitiveValue(e));
            }
        }
    }, []);

    const clearSourceBuffer = useCallback(() => {
        const ms = mediaSourceRef.current;
        if (ms && ms.readyState === 'open') {
            for (const idStr in sourceBuffersRef.current) {
                const trackId = parseInt(idStr, 10);
                const sb = sourceBuffersRef.current[trackId];
                if (sb) {
                    try {
                        appendQueuesRef.current[trackId] = [];
                        try { sb.abort(); } catch { /* ignore */ }
                        if (sb.updating) {
                            sb.addEventListener('updateend', () => {
                                try { ms.removeSourceBuffer(sb); } catch { /* ignore */ }
                            }, { once: true });
                        } else {
                            try { ms.removeSourceBuffer(sb); } catch { /* ignore */ }
                        }
                    } catch { /* already removed */ }
                }
            }
        }
        sourceBuffersRef.current = {};
        appendQueuesRef.current = {};
    }, []);

    const createSourceBuffer = useCallback((codec: string, trackId: number, isAudio: boolean): SourceBuffer | null => {
        const ms = mediaSourceRef.current;
        if (!ms || ms.readyState !== 'open') return null;
        try {
            // mp4box returns raw codec strings like "avc1.42E01E" — wrap in full MIME type
            const container = isAudio ? 'audio/mp4' : 'video/mp4';
            const mimeType = codec.includes('/') ? codec : `${container}; codecs="${codec}"`;
            const sb = ms.addSourceBuffer(mimeType);
            sb.addEventListener('updateend', () => drainAppendQueue(trackId));
            sb.addEventListener('error', () => console.warn(`[AdaptiveStreaming] SourceBuffer error for track ${trackId}`));
            return sb;
        } catch (e) {
            console.error(`[AdaptiveStreaming] Failed to create SourceBuffer for track ${trackId}:`, redactSensitiveValue(e));
            return null;
        }
    }, [drainAppendQueue]);

    // ── Start progressive download ───────────────────────────────────
    const startDownload = useCallback((fromOffset: number) => {
        if (!streamUrl || !mp4boxRef.current) return;

        console.log('[AdaptiveStreaming] ⬇️ startDownload from offset=', fromOffset);
        abortFetch();
        const mp4boxfile = mp4boxRef.current;
        const abortController = new AbortController();
        abortRef.current = abortController;
        fetchOffsetRef.current = fromOffset;

        (async () => {
            try {
                const rangeHeader = fromOffset > 0 ? `bytes=${fromOffset}-` : 'bytes=0-';
                const response = await fetch(streamUrl, {
                    headers: { Range: rangeHeader },
                    signal: abortController.signal,
                });

                if (fileSizeRef.current === 0) {
                    const cr = response.headers.get('content-range');
                    if (cr) {
                        const m = cr.match(/\/(\d+)/);
                        if (m) fileSizeRef.current = parseInt(m[1], 10);
                    }
                    if (fileSizeRef.current === 0) {
                        const cl = response.headers.get('content-length');
                        if (cl) fileSizeRef.current = parseInt(cl, 10);
                    }
                }

                if (!response.body) throw new Error('No response body');

                console.log('[AdaptiveStreaming] ⬇️ download started, reading stream...');
                const reader = response.body.getReader();
                const downloadStartTime = performance.now();
                totalFetchedRef.current = 0;
                speedSamplesRef.current = [];

                while (true) {
                    const { done, value } = await reader.read();
                    if (done) { console.log('[AdaptiveStreaming] ⬇️ stream complete'); break; }
                    if (abortController.signal.aborted) break;

                    const chunkBuffer = value.buffer.slice(
                        value.byteOffset,
                        value.byteOffset + value.byteLength,
                    );

                    const throttleBps = throttleBpsRef.current;
                    totalFetchedRef.current += chunkBuffer.byteLength;
                    if (throttleBps > 0) {
                        const expectedMs = (totalFetchedRef.current / throttleBps) * 1000;
                        const actualMs = performance.now() - downloadStartTime;
                        if (expectedMs > actualMs) {
                            await new Promise<void>(r => {
                                const timeout = setTimeout(r, expectedMs - actualMs);
                                abortController.signal.addEventListener('abort', () => {
                                    clearTimeout(timeout);
                                    r();
                                });
                            });
                        }
                    }

                    if (abortController.signal.aborted) break;

                    const fileStart = fetchOffsetRef.current;
                    const mp4boxBuffer = MP4BoxBuffer.fromArrayBuffer(chunkBuffer, fileStart);
                    const nextOffset = mp4boxfile.appendBuffer(mp4boxBuffer);
                    // ── Handle backward nextOffset (mp4box wants to rewind) ──
                    // For moov-at-end progressive files, the onReady callback
                    // already triggers fMP4 remux before we get here.
                    // For fragmented MP4s, mp4box may request an earlier offset
                    // for interleaved tracks — we can't rewind the HTTP stream,
                    // so just continue feeding sequential data. Each fMP4 fragment
                    // is self-contained so playback continues fine.
                    if (nextOffset < fileStart) {
                        console.warn('[AdaptiveStreaming] ⚠️ mp4box nextOffset regressed from', fileStart, 'to', nextOffset, '— continuing sequential download');
                        fetchOffsetRef.current = fileStart + chunkBuffer.byteLength;
                    } else {
                        fetchOffsetRef.current = nextOffset;
                    }

                    if (fileSizeRef.current > 0 && playerPhaseRef.current === 'loading') {
                        setState(s => ({
                            ...s,
                            loadProgress: Math.min(99, Math.round((nextOffset / fileSizeRef.current) * 100)),
                        }));
                    }

                    const now = performance.now();
                    speedSamplesRef.current.push({ time: now, bytes: chunkBuffer.byteLength });
                    speedSamplesRef.current = speedSamplesRef.current.filter(
                        s => now - s.time < SPEED_WINDOW_MS,
                    );
                }

                if (playerPhaseRef.current !== 'seeking' && playerPhaseRef.current !== 'error') {
                    // Flush mp4box to emit any remaining partial segment,
                    // critical for short videos where the last segment may be incomplete.
                    try { mp4boxfile.flush(); } catch { /* best-effort */ }
                    console.log('[AdaptiveStreaming] ⬇️ download complete, mp4box flushed');
                    // Do NOT set phase to 'ended' — download completion ≠ playback completion.
                    // The <video> element's 'ended' event or MediaSource 'sourceended' handles that.
                    setState(s => ({ ...s, loadProgress: 100 }));
                }
            } catch (err: any) {
                if (err?.name === 'AbortError') return;
                console.error('[AdaptiveStreaming] Download error:', redactSensitiveValue(err));
                if (playerPhaseRef.current !== 'error') {
                    playerPhaseRef.current = 'error';
                    setState(s => ({ ...s, phase: 'error', error: String(err) }));
                    // Fall back to native <video> — download stream failed
                    setDynamicFallback(true);
                }
            }
        })();
    }, [streamUrl, abortFetch]);

    // ── Quick moov discovery: fetch first 128KB to trigger onReady fast ──
    const discoverMoov = useCallback(async (mp4boxfile: ISOFile, signal: AbortSignal) => {
        console.log('[AdaptiveStreaming] 🔍 discoverMoov: fetching first 128KB...');
        try {
            const resp = await fetch(streamUrl, {
                headers: { Range: `bytes=0-${MOOV_DISCOVERY_BYTES - 1}` },
                signal,
            });
            console.log('[AdaptiveStreaming] 🔍 discoverMoov: response status=', resp.status, 'ok=', resp.ok);
            if (!resp.ok || !resp.body || signal.aborted) return;

            // Extract file size from Content-Range header
            if (fileSizeRef.current === 0) {
                const cr = resp.headers.get('content-range');
                if (cr) {
                    const m = cr.match(/\/(\d+)/);
                    if (m) fileSizeRef.current = parseInt(m[1], 10);
                }
                if (fileSizeRef.current === 0) {
                    const cl = resp.headers.get('content-length');
                    if (cl) fileSizeRef.current = parseInt(cl, 10);
                }
            }

            const data = await resp.arrayBuffer();
            if (signal.aborted || onReadyCalledRef.current) return;

            console.log('[AdaptiveStreaming] 🔍 discoverMoov: got', data.byteLength, 'bytes, feeding to mp4box...');
            const mp4boxBuffer = MP4BoxBuffer.fromArrayBuffer(data, 0);
            const nextOffset = mp4boxfile.appendBuffer(mp4boxBuffer);
            console.log('[AdaptiveStreaming] 🔍 discoverMoov: mp4box nextOffset=', nextOffset);
            moovEndOffsetRef.current = nextOffset > 0 ? nextOffset : MOOV_DISCOVERY_BYTES;
            // Save original bytes for the fresh playback file
            discoveryPrefixRef.current = data.slice(0);
            discoveryNextOffsetRef.current = nextOffset || data.byteLength;
        } catch (err: any) {
            if (err?.name !== 'AbortError') {
                console.warn('[AdaptiveStreaming] Moov discovery error:', redactSensitiveValue(err));
            }
        }
    }, [streamUrl]);

    // ── Retry moov discovery: extend to 512KB when first 128KB fails ──
    const discoverMoovRetry = useCallback(async (mp4boxfile: ISOFile, signal: AbortSignal) => {
        console.log('[AdaptiveStreaming] 🔄 discoverMoovRetry: extending range from 128KB to 512KB...');
        if (fileSizeRef.current > 0 && fileSizeRef.current <= MOOV_DISCOVERY_BYTES) {
            console.log('[AdaptiveStreaming] 🔄 discoverMoovRetry: file too small for retry, skipping');
            return;
        }
        try {
            const resp = await fetch(streamUrl, {
                headers: { Range: `bytes=${MOOV_DISCOVERY_BYTES}-${MOOV_RETRY_BYTES - 1}` },
                signal,
            });
            console.log('[AdaptiveStreaming] 🔄 discoverMoovRetry: response status=', resp.status, 'ok=', resp.ok);
            if (!resp.ok || signal.aborted || onReadyCalledRef.current) return;

            const data = await resp.arrayBuffer();
            if (signal.aborted || onReadyCalledRef.current) return;

            console.log('[AdaptiveStreaming] 🔄 discoverMoovRetry: got', data.byteLength, 'gap bytes, feeding to mp4box at offset', MOOV_DISCOVERY_BYTES);
            const mp4boxBuffer = MP4BoxBuffer.fromArrayBuffer(data, MOOV_DISCOVERY_BYTES);
            mp4boxfile.appendBuffer(mp4boxBuffer);

            // Extend discovery prefix: combine old 128KB + gap data → full 512KB range
            const oldPrefix = discoveryPrefixRef.current;
            if (oldPrefix) {
                const combined = new Uint8Array(MOOV_DISCOVERY_BYTES + data.byteLength);
                combined.set(new Uint8Array(oldPrefix), 0);
                combined.set(new Uint8Array(data), MOOV_DISCOVERY_BYTES);
                discoveryPrefixRef.current = combined.buffer;
            }
            discoveryNextOffsetRef.current = MOOV_RETRY_BYTES;
        } catch (err: any) {
            if (err?.name !== 'AbortError') {
                console.warn('[AdaptiveStreaming] Moov retry error:', redactSensitiveValue(err));
            }
        }
    }, [streamUrl]);

    // ── Tail moov discovery: fetch last 512KB to find moov-at-end ───
    const discoverMoovTail = useCallback(async (mp4boxfile: ISOFile, signal: AbortSignal) => {
        console.log('[AdaptiveStreaming] 🦊 discoverMoovTail: fileSize=', fileSizeRef.current);
        if (fileSizeRef.current <= MOOV_DISCOVERY_BYTES + MOOV_TAIL_BYTES) {
            console.log('[AdaptiveStreaming] 🦊 discoverMoovTail: file too small, skipping');
            return;
        }
        try {
            const tailStart = Math.max(0, fileSizeRef.current - MOOV_TAIL_BYTES);
            console.log('[AdaptiveStreaming] 🦊 discoverMoovTail: fetching bytes', tailStart, '- end');
            const resp = await fetch(streamUrl, {
                headers: { Range: `bytes=${tailStart}-` },
                signal,
            });
            console.log('[AdaptiveStreaming] 🦊 discoverMoovTail: response status=', resp.status, 'ok=', resp.ok);
            if (!resp.ok || !resp.body || signal.aborted || onReadyCalledRef.current) return;

            const data = await resp.arrayBuffer();
            if (signal.aborted || onReadyCalledRef.current) return;

            console.log('[AdaptiveStreaming] 🦊 discoverMoovTail: got', data.byteLength, 'bytes, feeding to mp4box at offset', tailStart);
            const mp4boxBuffer = MP4BoxBuffer.fromArrayBuffer(data, tailStart);
            mp4boxfile.appendBuffer(mp4boxBuffer);
            // Save tail data for the fresh playback file (needed for moov-at-end)
            discoverySuffixRef.current = data.slice(0);
            discoverySuffixOffsetRef.current = tailStart;
            moovEndOffsetRef.current = 0;
        } catch (err: any) {
            if (err?.name !== 'AbortError') {
                console.warn('[AdaptiveStreaming] Moov tail discovery error:', redactSensitiveValue(err));
            }
        }
    }, [streamUrl]);

    // ── Initialize segments callback ─────────────────────────────────
    const initSegments = useCallback((mp4boxfile: ISOFile) => {
        const tracks = tracksRef.current;
        console.log('[AdaptiveStreaming] 📐 initSegments starting for tracks:', tracks.map(t => t.id));

        // ── Register onSegment BEFORE initializeSegmentation ──────────
        // mp4box docs require onSegment to be set before segmentation starts.
        mp4boxfile.onSegment = (id: number, _user: unknown, buffer: ArrayBuffer, sampleNum: number, _last: boolean) => {
            lastSampleNumRef.current = sampleNum;
            const currentSb = sourceBuffersRef.current[id];
            if (!currentSb || mediaSourceRef.current?.readyState !== 'open') {
                console.warn('[AdaptiveStreaming] 📐 onSegment dropped — SourceBuffer not ready for track', id);
                return;
            }

            console.log('[AdaptiveStreaming] 📐 onSegment: track=', id, 'sample=', sampleNum, 'bytes=', buffer.byteLength);
            
            if (!appendQueuesRef.current[id]) {
                appendQueuesRef.current[id] = [];
            }
            const queue = appendQueuesRef.current[id];

            if (currentSb.updating) {
                queue.push(buffer);
            } else {
                try {
                    currentSb.appendBuffer(buffer);
                } catch (e: any) {
                    if (e.name === 'QuotaExceededError') {
                        queue.unshift(buffer);
                        currentSb.addEventListener('updateend', () => drainAppendQueue(id), { once: true });
                    } else {
                        console.warn(`[AdaptiveStreaming] appendBuffer failed for track ${id}:`, redactSensitiveValue(e));
                    }
                }
            }

            try { mp4boxfile.releaseUsedSamples(id, sampleNum); } catch { /* best-effort */ }
        };

        const initBuffers: { [trackId: number]: ArrayBuffer } = {};
        let hasInitializedAny = false;

        // Loop through all tracks to generate track-specific isolated initialization segments
        for (const track of tracks) {
            const sb = sourceBuffersRef.current[track.id];
            if (!sb) continue;

            // Clear any other track options to keep this one isolated
            for (const t of tracks) {
                try { (mp4boxfile as any).unsetSegmentOptions(t.id); } catch {}
            }
            (mp4boxfile as any).isFragmentationInitialized = false;

            console.log('[AdaptiveStreaming] 📐 initSegments: generating isolated options for track id=', track.id);
            mp4boxfile.setSegmentOptions(track.id, sb as unknown as object, {
                nbSamples: 30,
                rapAlignement: true,
            });

            try {
                const res = (mp4boxfile as any).initializeSegmentation();
                if (res && res.buffer) {
                    initBuffers[track.id] = res.buffer;
                    hasInitializedAny = true;
                }
            } catch (e) {
                console.warn(`[AdaptiveStreaming] 📐 Failed to generate init segment for track ${track.id}:`, redactSensitiveValue(e));
            }
        }

        // Now restore/set segment options for ALL active tracks together so mp4box segments them during playback
        for (const t of tracks) {
            try { (mp4boxfile as any).unsetSegmentOptions(t.id); } catch {}
        }
        (mp4boxfile as any).isFragmentationInitialized = false;

        for (const track of tracks) {
            const sb = sourceBuffersRef.current[track.id];
            if (sb) {
                mp4boxfile.setSegmentOptions(track.id, sb as unknown as object, {
                    nbSamples: 30,
                    rapAlignement: true,
                });
            }
        }

        if (!hasInitializedAny) {
            console.error('[AdaptiveStreaming] 📐 initSegments: failed to initialize any track segmentation');
            segmentationFailedRef.current = true;
            try { mp4boxfile.stop(); } catch {}
            clearSourceBuffer();
            playerPhaseRef.current = 'ready';
            setState(s => ({ ...s, phase: 'ready' }));
            setDynamicFallback(true);
            return;
        }

        // Initialize multi-track segmentation state on the file
        try {
            (mp4boxfile as any).initializeSegmentation();
        } catch (e) {
            console.error('[AdaptiveStreaming] 📐 Final initializeSegmentation crashed:', redactSensitiveValue(e));
            segmentationFailedRef.current = true;
            try { mp4boxfile.stop(); } catch {}
            clearSourceBuffer();
            playerPhaseRef.current = 'ready';
            setState(s => ({ ...s, phase: 'ready' }));
            setDynamicFallback(true);
            return;
        }

        // Append the isolated track-specific init segments to their respective SourceBuffers
        for (const trackIdStr in initBuffers) {
            const trackId = parseInt(trackIdStr, 10);
            const buf = initBuffers[trackId];
            const sb = sourceBuffersRef.current[trackId];
            if (sb && buf) {
                if (!appendQueuesRef.current[trackId]) {
                    appendQueuesRef.current[trackId] = [];
                }
                appendQueuesRef.current[trackId].push(buf);
                if (!sb.updating) drainAppendQueue(trackId);
            }
        }

        // ── mp4box.start() is REQUIRED for segmentation callbacks to fire ──
        mp4boxfile.start();
    }, [drainAppendQueue, clearSourceBuffer]);

    // ── Build MSE pipeline (shared by onReady and cache-hit paths) ───
    const buildMsePipeline = useCallback((mp4boxfile: ISOFile, tracks: VideoTrackInfo[]) => {
        console.log('[AdaptiveStreaming] 🏗️ buildMsePipeline: creating MediaSource, videoRef.current=', !!videoRef.current);
        const ms = new MediaSource();
        mediaSourceRef.current = ms;

        const openTimeout = setTimeout(() => {
            console.error('[AdaptiveStreaming] 🏗️ MediaSource failed to open within 15s — falling back to native video');
            if (playerPhaseRef.current === 'loading') {
                playerPhaseRef.current = 'error';
                setState(s => ({ ...s, phase: 'error', error: 'MediaSource failed to open (timeout)' }));
                setDynamicFallback(true);
            }
        }, 15000);

        ms.addEventListener('sourceopen', () => {
            console.log('[AdaptiveStreaming] 🏗️ sourceopen fired!');
            clearTimeout(openTimeout);
            const videoTrack = tracks.find(t => t.type === 'video');
            console.log('[AdaptiveStreaming] 🏗️ videoTrack:', videoTrack ? `id=${videoTrack.id} codec=${videoTrack.codec}` : 'NOT FOUND');
            if (!videoTrack?.codec) {
                console.error('[AdaptiveStreaming] 🏗️ No video codec!');
                playerPhaseRef.current = 'error';
                setState(s => ({ ...s, phase: 'error', error: 'No supported video codec' }));
                setDynamicFallback(true);
                return;
            }

            // ── Create a FRESH mp4boxfile for playback ───────────────
            // The scout file already consumed discovery bytes. We create a
            // brand-new file and feed it the ORIGINAL raw MP4 bytes (not a
            // generated fMP4 init segment which would crash mp4box).
            try { mp4boxfile.stop(); } catch {}
            try { mp4boxfile.flush(); } catch {}

            const playbackFile = createFile();
            mp4boxRef.current = playbackFile;

            playbackFile.onError = (_module: string, message: string) => {
                console.error('[AdaptiveStreaming] Playback mp4box error:', message);
                if (playerPhaseRef.current !== 'error') {
                    playerPhaseRef.current = 'error';
                    setState(s => ({ ...s, phase: 'error', error: message }));
                    setDynamicFallback(true);
                }
            };

            // Recreate all source buffers (both video and audio if present)
            let createdCount = 0;
            for (const track of tracks) {
                if (track.codec) {
                    const isAudio = track.type === 'audio';
                    const sb = createSourceBuffer(track.codec, track.id, isAudio);
                    if (sb) {
                        sourceBuffersRef.current[track.id] = sb;
                        createdCount++;
                    }
                }
            }

            const videoSb = sourceBuffersRef.current[videoTrack.id];
            if (!videoSb) {
                playerPhaseRef.current = 'error';
                setState(s => ({ ...s, phase: 'error', error: 'Failed to create video SourceBuffer' }));
                setDynamicFallback(true);
                return;
            }

            const prefix = discoveryPrefixRef.current;
            const suffix = discoverySuffixRef.current;
            const suffixOffset = discoverySuffixOffsetRef.current;
            const isMoovInTail = suffix !== null;

            // Must have at least one data source to proceed
            if (!prefix && !suffix) {
                console.error('[AdaptiveStreaming] 🏗️ No discovery data — falling back to native video');
                playerPhaseRef.current = 'error';
                setState(s => ({ ...s, phase: 'error', error: 'Missing discovery data' }));
                setDynamicFallback(true);
                return;
            }

            // Set onReady BEFORE feeding data — mp4box may fire it synchronously
            // during appendBuffer, so the callback must be registered first.
            playbackFile.onReady = () => {
                console.log('[AdaptiveStreaming] 🏗️ fresh file onReady — setting up segmentation');
                // Patch missing mehd on the *playback* file (the scout file
                // was patched earlier, but this is a fresh mp4box instance
                // with its own moov parsed from the discovery bytes).
                const pbMoov = (playbackFile as any).moov;
                if (pbMoov?.mvex && !pbMoov.mvex.mehd) {
                    console.log('[AdaptiveStreaming] 🏗️ Patching missing mehd on playback file');
                    pbMoov.mvex.mehd = { fragment_duration: 0 };
                }
                initSegments(playbackFile);
                // If initSegments triggered fallback (e.g. initializeSegmentation crash),
                // don't start the download — native <video> will handle playback.
                if (segmentationFailedRef.current) {
                    console.warn('[AdaptiveStreaming] 🏗️ segmentation failed — skipping startDownload');
                    return;
                }
                playerPhaseRef.current = 'playing';
                setState(s => ({ ...s, phase: 'playing' }));
                // For moov-at-end: resume from byte 0 to fill contiguously.
                // For moov-in-header: resume from where the prefix ended.
                const resumeOffset = isMoovInTail ? 0 : (discoveryNextOffsetRef.current || (prefix?.byteLength ?? 0));
                console.log('[AdaptiveStreaming] 🏗️ starting download from offset=', resumeOffset, 'isMoovInTail=', isMoovInTail);
                startDownload(resumeOffset);
            };

            if (isMoovInTail) {
                // ── Moov-at-end: extract ONLY the moov atom from suffix ──
                // Feeding the entire suffix (which includes mid-mdat media bytes) at a
                // non-contiguous offset triggers mp4box parsing errors like:
                //   "Invalid data found while parsing box of type 't] X'"
                // on platforms with different WebView fetch behavior (e.g. Windows).
                const moovAtom = extractMoovAtom(suffix, suffixOffset);
                if (moovAtom) {
                    console.log('[AdaptiveStreaming] 🏗️ moov-at-end: feeding moov atom at offset', moovAtom.moovOffset, 'size', moovAtom.moovData.byteLength);
                    playbackFile.appendBuffer(MP4BoxBuffer.fromArrayBuffer(moovAtom.moovData, moovAtom.moovOffset));
                } else {
                    // Fallback: couldn't isolate moov — feed entire suffix (existing behavior)
                    console.warn('[AdaptiveStreaming] 🏗️ Could not extract moov atom from suffix, feeding full tail as fallback');
                    playbackFile.appendBuffer(MP4BoxBuffer.fromArrayBuffer(suffix.slice(0), suffixOffset));
                }
                discoverySuffixRef.current = null;
            } else if (prefix) {
                // ── Moov-in-header: feed contiguous prefix bytes ──────
                console.log('[AdaptiveStreaming] 🏗️ feeding', prefix.byteLength, 'original bytes at offset 0');
                playbackFile.appendBuffer(MP4BoxBuffer.fromArrayBuffer(prefix.slice(0), 0));
            }

            discoveryPrefixRef.current = null; // free memory
        });

        ms.addEventListener('sourceended', () => {
            playerPhaseRef.current = 'ended';
            setState(s => ({ ...s, phase: 'ended' }));
        });

        if (videoRef.current) {
            videoRef.current.src = URL.createObjectURL(ms);
            // Listen for native video ended event — download completion ≠ playback done
            videoRef.current.addEventListener('ended', () => {
                if (playerPhaseRef.current !== 'error') {
                    playerPhaseRef.current = 'ended';
                    setState(s => ({ ...s, phase: 'ended' }));
                }
            });
            console.log('[AdaptiveStreaming] 🏗️ MediaSource blob URL set on video element');
        } else {
            console.error('[AdaptiveStreaming] 🏗️ videoRef.current is NULL — cannot set MediaSource src!');
        }
    }, [createSourceBuffer, startDownload, initSegments]);

    // ── Seek ─────────────────────────────────────────────────────────
    const seek = useCallback((time: number) => {
        const mp4boxfile = mp4boxRef.current;
        const ms = mediaSourceRef.current;
        if (!mp4boxfile || !ms || ms.readyState !== 'open') return;

        pendingSeekTimeRef.current = time;
        if (isSeekingRef.current) return;
        isSeekingRef.current = true;
        playerPhaseRef.current = 'seeking';
        setState(s => ({ ...s, phase: 'seeking' }));

        setTimeout(() => {
            const targetTime = pendingSeekTimeRef.current;
            if (targetTime === null) { isSeekingRef.current = false; return; }
            pendingSeekTimeRef.current = null;

            abortFetch();

            // Unset old segment options before clearing buffer to prevent stale refs
            for (const track of tracksRef.current) {
                try { mp4boxfile.unsetSegmentOptions(track.id); } catch { /* ignore */ }
            }

            clearSourceBuffer();
            const seekInfo = mp4boxfile.seek(targetTime, true);

            // Recreate all source buffers
            for (const track of tracksRef.current) {
                if (track.codec) {
                    const isAudio = track.type === 'audio';
                    const sb = createSourceBuffer(track.codec, track.id, isAudio);
                    if (sb) {
                        sourceBuffersRef.current[track.id] = sb;
                    }
                }
            }

            initSegments(mp4boxfile);

            if (videoRef.current) videoRef.current.currentTime = seekInfo.time;
            startDownload(seekInfo.offset);
            playerPhaseRef.current = 'playing';
            setState(s => ({ ...s, phase: 'playing' }));
            isSeekingRef.current = false;
        }, SEEK_DEBOUNCE_MS);
    }, [abortFetch, clearSourceBuffer, createSourceBuffer, initSegments, startDownload]);

    // ── Adaptive speed measurement ───────────────────────────────────
    useEffect(() => {
        if (useFallback) return;
        const interval = setInterval(() => {
            const samples = speedSamplesRef.current;
            if (samples.length < 2) return;
            const oldest = samples[0];
            const newest = samples[samples.length - 1];
            const elapsedSec = (newest.time - oldest.time) / 1000;
            if (elapsedSec <= 0) return;
            const totalBytes = samples.reduce((sum, s) => sum + s.bytes, 0);
            const kbps = Math.round((totalBytes * 8) / elapsedSec / 1000);
            setState(s => ({ ...s, measuredKbps: kbps }));

            // Log measured speed separately from throttle cap
            const throttleBps = throttleBpsRef.current;
            if (throttleBps > 0 || kbps > 0) {
                console.log('[AdaptiveStreaming] speed sample', {
                    measuredKbps: kbps,
                    throttleCapKbps: throttleBps > 0 ? Math.round(throttleBps / 1024) : 0,
                    quality: settings.quality,
                    adaptiveMode: settings.adaptiveMode,
                });
            }

            // Adaptive auto-quality: only when NOT throttled (quality === 'original')
            // This prevents the feedback loop where throttled speed is measured
            // and used to downgrade quality, trapping playback at lower settings.
            if (settings.adaptiveMode && playerPhaseRef.current === 'playing' && settings.quality === 'original') {
                for (const t of ADAPTIVE_THRESHOLDS) {
                    if (kbps >= t.minKbps) {
                        if (settings.quality !== t.quality) {
                            console.log('[AdaptiveStreaming] auto-quality', { from: settings.quality, to: t.quality, measuredKbps: kbps });
                            setQuality(t.quality);
                        }
                        break;
                    }
                }
            }
        }, SPEED_CHECK_INTERVAL_MS);
        return () => clearInterval(interval);
    }, [useFallback, settings.adaptiveMode, settings.quality, setQuality]);

    // ── Main initialization effect ───────────────────────────────────
    useEffect(() => {
        if (useFallback || !streamUrl) {
            console.log('[AdaptiveStreaming] 🚫 Skipping MSE: useFallback=', useFallback, 'streamUrl=', !!streamUrl);
            return;
        }

        console.log('[AdaptiveStreaming] 🚀 Starting initialization for:', fileName);
        playerPhaseRef.current = 'loading';
        onReadyCalledRef.current = false;
        moovEndOffsetRef.current = 0;
        setState(s => ({ ...s, phase: 'loading', error: null, loadProgress: 0 }));
        fileSizeRef.current = 0;
        appendQueuesRef.current = {};
        tracksRef.current = [];
        segmentationFailedRef.current = false;
        let fallbackTimer: ReturnType<typeof setTimeout> | null = null;

        const mp4boxfile = createFile();
        mp4boxRef.current = mp4boxfile;

        mp4boxfile.onError = (_module: string, message: string) => {
            console.error('[AdaptiveStreaming] mp4box error:', message);
            if (playerPhaseRef.current !== 'error') {
                playerPhaseRef.current = 'error';
                setState(s => ({ ...s, phase: 'error', error: message }));
                // Fall back to native <video> — mp4box cannot parse this file
                setDynamicFallback(true);
            }
        };

        mp4boxfile.onReady = (info: unknown) => {
            console.log('[AdaptiveStreaming] 📦 onReady FIRED!');
            if (onReadyCalledRef.current) { console.log('[AdaptiveStreaming] 📦 onReady already called, ignoring'); return; }
            onReadyCalledRef.current = true;
            if (fallbackTimer) { clearTimeout(fallbackTimer); fallbackTimer = null; }
            abortDiscovery();

            const movieInfo = info as Mp4MovieInfo;
            console.log('[AdaptiveStreaming] 📦 movieInfo:', movieInfo ? `tracks=${movieInfo.tracks?.length} duration=${movieInfo.duration}/${movieInfo.timescale}` : 'NULL');
            if (!movieInfo || !Array.isArray(movieInfo.tracks)) {
                console.error('[AdaptiveStreaming] 📦 Unexpected mp4box response — falling back to native video');
                playerPhaseRef.current = 'error';
                setState(s => ({ ...s, phase: 'error', error: 'Unexpected mp4box response' }));
                setDynamicFallback(true);
                return;
            }

            const tracks: VideoTrackInfo[] = [];
            for (const track of movieInfo.tracks) {
                tracks.push({
                    id: track.id,
                    type: track.video ? 'video' : 'audio',
                    width: track.video?.width,
                    height: track.video?.height,
                    bitrate: track.bitrate,
                    codec: track.codec,
                    duration: movieInfo.duration / movieInfo.timescale,
                });
            }
            console.log('[AdaptiveStreaming] 📦 parsed', tracks.length, 'tracks:', tracks.map(t => `${t.type}:${t.codec}`).join(', '));
            tracksRef.current = tracks;
            setState(s => ({ ...s, tracks, loadProgress: 100 }));

            // Progressive (non-fragmented) MP4: mp4box can't segment these files
            // (initializeSegmentation crashes on missing mvex/mehd). Use native <video>.
            if (!movieInfo.isFragmented) {
                console.log('[AdaptiveStreaming] 📦 Progressive MP4 detected — warming moov cache, then falling back to native <video>');
                // Pre-fetch tail to warm the browser's HTTP cache so the native
                // <video> element finds the moov atom instantly.
                if (fileSizeRef.current > 0) {
                    warmProgressiveMoovCache(streamUrl, fileSizeRef.current);
                }
                // Notify parent so it can trigger fMP4 remux in the background.
                // Always fall back to native video — the parent can override
                // later by providing a new stream URL once remux completes.
                if (onProgressiveDetected) {
                    onProgressiveDetected();
                }
                setDynamicFallback(true);
                return;
            }

            // Fragmented MP4 missing mehd box: mp4box v2.3.0 crashes in
            // initializeSegmentation because it accesses
            // this.moov.mvex.mehd.fragment_duration without null-checking.
            // Fix: inject a stub mehd so initializeSegmentation succeeds.
            // Duration of 0 is safe — MSE computes actual duration from
            // segment timestamps (same as DASH/CMAF live streams).
            const moov = (mp4boxfile as any).moov;
            if (moov?.mvex && !moov.mvex.mehd) {
                console.log('[AdaptiveStreaming] 📦 Patching missing mehd on fragmented MP4');
                moov.mvex.mehd = { fragment_duration: 0 };
            }

            // Capture accurate resume offset if discovery didn't set one (moov-at-end)
            if (moovEndOffsetRef.current === 0 && fetchOffsetRef.current > 0) {
                moovEndOffsetRef.current = fetchOffsetRef.current;
            }

            // Cache for future replays
            const cacheKey = extractCacheKey(streamUrl);
            if (cacheKey) setCachedMoov(cacheKey, tracks).catch(() => {});

            // Build MSE pipeline
            buildMsePipeline(mp4boxfile, tracks);
        };

        // ── Try cache, then discovery ──────────────────────────────
        const cacheKey = extractCacheKey(streamUrl);
        const onCacheResult = (cachedTracks: VideoTrackInfo[] | null) => {
            if (onReadyCalledRef.current) return;

            if (cachedTracks && cachedTracks.length > 0) {
                // Show cached metadata immediately, then discover moov for real data
                tracksRef.current = cachedTracks;
                setState(s => ({ ...s, tracks: cachedTracks }));
            }
            beginMoovDiscovery();
        };

        // ── Global safety net: if nothing works after 45s, fall back to native <video> ──
        const safetyTimer = setTimeout(() => {
            if (onReadyCalledRef.current) return;
            console.error('[AdaptiveStreaming] ⏰ Global safety timer fired — no successful MSE init after 45s, falling back to native video');
            if (playerPhaseRef.current !== 'error') {
                playerPhaseRef.current = 'error';
                setState(s => ({ ...s, phase: 'error', error: 'MSE initialization timed out' }));
            }
            abortFetch();
            abortDiscovery();
            setDynamicFallback(true);
        }, 10000);

        function beginMoovDiscovery() {
            const ctrl = new AbortController();
            discoveryAbortRef.current = ctrl;
            discoverMoov(mp4boxfile, ctrl.signal);

            fallbackTimer = setTimeout(async () => {
                console.log('[AdaptiveStreaming] ⏰ Fallback timer fired! onReadyCalled=', onReadyCalledRef.current, 'fileSize=', fileSizeRef.current);
                if (onReadyCalledRef.current) return;

                // Stage 1: Retry with larger range (512KB) before giving up on the header
                const retryCtrl = new AbortController();
                discoveryAbortRef.current = retryCtrl;
                await discoverMoovRetry(mp4boxfile, retryCtrl.signal);
                console.log('[AdaptiveStreaming] ⏰ Retry complete, onReadyCalled=', onReadyCalledRef.current);
                if (onReadyCalledRef.current) return;

                // Stage 2: Try tail for moov-at-end files
                const tailCtrl = new AbortController();
                discoveryAbortRef.current = tailCtrl;
                await discoverMoovTail(mp4boxfile, tailCtrl.signal);
                console.log('[AdaptiveStreaming] ⏰ Tail discovery complete, onReadyCalled=', onReadyCalledRef.current);
                if (!onReadyCalledRef.current) {
                    // Still no moov — download from beginning as last resort
                    console.log('[AdaptiveStreaming] ⏰ Still no moov, starting full download from byte 0');
                    startDownload(0);
                }
            }, MOOV_FALLBACK_TIMEOUT_MS);
        }

        if (cacheKey) {
            getCachedMoov(cacheKey).then(onCacheResult).catch(() => beginMoovDiscovery());
        } else {
            beginMoovDiscovery();
        }

        // ── Cleanup ─────────────────────────────────────────────────
        return () => {
            if (safetyTimer) clearTimeout(safetyTimer);
            if (fallbackTimer) clearTimeout(fallbackTimer);
            abortFetch();
            abortDiscovery();
            try { mp4boxfile.stop(); } catch { /* ignore */ }
            try { mp4boxfile.flush(); } catch { /* ignore */ }
            const ms = mediaSourceRef.current;
            if (ms) {
                if (ms.readyState === 'open') {
                    try { ms.endOfStream(); } catch { /* ignore */ }
                }
                try { clearSourceBuffer(); } catch { /* ignore */ }
            }
            if (videoRef.current) {
                try {
                    const url = videoRef.current.src;
                    videoRef.current.src = "";
                    videoRef.current.load();
                    if (url?.startsWith('blob:')) URL.revokeObjectURL(url);
                } catch { /* ignore */ }
            }
            mp4boxRef.current = null;
            mediaSourceRef.current = null;
            sourceBuffersRef.current = {};
            appendQueuesRef.current = {};
            discoveryPrefixRef.current = null;
            discoverySuffixRef.current = null;
        };
    }, [streamUrl, useFallback, startDownload, createSourceBuffer, abortFetch, abortDiscovery,
        clearSourceBuffer, initSegments, discoverMoov, discoverMoovRetry, discoverMoovTail, buildMsePipeline, fileName]);

    return {
        videoRef,
        phase: state.phase,
        error: state.error,
        tracks: state.tracks,
        loadProgress: state.loadProgress,
        currentQuality: settings.quality,
        setQuality,
        adaptiveMode: settings.adaptiveMode,
        setAdaptiveMode,
        measuredKbps: state.measuredKbps,
        seek,
        useFallback: effectiveUseFallback,
        fallbackUrl: streamUrl,
        abort: () => {
            abortFetch();
            abortDiscovery();
            const mp4boxfile = mp4boxRef.current;
            if (mp4boxfile) {
                try { mp4boxfile.stop(); } catch {}
                try { mp4boxfile.flush(); } catch {}
            }
            if (mediaSourceRef.current) {
                try { clearSourceBuffer(); } catch {}
            }
            playerPhaseRef.current = 'ended';
        },
    };
}
