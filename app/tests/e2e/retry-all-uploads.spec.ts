import { expect, test, type Page } from '@playwright/test';
import { desktopFixture, nativeCalls } from './desktop-413-fixture';

// Shipped App/providers; native transport is controlled. The matching native
// process/SQLite journeys verify eligibility and durable retry separately.
const command = 'cmd_transfer_retry_failed_upload';
const job = (id: string, status = 'failed', extra: Record<string, unknown> = {}) => ({
  id, direction: 'upload', kind: 'local_upload', status, filename: `${id}.zip`,
  path: `/fixtures/${id}.zip`, folderId: 9, protectionMode: 'standard',
  progress: 75, transferredBytes: 75, totalBytes: 100, queuePosition: 0,
  revision: 1, createdAt: 1, updatedAt: 1, ...extra,
});
const retryAll = (page: Page) => page.getByRole('button', { name: /^Retry all failed/ });
async function ready(page: Page) {
  await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible();
}
async function release(page: Page) {
  await page.evaluate(() => (window as any).__desktopTest.releaseRetry());
}
const states = (page: Page) => page.evaluate(() => (window as any).__desktopTest.transfers);

test('one keyboard action retries 100 failed uploads without changing other work or its inputs', async ({ page }) => {
  const failures = Array.from({ length: 100 }, (_, index) => job(`failed-${index}`, 'failed', {
    queuePosition: index, folderId: index % 2 ? null : 9,
    protectionMode: index % 3 ? 'standard' : 'vault', protectMetadata: index % 3 === 0,
    videoUploadMode: index % 2 ? 'media' : 'file', videoFilenameCaption: index % 2 === 0,
    ...(index === 99 ? { kind: 'url_upload', url: 'https://example.invalid/original.zip' } : {}),
  }));
  const unaffected = ['uploading', 'pending', 'completed', 'paused', 'cancelled', 'waiting_for_unlock', 'waiting_for_network', 'cooldown']
    .map(status => job(status, status)).concat([
      job('failed-download', 'failed', { direction: 'download', kind: 'download', messageId: 42 }),
      job('other-owner', 'failed', { ownerId: '202' }),
    ]);
  await desktopFixture(page, { transfers: [...failures, ...unaffected] }); await ready(page);
  const before = await states(page);
  await retryAll(page).focus(); await page.keyboard.press('Enter');
  await expect.poll(async () => (await nativeCalls(page, command)).length).toBe(100);
  await expect(page.getByText('Queued: 100 · Skipped: 0 · Failed: 0 · Not attempted: 0', { exact: true })).toBeVisible();
  const calls = await nativeCalls(page, command);
  expect(new Set(calls.map((call: any) => call.args.id)).size).toBe(100);
  expect(calls.every((call: any) => call.args.ownerId === '101')).toBe(true);
  const after = await states(page);
  for (const previous of before) {
    const current = after.find((candidate: any) => candidate.id === previous.id);
    if (!previous.id.startsWith('failed-') || previous.id === 'failed-download') expect(current).toEqual(previous);
    else {
      expect(current).toMatchObject({ status: 'pending', revision: 2, progress: 0, transferredBytes: 0 });
      for (const key of ['id', 'ownerId', 'path', 'url', 'folderId', 'protectionMode', 'protectMetadata', 'videoUploadMode', 'videoFilenameCaption', 'queuePosition']) expect(current[key]).toEqual(previous[key]);
    }
  }
  await expect(page.getByRole('dialog')).toHaveCount(0);
});

test('rapid activation and per-file Retry cannot submit overlapping retries', async ({ page }) => {
  await desktopFixture(page, { holdRetry: true, transfers: [job('first'), job('second')] }); await ready(page);
  await retryAll(page).evaluate(element => { (element as HTMLButtonElement).click(); (element as HTMLButtonElement).click(); });
  await expect.poll(async () => (await nativeCalls(page, command)).length).toBe(1);
  await expect(retryAll(page)).toBeDisabled();
  await expect(retryAll(page)).toHaveAttribute('aria-busy', 'true');
  await expect(page.getByRole('button', { name: 'Retry this file: first.zip', exact: true })).toBeDisabled();
  await release(page);
  await expect(page.getByText('Queued: 2 · Skipped: 0 · Failed: 0 · Not attempted: 0', { exact: true })).toBeVisible();
  expect(await nativeCalls(page, command)).toHaveLength(2);
  expect(await nativeCalls(page, 'cmd_transfer_retry')).toHaveLength(0);
});

test('a failed request does not prevent later failures from being queued', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  await desktopFixture(page, { failRetryIds: ['broken'], transfers: [job('first'), job('broken'), job('last')] }); await ready(page);
  await retryAll(page).click();
  await expect(page.getByText('Queued: 2 · Skipped: 0 · Failed: 1 · Not attempted: 0', { exact: true })).toBeVisible();
  await expect(retryAll(page)).toBeEnabled();
  expect((await states(page)).find((item: any) => item.id === 'broken').status).toBe('failed');
  expect(errors).toEqual([]);
});

test('a batch respects cancellation, removal, and failures arriving after its snapshot', async ({ page }) => {
  await desktopFixture(page, { holdRetry: true, transfers: [job('cancelled-during-request'), job('removed'), job('new-failure', 'uploading')] }); await ready(page);
  await retryAll(page).click();
  await expect.poll(async () => (await nativeCalls(page, command)).length).toBe(1);
  await page.evaluate(() => {
    const state = (window as any).__desktopTest;
    state.upsertTransfer('cancelled-during-request', { status: 'cancelled' });
    state.transfers = state.transfers.filter((item: any) => item.id !== 'removed');
    state.emit('transfer-removed', 'removed');
    state.upsertTransfer('new-failure', { status: 'failed' });
  });
  await expect(page.getByTestId('transfer-row-removed')).toHaveCount(0);
  await release(page);
  await expect(page.getByText('Queued: 0 · Skipped: 2 · Failed: 0 · Not attempted: 0', { exact: true })).toBeVisible();
  expect(await nativeCalls(page, command)).toHaveLength(1);
  expect((await states(page)).map((item: any) => item.status)).toEqual(['cancelled', 'failed']);
});

test('an account change stops the old batch and cannot retry the new account work', async ({ page }) => {
  await desktopFixture(page, { holdRetry: true, transfers: [job('first'), job('second'), job('new-owner', 'failed', { ownerId: '202' })] }); await ready(page);
  await retryAll(page).click(); await expect.poll(async () => (await nativeCalls(page, command)).length).toBe(1);
  await page.evaluate(() => { (window as any).__desktopTest.owner = '202'; document.dispatchEvent(new Event('visibilitychange')); });
  await expect(page.getByText('Work folder photo.jpg', { exact: true })).toBeVisible();
  await release(page);
  await expect(retryAll(page)).toBeEnabled();
  expect(await nativeCalls(page, command)).toHaveLength(1);
  expect((await states(page)).every((item: any) => item.status === 'failed')).toBe(true);
  await expect(page.getByText(/^Queued: \d+ · Skipped:/)).toHaveCount(0);
});

for (const action of ['Pause', 'Cancel']) {
  test(`${action} during retry stops the remaining batch before controlling the queue`, async ({ page }) => {
    await desktopFixture(page, { holdRetry: true, transfers: [job('first'), job('second'), job('active', 'uploading')] }); await ready(page);
    await retryAll(page).click(); await expect.poll(async () => (await nativeCalls(page, command)).length).toBe(1);
    await page.getByRole('button', { name: `${action} Uploads`, exact: true }).click();
    await release(page);
    const control = `cmd_transfer_${action.toLowerCase()}_all`;
    await expect.poll(async () => (await nativeCalls(page, control)).length).toBe(1);
    expect(await nativeCalls(page, command)).toHaveLength(1);
    expect((await states(page)).find((item: any) => item.id === 'second').status).toBe('failed');
    expect((await states(page)).find((item: any) => item.id === 'first').status).toBe(action === 'Pause' ? 'paused' : 'cancelled');
  });
}

test('protected retries receive fresh per-file tokens and cancellation stops remaining prompts', async ({ page }) => {
  await desktopFixture(page, { transfers: [job('secret-a', 'failed', { protectionMode: 'passphrase', protectMetadata: true }), job('secret-b', 'failed', { protectionMode: 'vault_and_passphrase' }), job('secret-c', 'failed', { protectionMode: 'passphrase' }), job('later')] }); await ready(page);
  await retryAll(page).click();
  for (let index = 0; index < 2; index++) {
    const disclaimer = page.getByRole('dialog', { name: 'Keep your encryption credentials safe', exact: true });
    await disclaimer.getByRole('button', { name: 'Confirm', exact: true }).click();
    const secret = page.getByRole('dialog', { name: 'Encrypted with File Passphrase', exact: true });
    await expect(secret.locator('input[type="password"]')).toHaveCount(2);
    for (const input of await secret.locator('input[type="password"]').all()) await input.fill(`synthetic-test-passphrase-${index}`);
    await secret.getByRole('button', { name: 'Confirm', exact: true }).click();
    await expect.poll(async () => (await nativeCalls(page, command)).length).toBe(index + 1);
  }
  await page.getByRole('dialog', { name: 'Keep your encryption credentials safe', exact: true })
    .getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(page.getByText('Queued: 2 · Skipped: 0 · Failed: 0 · Not attempted: 2', { exact: true })).toBeVisible();
  const calls = await nativeCalls(page, command);
  expect(calls.map((call: any) => call.args.id)).toEqual(['secret-a', 'secret-b']);
  expect(calls.map((call: any) => call.args.promptToken)).toEqual([701, 702]);
  expect(await nativeCalls(page, 'cmd_transfer_supply_prompt_token')).toHaveLength(0);
  expect((await states(page)).slice(2).every((item: any) => item.status === 'failed')).toBe(true);
});
