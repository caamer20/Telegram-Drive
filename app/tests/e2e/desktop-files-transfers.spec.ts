import { expect, test, type Page } from '@playwright/test';
import { desktopFixture, nativeCalls } from './desktop-413-fixture';

// All journeys use the shipped App/provider tree. Only native/Tauri transport
// and Telegram responses are controlled; these are not real-transfer tests.
const transfer = (id: string, direction: 'upload' | 'download', status: string, extra: Record<string, unknown> = {}) => ({
  id, direction, kind: direction === 'upload' ? 'local_upload' : 'download', status,
  folderId: 9, messageId: 70, path: `/fixtures/${id}.zip`, filename: `${id}.zip`,
  progress: 0, transferredBytes: 0, totalBytes: 0, speedBytesPerSec: 0,
  queuePosition: 0, createdAt: 1, updatedAt: 1, ...extra,
});

async function ready(page: Page) {
  await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible({ timeout: 30_000 });
}

test('desktop verification with unknown progress shows its stage without a fabricated percentage', async ({ page }) => {
  await desktopFixture(page, { transfers: [transfer('verify-unknown', 'download', 'verifying', { progress: undefined })] });
  await ready(page);
  const row = page.getByTestId('transfer-row-verify-unknown');
  await expect(row).toContainText('Verifying file');
  await expect(row.getByRole('progressbar')).toHaveCount(0);
  await expect(row).not.toContainText(/\d+%/);
  await expect(row.locator('[style*="18%"]')).toHaveCount(0);
});

test('a failed or skipped transfer is not presented as completed work', async ({ page }) => {
  await desktopFixture(page, { transfers: [
    transfer('failed-upload', 'upload', 'failed', { error: 'Connection interrupted' }),
    transfer('kept-original', 'download', 'completed', { progress: 100, downloadOutcome: 'skipped' }),
  ] });
  await ready(page);
  const center = page.getByTestId('transfer-center');
  await expect(center.locator('header').first()).not.toContainText('Completed');
  await expect(center).toContainText('Needs attention');
  await expect(page.getByTestId('transfer-row-kept-original')).toContainText('Skipped — existing file kept');
});

test('a download waiting through cooldown can still be paused and resumed at queue level', async ({ page }) => {
  await desktopFixture(page, { transfers: [transfer('download-cooldown', 'download', 'cooldown')] });
  await ready(page);
  const center = page.getByTestId('transfer-center');
  const row = page.getByTestId('transfer-row-download-cooldown');
  await expect(center.getByRole('button', { name: 'Pause Downloads', exact: true })).toBeVisible();
  await center.getByRole('button', { name: 'Pause Downloads', exact: true }).click();
  await expect(row).toContainText('Paused');
  await center.getByRole('button', { name: 'Resume Downloads', exact: true }).click();
  await expect(row).toContainText('Queued');
});

test('desktop transfer controls preserve queue direction, per-file cancellation, retry and details', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await desktopFixture(page, { transfers: [
    transfer('active-upload', 'upload', 'uploading', { progress: 68, transferredBytes: 680, totalBytes: 1000, protectionMode: 'standard', speedBytesPerSec: 100 }),
    transfer('active-download', 'download', 'downloading', { progress: 45, transferredBytes: 450, totalBytes: 1000 }),
    transfer('failed-upload', 'upload', 'failed', { error: 'Connection interrupted' }),
  ] });
  await ready(page);
  const center = page.getByTestId('transfer-center');
  const upload = page.getByTestId('transfer-row-active-upload');
  const download = page.getByTestId('transfer-row-active-download');
  const failed = page.getByTestId('transfer-row-failed-upload');
  await expect(failed.getByRole('button', { name: /^Cancel/ })).toHaveCount(0);
  await expect(upload.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '68');
  await expect(upload.getByRole('button', { name: /^Pause/ })).toHaveCount(0);
  await upload.locator('summary').click();
  await expect(upload.locator('dl')).toContainText('Photos');
  await center.getByRole('button', { name: 'Pause Uploads', exact: true }).click();
  await expect(upload).toContainText('Paused');
  await expect(download).toContainText('Downloading');
  expect((await nativeCalls(page, 'cmd_transfer_pause_all')).at(-1)?.args).toEqual({ direction: 'upload', ownerId: '101' });
  expect(await nativeCalls(page, 'cmd_transfer_pause')).toHaveLength(0);
  await center.getByRole('button', { name: 'Resume Uploads', exact: true }).click();
  await expect(upload).toContainText('Queued');
  expect((await nativeCalls(page, 'cmd_transfer_resume_all')).at(-1)?.args).toEqual({ direction: 'upload', ownerId: '101' });
  await failed.getByRole('button', { name: /Retry this file/ }).focus();
  await page.keyboard.press('Enter');
  await expect(failed).toContainText('Queued');
  expect((await nativeCalls(page, 'cmd_transfer_retry')).at(-1)?.args).toEqual({ id: 'failed-upload', ownerId: '101' });
  await upload.getByRole('button', { name: /^Cancel/ }).click();
  await expect(upload).toContainText('Cancelled');
  await expect(upload.getByRole('progressbar')).toHaveCount(0);
  expect((await nativeCalls(page, 'cmd_transfer_cancel')).at(-1)?.args).toEqual({ id: 'active-upload', ownerId: '101' });
  await expect(download).toContainText('Downloading');
  await center.getByRole('button', { name: 'Close Transfers', exact: true }).click();
  await expect(upload).toHaveCount(0);
  await center.getByRole('button', { name: 'Open Transfers', exact: true }).click();
  await expect(upload).toContainText('Cancelled');
  await page.evaluate(() => (window as any).__desktopTest.emit('transfer-upserted', {
    ...(window as any).__desktopTest.transfers.find((job: any) => job.id === 'active-upload'),
    status: 'completed', revision: 1, progress: 100,
  }));
  await expect(upload).toContainText('Cancelled');
  await center.getByRole('region', { name: 'Uploads', exact: true }).getByRole('button', { name: 'Clear', exact: true }).click();
  await expect(upload).toHaveCount(0);
  await expect(download).toBeVisible();
  expect((await nativeCalls(page, 'cmd_transfer_clear_terminal')).at(-1)?.args).toEqual({ direction: 'upload', includeFailedAndCancelled: true, ownerId: '101' });
});

test('the folder header grows only for visible selection actions and existing view, sort, search and picker actions work', async ({ page }) => {
  await desktopFixture(page, { pickedUploadPaths: ['/fixtures/New file.txt'] });
  await ready(page);
  await page.getByRole('group', { name: 'Photos', exact: true }).getByText('Photos', { exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Photos', exact: true })).toBeVisible();
  const file = page.getByText('Holiday folder photo.jpg', { exact: true });
  const header = page.getByRole('region', { name: 'Files', exact: true });
  const compact = (await header.boundingBox())!.height;
  const before = (await file.boundingBox())!.y;
  await file.click();
  await expect(page.getByText('1 file selected', { exact: true })).toBeVisible();
  const expanded = (await header.boundingBox())!.height;
  expect(expanded).toBeGreaterThan(compact);
  expect((await file.boundingBox())!.y - before).toBeCloseTo(expanded - compact, 0);
  for (const name of ['Download', 'Move to...', 'Share', 'Clear', 'Delete']) {
    await expect(header.getByRole('button', { name, exact: true })).toBeVisible();
  }
  await page.getByRole('button', { name: 'Clear', exact: true }).click();
  await expect(page.getByText('1 file selected', { exact: true })).toBeHidden();
  expect((await header.boundingBox())!.height).toBeCloseTo(compact, 0);
  expect((await file.boundingBox())!.y).toBeCloseTo(before, 0);
  await expect(page.getByRole('button', { name: 'Actions for Holiday folder photo.jpg', exact: true })).toBeVisible();
  await page.locator('[data-file-search]').fill('Holiday');
  await expect(file).toBeVisible();
  await page.getByRole('button', { name: 'Sort files', exact: true }).click();
  await page.getByRole('group', { name: 'Sort files', exact: true }).getByRole('button', { name: 'Date', exact: true }).click();
  await page.locator('[data-file-search]').click();
  await page.getByRole('button', { name: 'Switch to Grid', exact: true }).click();
  await expect(page.getByRole('group', { name: 'Holiday folder photo.jpg', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Switch to List', exact: true }).click();
  await expect(file).toBeVisible();
  await page.getByRole('button', { name: 'Upload', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: /^Store Upload normally/ }).click();
  await expect.poll(async () => (await nativeCalls(page, 'cmd_transfer_enqueue_many')).length).toBe(1);
  expect((await nativeCalls(page, 'cmd_transfer_enqueue_many'))[0].args.requests[0]).toMatchObject({ filename: 'New file.txt', ownerId: '101', folderId: 9, protectionMode: 'standard' });
});

test('file filter shortcuts toggle the existing filter panel and keep its outside dismissal', async ({ page }) => {
  await desktopFixture(page);
  await ready(page);
  const toolbar = page.getByRole('region', { name: 'Files', exact: true });
  const panel = page.getByRole('dialog', { name: 'Search filters', exact: true });
  for (const name of ['File type', 'Date']) {
    await toolbar.getByRole('button', { name, exact: true }).click();
    await expect(panel).toBeVisible();
    await toolbar.getByRole('button', { name, exact: true }).click();
    await expect(panel).toBeHidden();
  }
  await toolbar.getByRole('button', { name: 'File type', exact: true }).click();
  await page.locator('[data-file-search]').click();
  await expect(panel).toBeHidden();
});

test('focused toolbar controls handle Enter without opening the selected file', async ({ page }) => {
  await desktopFixture(page);
  await ready(page);
  await page.getByRole('group', { name: 'Photos', exact: true }).getByText('Photos', { exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Photos', exact: true })).toBeVisible();
  await page.getByText('Holiday folder photo.jpg', { exact: true }).click();
  await page.getByRole('button', { name: 'Switch to Grid', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('group', { name: 'Holiday folder photo.jpg', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Close preview', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Move to...', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('dialog')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Close preview', exact: true })).toHaveCount(0);
});
