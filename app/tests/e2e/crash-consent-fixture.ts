import { expect, type Page } from '@playwright/test';

/** Exercise the real error handler and preferences UI with a controllable native transport. */
export async function exerciseCrashConsent(page: Page, openPrivacy: () => Promise<void>, fixtureKey = '__desktopTest') {
  await openPrivacy();
  const consent = page.getByRole('switch', { name: 'Send anonymous crash reports' });
  await expect(consent).not.toBeChecked();
  await page.evaluate(() => {
    const ipc = (window as any).__TAURI_INTERNALS__;
    const invoke = ipc.invoke;
    const transport = { sent: [] as string[], failHeld: () => {} };
    (window as any).__crashTransport = transport;
    ipc.invoke = (command: string, args: any) => {
      if (command !== 'cmd_submit_crash_report') return invoke(command, args);
      transport.sent.push(args.report.errorType);
      if (args.report.errorType === 'Queued') return Promise.reject(new Error('Offline'));
      if (args.report.errorType === 'Held') return new Promise((_resolve, reject) => {
        transport.failHeld = () => reject(new Error('Late transport failure'));
      });
      return Promise.resolve();
    };
  });
  const crash = (name: string) => page.evaluate(name => {
    const error = new Error('Fixture error');
    error.name = name;
    window.dispatchEvent(new ErrorEvent('error', { error }));
  }, name);
  const queue = () => page.evaluate(() => JSON.parse(localStorage.getItem('telegram-drive-crash-reports') || '[]').map((entry: any) => entry.errorType));
  const sent = () => page.evaluate(() => (window as any).__crashTransport.sent);
  await consent.click();
  await expect(consent).toBeChecked();
  await crash('Queued');
  await expect.poll(queue).toEqual(['Queued']);
  await crash('Held');
  await expect.poll(sent).toEqual(['Queued', 'Held']);
  await consent.click();
  await expect.poll(queue).toEqual([]);
  await crash('Withdrawn');
  await consent.click();
  await expect(consent).toBeChecked();
  await page.evaluate(async () => {
    (window as any).__crashTransport.failHeld();
    await new Promise(resolve => setTimeout(resolve, 0));
  });
  expect(await queue()).toEqual([]);
  await crash('Fresh');
  await expect.poll(sent).toEqual(['Queued', 'Held', 'Fresh']);
  await crash('Queued');
  await expect.poll(queue).toEqual(['Queued']);
  await page.reload();
  await openPrivacy();
  await expect(consent).toBeChecked();
  await expect.poll(() => page.evaluate(key => (window as any)[key].calls
    .filter((call: any) => call.command === 'cmd_submit_crash_report')
    .map((call: any) => call.args.report.errorType), fixtureKey)).toEqual(['Queued']);
  await consent.click();
  await expect(consent).not.toBeChecked();
  await page.reload();
  await openPrivacy();
  await expect(consent).not.toBeChecked();
  expect(await queue()).toEqual([]);
}
