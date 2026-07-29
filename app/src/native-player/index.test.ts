import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  loadNativeResumePosition,
  nativePlayerErrorMessage,
  nativePlayerInvocationMessage,
  NativePlayerLaunchGuard,
  NativePlayerResult,
  NativePlayerSource,
  openNativePlayerWithStartupRetry,
  saveNativeResumePosition,
  shouldShowReturnedNativeError,
  shouldUseNativePlayer,
} from './index';

const source: NativePlayerSource = {
  folderId: null,
  messageId: 42,
  title: 'Movie',
  fileName: 'movie.mkv',
  startPositionMs: 100,
  autoplay: true,
};

const result = (overrides: Partial<NativePlayerResult> = {}): NativePlayerResult => ({
  positionMs: 9,
  durationMs: 10,
  completed: false,
  exitReason: 'back',
  ...overrides,
});

describe('native player orchestration', () => {
  beforeEach(() => localStorage.clear());

  it('routes only explicitly enabled Android video to native', () => {
    expect(shouldUseNativePlayer(true, 'movie.mp4', true)).toBe(true);
    expect(shouldUseNativePlayer(true, 'movie.mkv', false)).toBe(false);
    expect(shouldUseNativePlayer(false, 'movie.mp4', true)).toBe(false);
    expect(shouldUseNativePlayer(true, 'song.mp3', true)).toBe(false);
    expect(shouldUseNativePlayer(true, 'image.jpg', true)).toBe(false);
    expect(shouldUseNativePlayer(true, 'document.pdf', true)).toBe(false);
  });

  it('prevents duplicate opens', async () => {
    let resolve!: (value: NativePlayerResult) => void;
    const launch = vi.fn(() => new Promise<NativePlayerResult>(done => { resolve = done; }));
    const guard = new NativePlayerLaunchGuard();
    const first = guard.open(source, launch);
    const duplicate = await guard.open(source, launch);
    expect(duplicate).toBeNull();
    expect(launch).toHaveBeenCalledTimes(1);
    resolve(result());
    expect(await first).toMatchObject({ positionMs: 9, exitReason: 'back' });
  });

  it('persists resume position and clears completed playback', () => {
    saveNativeResumePosition(null, 42, result({ positionMs: 1_234 }));
    expect(loadNativeResumePosition(null, 42)).toBe(1_234);
    saveNativeResumePosition(null, 42, result({ positionMs: 10, completed: true }));
    expect(loadNativeResumePosition(null, 42)).toBe(0);
  });

  it('retries server-starting failures only within the bound', async () => {
    const launch = vi.fn()
      .mockRejectedValueOnce(new Error('Streaming server is still starting'))
      .mockRejectedValueOnce(new Error('stream server is still starting'))
      .mockResolvedValue(result());
    const sleep = vi.fn().mockResolvedValue(undefined);
    await expect(openNativePlayerWithStartupRetry(source, launch, sleep)).resolves.toMatchObject({ positionMs: 9 });
    expect(launch).toHaveBeenCalledTimes(3);
    expect(sleep).toHaveBeenCalledTimes(2);

    const authFailure = vi.fn().mockRejectedValue(new Error('HTTP 401'));
    await expect(openNativePlayerWithStartupRetry(source, authFailure, sleep)).rejects.toThrow('HTTP 401');
    expect(authFailure).toHaveBeenCalledTimes(1);
  });

  it('maps structured and invocation errors without exposing raw details', () => {
    expect(nativePlayerErrorMessage({ category: 'video-codec', code: 'UNSUPPORTED_VIDEO_PROFILE', message: 'raw' }))
      .toContain('video format');
    const invocation = nativePlayerInvocationMessage(
      new Error('failed http://127.0.0.1:49152/stream/home/7?token=private'),
    );
    expect(invocation).toBe('Native playback could not start. Please try again.');
    expect(invocation).not.toContain('private');
    expect(shouldShowReturnedNativeError(result({
      exitReason: 'error',
      error: { category: 'network', code: 'READ_TIMEOUT', message: 'safe' },
    }))).toBe(false);
  });

  it('never places URL, token, headers, or paths in plugin input or recovery identity', () => {
    const keys = Object.keys(source).map(key => key.toLowerCase());
    expect(keys).not.toContain('url');
    expect(keys).not.toContain('token');
    expect(keys).not.toContain('headers');
    expect(keys).not.toContain('path');
    const serialized = JSON.stringify(source).toLowerCase();
    expect(serialized).not.toContain('authorization');
    expect(serialized).not.toContain('streamurl');
  });
});
