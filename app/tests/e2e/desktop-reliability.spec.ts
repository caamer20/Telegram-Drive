import { expect, test } from '@playwright/test';
import { desktopFixture, nativeCalls, openSettings } from './desktop-fixture';

test('disconnected folder changes preserve the list and can be retried after reconnection', async ({ page }) => {
  await desktopFixture(page, { exerciseFolderMutations: true });
  await page.goto('/');
  const folder = (name: string) => page.getByRole('group', { name, exact: true });
  const connect = (online: boolean) => page.evaluate(online => { (window as any).__desktopTest.folderServiceOnline = online; }, online);
  const error = page.getByText('Telegram is unavailable. Reconnect and retry; your saved organization is safe.', { exact: true }).last();
  await expect(folder('Photos')).toBeVisible();
  const guide = page.locator('details').filter({ has: page.locator('summary', { hasText: 'Use an existing Telegram channel' }) });
  const summary = guide.locator('summary');
  const example = guide.getByText('Photos [TD]', { exact: true });
  await expect(summary).toBeVisible();
  await expect(example).not.toBeVisible();
  await summary.focus();
  await page.keyboard.press('Enter');
  await expect(example).toBeVisible();
  await expect(guide).toContainText('same account');
  await expect(guide).toContainText('Sync in the sidebar');
  await page.keyboard.press('Space');
  await expect(example).not.toBeVisible();
  await expect(summary).toBeFocused();
  await page.getByRole('button', { name: 'Create Folder', exact: true }).first().click();
  const create = page.getByRole('dialog', { name: 'Create Folder' });
  await create.getByRole('textbox').fill('Drafts');
  await create.getByRole('button', { name: /create/i }).click();
  await expect(error).toBeVisible();
  await expect(create.getByRole('textbox')).toHaveValue('Drafts');
  await expect(folder('Drafts')).toHaveCount(0);
  await connect(true);
  await create.getByRole('button', { name: /create/i }).click();
  await expect(folder('Drafts')).toBeVisible();

  // The Telegram fixture returns the display name after discovery removes the title marker.
  await page.evaluate(() => {
    const key = 'fixture-telegram-folders-101';
    const remote = JSON.parse(sessionStorage.getItem(key)!);
    sessionStorage.setItem(key, JSON.stringify([...remote, { id: 11, name: 'Archive' }]));
  });
  const scans = (await nativeCalls(page, 'cmd_scan_folders')).length;
  await page.getByRole('button', { name: 'Sync', exact: true }).click();
  await expect.poll(async () => (await nativeCalls(page, 'cmd_scan_folders')).length).toBeGreaterThan(scans);
  await expect(folder('Archive')).toBeVisible();

  await connect(false);
  await folder('Drafts').getByRole('button', { name: 'Folder settings' }).click();
  await page.getByRole('button', { name: 'Rename', exact: true }).click();
  const rename = page.getByRole('dialog', { name: 'Rename Folder', exact: true });
  const nameField = rename.getByRole('textbox', { name: 'Folder name', exact: true });
  await expect(rename.getByRole('button', { name: 'Close', exact: true })).toBeVisible();
  await expect(nameField).toBeFocused();
  await nameField.press('Escape');
  await expect(rename).toHaveCount(0);
  await folder('Drafts').getByRole('button', { name: 'Folder settings' }).click();
  await page.getByRole('button', { name: 'Rename', exact: true }).click();
  await expect(nameField).toBeFocused();
  await nameField.fill('Documents');
  await rename.getByRole('button', { name: 'Rename', exact: true }).click();
  await expect.poll(async () => (await nativeCalls(page, 'cmd_rename_folder')).length).toBe(1);
  await expect(nameField).toBeEnabled();
  await expect(folder('Drafts')).toBeVisible();
  await expect(folder('Documents')).toHaveCount(0);
  await connect(true);
  await nameField.press('Enter');
  await expect(folder('Documents')).toBeVisible();
  await connect(false);
  const remove = async () => {
    await folder('Documents').getByRole('button', { name: 'Folder settings' }).click();
    await page.getByRole('button', { name: 'Delete', exact: true }).click();
    await page.getByRole('dialog', { name: 'Delete Folder' }).getByRole('button', { name: 'Delete', exact: true }).click();
  };
  await remove();
  await expect.poll(async () => (await nativeCalls(page, 'cmd_delete_folder')).length).toBe(1);
  await expect(folder('Documents')).toBeVisible();
  await page.reload();
  await expect(folder('Documents')).toBeVisible();
  await connect(true);
  await remove();
  await expect(folder('Documents')).toHaveCount(0);
  await page.clock.install();
  await page.reload();
  await expect(folder('Photos')).toBeVisible();
  await expect(folder('Documents')).toHaveCount(0);
  await expect(page.getByText('Disconnected from Telegram', { exact: true })).toBeVisible();
  await page.evaluate(() => {
    sessionStorage.setItem('fixture-telegram-folders-101', JSON.stringify([{ id: 9, name: 'Photos from another device' }]));
    (window as any).__desktopTest.folderServiceOnline = true;
  });
  await page.clock.fastForward(10_001);
  await expect(folder('Photos from another device')).toBeVisible();
  await expect(folder('Photos')).toHaveCount(0);
});

test('a busy account read retries, and an empty cache stays loading until its folder completes', async ({ page }) => {
  await desktopFixture(page, { accountFailures: 100, holdFiles: true });
  await page.goto('/');
  await expect(page.getByRole('button', { name: 'Log Out', exact: true })).toBeVisible();
  await expect.poll(async () => (await nativeCalls(page, 'cmd_workspace_account')).length).toBeGreaterThan(0);
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toHaveCount(0);
  await page.evaluate(() => { (window as any).__desktopTest.accountFailures = 0; });
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible();
  expect((await nativeCalls(page, 'cmd_workspace_account')).length).toBeGreaterThan(1);
  await page.getByRole('button', { name: 'Saved Messages', exact: true }).click();
  await expect(page.getByLabel('Loading...', { exact: true })).toBeVisible();
  await expect(page.getByText('No files yet', { exact: true })).toHaveCount(0);
  await page.evaluate(() => (window as any).__desktopTest.releaseFiles());
  await expect(page.getByText('Holiday saved photo.jpg', { exact: true })).toBeVisible();
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toHaveCount(0);
});

test('a delayed old-account folder result cannot replace the new account files', async ({ page }) => {
  await desktopFixture(page, { holdFiles: true });
  await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible({ timeout: 30_000 });
  await page.getByRole('button', { name: 'Saved Messages', exact: true }).click();
  await expect.poll(async () => (await nativeCalls(page, 'cmd_get_files')).length).toBe(1);
  await page.evaluate(() => { (window as any).__desktopTest.owner = '202'; document.dispatchEvent(new Event('visibilitychange')); });
  await expect.poll(async () => (await nativeCalls(page, 'cmd_get_files')).length).toBe(2);
  await expect(page.getByText('Work saved photo.jpg', { exact: true })).toBeVisible();
  expect(await page.evaluate(() => (window as any).__desktopTest.completedFileRequests.map((request: any) => request.ownerId))).toEqual(['202']);
  await page.evaluate(() => (window as any).__desktopTest.releaseFiles());
  await expect.poll(() => page.evaluate(() => (window as any).__desktopTest.completedFileRequests.map((request: any) => request.ownerId))).toEqual(['202', '101']);
  // Let the completed old response and streamed event reach React and paint
  // before checking that neither restored the previous account's content.
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  await expect(page.getByText('Work saved photo.jpg', { exact: true })).toBeVisible();
  await expect(page.getByText('Holiday saved photo.jpg', { exact: true })).toHaveCount(0);
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toHaveCount(0);
});

test('sign-out cancellation and failure preserve files; successful retry clears only Telegram credentials', async ({ page }) => {
  await desktopFixture(page);
  await page.goto('/');
  const file = page.getByText('Holiday folder photo.jpg', { exact: true });
  await expect(file).toBeVisible({ timeout: 30_000 });
  const logout = page.getByRole('button', { name: 'Log Out', exact: true });
  await logout.click();
  await page.getByRole('dialog', { name: 'Sign Out' }).getByRole('button', { name: 'Cancel' }).click();
  expect(await nativeCalls(page, 'cmd_logout')).toHaveLength(0);
  await expect(file).toBeVisible();
  await page.evaluate(() => { (window as any).__desktopTest.logoutFails = true; });
  await logout.click();
  await page.getByRole('dialog', { name: 'Sign Out' }).getByRole('button', { name: 'Sign Out', exact: true }).click();
  await expect.poll(async () => (await nativeCalls(page, 'cmd_logout')).length).toBe(1);
  await expect(page.getByRole('dialog', { name: 'Sign Out' })).toHaveCount(0);
  await expect(page.getByText('The operation could not be completed. Try again or review the related settings.', { exact: true })).toBeVisible();
  await expect(file).toBeVisible();
  expect(await nativeCalls(page, 'cmd_clear_api_hash')).toHaveLength(0);
  expect(await page.evaluate(() => (window as any).__desktopTest.stores['config.json'].api_id)).toBe('12345');
  await expect(page.getByText('Private native diagnostic', { exact: true })).toHaveCount(0);
  await page.evaluate(() => { (window as any).__desktopTest.logoutFails = false; });
  await logout.click();
  await page.getByRole('dialog', { name: 'Sign Out' }).getByRole('button', { name: 'Sign Out', exact: true }).click();
  await expect(file).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'QR-first secure sign in', exact: true })).toBeVisible();
  const stores = await page.evaluate(() => (window as any).__desktopTest.stores);
  expect(stores['config.json'].api_id).toBeUndefined();
  expect(stores['settings.json'].supporter_activation).toBe('preserve-existing-license');
});

test('sign-out tells the user when Telegram did not confirm that the session ended', async ({ page }) => {
  await desktopFixture(page);
  await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible({ timeout: 30_000 });
  await page.evaluate(() => { (window as any).__desktopTest.remoteSignOutUnconfirmed = true; });
  await page.getByRole('button', { name: 'Log Out', exact: true }).click();
  await page.getByRole('dialog', { name: 'Sign Out' }).getByRole('button', { name: 'Sign Out', exact: true }).click();
  // Local sign-out still completes; the warning explains how to end the session remotely.
  await expect(page.getByRole('heading', { name: 'QR-first secure sign in', exact: true })).toBeVisible();
  await expect(page.getByText('Signed out on this device, but Telegram did not confirm that the session was ended. Open Telegram → Settings → Devices and end it there.', { exact: true })).toBeVisible();
  expect(await page.evaluate(() => (window as any).__desktopTest.stores['config.json'].api_id)).toBeUndefined();
});

test('a confirmed sign-out shows no remote-session warning', async ({ page }) => {
  await desktopFixture(page);
  await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible({ timeout: 30_000 });
  await page.getByRole('button', { name: 'Log Out', exact: true }).click();
  await page.getByRole('dialog', { name: 'Sign Out' }).getByRole('button', { name: 'Sign Out', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'QR-first secure sign in', exact: true })).toBeVisible();
  await expect(page.getByText(/Telegram did not confirm that the session was ended/)).toHaveCount(0);
});

test('settings persist across restart and a failed save can be retried from the application', async ({ page }) => {
  await desktopFixture(page);
  await page.goto('/');
  await openSettings(page);
  const ordered = page.getByRole('switch', { name: 'Strict upload order' });
  await expect(ordered).not.toBeChecked();
  await ordered.click();
  await expect(page.getByRole('group', { name: 'Concurrent Uploads', exact: true })).toHaveCount(0);
  await expect(page.getByRole('group', { name: 'Concurrent Downloads', exact: true })).toBeVisible();
  await expect.poll(async () => (await nativeCalls(page, 'cmd_transfer_set_limits')).at(-1)?.args.strictUploadOrder).toBe(true);
  const setting = page.getByRole('combobox', { name: 'Default video upload', exact: true });
  await setting.selectOption('media');
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('desktop-e2e-stores')!)['settings.json'].settings.videoUploadMode)).toBe('media');
  await page.reload();
  await openSettings(page);
  await expect(setting).toHaveValue('media');
  await expect(ordered).toBeChecked();
  await expect.poll(async () => (await nativeCalls(page, 'cmd_transfer_set_limits')).at(-1)?.args.strictUploadOrder).toBe(true);
  await ordered.click();
  await expect.poll(async () => (await nativeCalls(page, 'cmd_transfer_set_limits')).at(-1)?.args.strictUploadOrder).toBe(false);
  await expect(page.getByRole('group', { name: 'Concurrent Uploads', exact: true })).toBeVisible();
  await page.evaluate(() => { (window as any).__desktopTest.saveFails = true; });
  await setting.selectOption('file');
  await expect(page.getByRole('alert').getByRole('button', { name: 'Retry', exact: true })).toBeVisible();
  await page.evaluate(() => { (window as any).__desktopTest.saveFails = false; });
  await page.getByRole('alert').getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.getByRole('alert').getByRole('button', { name: 'Retry', exact: true })).toHaveCount(0);
  await page.reload();
  await openSettings(page);
  await expect(setting).toHaveValue('file');
});

test('keyboard folder navigation and grid image controls remain usable when dragging is disabled', async ({ page }) => {
  await desktopFixture(page);
  await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible({ timeout: 30_000 });
  const saved = page.getByRole('button', { name: 'Saved Messages', exact: true });
  await expect(saved).toBeEnabled();
  await saved.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByText('Holiday saved photo.jpg', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Recents', exact: true }).click();
  await page.getByRole('button', { name: 'Switch to Grid', exact: true }).click();
  const card = page.getByRole('group', { name: 'Holiday folder photo.jpg', exact: true });
  await expect(card).toBeEnabled();
  await card.dblclick();
  await expect(page.getByRole('img', { name: 'Holiday folder photo.jpg', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
  await expect(page.getByRole('button', { name: /Current zoom: 125%/ })).toBeVisible();
  await page.getByRole('button', { name: 'Fit image', exact: true }).click();
  await expect(page.getByRole('button', { name: /Current zoom: 100%/ })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('button', { name: 'Close preview', exact: true })).toHaveCount(0);
});

test('current-account inventory updates refresh the open folder and foreign updates are ignored', async ({ page }) => {
  await desktopFixture(page);
  await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Saved Messages', exact: true }).click();
  await expect(page.getByText('Holiday saved photo.jpg', { exact: true })).toBeVisible();
  const before = (await nativeCalls(page, 'cmd_get_files')).length;
  await page.evaluate(() => {
    const state = (window as any).__desktopTest;
    state.inventoryName = 'Edited in Telegram.jpg';
    state.emit('file-inventory-changed', { ownerId: '202', folderId: null });
  });
  await expect(page.getByText('Holiday saved photo.jpg', { exact: true })).toBeVisible();
  expect((await nativeCalls(page, 'cmd_get_files')).length).toBe(before);
  await page.evaluate(() => (window as any).__desktopTest.emit('file-inventory-changed', { ownerId: '101', folderId: null }));
  await expect(page.getByText('Edited in Telegram.jpg', { exact: true })).toBeVisible();
  await expect(page.getByText('Holiday saved photo.jpg', { exact: true })).toHaveCount(0);
  expect((await nativeCalls(page, 'cmd_get_files')).at(-1)?.args.forcePoll).toBe(true);
});

test('an expired inventory verification failure is shown instead of certifying cached rows', async ({ page }) => {
  await desktopFixture(page);
  await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Saved Messages', exact: true }).click();
  await expect(page.getByText('Holiday saved photo.jpg', { exact: true })).toBeVisible();
  await page.evaluate(() => {
    const state = (window as any).__desktopTest;
    state.inventoryFailure = true;
    state.emit('file-inventory-changed', { ownerId: '101', folderId: null });
  });
  await expect(page.getByText('Error loading files', { exact: true })).toBeVisible({ timeout: 15_000 });
  await expect(page.getByText('Holiday saved photo.jpg', { exact: true })).toHaveCount(0);
  await page.evaluate(() => {
    const state = (window as any).__desktopTest;
    state.inventoryFailure = false;
    state.emit('file-inventory-changed', { ownerId: '101', folderId: null });
  });
  await expect(page.getByText('Holiday saved photo.jpg', { exact: true })).toBeVisible();
});

test('an open folder keeps its inventory observed without reloading files and follows account changes', async ({ page }) => {
  await page.clock.install();
  await desktopFixture(page);
  await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Saved Messages', exact: true }).click();
  await expect(page.getByText('Holiday saved photo.jpg', { exact: true })).toBeVisible();
  const reads = (await nativeCalls(page, 'cmd_get_files')).length;
  await expect.poll(async () => (await nativeCalls(page, 'cmd_touch_file_inventory')).length).toBeGreaterThan(0);
  const before = (await nativeCalls(page, 'cmd_touch_file_inventory')).length;
  await page.clock.fastForward(61_000);
  await expect.poll(async () => (await nativeCalls(page, 'cmd_touch_file_inventory')).length).toBeGreaterThan(before);
  expect((await nativeCalls(page, 'cmd_get_files')).length).toBe(reads);
  await page.evaluate(() => { (window as any).__desktopTest.owner = '202'; document.dispatchEvent(new Event('visibilitychange')); });
  await expect(page.getByText('Work saved photo.jpg', { exact: true })).toBeVisible();
  const switched = (await nativeCalls(page, 'cmd_touch_file_inventory')).length;
  await page.clock.fastForward(61_000);
  await expect.poll(async () => (await nativeCalls(page, 'cmd_touch_file_inventory')).length).toBeGreaterThan(switched);
  const recent = (await nativeCalls(page, 'cmd_touch_file_inventory')).slice(switched);
  expect(recent.every(call => call.args.ownerId === '202' && call.args.folderId === null)).toBe(true);
});


test('a continuing inventory build retains rows and retries the same request until the snapshot completes', async ({ page }) => {
  await desktopFixture(page);
  await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Saved Messages', exact: true }).click();
  await expect(page.getByText('Holiday saved photo.jpg', { exact: true })).toBeVisible();
  const before = (await nativeCalls(page, 'cmd_get_files')).length;
  await page.evaluate(() => {
    const state = (window as any).__desktopTest;
    state.inventoryName = 'Completed large inventory.jpg';
    state.inventoryBuilding = 1;
    state.emit('file-inventory-changed', { ownerId: '101', folderId: null });
  });
  await expect.poll(async () => (await nativeCalls(page, 'cmd_get_files')).length).toBeGreaterThan(before);
  await expect(page.getByText('Holiday saved photo.jpg', { exact: true })).toBeVisible();
  await expect(page.getByText('Error loading files', { exact: true })).toHaveCount(0);
  await expect(page.getByText('Completed large inventory.jpg', { exact: true })).toBeVisible({ timeout: 10_000 });
  const calls = (await nativeCalls(page, 'cmd_get_files')).slice(before);
  expect(calls).toHaveLength(2);
  expect(calls[1].args.requestId).toBe(calls[0].args.requestId);
});


test('a visible folder renews observation after its inventory is evicted', async ({ page }) => {
  await page.clock.install();
  await desktopFixture(page);
  await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Saved Messages', exact: true }).click();
  await expect(page.getByText('Holiday saved photo.jpg', { exact: true })).toBeVisible();
  const reads = (await nativeCalls(page, 'cmd_get_files')).length;
  await page.evaluate(() => {
    const state = (window as any).__desktopTest;
    state.inventoryPresent = false;
    state.inventoryName = 'Renamed after inventory eviction.jpg';
  });
  await page.clock.fastForward(61_000);
  await expect.poll(async () => (await nativeCalls(page, 'cmd_get_files')).length).toBeGreaterThan(reads);
  await expect(page.getByText('Renamed after inventory eviction.jpg', { exact: true })).toBeVisible();
});


test('vault locking withdraws decrypted partial rows and a rejected completion cannot restore them', async ({ page }) => {
  await desktopFixture(page);
  await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Saved Messages', exact: true }).click();
  await expect(page.getByText('Holiday saved photo.jpg', { exact: true })).toBeVisible();
  await page.evaluate(() => {
    const state = (window as any).__desktopTest;
    state.protectedInterrupted = true;
    state.emit('file-inventory-changed', { ownerId: '101', folderId: null });
  });
  await expect(page.getByText('Private decrypted inventory name.pdf', { exact: true })).toBeVisible();
  await page.evaluate(() => {
    const state = (window as any).__desktopTest;
    state.vaultLocked = true;
    state.emit('vault-locked', {});
    state.releaseFiles();
  });
  await expect(page.getByText('Private decrypted inventory name.pdf', { exact: true })).toHaveCount(0);
  await expect(page.getByText('Holiday saved photo.jpg', { exact: true })).toBeVisible();
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  await expect(page.getByText('Private decrypted inventory name.pdf', { exact: true })).toHaveCount(0);
});


test('video cards request and display the shared thumbnail pipeline', async ({ page }) => {
  await desktopFixture(page, { video: true, settings: { viewMode: 'grid' } });
  await page.goto('/');
  await expect(page.getByText('Holiday folder video.mp4', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Saved Messages', exact: true }).click();
  await expect(page.getByText('Holiday saved video.mp4', { exact: true })).toBeVisible();
  await expect.poll(async () => (await nativeCalls(page, 'cmd_get_thumbnail')).length).toBeGreaterThan(0);
  await expect(page.getByRole('img', { name: 'Holiday saved video.mp4', exact: true })).toBeVisible();
});


test('an account change discards a pending thumbnail from the previous account', async ({ page }) => {
  await desktopFixture(page, { video: true, holdThumbnails: true, settings: { viewMode: 'grid' } });
  await page.goto('/');
  await expect(page.getByText('Holiday folder video.mp4', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Saved Messages', exact: true }).click();
  await expect.poll(async () => (await nativeCalls(page, 'cmd_get_thumbnail')).length).toBeGreaterThan(0);
  await page.evaluate(() => { (window as any).__desktopTest.owner = '202'; document.dispatchEvent(new Event('visibilitychange')); });
  const image = page.getByRole('img', { name: 'Work saved video.mp4', exact: true });
  await expect(image).toBeVisible();
  await expect(image).toHaveAttribute('src', /994488/);
  await page.evaluate(() => (window as any).__desktopTest.releaseThumbnails());
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  await expect(image).toHaveAttribute('src', /994488/);
  await expect(page.getByRole('img', { name: 'Holiday saved video.mp4', exact: true })).toHaveCount(0);
});

test('source replacement clears pending thumbnails even when the file name and size stay the same', async ({ page }) => {
  await desktopFixture(page, { video: true, holdThumbnails: true, settings: { viewMode: 'grid' } });
  await page.goto('/');
  await expect(page.getByText('Holiday folder video.mp4', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Saved Messages', exact: true }).click();
  await expect.poll(async () => (await nativeCalls(page, 'cmd_get_thumbnail')).length).toBeGreaterThan(0);
  await page.evaluate(() => {
    const state = (window as any).__desktopTest;
    state.holdThumbnails = false; state.thumbnailVersion = 1;
    state.emit('file-inventory-changed', { ownerId: '101', folderId: null });
  });
  const image = page.getByRole('img', { name: 'Holiday saved video.mp4', exact: true });
  await expect(image).toBeVisible();
  await expect(image).toHaveAttribute('src', /bb8844/);
  await page.evaluate(() => (window as any).__desktopTest.releaseThumbnails());
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  await expect(image).toHaveAttribute('src', /bb8844/);
});

test('switching accounts closes an already displayed preview from the previous account', async ({ page }) => {
  await desktopFixture(page, { settings: { viewMode: 'grid' } });
  await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Saved Messages', exact: true }).click();
  await page.getByRole('group', { name: 'Holiday saved photo.jpg', exact: true }).hover();
  await page.getByRole('button', { name: 'Preview Holiday saved photo.jpg', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Close preview', exact: true })).toBeVisible();
  await page.evaluate(() => { (window as any).__desktopTest.owner = '202'; document.dispatchEvent(new Event('visibilitychange')); });
  await expect(page.getByText('Work saved photo.jpg', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Close preview', exact: true })).toHaveCount(0);
  await expect(page.getByText('Holiday saved photo.jpg', { exact: true })).toHaveCount(0);
});

test('global Drive search collects pages and shows offline index coverage and errors', async ({ page }) => {
  await desktopFixture(page); await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible({ timeout: 30_000 });
  await page.evaluate(() => { (window as any).__desktopTest.searchPageSize = 1; });
  await page.getByPlaceholder('Search files...').fill('Holiday');
  await page.getByRole('button', { name: 'Search filters', exact: true }).click();
  await page.getByLabel('Search scope').selectOption('all');
  await page.getByLabel('Protection', { exact: true }).selectOption('plain');
  await page.getByRole('button', { name: 'Close', exact: true }).click();
  await page.getByRole('button', { name: 'Show more files', exact: true }).click();
  await expect(page.getByText('Holiday saved photo.jpg', { exact: true })).toBeVisible();
  await expect(page.getByRole('status').filter({ hasText: 'partial inventory' })).toContainText('Offline');
  expect((await nativeCalls(page, 'cmd_search_local')).some(call => call.args.query.offset === 1 && call.args.query.indexId === 'search-101')).toBe(true);
  expect(await nativeCalls(page, 'cmd_search_global')).toHaveLength(0);
  await page.evaluate(() => { const state=(window as any).__desktopTest;state.searchFails=true;state.emit('file-inventory-changed',{ownerId:'101'}); });
  await expect(page.getByText('Holiday saved photo.jpg', { exact: true })).toHaveCount(0);
  await expect(page.getByText('This operation could not finish. Refresh your library and retry.', { exact: true })).toBeVisible();
});

test('vault invalidation withdraws a pending global search result', async ({ page }) => {
  await desktopFixture(page); await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible();
  await page.evaluate(() => { (window as any).__desktopTest.searchHold = true; });
  await page.getByPlaceholder('Search files...').fill('Holiday');
  await page.getByRole('button', { name: 'Search filters', exact: true }).click();
  await page.getByLabel('Search scope').selectOption('all');
  await page.getByRole('button', { name: 'Close', exact: true }).click();
  await expect.poll(async () => (await nativeCalls(page,'cmd_search_local')).length).toBeGreaterThan(0);
  await page.evaluate(() => {const state=(window as any).__desktopTest;state.searchFails=true;state.emit('vault-locked',{});state.releaseSearch();});
  await expect(page.getByText('Holiday saved photo.jpg', { exact: true })).toHaveCount(0);
  await expect(page.getByText('This operation could not finish. Refresh your library and retry.', { exact: true })).toBeVisible();
});

test('facet-only Drive search waits for continuing inventory builds and refreshes on vault unlock', async ({ page }) => {
  await desktopFixture(page);await page.goto('/');await expect(page.getByText('Holiday folder photo.jpg',{exact:true})).toBeVisible();
  await page.evaluate(()=>{(window as any).__desktopTest.searchBuilding=1;});
  await page.getByRole('button',{name:'Search filters',exact:true}).click();await page.getByLabel('Search scope').selectOption('all');await page.getByLabel('File type').selectOption('image');await page.getByRole('button',{name:'Close',exact:true}).click();
  await expect(page.getByText('Holiday saved photo.jpg',{exact:true})).toBeVisible();
  expect((await nativeCalls(page,'cmd_search_local')).filter(call=>call.args.query.query==='').length).toBeGreaterThan(1);
  await page.evaluate(()=>{const state=(window as any).__desktopTest;state.searchFails=true;state.emit('vault-locked',{});});
  await expect(page.getByRole('alert').filter({hasText:'This operation could not finish'})).toBeVisible();
  await page.evaluate(()=>{const state=(window as any).__desktopTest;state.searchFails=false;state.emit('vault-unlocked',{});});
  await expect(page.getByText('Holiday saved photo.jpg',{exact:true})).toBeVisible();
});

test('duplicate message IDs in global search cannot enter ambiguous bulk selection', async ({ page }) => {
  await desktopFixture(page,{settings:{viewMode:'grid'}});await page.goto('/');await expect(page.getByText('Holiday folder photo.jpg',{exact:true})).toBeVisible();
  await page.evaluate(()=>{(window as any).__desktopTest.searchDuplicateIds=true;});
  await page.getByRole('button',{name:'Search filters',exact:true}).click();await page.getByLabel('Search scope').selectOption('all');await page.getByRole('button',{name:'Close',exact:true}).click();
  await expect(page.getByText('Holiday saved photo.jpg',{exact:true})).toBeVisible();
  await expect(page.getByRole('button',{name:'Select Holiday saved photo.jpg',exact:true})).toBeDisabled();
  await expect(page.getByRole('button',{name:'Select Holiday folder photo.jpg',exact:true})).toBeDisabled();
  await page.getByRole('button',{name:'Delete Holiday saved photo.jpg',exact:true}).click();
  await page.getByRole('dialog',{name:'Delete File',exact:true}).getByRole('button',{name:'Delete',exact:true}).click();
  await expect.poll(async()=>(await nativeCalls(page,'cmd_delete_file')).length).toBe(1);
  expect((await nativeCalls(page,'cmd_delete_file'))[0].args).toMatchObject({messageId:42,folderId:null,ownerId:'101'});
});


test('changing the vault passphrase sends the current passphrase and clears both secrets', async ({ page }) => {
  await desktopFixture(page, { encryptionReady: true, vaultState: 'unlocked' });
  await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible();
  await openSettings(page);
  await page.getByRole('button', { name: 'Encryption', exact: true }).click();
  await page.locator('summary').filter({ hasText: 'Change Vault Passphrase' }).click();
  await page.getByPlaceholder('Current vault passphrase', { exact: true }).fill('synthetic current passphrase');
  await page.getByPlaceholder('New vault passphrase', { exact: true }).fill('synthetic replacement passphrase');
  await page.getByPlaceholder('Confirm passphrase', { exact: true }).fill('synthetic replacement passphrase');
  await page.getByRole('button', { name: 'Change Vault Passphrase', exact: true }).click();
  await expect.poll(() => nativeCalls(page, 'cmd_change_vault_passphrase')).toHaveLength(1);
  expect((await nativeCalls(page, 'cmd_change_vault_passphrase'))[0].args).toEqual({ currentPassphrase: 'synthetic current passphrase', newPassphrase: 'synthetic replacement passphrase' });
  await expect(page.getByPlaceholder('Current vault passphrase', { exact: true })).toHaveValue('');
  await expect(page.getByPlaceholder('New vault passphrase', { exact: true })).toHaveValue('');
});
