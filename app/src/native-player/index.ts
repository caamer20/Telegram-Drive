import { invoke } from '@tauri-apps/api/core';
import { isVideoFile } from '../utils';
import { redactSensitiveText } from '../security/redaction';

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
  errorPresented: boolean;
}

export interface NativePlaybackState {
  state: 'idle' | 'buffering' | 'ready' | 'playing' | 'paused' | 'ended' | 'error';
  isPlaying: boolean;
  positionMs: number;
  durationMs: number;
}

export const ANDROID_NATIVE_PLAYER_ENABLED =
  import.meta.env.VITE_ANDROID_NATIVE_PLAYER === 'true';
export const NATIVE_PLAYER_BUILD_MARKER = ANDROID_NATIVE_PLAYER_ENABLED
  ? 'telegram-drive:native-player-enabled'
  : 'telegram-drive:webview-fallback';

export const NATIVE_PLAYER_STARTUP_ATTEMPTS = 3;
export const NATIVE_PLAYER_STARTUP_RETRY_MS = 300;

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

export function clearPendingNativePlayerRestore(): Promise<void> {
  return invoke('plugin:native-player|clear_pending_native_player_restore');
}

export async function cleanupNativePlayerForLogout(
  close: () => Promise<void> = closeNativePlayer,
  clearRestore: () => Promise<void> = clearPendingNativePlayerRestore,
): Promise<void> {
  await close().catch(() => undefined);
  await clearRestore().catch(() => undefined);
}

function isStreamServerStarting(error: unknown): boolean {
  const safe = redactSensitiveText(String(error)).toLowerCase();
  return safe.includes('streaming server is still starting') ||
    safe.includes('stream server is still starting');
}

export async function openNativePlayerWithStartupRetry(
  source: NativePlayerSource,
  launch: (source: NativePlayerSource) => Promise<NativePlayerResult> = openNativePlayer,
  sleep: (delayMs: number) => Promise<void> = delay => new Promise(resolve => setTimeout(resolve, delay)),
  attempts = NATIVE_PLAYER_STARTUP_ATTEMPTS,
): Promise<NativePlayerResult> {
  const boundedAttempts = Math.max(1, Math.min(attempts, NATIVE_PLAYER_STARTUP_ATTEMPTS));
  for (let attempt = 1; attempt <= boundedAttempts; attempt += 1) {
    try {
      return await launch(source);
    } catch (error) {
      if (!isStreamServerStarting(error) || attempt === boundedAttempts) throw error;
      await sleep(NATIVE_PLAYER_STARTUP_RETRY_MS * attempt);
    }
  }
  throw new Error('Native player startup retry exhausted');
}

export function nativePlayerErrorMessage(error: NativePlayerError): string {
  if (error.code === 'HTTP_401' || error.code === 'HTTP_403' || error.code === 'SESSION_EXPIRED') {
    return 'The private playback session expired. Reopen the file to create a new session.';
  }
  switch (error.category) {
    case 'network': return 'Playback was interrupted by the local stream. You can try again.';
    case 'server': return 'The local streaming server could not provide this media.';
    case 'container': return 'Android could not open this media container.';
    case 'video-codec': return 'This device cannot play the selected video format.';
    case 'audio-codec': return 'This device cannot play the selected audio format.';
    case 'decoder-init': return 'Android could not start a decoder for this media.';
    case 'decoder-runtime': return 'The Android decoder stopped during playback.';
    default: return 'Native playback failed. Please reopen the file and try again.';
  }
}

export function nativePlayerInvocationMessage(error: unknown): string {
  const safe = redactSensitiveText(String(error)).toLowerCase();
  if (safe.includes('still starting')) return 'The local player is still starting. Please try again in a moment.';
  if (safe.includes('already open')) return 'Native playback is already opening.';
  return 'Native playback could not start. Please try again.';
}

export function shouldShowReturnedNativeError(result: NativePlayerResult): boolean {
  return Boolean(result.error && !result.errorPresented);
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
