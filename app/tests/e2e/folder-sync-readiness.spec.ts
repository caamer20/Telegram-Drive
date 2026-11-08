import { expect, test, type Page } from '@playwright/test';
import type { SyncPair, SyncStatus } from '../../src/types/sync';
import { desktopFixture, nativeCalls, openSettings } from './desktop-fixture';

const pairs: SyncPair[] = [
  ['Work backup', true], ['Photos backup', true], ['Paused backup', false],
].map(([label, isActive], index) => ({
  id: index + 1, localPath: `/fixture/${index + 1}`, channelId: index + 9,
  folderKey: String(index + 9), label: String(label), syncDirection: 'upload_only',
  isActive: Boolean(isActive), createdAt: 1, accountOwner: '101',
  preferences: { ignorePatterns: [], propagateDeletions: false, pauseOnConflicts: true },
}));

function waitingStatus(): SyncStatus {
  return {
    enabled: true, running: false, activePairs: 2, pendingOps: 0, conflicts: 0, lastError: null,
    pairs: pairs.map(pair => ({
      pairId: pair.id, phase: pair.isActive ? 'waiting' : 'paused',
      ...(pair.isActive ? { waitReason: 'telegram' } : {}),
      pendingOps: 0, conflicts: 0, lastError: pair.isActive ? null : 'Paused by you', lastCheckedAt: null,
    })),
  };
}

async function emitStatus(page: Page, status: SyncStatus) {
  await page.evaluate(status => {
    const fixture = (window as any).__desktopTest;
    fixture.syncStatus = status;
    fixture.emit('sync-status-changed', status);
  }, status);
}

async function syncSettings(page: Page) {
  await openSettings(page);
  const settings = page.getByRole('dialog', { name: 'Settings', exact: true });
  await settings.getByRole('button', { name: 'Folder Sync', exact: true }).click();
  return settings;
}

// Real dashboard, providers, settings and native event subscription; native
// readiness/status is controlled here. Native lifecycle coverage is separate.
test('delayed Telegram readiness is visible and recovers every active mapping without changing manual pauses', async ({ page }) => {
  await desktopFixture(page, { syncPairs: pairs, syncStatus: waitingStatus() });
  await page.goto('/');
  await expect(page.getByRole('button', { name: 'Waiting for Telegram', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Folders synced', exact: true })).toHaveCount(0);
  const settings = await syncSettings(page);
  for (const label of ['Work backup', 'Photos backup']) {
    const mapping = settings.locator('.quiet-surface').filter({ has: page.getByText(label, { exact: true }) });
    await expect(mapping.getByText('Waiting for Telegram', { exact: true })).toBeVisible();
    await expect(mapping.getByRole('button', { name: 'Pause mapping', exact: true })).toBeVisible();
  }
  const paused = settings.locator('.quiet-surface').filter({ has: page.getByText('Paused backup', { exact: true }) });
  await expect(paused.getByText('Paused', { exact: true })).toBeVisible();
  await expect(paused.getByRole('button', { name: 'Resume mapping', exact: true })).toBeVisible();

  const ready = waitingStatus();
  ready.pairs = ready.pairs.map(pair => pair.pairId === 3 ? pair : { ...pair, phase: 'ready', waitReason: undefined, lastCheckedAt: 1 });
  await emitStatus(page, ready);
  await expect(settings.getByText('Up to date', { exact: true })).toHaveCount(2);
  await expect(paused.getByText('Paused', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Folders synced', exact: true })).toBeVisible();
  expect(await nativeCalls(page, 'cmd_set_sync_pair_active')).toHaveLength(0);
  expect(await nativeCalls(page, 'cmd_toggle_sync')).toHaveLength(0);
});

test('waiting statuses never mask real sync errors and disabled sync remains disabled', async ({ page }) => {
  const status = waitingStatus();
  status.lastError = 'Required vault is locked';
  status.pairs[0] = { ...status.pairs[0], phase: 'paused', lastError: status.lastError };
  await desktopFixture(page, { syncPairs: pairs, syncStatus: status });
  await page.goto('/');
  await expect(page.getByRole('button', { name: 'Folder Sync needs attention', exact: true })).toBeVisible();
  const settings = await syncSettings(page);
  await expect(settings.getByText('Required vault is locked', { exact: true })).toBeVisible();
  await expect(settings.getByText('Waiting for Telegram', { exact: true })).toBeVisible();

  const disabled = waitingStatus();
  disabled.enabled = false;
  await emitStatus(page, disabled);
  await expect(page.getByRole('button', { name: 'Folder Sync disabled', exact: true })).toBeVisible();
});

test('a mapping waiting for its first check is not reported as already synced', async ({ page }) => {
  const status = waitingStatus();
  status.pairs = status.pairs.map(pair => ({ ...pair, waitReason: undefined }));
  await desktopFixture(page, { syncPairs: pairs, syncStatus: status });
  await page.goto('/');
  await expect(page.getByRole('button', { name: 'Waiting for the next check', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Folders synced', exact: true })).toHaveCount(0);
});
