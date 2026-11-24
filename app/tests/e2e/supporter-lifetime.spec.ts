import { expect, test } from '@playwright/test';
import { desktopFixture, nativeCalls, openSettings } from './desktop-fixture';

for (const supporterState of ['active', 'needs_refresh'] as const) {
  test(`${supporterState} entitlement suppresses ads while loading, after refresh failure, and on restart`, async ({ page }) => {
    await desktopFixture(page, { supporterState, holdSupporter: true });
    await page.goto('/');
    await expect(page.getByText('Checking sponsor access', { exact: true })).toBeVisible();
    await expect(page.getByRole('complementary', { name: /Sponsored advertisement/ })).toHaveCount(0);
    await expect(page.locator('iframe[title="Sponsored"]')).toHaveCount(0);
    await page.evaluate(() => (window as any).__desktopTest.releaseSupporter());
    await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible();
    await openSettings(page);
    await page.getByRole('button', { name: /Lifetime (?:License|license purchased)/ }).click();
    await expect(page.getByRole('heading', { name: 'Lifetime license purchased' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Get lifetime ad-free · $5', exact: true })).toHaveCount(0);
    await page.getByText('License details and recovery', { exact: true }).click();
    await page.evaluate(() => { (window as any).__desktopTest.supporterRefreshFails = true; });
    await page.getByRole('button', { name: 'Refresh verification', exact: true }).click();
    await expect(page.getByText('Unable to reach the supporter service. Check your connection and try again. An existing payment is kept; do not pay again.', { exact: true })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Lifetime license purchased' })).toBeVisible();
    await expect(page.getByRole('complementary', { name: /Sponsored advertisement/ })).toHaveCount(0);
    expect(await nativeCalls(page, 'cmd_begin_supporter_checkout')).toHaveLength(0);
    await page.reload();
    await page.evaluate(() => (window as any).__desktopTest.releaseSupporter());
    await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible();
    await expect(page.getByRole('complementary', { name: /Sponsored advertisement/ })).toHaveCount(0);
  });
}

test('an expired existing purchase restores ad-free access without a second checkout', async ({ page }) => {
  await desktopFixture(page, { supporterState: 'expired' });
  await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible({ timeout: 30_000 });
  await openSettings(page);
  await page.getByRole('button', { name: /Lifetime (?:License|license purchased)/ }).click();
  const section = page.locator('#desktop-supporter-section');
  await expect(section).toContainText('do not pay again');
  await expect(section.getByRole('button', { name: 'Get lifetime ad-free · $5', exact: true })).toHaveCount(0);
  await section.getByText('Already supported? Restore with a recovery code', { exact: true }).click();
  await section.getByRole('textbox', { name: 'Recovery code', exact: true }).fill('BROWSER-FIXTURE-NOT-A-REAL-CODE');
  await expect(section.getByRole('button', { name: 'Restore purchase', exact: true })).toBeDisabled();
  await section.getByRole('checkbox').check();
  await section.getByRole('button', { name: 'Restore purchase', exact: true }).click();
  await expect(section.getByRole('heading', { name: 'Lifetime license purchased' })).toBeVisible();
  await expect(page.getByRole('complementary', { name: /Sponsored advertisement/ })).toHaveCount(0);
  expect(await nativeCalls(page, 'cmd_begin_supporter_checkout')).toHaveLength(0);
  expect(await nativeCalls(page, 'cmd_activate_supporter')).toHaveLength(1);
});

test('an open sponsor gateway closes automatically for verified active and offline-grace supporters', async ({ page }) => {
  await desktopFixture(page, { supporterState: 'inactive', gatewaySeen: false });
  for (const target of ['active', 'needs_refresh']) {
    await page.goto('/');
    const gateway = page.getByRole('dialog', { name: 'A quick message from our sponsor' });
    await expect(gateway).toBeVisible();
    await page.evaluate(value => {
      (window as any).__desktopTest.supporterState = value;
      window.dispatchEvent(new Event('online'));
    }, target);
    await expect(gateway).toHaveCount(0);
    await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible();
    await expect(page.getByRole('complementary', { name: /Sponsored advertisement/ })).toHaveCount(0);
    expect((await nativeCalls(page, 'plugin:store|set')).some(call => ['ad_gateway_passed', 'ad_click_thanks'].includes(call.args.key))).toBe(false);
    expect(await nativeCalls(page, 'plugin:shell|open')).toHaveLength(0);
    expect(await nativeCalls(page, 'cmd_begin_supporter_checkout')).toHaveLength(0);
  }
});

test('a free account can browse, accept the terms, resume the same checkout, and activate its lifetime receipt', async ({ page }) => {
  await desktopFixture(page, { supporterState: 'inactive', gatewaySeen: false, simulateCheckout: true });
  await page.goto('/');
  await page.getByRole('button', { name: 'Continue to files', exact: true }).click();
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Saved Messages', exact: true }).click();
  await expect(page.getByText('Holiday saved photo.jpg', { exact: true })).toBeVisible();
  await openSettings(page);
  await page.getByRole('button', { name: /Lifetime (?:License|license purchased)/ }).click();
  const section = page.locator('#desktop-supporter-section');
  await expect(section).toContainText('$5');
  await expect(section.getByRole('button', { name: 'Get lifetime ad-free · $5', exact: true })).toBeDisabled();
  expect(await nativeCalls(page, 'cmd_begin_supporter_checkout')).toHaveLength(0);
  await section.getByRole('checkbox').check();
  await section.getByRole('button', { name: 'Get lifetime ad-free · $5', exact: true }).click();
  const approval = 'https://www.sandbox.paypal.com/checkoutnow?token=fixture-order';
  await expect.poll(async () => (await nativeCalls(page, 'plugin:shell|open')).map(call => call.args.path)).toEqual([approval]);
  expect((await nativeCalls(page, 'cmd_begin_supporter_checkout'))[0].args.acceptedTermsVersion).toBe('2026-08-11');
  await page.reload();
  await openSettings(page);
  await page.getByRole('button', { name: /Lifetime (?:License|license purchased)/ }).click();
  await expect(section.getByRole('button', { name: 'Get lifetime ad-free · $5', exact: true })).toHaveCount(0);
  await section.getByRole('button', { name: 'Continue existing checkout', exact: true }).click();
  await expect.poll(async () => (await nativeCalls(page, 'plugin:shell|open')).map(call => call.args.path)).toEqual([approval]);
  await section.getByRole('button', { name: /check payment/i }).click();
  await expect(section.getByRole('button', { name: 'Get lifetime ad-free · $5', exact: true })).toHaveCount(0);
  await page.evaluate(() => { (window as any).__desktopTest.settleCheckout = true; });
  await expect(section.getByRole('heading', { name: 'Lifetime license purchased' })).toBeVisible({ timeout: 30_000 });
  await expect(section.getByText('FIXTURE-RECEIPT', { exact: true })).toBeVisible();
  await page.reload();
  await openSettings(page);
  await page.getByRole('button', { name: /Lifetime (?:License|license purchased)/ }).click();
  await expect(section.getByRole('heading', { name: 'Lifetime license purchased' })).toBeVisible();
  const receipt = section.getByText('FIXTURE-RECEIPT', { exact: true });
  await expect(receipt).toBeVisible();
  const confirm = section.getByRole('button', { name: 'Confirm', exact: true });
  await page.evaluate(() => { (window as any).__desktopTest.rejectReceiptAck = true; });
  await confirm.click();
  await expect(receipt).toBeVisible();
  await page.evaluate(() => { (window as any).__desktopTest.rejectReceiptAck = false; });
  await confirm.click();
  await expect(receipt).toHaveCount(0);
  await page.reload();
  await openSettings(page);
  await page.getByRole('button', { name: /Lifetime (?:License|license purchased)/ }).click();
  await expect(section.getByRole('heading', { name: 'Lifetime license purchased' })).toBeVisible();
  await expect(receipt).toHaveCount(0);
  expect(await nativeCalls(page, 'cmd_begin_supporter_checkout')).toHaveLength(0);
});
