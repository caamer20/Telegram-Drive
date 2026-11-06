import { expect, test, type Page } from '@playwright/test';
import { desktopFixture, openSettings } from './desktop-413-fixture';

const presets = [
  ['default-dark', 'Default Dark'], ['charcoal', 'Charcoal'], ['nord', 'Nord'],
  ['monokai', 'Monokai'], ['cyber-teal', 'Cyber Teal'], ['default-light', 'Default Light'],
  ['solarized-light', 'Solarized Light'], ['amoled', 'AMOLED'], ['ocean', 'Ocean'],
  ['forest', 'Forest'], ['plum', 'Plum'], ['paper', 'Paper'], ['rose', 'Rose'],
  ['mint', 'Mint'], ['sky', 'Sky'],
] as const;
const settings = (page: Page) => page.getByRole('dialog', { name: 'Settings', exact: true });
async function themes(page: Page) {
  await settings(page).getByRole('button', { name: 'Themes', exact: true }).click();
  await expect(settings(page).getByRole('button', { name: 'Create New Theme', exact: true })).toBeVisible();
}
async function accentContrast(page: Page) {
  return page.evaluate(() => {
    const style = getComputedStyle(document.documentElement);
    const luminance = (value: string) => {
      const match = value.trim().match(/^#([0-9a-f]{6})$/i);
      if (!match) throw new Error(`Expected a rendered hex theme token, got ${value}`);
      const channels = [0, 2, 4].map(offset => parseInt(match[1].slice(offset, offset + 2), 16) / 255)
        .map(channel => channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4);
      return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
    };
    const background = luminance(style.getPropertyValue('--color-app-accent'));
    const label = luminance(style.getPropertyValue('--color-app-accent-contrast'));
    return (Math.max(background, label) + 0.05) / (Math.min(background, label) + 0.05);
  });
}

// Real App/providers/settings UI, with only native/service boundaries controlled.
// This does not validate native theme notifications, credential storage or installed-app restart.
test('all built-in presets remain selectable, preserve their IDs and use readable accent labels', async ({ page }) => {
  await desktopFixture(page); await page.goto('/'); await openSettings(page); await themes(page);
  for (const [id, name] of presets) {
    const preset = settings(page).getByRole('button', { name, exact: true });
    await preset.click();
    await expect.poll(() => page.evaluate(() => localStorage.getItem('active-custom-theme-id'))).toBe(id);
    expect(await accentContrast(page), `${name} accent label`).toBeGreaterThanOrEqual(4.5);
    await expect(preset).toHaveAttribute('aria-pressed', 'true');
  }
  await page.reload(); await openSettings(page); await themes(page);
  await expect(settings(page).getByRole('button', { name: 'Sky', exact: true })).toHaveAttribute('aria-pressed', 'true');
});

test('selecting Default Dark preserves its translucent decorative border and the saved palette', async ({ page }) => {
  await desktopFixture(page); await page.goto('/'); await openSettings(page); await themes(page);
  await settings(page).getByRole('button', { name: 'Default Dark', exact: true }).click();
  await expect.poll(() => page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--color-app-border').trim())).toBe('rgba(255, 255, 255, 0.1)');
  expect(await accentContrast(page)).toBeGreaterThanOrEqual(4.5);
});

for (const performanceMode of [false, true]) {
  test(`base modes and presets survive Performance Mode ${performanceMode ? 'on' : 'off'}`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'no-preference', colorScheme: 'dark' });
    await desktopFixture(page); await page.goto('/'); await openSettings(page);
    const performance = settings(page).getByRole('switch', { name: 'Performance Mode', exact: true });
    if ((await performance.getAttribute('aria-checked')) !== String(performanceMode)) await performance.click();
    await expect(performance).toHaveAttribute('aria-checked', String(performanceMode)); await themes(page);
    for (const [name, preference, appearance] of [['Light Mode', 'light', 'light'], ['Dark Mode', 'dark', 'dark'], ['System', 'system', 'dark']] as const) {
      await settings(page).getByRole('button', { name, exact: true }).click();
      await expect(page.locator('html')).toHaveClass(new RegExp(`(^| )${appearance}( |$)`));
      await expect.poll(() => page.evaluate(() => localStorage.getItem('theme-preference'))).toBe(preference);
      await expect.poll(() => page.locator('body').evaluate(element => element.classList.contains('performance-mode'))).toBe(performanceMode);
    }
    await page.emulateMedia({ colorScheme: 'light' }); await expect(page.locator('html')).toHaveClass(/(^| )light( |$)/);
    for (const [id, name] of presets) {
      const preset = settings(page).getByRole('button', { name, exact: true });
      await preset.click();
      await expect.poll(() => page.evaluate(() => localStorage.getItem('active-custom-theme-id'))).toBe(id);
      await expect(preset).toHaveAttribute('aria-pressed', 'true');
      expect(await accentContrast(page), `${name}, Performance Mode ${performanceMode}`).toBeGreaterThanOrEqual(4.5);
      await expect.poll(() => page.locator('body').evaluate(element => element.classList.contains('performance-mode'))).toBe(performanceMode);
    }
    await page.reload(); await openSettings(page); await expect(performance).toHaveAttribute('aria-checked', String(performanceMode)); await themes(page);
    await expect(settings(page).getByRole('button', { name: 'Sky', exact: true })).toHaveAttribute('aria-pressed', 'true');
    await settings(page).getByRole('button', { name: 'Reset to Default', exact: true }).click();
    await expect(page.locator('html')).toHaveClass(/(^| )dark( |$)/);
    await expect.poll(() => page.evaluate(() => localStorage.getItem('active-custom-theme-id'))).toBe('');
    await expect.poll(() => page.evaluate(() => localStorage.getItem('theme-preference'))).toBe('default');
  });
}

for (const performanceMode of [false, true]) {
  test(`custom editing and recovery preserve the palette with Performance Mode ${performanceMode ? 'on' : 'off'}`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'no-preference' });
    await desktopFixture(page, { settings: { performanceMode } }); await page.goto('/'); await openSettings(page); await themes(page);
    await settings(page).getByRole('button', { name: 'Create New Theme', exact: true }).click();
    await settings(page).getByRole('textbox', { name: 'Name', exact: true }).fill('Travel theme');
    const base = settings(page).getByRole('group', { name: 'Base', exact: true });
    await base.getByRole('button', { name: 'Light Mode', exact: true }).click();
    await expect(page.locator('html')).toHaveClass(/(^| )light( |$)/);
    await base.getByRole('button', { name: 'Dark Mode', exact: true }).click();
    await expect(page.locator('html')).toHaveClass(/(^| )dark( |$)/);
    await settings(page).locator('input[type=text]').and(settings(page).getByRole('textbox', { name: 'Primary', exact: true })).fill('#2aabee');
    await settings(page).locator('input[type=text]').and(settings(page).getByRole('textbox', { name: 'Border', exact: true })).fill('rgba(255, 255, 255, 0.1)');
    await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('user-themes')!)[0].name)).toBe('Travel theme');
    const stored = await page.evaluate(() => localStorage.getItem('user-themes'));
    const id = await page.evaluate(() => localStorage.getItem('active-custom-theme-id'));
    expect(await accentContrast(page)).toBeGreaterThanOrEqual(4.5);
    await page.reload(); await openSettings(page); await themes(page);
    await expect(settings(page).getByRole('button', { name: 'Travel theme', exact: true })).toHaveAttribute('aria-pressed', 'true');
    await expect(settings(page).getByRole('textbox', { name: 'Name', exact: true })).toHaveValue('Travel theme');
    await settings(page).getByRole('button', { name: 'Delete Theme', exact: true }).click();
    const confirmation = page.getByRole('dialog', { name: 'Delete Theme', exact: true });
    await expect(confirmation).toBeVisible(); await page.keyboard.press('Escape'); await expect(confirmation).toBeHidden();
    await expect(settings(page)).toBeVisible();
    await expect(settings(page).getByRole('button', { name: 'Delete Theme', exact: true })).toBeFocused();
    expect(await page.evaluate(() => localStorage.getItem('user-themes'))).toBe(stored);
    expect(await page.evaluate(() => localStorage.getItem('active-custom-theme-id'))).toBe(id);
    await settings(page).getByRole('button', { name: 'Reset to Default', exact: true }).click();
    expect(await page.evaluate(() => localStorage.getItem('user-themes'))).toBe(stored);
    await settings(page).getByRole('button', { name: 'Travel theme', exact: true }).click();
    await settings(page).getByRole('button', { name: 'Delete Theme', exact: true }).click();
    await page.getByRole('dialog', { name: 'Delete Theme', exact: true }).getByRole('button', { name: 'Delete', exact: true }).click();
    await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('user-themes')!))).toEqual([]);
  });
}

test('normal-motion rapid categories, close/reopen and nested explanation keep one current settings context', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await desktopFixture(page); await page.goto('/'); await openSettings(page);
  for (const name of ['Themes', 'Privacy', 'Folder Sync', 'General', 'Themes', 'General']) await settings(page).getByRole('button', { name, exact: true }).click();
  await expect(settings(page).getByRole('switch', { name: 'Performance Mode', exact: true })).toBeVisible();
  await expect(settings(page).getByRole('button', { name: 'Create New Theme', exact: true })).toHaveCount(0);
  await page.keyboard.press('Escape'); await expect(settings(page)).toBeHidden(); await openSettings(page);
  await settings(page).getByRole('button', { name: 'Advanced', exact: true }).click();
  await settings(page).getByRole('button', { name: /REST API/ }).click();
  await settings(page).getByRole('button', { name: 'Understand REST permissions', exact: true }).click();
  const explanation = page.getByRole('dialog', { name: 'How REST access works', exact: true });
  await expect(explanation).toBeVisible(); await page.keyboard.press('Escape'); await expect(explanation).toBeHidden();
  await expect(settings(page)).toBeVisible();
  await expect(settings(page).getByRole('button', { name: 'Understand REST permissions', exact: true })).toBeFocused();
});

test('category search names its scope, exposes specialist categories and keeps narrow settings operable', async ({ page }) => {
  await desktopFixture(page); await page.goto('/'); await openSettings(page);
  const navigation = settings(page).getByRole('navigation', { name: 'Settings', exact: true });
  const search = navigation.getByRole('searchbox', { name: 'Find a category', exact: true });
  await search.fill('WebDAV');
  await expect(navigation.getByRole('button', { name: 'Advanced', exact: true })).toBeVisible();
  await navigation.getByRole('button', { name: 'Advanced', exact: true }).click();
  await settings(page).getByRole('button', { name: /^WebDAV/ }).click();
  await expect(settings(page).locator('#settings-dialog-title + p')).toHaveText('WebDAV');
  await expect(navigation.getByRole('button', { name: 'General', exact: true })).toHaveCount(0);
  await search.fill('unmatched-category');
  await expect(navigation.getByRole('status')).toHaveText('No matching categories');
  await search.fill('');
  for (const name of ['General', 'Themes', 'Privacy', 'Encryption', 'Folder Sync', 'Sharing', 'Advanced', 'Lifetime license purchased', 'About']) {
    await expect(navigation.getByRole('button', { name, exact: true })).toBeVisible();
  }
  await navigation.getByRole('button', { name: 'Themes', exact: true }).click();
  await page.setViewportSize({ width: 360, height: 780 });
  await settings(page).getByRole('button', { name: 'Create New Theme', exact: true }).click();
  await settings(page).getByRole('textbox', { name: 'Name', exact: true }).fill('Narrow theme');
  await expect.poll(() => settings(page).evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
  await expect(settings(page).locator('input[type=text]').and(settings(page).getByRole('textbox', { name: 'Primary', exact: true }))).toBeVisible();
  await settings(page).getByRole('button', { name: 'Done', exact: true }).click();
  await expect(settings(page)).toBeHidden();
});

test('a short-form custom accent keeps its palette and readable button text after restart', async ({ page }) => {
  await desktopFixture(page); await page.goto('/'); await openSettings(page); await themes(page);
  await settings(page).getByRole('button', { name: 'Create New Theme', exact: true }).click();
  await settings(page).getByRole('textbox', { name: 'Name', exact: true }).fill('Short accent');
  await settings(page).locator('input[type=text]').and(settings(page).getByRole('textbox', { name: 'Primary', exact: true })).fill('#123');
  const done = settings(page).getByRole('button', { name: 'Done', exact: true });
  await expect.poll(() => done.evaluate(element => getComputedStyle(element).color)).toBe('rgb(255, 255, 255)');
  await expect.poll(() => done.evaluate(element => getComputedStyle(element).backgroundColor)).toBe('rgb(17, 34, 51)');
  await page.reload(); await openSettings(page); await themes(page);
  await expect(settings(page).locator('input[type=text]').and(settings(page).getByRole('textbox', { name: 'Primary', exact: true }))).toHaveValue('#123');
  await expect.poll(() => done.evaluate(element => getComputedStyle(element).color)).toBe('rgb(255, 255, 255)');
});
