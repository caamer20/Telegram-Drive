import { expect, test } from '@playwright/test';
import { desktopFixture } from './desktop-fixture';

test.beforeEach(async ({ page }) => {
  await desktopFixture(page);
  await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible({ timeout: 30_000 });
});

for (const width of [1920, 1000]) {
  test(`folder header fits its visible title and controls at ${width}px`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    const header = page.getByRole('region', { name: 'Files', exact: true });
    await expect(header.getByRole('heading', { level: 1 })).toBeVisible();
    const unusedHeight = () => header.evaluate(element => {
      const style = getComputedStyle(element);
      const content = element.firstElementChild!.getBoundingClientRect();
      return element.getBoundingClientRect().height - content.height
        - parseFloat(style.paddingTop) - parseFloat(style.paddingBottom)
        - parseFloat(style.borderTopWidth) - parseFloat(style.borderBottomWidth);
    });
    await expect.poll(unusedHeight).toBeCloseTo(0, 0);

    // Active filters can wrap; the header must follow the new content height.
    await header.getByRole('button', { name: 'File type', exact: true }).click();
    await page.getByRole('combobox', { name: 'File type', exact: true }).selectOption('image');
    await page.getByRole('combobox', { name: 'Date', exact: true }).selectOption('30d');
    await page.getByRole('combobox', { name: 'Size', exact: true }).selectOption('small');
    await page.getByRole('dialog', { name: 'Search filters', exact: true }).getByRole('button', { name: 'Close', exact: true }).click();
    await expect.poll(unusedHeight).toBeCloseTo(0, 0);
    const bounds = (await header.boundingBox())!;
    for (const button of await header.getByRole('button').all()) {
      const box = (await button.boundingBox())!;
      expect(box.x).toBeGreaterThanOrEqual(bounds.x);
      expect(box.x + box.width).toBeLessThanOrEqual(bounds.x + bounds.width);
      expect(box.y + box.height).toBeLessThanOrEqual(bounds.y + bounds.height);
    }
    await header.screenshot({ path: testInfo.outputPath(`folder-header-${width}.png`) });
  });
}

test('filters open on every click and support keyboard dismissal and menu switching', async ({ page }) => {
  const trigger = page.getByRole('button', { name: 'Search filters', exact: true });
  const scope = page.getByRole('combobox', { name: 'Search scope', exact: true });
  for (let attempt = 0; attempt < 3; attempt++) {
    await trigger.click();
    await expect(trigger).toHaveAttribute('aria-expanded', 'true');
    await expect(scope).toBeVisible();
    await trigger.click();
    await expect(scope).toHaveCount(0);
  }
  await trigger.focus();
  await page.keyboard.press('Enter');
  await expect(scope).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(scope).toHaveCount(0);
  await expect(trigger).toBeFocused();
  await page.keyboard.press('Space');
  await expect(scope).toBeVisible();
  await page.getByRole('button', { name: 'Sort files', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(scope).toHaveCount(0);
  await trigger.click();
  await expect(page.getByRole('group', { name: 'Sort files', exact: true })).toHaveCount(0);
  await page.locator('[data-file-search]').click();
  await expect(scope).toHaveCount(0);
  await page.keyboard.press('Escape');
  await expect(page.locator('[data-file-search]')).not.toBeFocused();
  await trigger.click();
  await page.getByRole('dialog', { name: 'Search filters', exact: true }).getByRole('button', { name: 'Close', exact: true }).click();
  await expect(scope).toHaveCount(0);
  await expect(trigger).toBeFocused();
});

test('filters apply immediately, persist when closed, and reset without clearing search', async ({ page }) => {
  const trigger = page.getByRole('button', { name: 'Search filters', exact: true });
  const file = page.getByText('Holiday folder photo.jpg', { exact: true });
  await page.locator('[data-file-search]').fill('Holiday');
  await trigger.click();
  await page.getByRole('combobox', { name: 'File type', exact: true }).selectOption('video');
  await expect(file).toHaveCount(0);
  await page.getByRole('combobox', { name: 'File type', exact: true }).selectOption('image');
  await expect(file).toBeVisible();
  await page.getByRole('combobox', { name: 'Size', exact: true }).selectOption('large');
  await expect(file).toHaveCount(0);
  await page.getByRole('button', { name: 'Reset filters', exact: true }).click();
  await expect(file).toBeVisible();
  await expect(page.locator('[data-file-search]')).toHaveValue('Holiday');
  await trigger.click();
  await page.locator('[data-file-search]').fill('');
  await trigger.click();
  await page.getByRole('combobox', { name: 'Search scope', exact: true }).selectOption('all');
  await trigger.click();
  await expect(trigger).toHaveAttribute('data-active', 'true');
  await trigger.click();
  await expect(page.getByRole('combobox', { name: 'Search scope', exact: true })).toHaveValue('all');
  await page.getByRole('button', { name: 'Reset filters', exact: true }).click();
  await trigger.click();
  await expect(trigger).toHaveAttribute('data-active', 'false');
});

for (const theme of ['dark', 'light'] as const) {
  test(`filter controls fit the minimum desktop window in ${theme} theme`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width: 1000, height: 650 });
    if (theme === 'light') {
      await page.getByRole('button', { name: 'Preferences', exact: true }).click();
      await page.getByRole('button', { name: 'Light Mode', exact: true }).click();
    }
    const trigger = page.getByRole('button', { name: 'Search filters', exact: true });
    const triggerBounds = (await trigger.boundingBox())!;
    const uploadBounds = (await page.getByRole('button', { name: 'Upload', exact: true }).boundingBox())!;
    expect(triggerBounds.x + triggerBounds.width).toBeLessThanOrEqual(uploadBounds.x);
    await trigger.click({ position: { x: triggerBounds.width - 4, y: triggerBounds.height / 2 } });
    await expect(page.getByRole('combobox', { name: 'Search scope', exact: true })).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath(`filters-${theme}.png`) });
    const panel = page.getByRole('dialog', { name: 'Search filters', exact: true });
    await expect(panel).toBeVisible();
    const bounds = await panel.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(1000);
    expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(650);
    const search = await page.locator('[data-file-search]').boundingBox();
    expect(search!.width).toBeGreaterThan(100);
    for (const select of await panel.getByRole('combobox').all()) {
      const box = await select.boundingBox();
      expect(box!.width).toBeGreaterThan(200);
      await expect(select).toHaveCSS('color-scheme', theme);
    }
  });
}
