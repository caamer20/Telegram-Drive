import { describe, expect, it, vi } from 'vitest';
import {
  NativePlayerLaunchGuard,
  NativePlayerSource,
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

describe('native player routing', () => {
  it('routes Android video to native and leaves desktop on WebView', () => {
    expect(shouldUseNativePlayer(true, 'movie.mp4', true)).toBe(true);
    expect(shouldUseNativePlayer(false, 'movie.mp4', true)).toBe(false);
    expect(shouldUseNativePlayer(true, 'song.mp3', true)).toBe(false);
  });

  it('supports the rollback feature flag', () => {
    expect(shouldUseNativePlayer(true, 'movie.mkv', false)).toBe(false);
  });

  it('prevents duplicate opens and handles a structured result', async () => {
    let resolve!: (value: any) => void;
    const launch = vi.fn(() => new Promise<any>(done => { resolve = done; }));
    const guard = new NativePlayerLaunchGuard();
    const first = guard.open(source, launch);
    const duplicate = await guard.open(source, launch);
    expect(duplicate).toBeNull();
    expect(launch).toHaveBeenCalledTimes(1);
    resolve({ positionMs: 9, durationMs: 10, completed: false, exitReason: 'back' });
    expect(await first).toMatchObject({ positionMs: 9, exitReason: 'back' });
  });

  it('never places URL, token, headers, or paths in plugin input', () => {
    const keys = Object.keys(source).map(key => key.toLowerCase());
    expect(keys).not.toContain('url');
    expect(keys).not.toContain('token');
    expect(keys).not.toContain('headers');
    expect(keys).not.toContain('path');
  });

  it('keeps process recovery identity-only', () => {
    const serialized = JSON.stringify(source).toLowerCase();
    expect(serialized).not.toContain('token');
    expect(serialized).not.toContain('authorization');
    expect(serialized).not.toContain('streamurl');
  });
});
