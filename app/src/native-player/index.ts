import { invoke } from '@tauri-apps/api/core';
import { isVideoFile } from '../utils';

export interface NativePlayerSource {
  folderId: number | null;
  messageId: number;
  title: string;
  fileName?: string;
  mimeType?: string;
  startPositionMs?: number;
  autoplay?: boolean;
}

export interface NativePlayerError {
  category:
    | 'network'
    | 'authentication'
    | 'server'
    | 'container'
    | 'video-codec'
    | 'audio-codec'
    | 'decoder-init'
    | 'decoder-runtime'
    | 'unknown';
  code: string;
  message: string;
}

export interface NativePlayerResult {
  positionMs: number;
  durationMs: number;
  completed: boolean;
  exitReason: 'back' | 'ended' | 'error' | 'external';
  error?: NativePlayerError;
}

export interface NativePlaybackState {
  state: 'idle' | 'buffering' | 'ready' | 'ended' | 'error';
  isPlaying: boolean;
  positionMs: number;
  durationMs: number;
}

export const ANDROID_NATIVE_PLAYER_ENABLED =
  import.meta.env.VITE_ANDROID_NATIVE_PLAYER !== 'false';

export function shouldUseNativePlayer(
  isAndroid: boolean,
  fileName: string,
  enabled = ANDROID_NATIVE_PLAYER_ENABLED,
): boolean {
  return isAndroid && enabled && isVideoFile(fileName);
}

export function openNativePlayer(source: NativePlayerSource): Promise<NativePlayerResult> {
  return invoke<NativePlayerResult>('plugin:native-player|open_native_player', { source });
}

export function closeNativePlayer(): Promise<void> {
  return invoke('plugin:native-player|close_native_player');
}

export function getNativePlaybackState(): Promise<NativePlaybackState> {
  return invoke<NativePlaybackState>('plugin:native-player|get_native_playback_state');
}

/** Internal identity-only recovery used after Android kills and recreates the app process. */
export function takePendingNativePlayerRestore(): Promise<NativePlayerSource | null> {
  return invoke<NativePlayerSource | null>(
    'plugin:native-player|take_pending_native_player_restore',
  );
}

export class NativePlayerLaunchGuard {
  private opening = false;

  async open(
    source: NativePlayerSource,
    launch: (source: NativePlayerSource) => Promise<NativePlayerResult> = openNativePlayer,
  ): Promise<NativePlayerResult | null> {
    if (this.opening) return null;
    this.opening = true;
    try {
      return await launch(source);
    } finally {
      this.opening = false;
    }
  }
}

function resumeKey(folderId: number | null, messageId: number): string {
  return `native-player-resume:${folderId ?? 'home'}:${messageId}`;
}

export function loadNativeResumePosition(folderId: number | null, messageId: number): number {
  try {
    const parsed = Number(localStorage.getItem(resumeKey(folderId, messageId)));
    return Number.isFinite(parsed) && parsed >= 0 ? parsed : 0;
  } catch {
    return 0;
  }
}

export function saveNativeResumePosition(
  folderId: number | null,
  messageId: number,
  result: NativePlayerResult,
): void {
  try {
    const key = resumeKey(folderId, messageId);
    if (result.completed || result.positionMs <= 0) localStorage.removeItem(key);
    else localStorage.setItem(key, String(Math.floor(result.positionMs)));
  } catch {
    // Resume persistence is best-effort and never stores credentials.
  }
}
