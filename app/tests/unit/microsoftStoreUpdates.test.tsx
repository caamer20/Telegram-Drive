import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  check: vi.fn(),
  installation: vi.fn(),
  install: vi.fn(),
  open: vi.fn(),
}));

vi.mock('@tauri-apps/plugin-updater', () => ({ check: mocks.check }));
vi.mock('@tauri-apps/plugin-os', () => ({ type: () => 'windows' }));
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: mocks.open }));
vi.mock('../../src/services/installationInfo', () => ({
  getInstallationInfo: mocks.installation,
  RELEASES_URL: 'https://github.com/caamer20/Telegram-Drive/releases/latest',
}));
vi.mock('../../src/services/updateReliability', () => ({ installVerifiedUpdate: mocks.install }));

import { useUpdateCheck } from '../../src/hooks/useUpdateCheck';

describe('Microsoft Store update ownership', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.check.mockResolvedValue({ version: '4.0.0' });
  });

  it('never queries or launches the standalone installer for an MSIX installation', async () => {
    mocks.installation.mockResolvedValue({ managedByPackageManager: true, packageManager: 'microsoft-store' });
    const { result, unmount } = renderHook(() => useUpdateCheck());
    await act(async () => { await result.current.checkForUpdates(); });
    expect(result.current.available).toBe(false);
    expect(result.current.managedByPackageManager).toBe(true);
    expect(mocks.check).not.toHaveBeenCalled();
    await act(async () => { await result.current.downloadAndInstall(); });
    expect(mocks.install).not.toHaveBeenCalled();
    expect(mocks.open).not.toHaveBeenCalled();
    unmount();
  });

  it('keeps signed self-updates available to the standalone Windows installer', async () => {
    mocks.installation.mockResolvedValue({ managedByPackageManager: false, packageManager: null });
    const { result, unmount } = renderHook(() => useUpdateCheck());
    await act(async () => { await result.current.checkForUpdates(); });
    expect(result.current.available).toBe(true);
    expect(mocks.check).toHaveBeenCalledOnce();
    await act(async () => { await result.current.downloadAndInstall(); });
    expect(mocks.install).toHaveBeenCalledOnce();
    unmount();
  });

  it('preserves the existing pacman handoff without installing an update itself', async () => {
    mocks.installation.mockResolvedValue({ managedByPackageManager: true, packageManager: 'pacman' });
    const { result, unmount } = renderHook(() => useUpdateCheck());
    await act(async () => { await result.current.checkForUpdates(); });
    await act(async () => { await result.current.downloadAndInstall(); });
    expect(mocks.open).toHaveBeenCalledWith('https://github.com/caamer20/Telegram-Drive/releases/latest');
    expect(mocks.install).not.toHaveBeenCalled();
    unmount();
  });
});
