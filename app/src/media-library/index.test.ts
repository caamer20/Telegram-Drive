import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import {
  clearMediaLibraryData,
  closeMediaLibrary,
  getMediaLibraryState,
  openMediaLibrary,
} from './index';

describe('native media library wrapper', () => {
  beforeEach(() => invoke.mockReset());

  it('uses the stable plugin commands without credentials', async () => {
    invoke.mockResolvedValueOnce({ exitReason: 'back', accountId: 42 });
    await expect(openMediaLibrary()).resolves.toEqual({ exitReason: 'back', accountId: 42 });
    expect(invoke).toHaveBeenCalledWith('plugin:media-library|open_media_library');

    invoke.mockResolvedValueOnce(undefined);
    await closeMediaLibrary();
    expect(invoke).toHaveBeenCalledWith('plugin:media-library|close_media_library');

    invoke.mockResolvedValueOnce({ status: 'offline', isOpen: true, online: false, syncRunning: false });
    await getMediaLibraryState();
    expect(invoke).toHaveBeenCalledWith('plugin:media-library|get_media_library_state');

    invoke.mockResolvedValueOnce(undefined);
    await clearMediaLibraryData();
    expect(invoke).toHaveBeenCalledWith('plugin:media-library|clear_media_library_data');
  });

  it('does not pass a token, URL, or account selector in public calls', async () => {
    invoke.mockResolvedValue(undefined);
    await Promise.all([openMediaLibrary(), closeMediaLibrary(), getMediaLibraryState(), clearMediaLibraryData()]);
    for (const call of invoke.mock.calls) {
      expect(call).toHaveLength(1);
      expect(JSON.stringify(call).toLowerCase()).not.toMatch(/token|authorization|baseurl/);
    }
  });
});
