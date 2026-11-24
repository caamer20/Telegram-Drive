import { expect, test } from '@playwright/test';
import { readFile } from 'node:fs/promises';

test.use({ userAgent: 'Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 Chrome/131 Mobile Safari/537.36' });

test('Android visitors can find the signed APK release when desktop release metadata is offline', async ({ page }) => {
  const html = await readFile(new URL('../../../Docs/Telegram-Drive.html', import.meta.url), 'utf8');
  await page.route('**/*', route => route.request().resourceType() === 'document'
    ? route.fulfill({ contentType: 'text/html', body: html })
    : route.abort());
  await page.goto('http://product-page.test/');
  await expect(page.getByText('GitHub release details are temporarily unavailable')).toBeVisible();
  const link = page.locator('[data-platform="android"] a').first();
  await expect(link).toHaveAttribute('href', /^https:\/\/github\.com\/caamer20\/Telegram-Drive\/releases\/tag\/Androidv\d+\.\d+\.\d+$/);
  const releaseUrl = new URL((await link.getAttribute('href'))!);
  await expect(page.locator('#heroDownload')).toHaveAttribute('href', releaseUrl.href);

  const version = releaseUrl.pathname.split('/Androidv')[1];
  const readme = await readFile(new URL('../../../README.md', import.meta.url), 'utf8');
  const summaryVersion = readme.match(/Android \/ (?:Android|Google) TV \*\*([^*]+)\*\*/)?.[1];
  expect(summaryVersion, 'README download summary version').toBe(version);
  expect(readme).toContain(`[Download Android ${version}](${releaseUrl.href})`);
  const assetUrl = new URL(`../download/Androidv${version}/Telegram-Drive-v${version}-android-universal.apk`, releaseUrl);
  for (const platform of ['Android phones and tablets', 'Android TV / Google TV']) {
    const row = readme.split('\n').find(line => line.startsWith(`| **${platform}** |`));
    expect(row, `README signed APK for ${platform}`).toContain(`(${assetUrl.href})`);
  }
});
