import { expect, test } from '@playwright/test';
import { desktopFixture } from './desktop-fixture';

test('global search separates failures, partial pages and a completed empty result', async ({ page }) => {
  await desktopFixture(page, { searchJourney: true, settings: { viewMode: 'grid' } });
  await page.goto('/');
  await page.getByRole('button', { name: 'Search filters', exact: true }).click({ timeout: 30_000 });
  await page.getByLabel('Search scope').selectOption('all');
  await page.getByRole('button', { name: 'Close', exact: true }).click();
  const input = page.locator('[data-file-search]');
  await input.fill('Match');
  const feedback = page.getByRole('alert').filter({ hasText: 'This operation could not finish' });
  await expect(feedback).toBeVisible();
  await expect(page.getByRole('heading', { name: 'This folder is empty' })).toHaveCount(0);
  await expect(page.getByText('No matching files. Scan your folders or adjust the filters.', { exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.getByText('Match first.pdf', { exact: true })).toBeVisible();
  const more = page.getByRole('button', { name: 'Show more files', exact: true });
  await more.click();
  await expect(feedback).toBeVisible();
  await expect(page.getByText('Match first.pdf', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.getByText('Match second.pdf', { exact: true })).toBeVisible();
  const matches = page.getByRole('group', { name: /^Match .*\.pdf$/ });
  await expect.poll(() => matches.evaluateAll(rows => rows.map(row => row.getAttribute('aria-label')))).toEqual([
    'Match first.pdf', 'Match second.pdf',
  ]);
  await expect(more).toHaveCount(0);
  await input.fill('partial');
  await expect(page.getByText('0 indexed results · partial inventory', { exact: true })).toBeVisible();
  await expect(more).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'This folder is empty' })).toHaveCount(0);
  await expect(page.getByText('No matching files. Scan your folders or adjust the filters.', { exact: true })).toHaveCount(0);
  await input.fill('nonexistent');
  await expect(page.getByText('No matching files. Scan your folders or adjust the filters.', { exact: true })).toBeVisible();
  await expect(feedback).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'This folder is empty' })).toHaveCount(0);
  await expect(page.getByText('Match first.pdf', { exact: true })).toHaveCount(0);
});
