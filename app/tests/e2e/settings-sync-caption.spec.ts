import { expect, test } from '@playwright/test';
import { desktopFixture, nativeCalls, openSettings } from './desktop-fixture';

test('settings sync restores the video filename-caption choice for the next upload', async ({ page }) => {
  await desktopFixture(page, {
    pickedUploadPaths: ['/fixtures/Example.mp4'],
    settings: { telegramSettingsSyncEnabled: true, videoUploadMode: 'media', videoFilenameCaption: false, uploadProtectionPreference: 'standard' },
  });
  await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible({ timeout: 30_000 });
  // Substitute only the Telegram backup transport. Settings capture, restore and upload use the shipped UI.
  await page.evaluate(() => {
    const ipc = (window as any).__TAURI_INTERNALS__;
    const original = ipc.invoke.bind(ipc);
    let saved: Record<string, unknown> | undefined;
    const status = () => ({ available: Boolean(saved), device_id: 'fixture-device', updated_at: 42, current_device: true });
    ipc.invoke = async (command: string, args: any, ...rest: unknown[]) => {
      if (command === 'cmd_get_settings_sync_status') return status();
      if (command === 'cmd_upload_settings_sync') {
        saved = structuredClone(args.settings);
        return status();
      }
      if (command === 'cmd_download_settings_sync') return { ...status(), settings: saved };
      return original(command, args, ...rest);
    };
  });
  await openSettings(page);
  const tab = (name: string) => page.getByRole('button', { name, exact: true });
  await tab('Privacy').click();
  await page.getByLabel('Sync passphrase').fill('fixture-only passphrase');
  await tab('Upload this device').click();
  await expect(tab('Download and apply')).toBeEnabled();
  await tab('General').click();
  const caption = page.getByRole('switch', { name: 'Filename captions' });
  await expect(caption).not.toBeChecked();
  await caption.click();
  await expect(caption).toBeChecked();
  await tab('Privacy').click();
  await page.getByLabel('Sync passphrase').fill('fixture-only passphrase');
  await tab('Download and apply').click();
  await tab('Apply settings').click();
  await expect(page.getByText('Encrypted settings downloaded and applied.')).toBeVisible();
  await tab('General').click();
  await expect(caption).not.toBeChecked();
  await page.keyboard.press('Escape');
  await tab('Upload').click();
  await expect.poll(async () => (await nativeCalls(page, 'cmd_transfer_enqueue_many')).at(-1)?.args.requests[0].videoFilenameCaption).toBe(false);
});
