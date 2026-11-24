import { expect, test } from '@playwright/test';
import { desktopFixture, openSettings } from './desktop-fixture';
import { exerciseCrashConsent } from './crash-consent-fixture';

test('Desktop crash report consent discards pending reports across withdrawal, re-enabling and restart', async ({ page }) => {
  await desktopFixture(page, { settings: { crashReportingConsentSeen: false } });
  await page.goto('/');
  await page.getByRole('button', { name: 'Allow crash reports' }).click();
  const openPrivacy = async () => {
    await openSettings(page);
    await page.getByRole('button', { name: 'Privacy', exact: true }).click();
  };
  await openPrivacy();
  const reporting = page.getByRole('switch', { name: 'Send anonymous crash reports' });
  await expect(reporting).toBeChecked();
  await reporting.click();
  await expect(reporting).not.toBeChecked();
  await page.reload();
  await exerciseCrashConsent(page, openPrivacy);
});
