import { expect, test } from '@playwright/test';
import { desktopFixture, nativeCalls } from './desktop-fixture';

for (const direction of ['upload', 'download'] as const) {
  test(`Clear finished removes completed, failed and cancelled ${direction}s without clearing live work`, async ({ page }) => {
    await desktopFixture(page, { populatedTransfers: true });
    await page.goto('/');
    const center = page.getByRole('complementary', { name: 'Transfer activity', exact: true });
    const original = `Fixture ${direction}.bin`;
    await expect(center.getByText(original, { exact: true })).toBeVisible();

    const terminal = ['completed', 'failed', 'cancelled'];
    const retained = ['pending', 'paused', 'waiting_for_network', 'waiting_for_unlock', 'cooldown',
      direction === 'upload' ? 'uploading' : 'downloading'];
    await page.evaluate(({ direction, statuses }) => {
      const state = (window as any).__desktopTest;
      for (const [index, status] of [...statuses, 'foreign', 'persisting'].entries()) {
        const filename = `clear-${direction}-${status}.bin`;
        const job = { id: `clear-${direction}-${status}`, direction,
          kind: direction === 'upload' ? 'local_upload' : 'download',
          status: status === 'foreign' ? 'failed' : status === 'persisting' ? 'cancelled' : status,
          persistencePending: status === 'persisting', filename, path: `/fixture/${filename}`,
          messageId: 100 + index, ownerId: status === 'foreign' ? '202' : '101', folderId: 9,
          progress: 40, transferredBytes: 400, totalBytes: 1000, queuePosition: 10 + index,
          revision: 1, createdAt: Date.now(), updatedAt: Date.now() };
        state.jobs.push(job);
        state.emit('transfer-upserted', job);
      }
    }, { direction, statuses: [...terminal, ...retained] });
    for (const status of [...terminal, ...retained]) {
      await expect(center.getByText(`clear-${direction}-${status}.bin`, { exact: true })).toBeVisible();
    }
    await expect(center.getByText(`clear-${direction}-foreign.bin`, { exact: true })).toHaveCount(0);
    await expect(center.getByText(`clear-${direction}-persisting.bin`, { exact: true })).toBeVisible();

    await center.getByRole('button', {
      name: `Cancel ${original}`,
      exact: true,
    }).click();
    const cancelledRow = center.locator('article').filter({ has: page.getByText(original, { exact: true }) });
    await expect(cancelledRow).toContainText('Cancelled');
    const section = center.locator('section').filter({ has: page.getByText(direction === 'upload' ? 'Uploads' : 'Downloads', { exact: true }) });
    await section.getByRole('button', { name: 'Clear', exact: true }).click();

    for (const status of terminal) {
      await expect(center.getByText(`clear-${direction}-${status}.bin`, { exact: true })).toHaveCount(0);
    }
    await expect(center.getByText(original, { exact: true })).toHaveCount(0);
    if (direction === 'upload') await expect(center.getByText('Fixture retry.bin', { exact: true })).toHaveCount(0);
    else await expect(center.getByText('Fixture finished.bin', { exact: true })).toHaveCount(0);
    for (const status of retained) {
      await expect(center.getByText(`clear-${direction}-${status}.bin`, { exact: true })).toBeVisible();
    }
    const otherDirection = direction === 'upload' ? 'download' : 'upload';
    await expect(center.getByText(`Fixture ${otherDirection}.bin`, { exact: true })).toBeVisible();
    await expect(center.getByText(direction === 'upload' ? 'Fixture finished.bin' : 'Fixture retry.bin', { exact: true })).toBeVisible();
    expect((await nativeCalls(page, 'cmd_transfer_clear_terminal')).map(call => call.args)).toEqual([
      { direction, includeFailedAndCancelled: true, ownerId: '101' },
    ]);
    const remaining = await page.evaluate(() => (window as any).__desktopTest.jobs.map((job: any) => job.id));
    expect(remaining).toContain(`clear-${direction}-foreign`);
    expect(remaining).toContain(`clear-${direction}-persisting`);
    await expect(center.getByText(`clear-${direction}-persisting.bin`, { exact: true })).toBeVisible();
    for (const status of retained) expect(remaining).toContain(`clear-${direction}-${status}`);
    for (const status of terminal) expect(remaining).not.toContain(`clear-${direction}-${status}`);

    // Repeated clearing keeps the same live, waiting and foreign-account records.
    await section.getByRole('button', { name: 'Clear', exact: true }).click();
    await expect.poll(async () => (await nativeCalls(page, 'cmd_transfer_clear_terminal')).length).toBe(2);
    expect(await page.evaluate(() => (window as any).__desktopTest.jobs.map((job: any) => job.id))).toEqual(remaining);
  });
}
