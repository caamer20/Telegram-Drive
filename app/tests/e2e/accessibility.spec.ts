import {expect,test,type Page} from '@playwright/test';
import axe from 'axe-core';
import {desktopFixture,nativeCalls,openSettings} from './desktop-fixture';

async function audit(page:Page,context='[role="dialog"][aria-modal="true"]') {
  await page.evaluate(async()=>{
    await Promise.allSettled(document.getAnimations().filter(animation=>Number.isFinite(animation.effect?.getComputedTiming().endTime)).map(animation=>animation.finished));
  });
  await page.addScriptTag({content:axe.source});
  return page.evaluate(async context=> {
    const result=await (window as any).axe.run(context);
    return result.violations.map((violation:any)=>({id:violation.id,nodes:violation.nodes.map((node:any)=>({target:node.target,html:node.html,failureSummary:node.failureSummary}))}));
  },context);
}

async function encryption(page:Page) {
  await openSettings(page);
  const dialog=page.getByRole('dialog');
  await dialog.getByRole('button',{name:'Encryption',exact:true}).click();
  await expect(dialog.getByText('Protect Filenames & Metadata',{exact:true})).toBeVisible();
  return dialog;
}

test('all twelve Settings tabs and expanded network controls pass axe',async ({page})=>{
  test.setTimeout(120_000);
  await desktopFixture(page,{encryptionReady:true,vaultState:'unlocked',networkEnabled:true,settings:{proxyEnabled:true,vpnEnabled:true,bandwidth_schedule:true}});
  await page.goto('/');await openSettings(page);
  const dialog=page.getByRole('dialog');
  const findings:{tab:string;violations:unknown}[]=[];
  for(const tab of ['General','Themes','Privacy','Encryption','Folder Sync','Sharing','Advanced','Lifetime license purchased','About']) {
    await dialog.getByRole('button',{name:tab,exact:true}).click();
    await expect(dialog.locator('#settings-dialog-title + p')).toHaveText(tab);
    await page.waitForTimeout(180); // Outgoing tab animation must finish before inspecting the active view.
    findings.push({tab,violations:await audit(page)});
  }
  for(const tab of ['WebDAV','Proxy','VPN & network','REST API']) {
    await dialog.getByRole('button',{name:'Advanced',exact:true}).click();
    await dialog.getByRole('button',{name:new RegExp(`^${tab}`)}).click();
    await page.waitForTimeout(180);
    findings.push({tab,violations:await audit(page)});
  }
  expect(findings.filter(result=>(result.violations as any[]).length>0)).toEqual([]);
});

test('encryption switches expose their state and support keyboard activation',async ({page})=>{
  await desktopFixture(page,{encryptionReady:true,vaultState:'unlocked'});await page.goto('/');
  const dialog=await encryption(page);
  for(const name of ['Protect Filenames & Metadata','Lock on Sleep/Background']) {
    const toggle=dialog.getByRole('switch',{name,exact:true});
    await expect(toggle).toBeVisible();
    const before=await toggle.getAttribute('aria-checked');
    expect(['true','false']).toContain(before);
    await toggle.focus();await page.keyboard.press('Space');
    await expect(toggle).toHaveAttribute('aria-checked',before==='true'?'false':'true');
    await page.keyboard.press('Enter');await expect(toggle).toHaveAttribute('aria-checked',before!);
  }
  expect((await nativeCalls(page,'cmd_update_encryption_settings')).length).toBeGreaterThan(1);
});

for(const vaultState of ['none','locked'] as const) test(`vault ${vaultState} password visibility is named and keyboard operable`,async ({page})=>{
  await desktopFixture(page,{encryptionReady:true,vaultState});await page.goto('/');
  const dialog=await encryption(page);
  const password=dialog.getByPlaceholder('Vault passphrase (min 8 characters)',{exact:true});
  await password.fill('fixture-passphrase');await expect(password).toHaveAttribute('type','password');
  const show=dialog.getByRole('button',{name:'Show passphrase',exact:true});
  await expect(show).toBeVisible();await show.focus();await page.keyboard.press('Enter');
  await expect(password).toHaveAttribute('type','text');
  await dialog.getByRole('button',{name:'Hide passphrase',exact:true}).press('Space');
  await expect(password).toHaveAttribute('type','password');
  expect(await audit(page)).toEqual([]);
});

test('populated Transfer Center actions and states pass axe through the native event boundary',async ({page})=>{
  await desktopFixture(page,{populatedTransfers:true});await page.goto('/');
  const center=page.getByRole('complementary',{name:'Transfer activity',exact:true});
  await expect(center).toContainText('Fixture download.bin');
  expect(await audit(page,'aside[aria-label="Transfer activity"]')).toEqual([]);
  const sections=center.locator('section');
  await sections.filter({hasText:'Fixture download.bin'}).getByRole('button',{name:'Pause Downloads',exact:true}).click();
  await expect(sections.filter({hasText:'Fixture download.bin'})).toContainText('Paused');
  await sections.filter({hasText:'Fixture download.bin'}).getByRole('button',{name:'Resume Downloads',exact:true}).click();
  await expect(sections.filter({hasText:'Fixture download.bin'})).toContainText('Downloading');
  await center.getByRole('button',{name:'Retry this file: Fixture retry.bin',exact:true}).click();
  await expect.poll(async()=> (await nativeCalls(page,'cmd_transfer_retry')).length).toBe(1);
  await center.getByRole('button',{name:'Cancel Fixture download.bin',exact:true}).click();
  await expect(center).toContainText('Cancelled');
  expect(await audit(page,'aside[aria-label="Transfer activity"]')).toEqual([]);
  await sections.filter({hasText:'Fixture download.bin'}).getByRole('button',{name:'Clear',exact:true}).click();
  await expect(center).not.toContainText('Fixture finished.bin');
});

test('TV directional navigation reaches scrollable content and stays within an open dialog',async ({page})=>{
  await desktopFixture(page,{encryptionReady:true,vaultState:'unlocked'});
  await page.goto('/?a11y-fixture=tv');
  const top=page.getByRole('button',{name:'Top item',exact:true});await expect(top).toBeVisible();
  await top.focus();await page.keyboard.press('ArrowDown');
  const below=page.getByRole('button',{name:'Below fold',exact:true});
  await expect(below).toBeFocused();await expect(below).toBeInViewport();
  expect(await page.getByTestId('tv-scroll-region').evaluate(element=>element.scrollTop)).toBeGreaterThan(0);
  const open=page.getByRole('button',{name:'Open settings',exact:true});await open.click();
  const dialog=page.getByRole('dialog',{name:'Settings',exact:true});await expect(dialog).toBeVisible();
  await dialog.getByRole('button',{name:'Close',exact:true}).focus();
  for(const key of ['ArrowDown','ArrowLeft','ArrowUp','ArrowRight']) {
    await page.keyboard.press(key);
    expect(await dialog.evaluate(element=>element.contains(document.activeElement))).toBe(true);
  }
  // Directional navigation may land on a native select; Escape first leaves
  // native editing there. Focus Close to exercise the dialog-level dismissal.
  await dialog.getByRole('button',{name:'Close',exact:true}).focus();
  await page.keyboard.press('Escape');await expect(dialog).toBeHidden();await expect(open).toBeFocused();
});

test('TV native fields retain caret, number and selection arrow behavior',async ({page})=>{
  await desktopFixture(page);await page.goto('/?a11y-fixture=tv');
  const text=page.getByRole('textbox',{name:'Text input',exact:true});await text.fill('abc');await text.press('End');await text.press('ArrowLeft');
  await expect(text).toBeFocused();expect(await text.evaluate(element=>(element as HTMLInputElement).selectionStart)).toBe(2);
  const number=page.getByRole('spinbutton',{name:'Number input',exact:true});await number.fill('1');await number.press('ArrowUp');await expect(number).toBeFocused();await expect(number).toHaveValue('2');
  // The identical control establishes Chromium's native keyboard sequence.
  // ArrowDown opens this host's picker; type-ahead selects and Enter commits.
  for(const mode of ['tv-control','tv']) {
    await page.goto(`/?a11y-fixture=${mode}`);
    const select=page.getByRole('combobox',{name:'Native choice',exact:true});
    await select.evaluate(element=>{
      (window as any).__nativeArrowCancelled=[];
      element.addEventListener('keydown',event=>{
        if((event as KeyboardEvent).key==='ArrowDown')(window as any).__nativeArrowCancelled.push(event.defaultPrevented);
      });
    });
    await select.focus();await select.press('ArrowDown');await select.press('s');await select.press('Enter');
    await expect(select).toBeFocused();await expect(select).toHaveValue('b');
    expect(await page.evaluate(()=>(window as any).__nativeArrowCancelled)).toEqual([false]);
  }
});

test('vault creation, recovery drill, lock and unlock remain keyboard accessible',async ({page})=>{
  await desktopFixture(page,{encryptionReady:true,vaultState:'none'});await page.goto('/');
  const dialog=await encryption(page);
  await dialog.getByPlaceholder('Vault passphrase (min 8 characters)',{exact:true}).fill('fixture-original');
  await dialog.getByPlaceholder('Confirm passphrase',{exact:true}).fill('fixture-original');
  await dialog.getByRole('checkbox',{name:/I understand that I am responsible/}).check();
  expect(await audit(page)).toEqual([]);
  await dialog.getByRole('button',{name:'Create Vault',exact:true}).press('Enter');
  await expect(dialog.getByRole('heading',{name:'Required recovery drill',exact:true})).toBeVisible();
  await dialog.getByLabel('Recovery bundle passphrase',{exact:true}).fill('fixture-recovery');
  await dialog.getByRole('button',{name:'Create recovery bundle',exact:true}).press('Enter');
  await expect(dialog.getByLabel('Generated recovery bundle')).toHaveValue('controlled-recovery-bundle');
  expect(await audit(page)).toEqual([]);
  await dialog.getByRole('checkbox',{name:'I saved this bundle somewhere separate from this device.',exact:true}).check();
  await dialog.getByRole('button',{name:'Continue to restore test',exact:true}).press('Enter');
  await dialog.getByLabel('Recovery bundle to verify',{exact:true}).fill('controlled-recovery-bundle');
  await dialog.getByLabel('Recovery verification passphrase',{exact:true}).fill('fixture-recovery');
  await dialog.getByRole('button',{name:'Verify and finish setup',exact:true}).press('Enter');
  await expect(dialog.getByRole('heading',{name:'Required recovery drill',exact:true})).toHaveCount(0);
  expect((await nativeCalls(page,'cmd_verify_vault_recovery')).length).toBe(1);
  await dialog.getByRole('button',{name:'Lock Vault Now',exact:true}).press('Enter');
  await expect(dialog.getByRole('button',{name:'Unlock Vault',exact:true})).toBeVisible();
  await dialog.getByPlaceholder('Vault passphrase (min 8 characters)',{exact:true}).fill('fixture-original');
  await dialog.getByRole('button',{name:'Unlock Vault',exact:true}).press('Enter');
  await expect(dialog.getByRole('button',{name:'Lock Vault Now',exact:true})).toBeVisible();
  expect(await audit(page)).toEqual([]);
});

test('Escape closes only the current encryption explanation and restores focus to its opener',async ({page})=>{
  await desktopFixture(page,{encryptionReady:true,vaultState:'unlocked'});await page.goto('/');
  const settings=await encryption(page);
  const opener=settings.getByRole('button',{name:/Protection How it works/});await opener.click();
  const explanation=page.getByRole('dialog',{name:'How file protection works',exact:true});await expect(explanation).toBeVisible();
  expect(await audit(page,'[aria-labelledby="protection-info-title"]')).toEqual([]);
  await explanation.getByRole('button',{name:'Understood',exact:true}).focus();
  await page.keyboard.press('Tab');expect(await explanation.evaluate(element=>element.contains(document.activeElement))).toBe(true);
  await page.keyboard.press('Escape');await expect(explanation).toHaveCount(0);
  await expect(settings).toBeVisible();await expect(opener).toBeFocused();
});

test('a recovery-import confirmation can be cancelled with Escape without closing Settings or importing keys',async ({page})=>{
  await desktopFixture(page,{encryptionReady:true,vaultState:'unlocked',settings:{vaultRecoveryDrillCompleted:true,vaultRecoveryDrillVaultId:'fixture-vault'}});await page.goto('/');
  const settings=await encryption(page);
  await settings.getByRole('button',{name:'Import Recovery Bundle',exact:true}).click();
  await settings.getByPlaceholder('Paste recovery bundle...', {exact:true}).fill('controlled-recovery-bundle');
  await settings.getByPlaceholder('Recovery passphrase',{exact:true}).fill('fixture-recovery');
  const opener=settings.getByRole('button',{name:'Import',exact:true});await opener.click();
  const confirm=page.getByRole('dialog',{name:'Import Recovery Bundle',exact:true});await expect(confirm).toBeVisible();
  expect(await audit(page,'[aria-labelledby="confirm-dialog-title"]')).toEqual([]);
  await page.keyboard.press('Escape');await expect(confirm).toHaveCount(0);
  await expect(settings).toBeVisible();await expect(opener).toBeFocused();
  expect(await nativeCalls(page,'cmd_import_vault_recovery')).toEqual([]);
});

test('TV Escape exits native field editing and arrows can leave the field without Tab',async ({page})=>{
  await desktopFixture(page,{encryptionReady:true,vaultState:'unlocked'});await page.goto('/?a11y-fixture=tv');
  for(const name of ['Edge number','Fixed edge number']) {
    const edge=page.getByRole('spinbutton',{name,exact:true});
    await edge.focus();await edge.press('Escape');await edge.press('ArrowUp');
    await expect(edge).toBeFocused();await expect(edge).toHaveValue('1');
  }
  const text=page.getByRole('textbox',{name:'Text input',exact:true});
  await page.getByRole('button',{name:'Open settings',exact:true}).focus();await page.keyboard.press('ArrowDown');
  await expect(text).toBeFocused();
  await text.press('End');await text.press('ArrowLeft');expect(await text.evaluate(element=>(element as HTMLInputElement).selectionStart)).toBe(2);
  await text.press('Escape');await text.press('ArrowRight');
  const number=page.getByRole('spinbutton',{name:'Number input',exact:true});await expect(number).toBeFocused();
  await number.press('ArrowUp');await expect(number).toHaveValue('2');
  await number.press('Escape');await number.press('Enter');await number.press('ArrowUp');await expect(number).toHaveValue('3');
  await number.press('Escape');await number.press('ArrowRight');
  const select=page.getByRole('combobox',{name:'Native choice',exact:true});await expect(select).toBeFocused();
  // Type-ahead has already selected the value. Enter can open a native picker
  // on Linux Chromium, which consumes Escape before the field handler sees it.
  await select.press('ArrowDown');await select.press('s');await expect(select).toHaveValue('b');
  await select.press('Escape');await select.press('ArrowDown');
  await expect(number).toBeFocused();
  await number.press('Escape');await number.press('Tab');await page.keyboard.press('Shift+Tab');
  await expect(number).toBeFocused();
  const previous=Number(await number.inputValue());await number.press('ArrowUp');await expect(number).toHaveValue(String(previous+1));
  await number.press('Escape');await number.evaluate(element=>(element as HTMLElement).blur());await number.focus();
  await number.press('ArrowUp');await expect(number).toHaveValue(String(previous+2));
  await page.getByRole('button',{name:'Open settings',exact:true}).click();
  const dialog=page.getByRole('dialog',{name:'Settings',exact:true});
  await dialog.getByRole('button',{name:'Encryption',exact:true}).click();
  const protection=dialog.getByRole('combobox').first();await protection.focus();
  await page.keyboard.down('Escape');await expect(dialog).toBeVisible();
  await page.keyboard.down('Escape');await page.waitForTimeout(250);await expect(dialog).toBeVisible();await page.keyboard.up('Escape');
  await page.keyboard.press('Escape');await expect(dialog).toBeHidden();
  await page.getByRole('button',{name:'Open settings',exact:true}).click();
  await dialog.getByRole('button',{name:'Encryption',exact:true}).click();
  await protection.focus();await protection.press('Escape');await protection.press('ArrowDown');
  await expect(protection).not.toBeFocused();expect(await dialog.evaluate(element=>element.contains(document.activeElement))).toBe(true);
  await dialog.getByRole('button',{name:'Close',exact:true}).press('Escape');await expect(dialog).toBeHidden();
});
