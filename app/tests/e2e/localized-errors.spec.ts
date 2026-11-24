import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { desktopFixture, nativeCalls } from './desktop-fixture';

const es = JSON.parse(readFileSync(new URL('../../src/i18n/locales/es.json', import.meta.url), 'utf8'));

test('Spanish sign-in explains an expired code and keeps the phone retry available', async ({ page }) => {
  await desktopFixture(page, { signedOut: true, settings: { language: 'es' } });
  await page.goto('/');
  await page.getByLabel(es.auth.api_id, { exact: true }).fill('12345');
  await page.getByLabel(es.auth.api_hash, { exact: true }).fill('test-client-hash');
  await page.locator('form button[type=submit]').click();
  await page.getByRole('button', { name: es.auth.phone_number, exact: true }).click();
  await page.getByLabel(es.auth.phone_number, { exact: true }).fill('+15555550123');
  await page.getByRole('button', { name: es.auth.continue, exact: true }).click();
  await page.getByLabel(es.auth.telegram_code, { exact: true }).fill('11111');
  await page.evaluate(() => {
    const native = (window as any).__TAURI_INTERNALS__;
    const invoke = native.invoke;
    native.invoke = (command: string, args: any) => {
      if (command === 'cmd_auth_sign_in' && args.code === '11111') return Promise.reject('That authentication code has expired. Request a new code.');
      if (command === 'cmd_auth_check_password') return Promise.reject('No password session found');
      return invoke(command, args);
    };
  });
  await page.getByRole('button', { name: es.auth.sign_in, exact: true }).click();
  await expect(page.getByText('El código ha caducado. Solicita otro.', { exact: true })).toBeVisible();
  await page.getByLabel(es.auth.telegram_code, { exact: true }).fill('12345');
  await page.getByRole('button', { name: es.auth.sign_in, exact: true }).click();
  await page.getByLabel(es.auth.cloud_password, { exact: true }).fill('fixture-expired-session');
  await page.locator('form button[type=submit]').click();
  await expect(page.getByLabel(es.auth.phone_number, { exact: true })).toBeVisible();
  await expect(page.getByText('El código ha caducado. Solicita otro.', { exact: true })).toBeVisible();
});

test('Spanish recovery explains a rejected bundle and allows safe verification again', async ({ page }) => {
  await desktopFixture(page, { vaultState: 'unlocked', encryptionReady: true, settings: { language: 'es' } });
  await page.goto('/');
  await page.getByRole('button', { name: es.common.preferences, exact: true }).click();
  await page.getByRole('menu').getByRole('button', { name: es.common.preferences, exact: true }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('button', { name: es.settings.tab_encryption, exact: true }).click();
  await page.evaluate(() => {
    const native = (window as any).__TAURI_INTERNALS__;
    const invoke = native.invoke;
    native.invoke = (command: string, args: any) => {
      if (command === 'cmd_export_vault_recovery') return Promise.resolve('synthetic-vault-bundle');
      if (command === 'cmd_verify_vault_recovery') {
        if (args.bundleBase64 !== 'synthetic-vault-bundle') return Promise.reject('[WrongKeyOrCorrupt] synthetic-sensitive-detail');
        return Promise.resolve({ matches_vault: true, missing_profiles: 0, complete: true });
      }
      return invoke(command, args);
    };
  });
  await dialog.getByLabel(es.recovery_copy.passphrase_label, { exact: true }).fill('fixture-passphrase');
  await dialog.getByRole('button', { name: es.recovery_copy.create_bundle, exact: true }).click();
  await dialog.getByText(es.recovery_copy.saved_label, { exact: true }).click();
  await dialog.getByRole('button', { name: es.recovery_copy.continue_restore, exact: true }).click();
  const bundle = dialog.getByLabel(es.recovery_copy.verify_bundle_label, { exact: true });
  await bundle.fill('damaged-synthetic-bundle');
  await dialog.getByLabel(es.recovery_copy.verification_passphrase, { exact: true }).fill('fixture-passphrase');
  const verify = dialog.getByRole('button', { name: es.recovery_copy.verify_finish, exact: true });
  await verify.click();
  await expect(page.getByText(es.recovery_copy.test_failed.replace('{{error}}', 'Comprueba la frase de contraseña y la copia del paquete.'), { exact: true })).toBeVisible();
  await expect(page.getByText(/synthetic-sensitive-detail/)).toHaveCount(0);
  await bundle.fill('synthetic-vault-bundle');
  await verify.click();
  await expect(verify).toHaveCount(0);
  expect(await nativeCalls(page, 'cmd_import_vault_recovery')).toHaveLength(0);
});

test('Spanish transfer errors use the native category and retry the original job', async ({ page }) => {
  await desktopFixture(page, { populatedTransfers: true, settings: { language: 'es' } });
  await page.goto('/');
  await expect(page.getByText('Holiday folder photo.jpg', { exact: true })).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId('transfer-row-upload-error')).toBeVisible();
  await page.evaluate(() => {
    const state = (window as any).__desktopTest;
    const job = state.jobs.find((item: any) => item.id === 'upload-error');
    Object.assign(job, { error: 'synthetic-private-path', errorCategory: 'storage', revision: job.revision + 1 });
    state.emit('transfer-upserted', { ...job });
  });
  const row = page.getByTestId('transfer-row-upload-error');
  await expect(row.getByRole('alert')).toHaveText(es.activity.category_storage);
  await expect(row).not.toContainText('synthetic-private-path');
  await row.getByRole('button', { name: /Fixture retry.bin/ }).click();
  await expect.poll(async () => (await nativeCalls(page, 'cmd_transfer_retry')).some(call => call.args.id === 'upload-error')).toBe(true);
});
