import { expect, test } from '@playwright/test';
import { desktopFixture, openSettings } from './desktop-413-fixture';

test('normal-motion settings changes detach old panes and close immediately releases controls', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await desktopFixture(page, { settings: { performanceMode: false } });
  await page.goto('/');
  await openSettings(page);
  const dialog = page.getByRole('dialog', { name: 'Settings', exact: true });
  for (const category of ['Themes', 'Folder Sync', 'General', 'Themes', 'General']) {
    await dialog.getByRole('button', { name: category, exact: true }).click();
  }
  await expect(dialog.getByRole('switch', { name: 'Performance Mode', exact: true })).toBeVisible();
  await expect(page.locator('.settings-modal [data-ui-motion][aria-hidden="true"]')).toHaveCount(0);
  await dialog.getByRole('button', { name: 'Done', exact: true }).click();
  const released = await page.evaluate(() => {
    const outgoing = document.querySelector('[aria-labelledby="settings-dialog-title"]');
    return !outgoing || Boolean(outgoing.closest('[inert][aria-hidden="true"]'));
  });
  expect(released, 'A closing dialog must release input and accessibility immediately, before its visual exit ends').toBe(true);
  await openSettings(page);
  await expect(dialog.getByRole('switch', { name: 'Performance Mode', exact: true })).toBeEnabled();
  await page.keyboard.press('Escape');
  await expect(page.locator('[aria-labelledby="settings-dialog-title"]')).toHaveCount(0);
});

test('changing the OS motion preference preserves the manual performance setting', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await desktopFixture(page, { settings: { performanceMode: false } });
  await page.goto('/');
  await openSettings(page);
  const performance = page.getByRole('switch', { name: 'Performance Mode', exact: true });
  await expect(performance).toHaveAttribute('aria-checked', 'false');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  await expect(performance).toHaveAttribute('aria-checked', 'false');
});

// Real App/provider and persisted-settings journeys; the native boundary is simulated.
test('system reduced motion does not overwrite the saved Performance Mode choice', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await desktopFixture(page, { settings: { performanceMode: false } });
  await page.goto('/');
  await openSettings(page);
  const performance = page.getByRole('switch', { name: 'Performance Mode', exact: true });
  await expect(performance).toHaveAttribute('aria-checked', 'false');
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await expect(performance).toHaveAttribute('aria-checked', 'false');
  await performance.click();
  await expect(performance).toHaveAttribute('aria-checked', 'true');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await expect(performance).toHaveAttribute('aria-checked', 'true');
  await page.reload();
  await openSettings(page);
  await expect(performance).toHaveAttribute('aria-checked', 'true');
});

test('manual Performance Mode can stay off while the OS reduces motion and after restart', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await desktopFixture(page, { settings: { performanceMode: true } });
  await page.goto('/');
  await openSettings(page);
  const performance = page.getByRole('switch', { name: 'Performance Mode', exact: true });
  await expect(performance).toHaveAttribute('aria-checked', 'true');
  await performance.click();
  await expect(performance).toHaveAttribute('aria-checked', 'false');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await expect(performance).toHaveAttribute('aria-checked', 'false');
  await page.reload();
  await openSettings(page);
  await expect(performance).toHaveAttribute('aria-checked', 'false');
});

test('startup stays still while saved preferences are loading and retains the original logo', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await desktopFixture(page, { holdSettings: true, holdSupporter: true, settings: { performanceMode: true } });
  await page.goto('/');
  const logo = page.getByRole('img', { name: 'Telegram Drive', exact: true });
  await expect(logo).toBeVisible();
  await expect(logo).toHaveAttribute('src', '/logo.svg');
  await expect.poll(() => logo.evaluate(element => element.getAnimations().filter(animation => animation.playState === 'running').length)).toBe(0);
  await page.evaluate(() => (window as any).__desktopTest.releaseSettings());
  await expect(page.locator('body')).toHaveClass(/performance-mode/);
  await page.evaluate(() => (window as any).__desktopTest.releaseSupporter());
  await openSettings(page);
  await expect(page.getByRole('switch', { name: 'Performance Mode', exact: true })).toHaveAttribute('aria-checked', 'true');
  await page.keyboard.press('Escape');
  await expect(page.locator('aside img[src="/logo.svg"]')).toBeVisible();
});
