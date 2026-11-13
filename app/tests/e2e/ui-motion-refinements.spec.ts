import { expect, test } from '@playwright/test';
import { desktopFixture, openSettings } from './desktop-413-fixture';

// Actual App/provider journeys; Telegram and OS IPC are controlled by the fixture.
test('light-theme transfer status labels remain readable', async ({ page }) => {
  await desktopFixture(page, { transfers: [
    { id: 'badge-success', direction: 'upload', kind: 'local_upload', status: 'completed', filename: 'uploaded.zip', progress: 100 },
    { id: 'badge-failure', direction: 'upload', kind: 'local_upload', status: 'failed', filename: 'retry.zip', error: 'Connection interrupted' },
  ] });
  await page.goto('/'); await openSettings(page);
  const settings = page.getByRole('dialog', { name: 'Settings', exact: true });
  await settings.getByRole('button', { name: 'Themes', exact: true }).click();
  await settings.getByRole('button', { name: 'Light Mode', exact: true }).click();
  await settings.getByRole('button', { name: 'Done', exact: true }).click();
  for (const id of ['badge-success', 'badge-failure']) {
    const badge = page.getByTestId(`transfer-row-${id}`).locator('span.rounded-full').first();
    await expect(badge).toBeVisible();
    const contrast = await badge.evaluate(element => {
      const ctx = document.createElement('canvas').getContext('2d')!;
      const rgba = (color: string) => {
        ctx.clearRect(0, 0, 1, 1); ctx.fillStyle = color; ctx.fillRect(0, 0, 1, 1);
        const [r, g, b, a] = ctx.getImageData(0, 0, 1, 1).data; return [r / 255, g / 255, b / 255, a / 255];
      };
      let background = [1, 1, 1, 1];
      const parents: Element[] = []; for (let current: Element | null = element; current; current = current.parentElement) parents.unshift(current);
      for (const parent of parents) { const color = rgba(getComputedStyle(parent).backgroundColor); background = color.slice(0, 3).map((c, i) => c * color[3] + background[i] * (1 - color[3])).concat(1); }
      const luminance = (color: number[]) => color.slice(0, 3).map(c => c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4).reduce((sum, c, i) => sum + c * [0.2126, 0.7152, 0.0722][i], 0);
      const a = luminance(rgba(getComputedStyle(element).color)); const b = luminance(background);
      return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
    });
    expect(contrast, `${id} text contrast`).toBeGreaterThanOrEqual(4.5);
  }
});

for (const source of ['system', 'manual'] as const) {
  test(`${source} motion reduction finishes an active settings transition`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'no-preference' });
    await desktopFixture(page, { settings: { performanceMode: false } });
    await page.goto('/'); await openSettings(page);
    const dialog = page.getByRole('dialog', { name: 'Settings', exact: true });
    await dialog.getByRole('button', { name: 'Themes', exact: true }).click();
    await expect(dialog.getByRole('button', { name: 'Create New Theme', exact: true })).toBeVisible();
    const active = await dialog.evaluate(async element => {
      const originalAnimate = Element.prototype.animate;
      const animations: Animation[] = [];
      let started!: (animation: Animation) => void;
      const created = new Promise<Animation>(resolve => { started = resolve; });
      // Slow the real General-pane animation at creation. Polling after a frame
      // can miss its entire 120 ms lifetime when a CI runner is busy.
      Element.prototype.animate = function (this: Element, ...args: Parameters<Element['animate']>) {
        const animation = originalAnimate.apply(this, args);
        if (element.contains(this) && this.matches('[data-ui-motion][aria-label="General"]')) {
          animation.playbackRate = 0.01;
          animations.push(animation);
          started(animation);
        }
        return animation;
      };
      let timeout: ReturnType<typeof setTimeout> | undefined;
      try {
        (Array.from(element.querySelectorAll('button')).find(button => button.textContent === 'General') as HTMLButtonElement).click();
        await Promise.race([
          created.then(animation => animation.ready),
          new Promise<never>((_, reject) => {
            timeout = setTimeout(() => reject(new Error('The General pane did not start an animation')), 10_000);
          }),
        ]);
        return animations.filter(animation => animation.playState === 'running').length;
      } finally {
        clearTimeout(timeout);
        Element.prototype.animate = originalAnimate;
      }
    });
    expect(active).toBeGreaterThan(0);
    await expect(dialog.getByRole('switch', { name: 'Performance Mode', exact: true })).toBeVisible();
    if (source === 'system') await page.emulateMedia({ reducedMotion: 'reduce' });
    else await dialog.getByRole('switch', { name: 'Performance Mode', exact: true }).click();
    await expect(page.locator('html')).toHaveClass(/motion-reduced/);
    await expect.poll(() => page.evaluate(() => Array.from(document.querySelectorAll('[data-ui-motion]')).flatMap(node => node.getAnimations()).filter(animation => animation.playState === 'running').length)).toBe(0);
    await expect(dialog.getByRole('switch', { name: 'Performance Mode', exact: true })).toHaveAttribute('aria-checked', String(source === 'manual'));
    await expect(page.locator('.settings-modal [data-ui-motion][aria-hidden="true"]')).toHaveCount(0);
    await dialog.getByRole('button', { name: 'Done', exact: true }).click();
    await expect(page.locator('[aria-labelledby="settings-dialog-title"]')).toHaveCount(0);
  });
}

test('normal-motion sign-in changes remove the previous form immediately', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await desktopFixture(page, { signedOut: true }); await page.goto('/');
  await page.getByLabel('API ID', { exact: true }).fill('12345');
  await page.getByLabel('API Hash', { exact: true }).fill('fixture-client-hash');
  await page.getByRole('button', { name: 'Continue to QR sign in', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Phone Number', exact: true })).toBeVisible();
  await expect(page.locator('#telegram-api-id')).toHaveCount(0);
  await page.getByRole('button', { name: 'Phone Number', exact: true }).click();
  await page.getByLabel('Phone Number', { exact: true }).fill('+15555550123');
  await page.getByRole('button', { name: 'Continue', exact: true }).click();
  await expect(page.getByLabel('Telegram Code', { exact: true })).toBeVisible();
  await expect(page.locator('#telegram-phone-number')).toHaveCount(0);
});


test('drag feedback disappears immediately on leave', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await desktopFixture(page, { nativeDrag: true }); await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible();
  await page.getByText('Photos', { exact: true }).click();
  await page.evaluate(() => (window as any).__desktopTest.emit('tauri://drag-enter', { paths: ['/fixtures/file.zip'], position: { x: 300, y: 300 } }));
  await expect(page.getByRole('heading', { name: 'Drop to upload to Photos', exact: true })).toBeVisible();
  const removed = await page.evaluate(async () => {
    (window as any).__desktopTest.emit('tauri://drag-leave', {});
    await new Promise(requestAnimationFrame);
    return !Array.from(document.querySelectorAll('h3')).some(element => element.textContent === 'Drop to upload to Photos');
  });
  expect(removed, 'Stale drop feedback leaves the page immediately').toBe(true);
});

test('unavailable drag upload closes its fallback immediately and opens the normal picker', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await desktopFixture(page, { failNativeDrop: true }); await page.goto('/');
  const heading = page.getByRole('heading', { name: 'Drag-and-drop is unavailable', exact: true });
  await expect(heading).toBeVisible();
  await page.getByRole('button', { name: 'Open Upload Dialog', exact: true }).click();
  expect(await heading.count(), 'Closed fallback must not retain operable controls').toBe(0);
  await expect.poll(() => page.evaluate(() => (window as any).__desktopTest.calls.some((call: any) => call.command === 'plugin:dialog|open'))).toBe(true);
});


test('a native drop preserves the destination and cannot queue duplicate native delivery', async ({ page }) => {
  await desktopFixture(page, { nativeDrag: true }); await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible();
  await page.getByText('Photos', { exact: true }).click();
  await page.evaluate(() => {
    const fixture = (window as any).__desktopTest;
    const drop = { paths: ['/fixtures/Native drop.zip'], position: { x: 300, y: 300 } };
    fixture.emit('tauri://drag-drop', drop); fixture.emit('tauri://drag-drop', drop);
  });
  await page.getByRole('dialog').getByRole('button', { name: /^Store Upload normally/ }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__desktopTest.calls.filter((call: any) => call.command === 'cmd_transfer_enqueue_many').length)).toBe(1);
  const calls = await page.evaluate(() => (window as any).__desktopTest.calls);
  expect(calls.filter((call: any) => call.command === 'cmd_validate_dropped_paths')).toHaveLength(1);
  expect(calls.find((call: any) => call.command === 'cmd_transfer_enqueue_many').args.requests[0]).toMatchObject({ filename: 'Native drop.zip', ownerId: '101', folderId: 9, protectionMode: 'standard' });
});
