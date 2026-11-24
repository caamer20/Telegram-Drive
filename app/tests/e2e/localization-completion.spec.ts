import { expect, test } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { desktopFixture, nativeCalls } from './desktop-fixture';

const catalog = async (locale: string) => JSON.parse(await readFile(new URL(`../../src/i18n/locales/${locale}.json`, import.meta.url), 'utf8'));

test('Spanish sign-in explains credentials, linked portal and validation without changing the login flow', async ({ page }) => {
  await desktopFixture(page, { signedOut: true, settings: { language: 'es' } });
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Inicio de sesión seguro con QR', exact: true })).toBeVisible();
  const es = await catalog('es');
  await expect(page.getByText(es.auth_copy.qr_description, { exact: true })).toBeVisible();
  await page.getByRole('button', { name: es.auth.how_to_get_credentials, exact: true }).click();
  await expect(page.getByText(es.auth_copy.help_intro, { exact: true })).toBeVisible();
  await expect(page.getByText(es.auth_copy.create_instruction, { exact: true })).toBeVisible();
  await expect(page.getByText(es.auth_copy.copy_instruction.replace('{{apiId}}', es.auth.api_id).replace('{{apiHash}}', es.auth.api_hash), { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'my.telegram.org', exact: true }).click();
  expect((await nativeCalls(page, 'plugin:shell|open')).at(-1)?.args.path).toBe('https://my.telegram.org');
  await page.getByRole('button', { name: es.auth.close_help, exact: true }).click();
  await page.getByLabel(es.auth.api_id, { exact: true }).fill('123 45');
  await page.getByLabel(es.auth.api_hash, { exact: true }).fill('fixture-hash');
  await page.getByRole('button', { name: es.auth_copy.continue_qr, exact: true }).click();
  await expect(page.getByText(es.auth_copy.no_spaces, { exact: true })).toBeVisible();
  expect(await nativeCalls(page, 'cmd_store_api_hash')).toHaveLength(0);
  await page.getByLabel(es.auth.api_id, { exact: true }).fill('12345');
  await page.getByRole('button', { name: es.auth_copy.continue_qr, exact: true }).click();
  await expect(page.getByText(es.auth_copy.qr_path, { exact: true })).toBeVisible();
  expect(await nativeCalls(page, 'cmd_auth_qr_login')).toHaveLength(1);
});

test('onboarding, Help and shortcut data arrays follow the saved language through the dashboard', async ({ page }) => {
  await desktopFixture(page, { settings: { language: 'es', driveTourSeen: false } });
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Tu cuenta de Telegram se convierte en la unidad', exact: true })).toBeVisible();
  const es = await catalog('es');
  const tour = page.getByRole('dialog');
  await expect(tour).toContainText(es.drive_intro.drive_body);
  await tour.getByRole('button', { name: es.workspace.next, exact: true }).click();
  await expect(tour).toContainText(es.drive_intro.folders_title);
  await tour.getByRole('button', { name: es.workspace.next, exact: true }).click();
  await expect(tour).toContainText(es.drive_intro.protection_body);
  await tour.getByRole('button', { name: es.drive_intro.help, exact: true }).click();
  const help = page.getByRole('dialog');
  await expect(help).toContainText(es.help_topics.storage_question);
  await expect(help).toContainText(es.help_topics.storage_answer);
  await help.getByText(es.help_topics.guest_question, { exact: true }).click();
  await expect(help).toContainText(es.help_topics.guest_answer);
  await help.getByRole('button', { name: es.ui_copy.close_help, exact: true }).click();
  await page.keyboard.press('?');
  const shortcuts = page.getByRole('dialog');
  for (const value of Object.values(es.shortcut_labels)) await expect(shortcuts).toContainText(value as string);
  await expect(shortcuts.getByText('⌘/Ctrl F', { exact: true })).toBeVisible();
  await shortcuts.getByRole('button', { name: es.ui_copy.close_shortcuts, exact: true }).click();
  expect(await nativeCalls(page, 'cmd_auth_qr_login')).toHaveLength(0);
  expect(await nativeCalls(page, 'cmd_begin_supporter_checkout')).toHaveLength(0);
});


test('encrypted settings warnings, entered passphrase and badge explanation survive a language switch', async ({ page }) => {
  await desktopFixture(page, { settings: { language: 'es', telegramSettingsSyncEnabled: true, videoFilenameCaption: false } });
  await page.goto('/?a11y-fixture=locale-arrays');
  await expect(page.getByRole('heading', { name: 'Sincronización cifrada de ajustes', exact: true })).toBeVisible();
  const es = await catalog('es');
  const copy = page.locator('[data-copy-view]');
  await expect(copy).toContainText(es.settings_sync_copy.exclusions);
  const passphrase = copy.getByPlaceholder(es.settings_sync_copy.placeholder, { exact: true });
  await passphrase.fill('fixture-sync-passphrase');
  await page.getByLabel('Fixture language').selectOption('ar');
  await expect(page.locator('[data-locale-arrays]')).toHaveAttribute('data-locale-arrays', 'ar');
  const ar = await catalog('ar');
  await expect(copy).toContainText(ar.settings_sync_copy.warning);
  await expect(copy.getByPlaceholder(ar.settings_sync_copy.placeholder, { exact: true })).toHaveValue('fixture-sync-passphrase');
  const badge = page.locator('[data-badge] button');
  await expect(badge).toHaveAccessibleName(ar.protection_labels.explain.replace('{{state}}', ar.protection_labels.key_missing));
  await badge.click();
  const explanation = page.getByRole('dialog');
  await expect(explanation).toContainText(ar.protection_copy.locked);
  await page.getByLabel('Fixture language').selectOption('fr');
  const fr = await catalog('fr');
  await expect(explanation).toContainText(fr.protection_copy.locked);
  await explanation.getByRole('button', { name: fr.protection_copy.close, exact: true }).click();
  await copy.getByRole('button', { name: fr.settings_sync_copy.upload, exact: true }).click();
  const confirm = page.getByRole('dialog');
  await expect(confirm).toContainText(fr.settings_sync_copy.replace_description);
  await confirm.getByRole('button', { name: fr.settings_sync_copy.replace, exact: true }).click();
  await expect(page.getByText(fr.settings_sync_copy.uploaded, { exact: true })).toBeVisible();
  const calls = await nativeCalls(page, 'cmd_upload_settings_sync');
  expect(calls).toHaveLength(1);
  expect(calls[0].args.passphrase).toBe('fixture-sync-passphrase');
  expect(calls[0].args.settings.videoFilenameCaption).toBe(false);
  for (const excluded of ['supporter_activation', 'proxyPassword', 'crashReportingEnabled']) expect(calls[0].args.settings).not.toHaveProperty(excluded);
});


test('translated theme and advanced arrays preserve stored identifiers and owner theme names', async ({ page }) => {
  const ownerName = 'Owner theme: café & 東京';
  await desktopFixture(page, { settings: { language: 'es' } });
  await page.addInitScript(name => {
    localStorage.setItem('user-themes', JSON.stringify([{ id: 'owner-fixture', name, isDark: true, palette: {
      bg: '#101010', surface: '#202020', primary: '#30aaff', secondary: '#888888', text: '#ffffff', subtext: '#cccccc', border: '#444444', hover: '#333333',
    } }]));
  }, ownerName);
  await page.goto('/?a11y-fixture=locale-arrays');
  await page.getByLabel('Fixture view').selectOption('themes');
  const es = await catalog('es');
  await expect(page.getByTitle(ownerName, { exact: true })).toBeVisible();
  await page.getByTitle(es.theme_names.ocean, { exact: true }).click();
  await expect.poll(() => page.evaluate(() => localStorage.getItem('active-custom-theme-id'))).toBe('ocean');
  await page.getByLabel('Fixture language').selectOption('ar');
  const ar = await catalog('ar');
  await expect(page.getByTitle(ar.theme_names.ocean, { exact: true })).toBeVisible();
  await expect(page.getByTitle(ownerName, { exact: true })).toBeVisible();
  expect(await page.evaluate(() => localStorage.getItem('active-custom-theme-id'))).toBe('ocean');
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('user-themes')!));
  expect(stored[0].name).toBe(ownerName);
  expect(stored[0].id).toBe('owner-fixture');
  await page.getByRole('button', { name: ar.common.system, exact: true }).click();
  await expect.poll(() => page.evaluate(() => localStorage.getItem('theme-preference'))).toBe('system');
  expect(await page.evaluate(() => localStorage.getItem('active-custom-theme-id'))).toBe('');
  await page.getByLabel('Fixture view').selectOption('advanced');
  await expect(page.locator('[data-copy-view]')).toContainText(ar.advanced_copy.webdav_description);
  await page.getByRole('button', { name: /WebDAV/ }).click();
  await expect(page.locator('[data-last-action]')).toHaveText('webdav');
});
