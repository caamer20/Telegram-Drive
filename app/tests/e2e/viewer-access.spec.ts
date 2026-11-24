import { expect, test } from '@playwright/test';
import { desktopFixture, nativeCalls } from './desktop-fixture';

function readablePdf(): string {
  const content = 'BT /F1 18 Tf 20 150 Td (Accessible fixture text) Tj ET\n';
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>',
    `<< /Length ${content.length} >>\nstream\n${content}endstream`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
  ];
  let body = '%PDF-1.4\n';
  const offsets: number[] = [];
  objects.forEach((object, index) => {
    offsets.push(body.length);
    body += `${index + 1} 0 obj\n${object}\nendobj\n`;
  });
  const xref = body.length;
  body += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const offset of offsets) body += `${String(offset).padStart(10, '0')} 00000 n \n`;
  body += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(body, 'latin1').toString('base64');
}

test('image and PDF viewers announce their contents, contain keyboard focus and return it after Escape', async ({ page }) => {
  await desktopFixture(page, { pdfBase64: readablePdf(), settings: { viewMode: 'grid' } });
  await page.goto('/');
  await page.getByRole('button', { name: 'Saved Messages', exact: true }).click();
  for (const name of ['Holiday saved photo.jpg', 'Fixture report.pdf']) {
    const card = page.getByRole('group', { name, exact: true });
    const opener = card.getByRole('button', { name: 'Preview ' + name, exact: true });
    await card.hover();
    await opener.focus();
    await opener.press('Enter');
    const dialog = page.getByRole('dialog', { name, exact: true });
    await expect(dialog).toBeVisible({ timeout: 30_000 });
    await expect(dialog).toHaveAttribute('aria-modal', 'true');
    const inside = () => dialog.evaluate(el => el.contains(document.activeElement));
    await expect.poll(inside).toBe(true);
    if (name.endsWith('.pdf')) {
      const pdfPage = dialog.getByRole('region', { name: 'Page 1 of 1' });
      await pdfPage.focus();
      await expect(pdfPage).toContainText('Accessible fixture text', { timeout: 30_000 });
    } else {
      await expect(dialog.getByRole('img', { name })).toBeVisible();
    }
    for (const key of ['Shift+Tab', ...Array(14).fill('Tab')]) {
      await page.keyboard.press(key);
      expect(await inside()).toBe(true);
    }
    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
    await expect(opener).toBeFocused();
  }
});

test('an unavailable PDF stream ends loading and offers keyboard-accessible recovery', async ({ page }) => {
  await desktopFixture(page, { rejectPdfStream: true, settings: { viewMode: 'grid' } });
  await page.goto('/');
  await page.getByRole('button', { name: 'Saved Messages', exact: true }).click();
  await page.getByRole('group', { name: 'Unavailable.pdf', exact: true }).hover();
  await page.getByRole('button', { name: 'Preview Unavailable.pdf' }).click();
  const dialog = page.getByRole('dialog', { name: 'Unavailable.pdf' });
  await expect(dialog).toBeVisible({ timeout: 30_000 });
  await expect(dialog.getByRole('alert')).toContainText('Unable to open PDF');
  await expect(dialog.getByText('Loading document…')).toHaveCount(0);
  const recovery = dialog.getByRole('button', { name: 'Open with External App' });
  await recovery.press('Enter');
  await expect.poll(async () => (await nativeCalls(page, 'cmd_open_file_externally')).length).toBe(1);
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
});
