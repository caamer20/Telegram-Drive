import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
const { invokeMock, openMock, updateCheckMock, installationMock } = vi.hoisted(() => ({ invokeMock: vi.fn(), openMock: vi.fn(), updateCheckMock: vi.fn(), installationMock: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: openMock }));
vi.mock('@tauri-apps/plugin-os', () => ({ type: () => 'windows' }));
vi.mock('@tauri-apps/plugin-updater', () => ({ check: updateCheckMock }));
vi.mock('../../src/services/installationInfo', () => ({
  getInstallationInfo: installationMock,
  RELEASES_URL: 'https://github.com/caamer20/Telegram-Drive/releases/latest',
}));
vi.mock('../../src/components/desktop/dashboard/ThemesTab', () => ({ ThemesTab: () => null }));
vi.mock('react-i18next', () => ({
  initReactI18next: { type: '3rdParty', init: vi.fn() },
  useTranslation: () => ({
    t: (key: string, values?: { error?: string }) => {
      if (key === 'settings.failed_prefix') return `Failed: ${values?.error ?? ''}`;
      if (key === 'common.loading') return 'Loading...';
      if (key === 'settings.clear_all') return 'Clear All';
      if (key === 'settings.retry_encryption_check') return 'Check again';
      if (key === 'settings.ffmpeg_required_title') return 'FFmpeg is needed for HLS playback';
      if (key === 'settings.ffmpeg_required_desc') return 'FFmpeg was not detected.';
      if (key === 'settings.ffmpeg_restart_hint') return 'Add ffmpeg.exe to PATH, then restart.';
      if (key === 'settings.ffmpeg_download_windows') return 'Download FFmpeg for Windows';
      return key;
    },
  }),
}));
vi.mock('../../src/context/SettingsContext', async () => {
  const { DEFAULT_SETTINGS } = await import('../../src/config/defaultSettings');
  return {
    useSettings: () => ({
      settings: DEFAULT_SETTINGS,
      updateSetting: vi.fn(),
      updateSettings: vi.fn(),
      resetSettings: vi.fn(),
    }),
  };
});
vi.mock('../../src/context/ConfirmContext', () => ({
  useConfirm: () => ({ confirm: vi.fn().mockResolvedValue(true) }),
}));

import { SettingsModal } from '../../src/components/desktop/dashboard/SettingsModal';

describe('SettingsModal transcode cache state', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    openMock.mockReset();
    openMock.mockResolvedValue(undefined);
    updateCheckMock.mockReset();
    installationMock.mockReset();
    installationMock.mockResolvedValue({ managedByPackageManager: false, packageManager: null });
    invokeMock.mockImplementation((command: string) => {
      if (command === 'cmd_get_detailed_transcode_cache') {
        return Promise.reject(new Error('Windows cache access denied'));
      }
      if (command === 'cmd_get_api_settings') {
        return Promise.resolve({ enabled: false, port: 8550, key_set: false, running: false });
      }
      if (command === 'cmd_get_webdav_settings') {
        return Promise.resolve({ supported: true, enabled: false, port: 8551, write_enabled: false, token_set: false, running: false, last_error: null });
      }
      if (command === 'cmd_get_offline_cache_status') {
        return Promise.resolve({ entries: [], total_bytes: 0, max_bytes: 0 });
      }
      if (command === 'cmd_get_transcode_capabilities') {
        return Promise.resolve({ available: false, variants: [], mode: 'original' });
      }
      return Promise.resolve(null);
    });
  });

  it('shows a recoverable error instead of an infinite loading state', async () => {
    render(<SettingsModal ownerId={null} isOpen onClose={vi.fn()} />);

    expect((await screen.findByRole('alert')).textContent).toContain('Windows cache access denied');
    await waitFor(() => {
      expect(screen.queryByLabelText('Loading...')).toBeNull();
    });
    expect((screen.getByRole('button', { name: 'Clear All' }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('directs Windows users to the official FFmpeg download page when HLS is unavailable', async () => {
    render(<SettingsModal ownerId={null} isOpen onClose={vi.fn()} />);

    expect(await screen.findByText('FFmpeg is needed for HLS playback')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Download FFmpeg for Windows' }));

    await waitFor(() => {
      expect(openMock).toHaveBeenCalledWith('https://ffmpeg.org/download.html#build-windows');
    });
  });

  it('opens Microsoft Store when an MSIX user manually checks for updates', async () => {
    installationMock.mockResolvedValue({ managedByPackageManager: true, packageManager: 'microsoft-store' });
    render(<SettingsModal ownerId={null} isOpen onClose={vi.fn()} />);
    fireEvent.click(await screen.findByRole('button', { name: 'settings.check_now' }));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('cmd_open_microsoft_store_updates');
    });
    expect(updateCheckMock).not.toHaveBeenCalled();
    expect(await screen.findByText('Microsoft Store')).toBeTruthy();
  });
});
