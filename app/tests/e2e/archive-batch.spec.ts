import { expect, test } from '@playwright/test';
import { desktopFixture, nativeCalls } from './desktop-413-fixture';

test('Extract All shares a download, continues after a damaged entry, and releases late output on close', async ({ page }) => {
  await desktopFixture(page, { files: [{ name: 'Research.zip', file_ext: 'zip', mime_type: 'application/zip', type: 'archive' }] });
  await page.goto('/');
  await page.getByText('Research.zip', { exact: true }).dblclick();
  const viewer = page.getByRole('dialog', { name: 'Archive contents: Research.zip' });
  const extract = viewer.getByRole('button', { name: /Extract All \(3\)/i });
  await page.evaluate(() => { (window as any).__desktopTest.archiveFailIndex = 1; });
  await extract.click();
  await expect.poll(async () => (await nativeCalls(page, 'initiate_upload')).length).toBe(2);
  await expect(extract).toBeVisible();
  const sourceReads = [...await nativeCalls(page, 'cmd_extract_archive_entry'), ...await nativeCalls(page, 'cmd_open_archive_batch')];
  expect(sourceReads).toHaveLength(1);
  expect(sourceReads[0].args).toMatchObject({ messageId: 42, folderId: 9 });
  const entries = await nativeCalls(page, 'cmd_extract_archive_batch_entry');
  const batchId = entries[0].args.batchId;
  expect(entries.map(({ args }) => [args.batchId, args.entryIndex])).toEqual([[batchId, 0], [batchId, 1], [batchId, 2]]);
  const uploads = await nativeCalls(page, 'initiate_upload');
  expect(uploads.map(({ args }) => [args.path.split('/').pop(), args.folderId])).toEqual([['one.txt', 9], ['three.txt', 9]]);
  expect((await nativeCalls(page, 'cmd_delete_temp_zip')).map(({ args }) => args.path)).toEqual(uploads.map(({ args }) => args.path));
  expect((await nativeCalls(page, 'cmd_close_archive_batch')).map(({ args }) => args.batchId)).toEqual([batchId]);

  await page.evaluate(() => { (window as any).__desktopTest.archiveHoldIndex = 0; });
  await extract.click();
  await expect.poll(async () => (await nativeCalls(page, 'cmd_extract_archive_batch_entry')).length).toBe(4);
  // Reload a real browser document while the old IPC request is suspended.
  // The companion native process journey exercises late source publication
  // through the same resource cleanup used by the native page-start hook.
  await page.reload();
  await page.getByText('Research.zip', { exact: true }).dblclick();
  await page.evaluate(() => { (window as any).__desktopTest.archiveHoldIndex = 0; });
  await extract.click();
  await expect.poll(async () => (await nativeCalls(page, 'cmd_extract_archive_batch_entry')).length).toBe(1);
  await page.keyboard.press('Escape');
  await expect(viewer).toHaveCount(0);
  await page.evaluate(() => { (window as any).__desktopTest.releaseArchive(); });
  await expect.poll(async () => (await nativeCalls(page, 'cmd_close_archive_batch')).length).toBe(1);
  expect(await nativeCalls(page, 'cmd_delete_temp_zip')).toHaveLength(1);
  expect(await nativeCalls(page, 'initiate_upload')).toHaveLength(0);
});
