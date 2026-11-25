import {expect,test} from '@playwright/test';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve,sep} from 'node:path';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
const appRoot=fileURLToPath(new URL('../..',import.meta.url));
import {desktopFixture,nativeCalls} from './desktop-fixture';

// Serve the actual production artifacts, not Vite's development JSON transform.
// The native application boundary remains controlled by desktopFixture.
test('all 24 built language catalogs load losslessly through the real application',async ({browser}, testInfo)=>{
  test.setTimeout(240_000);
  const verifiedDist = process.env.TELEGRAM_DRIVE_E2E_DIST;
  const dist = verifiedDist ? resolve(appRoot, verifiedDist) : testInfo.outputPath('production');
  if (!verifiedDist) {
    execFileSync(process.execPath, [resolve(appRoot, 'node_modules/vite/bin/vite.js'), 'build', '--outDir', dist], {
      cwd: appRoot, timeout: 120_000, stdio: 'pipe',
    });
  }
  await readFile(resolve(dist, 'index.html'));
  const server=createServer(async(request,response)=>{
    try {
      const pathname=decodeURIComponent(new URL(request.url!,'http://localhost').pathname);
      const file=resolve(dist,pathname==='/'?'index.html':'.'+pathname);
      if(!file.startsWith(dist+sep)) {response.writeHead(403).end();return;}
      const body=await readFile(file);
      const extension=file.split('.').at(-1);
      response.setHeader('Content-Type',({html:'text/html',js:'text/javascript',css:'text/css',json:'application/json',svg:'image/svg+xml'} as Record<string,string>)[extension!]??'application/octet-stream');
      response.end(body);
    } catch {response.writeHead(404).end();}
  });
  await new Promise<void>(done=>server.listen(0,'127.0.0.1',done));
  const address=server.address();if(!address||typeof address==='string')throw new Error('Missing production server port');
  const origin=`http://127.0.0.1:${address.port}`;
  const languages=['en','ar','bn-BD','de','es','fa-IR','fil-PH','fr','hi','id','it','ja','ko','ms-MY','pl-PL','pt-BR','ru','th-TH','tr','uk-UA','ur-PK','vi','zh-CN','zh-TW'];
  try {
    // A flat key table from an earlier release remains readable by the new loader.
    const compatibility = await browser.newContext();
    try {
      const page = await compatibility.newPage();
      await page.route(/\/assets\/translation-keys-[^/]+\.json$/, async route => {
        const current = await (await route.fetch()).json();
        const keys = current.groups.flatMap(([prefix, ...names]: string[]) => names.map(name => prefix ? `${prefix}.${name}` : name));
        await route.fulfill({ json: { format: 'td-keys-v2', id: current.id, keys, shared: current.shared } });
      });
      await desktopFixture(page);
      await page.goto(origin);
      await expect(page.getByRole('button', { name: 'Preferences', exact: true })).toBeVisible();
      await expect(page.locator('summary', { hasText: 'Use an existing Telegram channel' })).toBeVisible();
    } finally {
      await compatibility.close();
    }
    for(const delayed of [/\/assets\/en-[^/]+\.json$/, /\/assets\/translation-keys-[^/]+\.json$/]) {
      const context=await browser.newContext();const page=await context.newPage();page.setDefaultTimeout(10_000);
      let release!:()=>void;const gate=new Promise<void>(done=>{release=done;});let requested=false;
      try {
        await page.route(delayed,async route=>{requested=true;await gate;await route.continue();});
        await desktopFixture(page,{holdSupporter:true});await page.goto(origin);
        await expect.poll(()=>requested).toBe(true);
        await expect(page.locator('[data-language-bootstrap="loading"]')).toBeVisible();
        expect(await nativeCalls(page,'cmd_get_supporter_status')).toHaveLength(0);
        await expect(page.locator('iframe')).toHaveCount(0);
        release();
        await expect(page.getByText('Checking sponsor access',{exact:true})).toBeVisible();
        await expect(page.locator('iframe')).toHaveCount(0);
        await page.evaluate(()=>(window as any).__desktopTest.releaseSupporter());
        await expect(page.getByRole('button',{name:'Preferences',exact:true})).toBeVisible();
        await expect(page.locator('[data-language-bootstrap]')).toHaveCount(0);
      } finally {release();await context.close();}
    }
    for(const broken of [/\/assets\/en-[^/]+\.json$/, /\/assets\/translation-keys-[^/]+\.json$/].flatMap(pattern=>[ {pattern,malformed:false}, {pattern,malformed:true} ])) {
      const context=await browser.newContext();const page=await context.newPage();page.setDefaultTimeout(10_000);
      try {
        await page.route(broken.pattern,route=>route.fulfill(broken.malformed?{status:200,contentType:'application/json',body:JSON.stringify({format:'broken-resource'})}:{status:503,body:'Unavailable local resource'}));
        await desktopFixture(page);await page.goto(origin);
        const error=page.locator('[data-language-bootstrap="error"]');await expect(error).toBeVisible();
        await expect(error).not.toContainText('common.');
        expect(await nativeCalls(page,'cmd_get_supporter_status')).toHaveLength(0);
        await page.unroute(broken.pattern);await error.getByRole('button',{name:'Retry',exact:true}).click();
        await expect(page.getByRole('button',{name:'Preferences',exact:true})).toBeVisible();
        await expect(page.locator('iframe')).toHaveCount(0);
      } finally {await context.close();}
    }
    {
      const context=await browser.newContext();const page=await context.newPage();page.setDefaultTimeout(10_000);
      const resource=/\/assets\/en-[^/]+\.json$/;
      let release!:()=>void;const gate=new Promise<void>(done=>{release=done;});let requested=false;
      try {
        await page.route(resource,async route=>{requested=true;await gate;await route.continue().catch(()=>undefined);});
        await desktopFixture(page);await page.goto(origin);await expect.poll(()=>requested).toBe(true);
        const error=page.locator('[data-language-bootstrap="error"]');await expect(error).toBeVisible({timeout:10_000});
        release();await page.waitForLoadState('networkidle');await expect(error).toBeVisible();
        expect(await nativeCalls(page,'cmd_get_supporter_status')).toHaveLength(0);
        await page.unroute(resource);await error.getByRole('button',{name:'Retry',exact:true}).click();
        await expect(page.getByRole('button',{name:'Preferences',exact:true})).toBeVisible();
      } finally {release();await context.close();}
    }
    for(const language of languages) {
      const context=await browser.newContext();const page=await context.newPage();page.setDefaultTimeout(10_000);
      try {
        const catalog=JSON.parse(await readFile(resolve(appRoot,'src/i18n/locales',language+'.json'),'utf8'));
        const packets:any[]=[];
        page.on('response',response=>{
          if(response.url().endsWith('.json')&&new URL(response.url()).pathname.startsWith('/assets/'))void response.json().then(packet=>packets.push(packet));
        });
        await desktopFixture(page,{settings:{language}});await page.goto(origin);
        await expect(page.locator('html')).toHaveAttribute('lang',language);
        await expect(page.getByRole('button',{name:catalog.common.preferences,exact:true})).toBeVisible();
        await page.getByRole('button',{name:catalog.common.preferences,exact:true}).click();
        await page.getByRole('menu').getByRole('button',{name:catalog.common.preferences,exact:true}).click();
        await expect(page.getByRole('dialog')).toBeVisible();
        await expect(page.getByRole('dialog').getByRole('button',{name:catalog.settings.tab_encryption,exact:true})).toBeVisible();
        {
          await expect.poll(()=>packets.filter(packet=>packet.format==='td-locale-v2').length).toBe(language==='en'?1:2);
          const keys=packets.find(packet=>packet.format==='td-keys-v3');expect(keys).toBeTruthy();
          const packed=packets.find(packet=>packet.format==='td-locale-v2'&&packet.language===language);expect(packed.id).toBe(keys.id);
          const flat:Record<string,string>={};
          const flatten=(value:Record<string,unknown>,prefix='')=>{for(const [name,entry]of Object.entries(value)){const key=prefix?prefix+'.'+name:name;if(typeof entry==='string')flat[key]=entry;else flatten(entry as Record<string,unknown>,key);}};
          flatten(catalog);
          const paths = keys.groups.flatMap(([prefix, ...names]: string[]) => names.map(name => prefix ? `${prefix}.${name}` : name));
          const unpacked=Object.fromEntries(paths.flatMap((key:string,index:number)=>packed.values[index]===null?[]:[[key,typeof packed.values[index]==='number'?keys.shared[packed.values[index]]:packed.values[index]]]));
          expect(unpacked).toEqual(flat);
        }
      } finally {await context.close();}
    }
    const context=await browser.newContext();const page=await context.newPage();page.setDefaultTimeout(10_000);
    try {
      const arabic=JSON.parse(await readFile(resolve(appRoot,'src/i18n/locales/ar.json'),'utf8'));
      const resource=/\/assets\/ar-[^/]+\.json$/;
      let delivered=false;
      await page.route(resource,async route=>{
        const response=await route.fetch();const packet=await response.json();delete packet.format;
        await route.fulfill({response,body:JSON.stringify(packet)});delivered=true;
      });
      await desktopFixture(page,{settings:{language:'en'}});await page.goto(origin);
      await page.getByRole('button',{name:'Preferences',exact:true}).click();
      await page.getByRole('menu').getByRole('button',{name:'Preferences',exact:true}).click();
      const choice=page.getByRole('combobox',{name:'Application Language',exact:true});
      await choice.selectOption('ar');
      await expect.poll(()=>delivered).toBe(true);await page.waitForLoadState('networkidle');
      // An invalid resource must not publish a partial bundle or hide the usable language.
      await expect(page.getByRole('button',{name:'Preferences',exact:true})).toBeVisible();
      await expect(page.locator('html')).toHaveAttribute('lang','en');
      expect((await nativeCalls(page,'cmd_set_native_language')).some(call=>call.args.language==='ar')).toBe(false);
      await page.unroute(resource);
      await choice.selectOption('en');await expect(page.locator('html')).toHaveAttribute('lang','en');
      await choice.selectOption('ar');
      await expect(page.getByRole('button',{name:arabic.common.preferences,exact:true})).toBeVisible();
      await expect.poll(async()=>{const calls=await nativeCalls(page,'cmd_set_native_language');return calls.at(-1)?.args.language;}).toBe('ar');
    } finally {await context.close();}
  } finally {await new Promise<void>((done,reject)=>server.close(error=>error?reject(error):done()));}
});
