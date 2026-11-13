import { expect, test, type Page } from '@playwright/test';
import { desktopFixture, nativeCalls, openSettings } from './desktop-413-fixture';

// The shipped application/provider tree drives these journeys. The controlled
// native boundary verifies upload intent, not native encryption or live Telegram.
const pickerPaths = ['/fixtures/First.txt', '/fixtures/Second.mp4'];

async function ready(page: Page) {
  await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible({ timeout: 30_000 });
}

async function openEncryptionSettings(page: Page) {
  await openSettings(page);
  await page.getByRole('button', { name: 'Encryption', exact: true }).click();
}

async function expectQueued(page: Page, count = 1) {
  await expect.poll(async () => (await nativeCalls(page, 'cmd_transfer_enqueue_many')).length).toBe(count);
  return (await nativeCalls(page, 'cmd_transfer_enqueue_many')).at(-1)!.args.requests;
}

async function enterFilePassphrase(page: Page) {
  const disclaimer = page.getByRole('dialog', { name: 'Keep your encryption credentials safe', exact: true });
  await disclaimer.getByRole('button', { name: 'Confirm', exact: true }).click();
  const secret = page.getByRole('dialog', { name: 'Encrypted with File Passphrase', exact: true });
  await expect(secret.locator('input[type="password"]')).toHaveCount(2);
  for (const input of await secret.locator('input[type="password"]').all()) await input.fill('fixture-file-passphrase');
  await secret.getByRole('button', { name: 'Confirm', exact: true }).click();
}

const preference = (page: Page) => page.getByRole('combobox', { name: 'Default upload mode', exact: true });
const chooser = (page: Page) => page.getByRole('dialog', { name: /How should we store/ });

test('the default upload mode is an explicit Ask every time preference', async ({ page }) => {
  await desktopFixture(page, { pickedUploadPaths: pickerPaths });
  await ready(page);
  await openEncryptionSettings(page);
  await expect(preference(page)).toHaveValue('ask');
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Upload', exact: true }).click();
  await expect(chooser(page)).toBeVisible();
  await chooser(page).getByRole('button', { name: /^Store Upload normally/ }).click();
  expect(await expectQueued(page)).toEqual(expect.arrayContaining([expect.objectContaining({ protectionMode: 'standard' })]));
  await page.getByRole('button', { name: 'Upload', exact: true }).click();
  await expect(chooser(page)).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(chooser(page)).toHaveCount(0);
  expect(await nativeCalls(page, 'cmd_transfer_enqueue_many')).toHaveLength(1);
});

test('the selected default survives restart and switching back to Ask restores the chooser', async ({ page }) => {
  await desktopFixture(page, { pickedUploadPaths: pickerPaths });
  await ready(page);
  await openEncryptionSettings(page);
  await preference(page).selectOption('standard');
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('desktop-e2e-stores')!)['settings.json'].settings.uploadProtectionPreference)).toBe('standard');
  await page.reload();
  await openEncryptionSettings(page);
  await expect(preference(page)).toHaveValue('standard');
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Upload', exact: true }).click();
  expect(await expectQueued(page)).toHaveLength(2);
  await openEncryptionSettings(page);
  await preference(page).selectOption('ask');
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Upload', exact: true }).click();
  await expect(chooser(page)).toBeVisible();
  await chooser(page).getByRole('button', { name: 'Cancel', exact: true }).click();
  expect(await nativeCalls(page, 'cmd_transfer_enqueue_many')).toHaveLength(1);
});

test('resetting preferences restores Ask every time', async ({ page }) => {
  await desktopFixture(page, { pickedUploadPaths: pickerPaths, settings: { uploadProtectionPreference: 'standard' } });
  await ready(page);
  await openEncryptionSettings(page);
  await expect(preference(page)).toHaveValue('standard');
  await page.getByRole('button', { name: 'Reset to Defaults', exact: true }).click();
  await page.getByRole('button', { name: 'No thanks', exact: true }).click();
  await page.getByRole('button', { name: 'Skip drive introduction', exact: true }).click();
  await expect(preference(page)).toHaveValue('ask');
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Upload', exact: true }).click();
  await expect(chooser(page)).toBeVisible();
  expect(await nativeCalls(page, 'cmd_transfer_enqueue_many')).toHaveLength(0);
});

test('an invalid saved preference falls back to an explicit upload choice', async ({ page }) => {
  await desktopFixture(page, { pickedUploadPaths: pickerPaths, settings: { uploadProtectionPreference: 'future-mode' } });
  await ready(page);
  await openEncryptionSettings(page);
  await expect(preference(page)).toHaveValue('ask');
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Upload', exact: true }).click();
  await expect(chooser(page)).toBeVisible();
  expect(await nativeCalls(page, 'cmd_transfer_enqueue_many')).toHaveLength(0);
});

test('a failed default save can be retried and persists only after recovery', async ({ page }) => {
  await desktopFixture(page);
  await ready(page);
  await openEncryptionSettings(page);
  await page.evaluate(() => { (window as any).__desktopTest.saveFails = true; });
  await preference(page).selectOption('encrypted');
  const retry = page.getByRole('alert').getByRole('button', { name: 'Retry', exact: true });
  await expect(retry).toBeVisible();
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('desktop-e2e-stores') || '{}')['settings.json']?.settings.uploadProtectionPreference)).not.toBe('encrypted');
  await page.evaluate(() => { (window as any).__desktopTest.saveFails = false; });
  await retry.click();
  await expect(retry).toHaveCount(0);
  await page.reload();
  await openEncryptionSettings(page);
  await expect(preference(page)).toHaveValue('encrypted');
});

test('a failed preference read requires an explicit choice rather than a saved automatic mode', async ({ page }) => {
  await desktopFixture(page, { settingsReadFails: true, pickedUploadPaths: pickerPaths, settings: { uploadProtectionPreference: 'standard' } });
  await ready(page);
  await page.getByRole('button', { name: 'No thanks', exact: true }).click();
  await page.getByRole('button', { name: 'Skip drive introduction', exact: true }).click();
  await page.getByRole('button', { name: 'Upload', exact: true }).click();
  await expect(chooser(page)).toBeVisible();
  expect(await nativeCalls(page, 'cmd_transfer_enqueue_many')).toHaveLength(0);
  await chooser(page).getByRole('button', { name: /^Store Upload normally/ }).click();
  expect(await expectQueued(page)).toHaveLength(2);
});

for (const encryptionDefaultMode of ['standard', 'vault', 'passphrase', 'vault_and_passphrase']) {
  test(`Always Encrypted preserves ${encryptionDefaultMode} preparation for every file`, async ({ page }) => {
    await desktopFixture(page, {
      pickedUploadPaths: pickerPaths, vault: { exists: true, is_unlocked: true },
      settings: { uploadProtectionPreference: 'encrypted', encryptionDefaultMode, encryptionProtectMetadata: false },
    });
    await ready(page);
    await page.getByRole('button', { name: 'Upload', exact: true }).click();
    if (encryptionDefaultMode.includes('passphrase')) await enterFilePassphrase(page);
    const requests = await expectQueued(page);
    await expect(chooser(page)).toHaveCount(0);
    expect(requests).toHaveLength(2);
    const mode = encryptionDefaultMode === 'standard' ? 'vault' : encryptionDefaultMode;
    for (const request of requests) expect(request).toMatchObject({ ownerId: '101', folderId: 9, protectionMode: mode, protectMetadata: false });
    if (encryptionDefaultMode.includes('passphrase')) {
      expect(await nativeCalls(page, 'cmd_stage_file_passphrase')).toHaveLength(2);
      expect(requests.map((request: any) => request.promptToken)).toEqual([701, 702]);
    } else {
      expect(await nativeCalls(page, 'cmd_stage_file_passphrase')).toHaveLength(0);
    }
    if (encryptionDefaultMode === 'vault') {
      await page.getByRole('button', { name: 'Upload', exact: true }).click();
      for (const request of await expectQueued(page, 2)) expect(request).toMatchObject({ protectionMode: 'vault', protectMetadata: false });
      await expect(chooser(page)).toHaveCount(0);
    }
  });
}

test('canceling a required file passphrase never queues an unencrypted fallback', async ({ page }) => {
  await desktopFixture(page, { pickedUploadPaths: pickerPaths, settings: { uploadProtectionPreference: 'encrypted', encryptionDefaultMode: 'passphrase' } });
  await ready(page);
  await page.getByRole('button', { name: 'Upload', exact: true }).click();
  await page.getByRole('dialog', { name: 'Keep your encryption credentials safe', exact: true }).getByRole('button', { name: 'Confirm', exact: true }).click();
  const secret = page.getByRole('dialog', { name: 'Encrypted with File Passphrase', exact: true });
  await secret.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(secret).toHaveCount(0);
  await expect(chooser(page)).toHaveCount(0);
  // Reopening Preferences proves the upload preparation returned after cancellation.
  await openEncryptionSettings(page);
  await expect(preference(page)).toHaveValue('encrypted');
  expect(await nativeCalls(page, 'cmd_stage_file_passphrase')).toHaveLength(0);
  expect(await nativeCalls(page, 'cmd_transfer_enqueue_many')).toHaveLength(0);
});

test('Always Encrypted can be canceled or unlocked and requires Continue before queueing', async ({ page }) => {
  await desktopFixture(page, { pickedUploadPaths: pickerPaths, vault: { exists: true, is_unlocked: false }, settings: { uploadProtectionPreference: 'encrypted', encryptionDefaultMode: 'vault' } });
  await ready(page);
  await page.getByRole('button', { name: 'Upload', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Unlock Vault', exact: true });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('combobox')).toHaveCount(0);
  await expect(dialog.getByRole('button', { name: 'Continue', exact: true })).toBeDisabled();
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  expect(await nativeCalls(page, 'cmd_transfer_enqueue_many')).toHaveLength(0);
  await page.getByRole('button', { name: 'Upload', exact: true }).click();
  await dialog.getByPlaceholder('Vault passphrase (min 8 characters)').fill('wrong-passphrase');
  await dialog.getByRole('button', { name: 'Unlock Vault', exact: true }).click();
  await expect(dialog.getByRole('button', { name: 'Continue', exact: true })).toBeDisabled();
  await dialog.getByPlaceholder('Vault passphrase (min 8 characters)').fill('fixture-vault-passphrase');
  await dialog.getByRole('button', { name: 'Unlock Vault', exact: true }).click();
  await expect(dialog.getByRole('button', { name: 'Continue', exact: true })).toBeEnabled();
  expect(await nativeCalls(page, 'cmd_transfer_enqueue_many')).toHaveLength(0);
  await dialog.getByRole('button', { name: 'Continue', exact: true }).click();
  expect(await expectQueued(page)).toEqual(expect.arrayContaining([expect.objectContaining({ protectionMode: 'vault' })]));
});

test('Always Encrypted preserves vault creation acknowledgments and explicit Continue', async ({ page }) => {
  await desktopFixture(page, { pickedUploadPaths: pickerPaths, settings: { uploadProtectionPreference: 'encrypted' } });
  await ready(page);
  await page.getByRole('button', { name: 'Upload', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Create Vault', exact: true });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('combobox')).toHaveCount(0);
  await dialog.getByPlaceholder('Vault passphrase (min 8 characters)').fill('fixture-vault-passphrase');
  await dialog.getByPlaceholder('Confirm passphrase', { exact: true }).fill('fixture-vault-passphrase');
  const create = dialog.getByRole('button', { name: 'Create Vault', exact: true });
  await expect(create).toBeDisabled();
  await dialog.getByRole('checkbox', { name: /responsible for safeguarding/ }).check();
  await create.click();
  const proceed = page.getByRole('dialog').getByRole('button', { name: 'Continue', exact: true });
  await expect(proceed).toBeEnabled();
  expect(await nativeCalls(page, 'cmd_transfer_enqueue_many')).toHaveLength(0);
  await proceed.click();
  expect(await expectQueued(page)).toEqual(expect.arrayContaining([expect.objectContaining({ protectionMode: 'vault' })]));
});

for (const availability of ['blocked', 'disabled', 'error', 'ready']) {
  test(`Always Encrypted fails closed when encryption is ${availability === 'ready' ? 'ready but upload-disabled' : availability} and recovers after retry`, async ({ page }) => {
    await desktopFixture(page, {
      pickedUploadPaths: pickerPaths, vault: { exists: true, is_unlocked: true },
      settings: { uploadProtectionPreference: 'encrypted', encryptionDefaultMode: 'vault' },
      encryptionCapabilityFails: availability === 'error',
      encryptionCapabilities: { availability: availability === 'error' ? 'ready' : availability, upload_enabled: false },
    });
    await ready(page);
    await page.getByRole('button', { name: 'Upload', exact: true }).click();
    const retry = page.getByRole('dialog').getByRole('button', { name: 'Check again', exact: true });
    await expect(retry).toBeVisible();
    await expect(chooser(page)).toHaveCount(0);
    expect(await nativeCalls(page, 'cmd_transfer_enqueue_many')).toHaveLength(0);
    await page.evaluate(() => {
      const fixture = (window as any).__desktopTest;
      fixture.encryptionCapabilityFails = false;
      fixture.encryptionCapabilities.availability = 'ready';
      fixture.encryptionCapabilities.upload_enabled = true;
    });
    await retry.click();
    const proceed = page.getByRole('dialog').getByRole('button', { name: 'Continue', exact: true });
    await expect(proceed).toBeEnabled();
    await proceed.click();
    expect(await expectQueued(page)).toEqual(expect.arrayContaining([expect.objectContaining({ protectionMode: 'vault' })]));
  });
}

test('Always Encrypted waits for capability loading and requires Continue after preparation appears', async ({ page }) => {
  await desktopFixture(page, {
    pickedUploadPaths: pickerPaths, holdEncryptionCapabilities: true,
    vault: { exists: true, is_unlocked: true }, settings: { uploadProtectionPreference: 'encrypted', encryptionDefaultMode: 'vault' },
  });
  await ready(page);
  await page.getByRole('button', { name: 'Upload', exact: true }).click();
  const proceed = page.getByRole('dialog').getByRole('button', { name: 'Continue', exact: true });
  await expect(proceed).toBeDisabled();
  await expect(chooser(page)).toHaveCount(0);
  expect(await nativeCalls(page, 'cmd_transfer_enqueue_many')).toHaveLength(0);
  await page.evaluate(() => (window as any).__desktopTest.releaseEncryptionCapabilities());
  await expect(proceed).toBeEnabled();
  expect(await nativeCalls(page, 'cmd_transfer_enqueue_many')).toHaveLength(0);
  await proceed.click();
  for (const request of await expectQueued(page)) expect(request).toMatchObject({ protectionMode: 'vault' });
});

test('the default remains changeable while encryption is unavailable', async ({ page }) => {
  await desktopFixture(page, { pickedUploadPaths: pickerPaths, settings: { uploadProtectionPreference: 'encrypted' }, encryptionCapabilities: { availability: 'disabled', upload_enabled: false, core_available: false } });
  await ready(page);
  await openEncryptionSettings(page);
  await preference(page).selectOption('standard');
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Upload', exact: true }).click();
  expect(await expectQueued(page)).toEqual(expect.arrayContaining([expect.objectContaining({ protectionMode: 'standard' })]));
});

test('pending passphrase staging retains its mode and metadata when upload defaults change', async ({ page }) => {
  await desktopFixture(page, { holdPassphraseStaging: true, pickedUploadPaths: pickerPaths, settings: { uploadProtectionPreference: 'encrypted', encryptionDefaultMode: 'passphrase', encryptionProtectMetadata: false } });
  await ready(page);
  await page.getByRole('button', { name: 'Upload', exact: true }).click();
  await enterFilePassphrase(page);
  await expect.poll(async () => (await nativeCalls(page, 'cmd_stage_file_passphrase')).length).toBe(2);
  expect(await nativeCalls(page, 'cmd_transfer_enqueue_many')).toHaveLength(0);
  await openEncryptionSettings(page);
  await preference(page).selectOption('standard');
  await page.getByRole('combobox', { name: 'Default Upload Protection', exact: true }).selectOption('vault');
  await page.getByRole('switch', { name: 'Protect Filenames & Metadata', exact: true }).click();
  await page.evaluate(() => (window as any).__desktopTest.releasePassphrases());
  for (const request of await expectQueued(page)) expect(request).toMatchObject({ protectionMode: 'passphrase', protectMetadata: false });
});

test('an overlapping upload request supersedes the old choice without queueing its files', async ({ page }) => {
  await desktopFixture(page, { nativeDrag: true, pickedUploadPaths: pickerPaths });
  await ready(page);
  await page.getByRole('button', { name: 'Upload', exact: true }).click();
  await expect(chooser(page)).toBeVisible();
  await page.evaluate(() => (window as any).__desktopTest.emit('tauri://drag-drop', { paths: ['/fixtures/Replacement.txt'], position: { x: 300, y: 300 } }));
  await expect(page.getByRole('heading', { name: 'How should we store your selection? Files: 1', exact: true })).toBeVisible();
  await chooser(page).getByRole('button', { name: /^Store Upload normally/ }).click();
  expect(await expectQueued(page)).toEqual([expect.objectContaining({ filename: 'Replacement.txt', protectionMode: 'standard' })]);
  expect(await nativeCalls(page, 'cmd_transfer_enqueue_many')).toHaveLength(1);
});

test('an upload requested before preference hydration waits and uses the saved encrypted intent', async ({ page }) => {
  await desktopFixture(page, { holdSettings: true, pickedUploadPaths: pickerPaths, vault: { exists: true, is_unlocked: true }, settings: { uploadProtectionPreference: 'encrypted', encryptionDefaultMode: 'vault', encryptionProtectMetadata: false } });
  await page.goto('/');
  await expect.poll(async () => (await nativeCalls(page, 'plugin:store|get')).filter(call => call.args.rid === 'settings.json' && call.args.key === 'settings').length).toBeGreaterThan(0);
  await page.getByRole('button', { name: 'Upload', exact: true }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await expect(chooser(page)).toHaveCount(0);
  expect(await nativeCalls(page, 'cmd_transfer_enqueue_many')).toHaveLength(0);
  await page.evaluate(() => (window as any).__desktopTest.releaseSettings());
  for (const request of await expectQueued(page)) expect(request).toMatchObject({ protectionMode: 'vault', protectMetadata: false });
  await expect(chooser(page)).toHaveCount(0);
});

test('a ready automatic encrypted upload supersedes an older request awaiting Continue', async ({ page }) => {
  await desktopFixture(page, {
    nativeDrag: true, pickedUploadPaths: pickerPaths, vault: { exists: true, is_unlocked: false },
    settings: { uploadProtectionPreference: 'encrypted', encryptionDefaultMode: 'vault' },
  });
  await ready(page);
  await page.getByRole('button', { name: 'Upload', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Unlock Vault', exact: true });
  await dialog.getByPlaceholder('Vault passphrase (min 8 characters)').fill('fixture-vault-passphrase');
  await dialog.getByRole('button', { name: 'Unlock Vault', exact: true }).click();
  await expect(dialog.getByRole('button', { name: 'Continue', exact: true })).toBeEnabled();
  expect(await nativeCalls(page, 'cmd_transfer_enqueue_many')).toHaveLength(0);
  await page.evaluate(() => (window as any).__desktopTest.emit('tauri://drag-drop', { paths: ['/fixtures/Replacement.txt'], position: { x: 300, y: 300 } }));
  expect(await expectQueued(page)).toEqual([expect.objectContaining({ filename: 'Replacement.txt', protectionMode: 'vault' })]);
  await expect(dialog).toHaveCount(0);
  expect(await nativeCalls(page, 'cmd_transfer_enqueue_many')).toHaveLength(1);
});

test('changing the default cannot downgrade protection on a queued upload retry', async ({ page }) => {
  await desktopFixture(page, {
    vault: { exists: true, is_unlocked: true },
    transfers: [{ id: 'protected-retry', direction: 'upload', kind: 'local_upload', status: 'failed', filename: 'Protected.txt', path: '/fixtures/Protected.txt', folderId: 9, protectionMode: 'vault', protectMetadata: true, error: 'Connection interrupted', progress: 0, totalBytes: 100, transferredBytes: 0, speedBytesPerSec: 0, queuePosition: 0, createdAt: 1, updatedAt: 1 }],
  });
  await ready(page);
  await openEncryptionSettings(page);
  await preference(page).selectOption('standard');
  await page.keyboard.press('Escape');
  const row = page.getByTestId('transfer-row-protected-retry');
  await row.getByRole('button', { name: /Retry this file/ }).click();
  await expect(row).toContainText('Queued');
  expect((await nativeCalls(page, 'cmd_transfer_retry')).at(-1)!.args).toMatchObject({ id: 'protected-retry', ownerId: '101' });
  expect(await page.evaluate(() => (window as any).__desktopTest.transfers.find((job: any) => job.id === 'protected-retry'))).toMatchObject({ protectionMode: 'vault', protectMetadata: true });
  await expect(chooser(page)).toHaveCount(0);
  expect(await nativeCalls(page, 'cmd_transfer_enqueue_many')).toHaveLength(0);
});

test('pending protected preparation cannot enqueue files after switching accounts', async ({ page }) => {
  await desktopFixture(page, { pickedUploadPaths: pickerPaths, vault: { exists: true, is_unlocked: false }, settings: { uploadProtectionPreference: 'encrypted', encryptionDefaultMode: 'vault' } });
  await ready(page);
  await page.getByRole('button', { name: 'Upload', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Unlock Vault', exact: true });
  await expect(dialog).toBeVisible();
  await page.evaluate(() => { (window as any).__desktopTest.owner = '202'; document.dispatchEvent(new Event('visibilitychange')); });
  await expect(page.getByText('Work folder photo.jpg', { exact: true })).toBeVisible();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Upload', exact: true })).toBeEnabled();
  expect(await nativeCalls(page, 'cmd_transfer_enqueue_many')).toHaveLength(0);
  expect(await nativeCalls(page, 'cmd_unlock_vault')).toHaveLength(0);
});

test('Always Standard applies to native drops, zipped folders and URL uploads', async ({ page }) => {
  await desktopFixture(page, { nativeDrag: true, pickedUploadFolder: '/fixtures/Archive', settings: { uploadProtectionPreference: 'standard', zipFolders: true } });
  await ready(page);
  await page.evaluate(() => (window as any).__desktopTest.emit('tauri://drag-drop', { paths: ['/fixtures/Dropped.txt'], position: { x: 300, y: 300 } }));
  expect((await expectQueued(page))[0]).toMatchObject({ filename: 'Dropped.txt', folderId: 9, protectionMode: 'standard' });
  await page.getByRole('button', { name: 'Upload Folder...', exact: true }).click();
  expect((await expectQueued(page, 2))[0]).toMatchObject({ filename: 'Archive.zip', folderId: 9, protectionMode: 'standard' });
  await page.getByRole('button', { name: 'Preferences', exact: true }).click();
  await page.getByRole('menu').getByRole('button', { name: 'Remote Upload', exact: true }).click();
  const remote = page.getByRole('dialog', { name: 'Remote Upload (URL)', exact: true });
  await remote.getByPlaceholder('https://example.com/file.zip').fill('https://example.invalid/Remote.zip');
  await remote.getByRole('combobox').selectOption('9');
  await remote.getByRole('button', { name: 'Start Upload', exact: true }).click();
  expect((await expectQueued(page, 3))[0]).toMatchObject({ filename: 'Remote.zip', folderId: 9, protectionMode: 'standard', url: 'https://example.invalid/Remote.zip' });
  await expect(chooser(page)).toHaveCount(0);
});

test('Always Standard skips the chooser for repeated multi-file uploads', async ({ page }) => {
  await desktopFixture(page, { pickedUploadPaths: pickerPaths, settings: { uploadProtectionPreference: 'standard' } });
  await ready(page);
  for (let count = 1; count <= 2; count++) {
    await page.getByRole('button', { name: 'Upload', exact: true }).click();
    await expect.poll(async () => (await nativeCalls(page, 'cmd_transfer_enqueue_many')).length).toBe(count);
    await expect(page.getByRole('dialog', { name: /How should we store/ })).toHaveCount(0);
    const requests = (await nativeCalls(page, 'cmd_transfer_enqueue_many')).at(-1)!.args.requests;
    expect(requests).toHaveLength(2);
    expect(requests).toEqual(expect.arrayContaining([
      expect.objectContaining({ filename: 'First.txt', ownerId: '101', folderId: 9, protectionMode: 'standard' }),
      expect.objectContaining({ filename: 'Second.mp4', ownerId: '101', folderId: 9, protectionMode: 'standard' }),
    ]));
  }
});
