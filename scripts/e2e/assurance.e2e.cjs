const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawnSync } = require('node:child_process');

// Exercise the shipped CLI programs as child processes against real files.
// No generator implementation or internal function is imported by this suite.
const scripts = path.resolve(__dirname, '..');
function fixture(t) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'telegram-drive-assurance-e2e-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  return directory;
}
function run(name, ...args) {
  const result = spawnSync(process.execPath, [path.join(scripts, name), ...args], {
    encoding: 'utf8', timeout: 15_000,
  });
  assert.ifError(result.error);
  return result;
}
function success(result) {
  assert.equal(result.status, 0, result.stderr || result.stdout);
}

test('the release gate checks the exact supporter contract with one authenticated TLS connection to the configured origin', async t => {
  const https = require('node:https');
  const { execFile } = require('node:child_process');
  const directory = fixture(t);
  const cert = path.join(directory, 'loopback.pem');
  const key = path.join(directory, 'loopback.key');
  const certificateConfig = path.join(directory, 'openssl.cnf');
  fs.writeFileSync(certificateConfig, [
    '[req]', 'distinguished_name=subject', 'x509_extensions=server', 'prompt=no',
    '[subject]', 'CN=localhost', '[server]', 'subjectAltName=IP:127.0.0.1',
    'basicConstraints=critical,CA:TRUE', 'extendedKeyUsage=serverAuth',
  ].join('\n'));
  let openssl = 'openssl';
  if (process.platform === 'win32' && spawnSync(openssl, ['version']).error?.code === 'ENOENT') {
    const location = spawnSync('git', ['--exec-path'], { encoding: 'utf8' });
    success(location);
    openssl = path.resolve(location.stdout.trim(), '../../../usr/bin/openssl.exe');
  }
  const generated = spawnSync(openssl, ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1',
    '-config', certificateConfig, '-keyout', key, '-out', cert], { encoding: 'utf8' });
  assert.ifError(generated.error);
  success(generated);
  const publicKey = crypto.generateKeyPairSync('ed25519').publicKey.export({ format: 'jwk' }).x;
  const good = { status: 'ok', price: '5.00', currency: 'USD', max_active_devices: 3, terms_version: '2026-08-11', entitlement_public_key: publicKey };
  let health = good;
  let responseCode = 200;
  const requests = [];
  const server = https.createServer({ key: fs.readFileSync(key), cert: fs.readFileSync(cert) }, (req, res) => {
    requests.push([req.method, req.url]);
    if (responseCode === 0) return;
    res.writeHead(responseCode, { 'content-type': 'application/json', ...(responseCode === 302 ? { location: '/redirect' } : {}) });
    res.end(JSON.stringify(health));
  });
  await new Promise((resolve, reject) => server.once('error', reject).listen(0, '127.0.0.1', resolve));
  t.after(() => { server.closeAllConnections(); return new Promise(resolve => server.close(resolve)); });
  const origin = 'https://127.0.0.1:' + server.address().port;
  const check = (service = origin, expectedKey = publicKey) => new Promise(resolve => {
    execFile(process.execPath, [path.join(scripts, 'check-supporter-release.cjs')], {
      timeout: 16_000,
      env: { ...process.env, NODE_EXTRA_CA_CERTS: cert, TELEGRAM_DRIVE_SUPPORTER_SERVICE_URL: service, TELEGRAM_DRIVE_SUPPORTER_PUBLIC_KEY: expectedKey },
    }, (error, stdout, stderr) => resolve({ status: error ? error.code : 0, stdout, stderr }));
  });
  success(await check());
  assert.deepEqual(requests, [['GET', '/health']]);
  for (const [service, expectedKey] of [[origin.replace('https:', 'http:'), publicKey],
    [origin + '\n', publicKey], [origin + '/', publicKey], [origin + '/health', publicKey], [origin + '?q=1', publicKey],
    [origin.replace('https://', 'https://fixture@'), publicKey], [origin, ''], [origin, '_'.repeat(43)]]) {
    assert.equal((await check(service, expectedKey)).status, 1);
  }
  assert.equal(requests.length, 1, 'invalid build inputs must fail before any request');
  for (const mismatch of [{ status: 'degraded' }, { price: '5' }, { currency: 'EUR' }, { max_active_devices: 4 },
    { terms_version: 'different' }, { terms_version: undefined }, { entitlement_public_key: crypto.randomBytes(32).toString('base64url') }]) {
    health = { ...good, ...mismatch };
    assert.equal((await check()).status, 1, Object.keys(mismatch).join());
  }
  health = good;
  for (responseCode of [302, 503, 0]) assert.equal((await check()).status, 1);
  assert.ok(requests.every(([method, url]) => method === 'GET' && url === '/health'));
});

test('release artifacts pass through SBOM generation and an independently verified checksum manifest', t => {
  const directory = fixture(t);
  const report = path.join(directory, 'gradle-dependencies.txt');
  fs.writeFileSync(report, '+--- androidx.core:core-ktx:1.12.0\n\\--- com.squareup.okhttp3:okhttp:4.11.0 -> 4.12.0\n');
  const sbomPath = path.join(directory, 'artifacts', 'android-sbom.cdx.json');
  success(run('generate-gradle-sbom.cjs', report, sbomPath));
  const sbom = JSON.parse(fs.readFileSync(sbomPath, 'utf8'));
  assert.equal(sbom.bomFormat, 'CycloneDX');
  assert.equal(sbom.specVersion, '1.5');
  assert.equal(sbom.components.length, 2);
  assert.ok(sbom.components.some(component => component.name === 'okhttp' && component.version === '4.12.0'));

  const manifest = path.join(directory, 'SHA256SUMS.txt');
  success(run('generate-checksums.cjs', directory, manifest));
  const contents = fs.readFileSync(manifest, 'utf8');
  const entries = contents.trim().split('\n');
  assert.equal(entries.length, 2);
  for (const entry of entries) {
    const match = /^([0-9a-f]{64})  (.+)$/.exec(entry);
    assert.ok(match, `Invalid checksum record: ${entry}`);
    const [, expected, name] = match;
    assert.notEqual(name, 'SHA256SUMS.txt');
    const actual = crypto.createHash('sha256').update(fs.readFileSync(path.join(directory, name))).digest('hex');
    assert.equal(actual, expected, `Checksum differs for ${name}`);
  }
  success(run('generate-checksums.cjs', directory, manifest));
  assert.equal(fs.readFileSync(manifest, 'utf8'), contents, 'Regeneration must not checksum the manifest itself.');
});

test('an empty dependency report fails without publishing an SBOM', t => {
  const directory = fixture(t);
  const report = path.join(directory, 'empty.txt');
  const output = path.join(directory, 'sbom.json');
  fs.writeFileSync(report, 'No resolved dependencies\n');
  assert.notEqual(run('generate-gradle-sbom.cjs', report, output).status, 0);
  assert.equal(fs.existsSync(output), false);
});

test('an empty release directory fails without publishing a checksum manifest', t => {
  const directory = fixture(t);
  const output = path.join(directory, 'SHA256SUMS.txt');
  assert.notEqual(run('generate-checksums.cjs', directory, output).status, 0);
  assert.equal(fs.existsSync(output), false);
});

test('the baseline expiry check passes with margin, warns inside the window and fails once expired', t => {
  const directory = fixture(t);
  const day = 24 * 60 * 60 * 1000;
  const dated = offsetDays => new Date(Date.now() + offsetDays * day).toISOString().slice(0, 10);
  const write = (npmOffset, rustOffset) => {
    fs.writeFileSync(path.join(directory, 'npm-audit-allowlist.json'), JSON.stringify({ expires: dated(npmOffset), advisories: {} }));
    fs.writeFileSync(path.join(directory, 'rust-advisory-baseline.json'), JSON.stringify({ expires: dated(rustOffset), advisories: {}, yankedCrates: {} }));
  };

  write(60, 60);
  const healthy = run('check-baseline-expiry.cjs', directory);
  success(healthy);
  assert.match(healthy.stdout, /npm-audit-allowlist\.json is valid through/);
  assert.match(healthy.stdout, /rust-advisory-baseline\.json is valid through/);

  // One baseline inside the warning window is enough to fail the check.
  write(60, 5);
  const closing = run('check-baseline-expiry.cjs', directory);
  assert.notEqual(closing.status, 0);
  assert.match(closing.stderr, /rust-advisory-baseline\.json expires on/);
  assert.doesNotMatch(closing.stderr, /npm-audit-allowlist/);
  // A shorter window accepts the same dates.
  success(run('check-baseline-expiry.cjs', '--warn-days', '3', directory));

  write(-1, 60);
  const expired = run('check-baseline-expiry.cjs', directory);
  assert.notEqual(expired.status, 0);
  assert.match(expired.stderr, /npm-audit-allowlist\.json expired on/);

  fs.writeFileSync(path.join(directory, 'npm-audit-allowlist.json'), JSON.stringify({ advisories: {} }));
  const undated = run('check-baseline-expiry.cjs', directory);
  assert.notEqual(undated.status, 0);
  assert.match(undated.stderr, /no valid "expires" date/);
});

test('the Android publication guard inspects tracked files and every outgoing commit', t => {
  const directory = fixture(t);
  const git = (...args) => {
    const result = spawnSync('git', ['-C', directory, '-c', 'user.name=E2E', '-c', 'user.email=e2e@example.invalid', '-c', 'commit.gpgsign=false', ...args], { encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    return result.stdout.trim();
  };
  const write = (name, contents = 'synthetic\n') => {
    fs.mkdirSync(path.dirname(path.join(directory, name)), { recursive: true });
    fs.writeFileSync(path.join(directory, name), contents);
  };
  const review = path.join(directory, 'review.json');
  const check = (...args) => run('check-android-publication.cjs', '--repo', directory, '--review-file', review, ...args);

  git('init', '--quiet', '--initial-branch=main');
  write('app/src/main.ts');
  write('app/src/services/androidTransferPolicy.ts');
  write('review.json', JSON.stringify({ pending_owner_review: ['app/src/services/androidTransferPolicy.ts'] }));
  git('add', '.');
  git('commit', '--quiet', '-m', 'shared application files');
  const published = git('rev-parse', 'HEAD');

  // Shared files awaiting a decision are reported, never silently approved.
  const baseline = check('--range', `${published}..HEAD`);
  success(baseline);
  assert.match(baseline.stderr, /await the owner's decision/);
  assert.notEqual(check('--range', `${published}..HEAD`, '--strict').status, 0);

  // An Android-related path nobody listed is refused.
  write('app/src/androidNewBridge.ts');
  git('add', '.');
  git('commit', '--quiet', '-m', 'unlisted');
  const unlisted = check('--range', `${published}..HEAD`);
  assert.notEqual(unlisted.status, 0);
  assert.match(unlisted.stderr, /androidNewBridge\.ts: Android-related path/);
  git('reset', '--quiet', '--hard', published);

  // Project files are refused even when a later commit removes them again:
  // the earlier commit would still be published.
  write('app/src-tauri/gen/android/app/build.gradle.kts');
  write('app/src-tauri/gen/android/app/src/main/AndroidManifest.xml');
  write('signing/release.keystore');
  git('add', '.');
  git('commit', '--quiet', '-m', 'android project');
  const committed = check('--range', `${published}..HEAD`);
  assert.notEqual(committed.status, 0);
  for (const expected of [/build\.gradle\.kts: Gradle build configuration/, /AndroidManifest\.xml: Android manifest/, /release\.keystore: signing key store/]) {
    assert.match(committed.stderr, expected);
  }
  git('rm', '--quiet', '-r', 'app/src-tauri/gen', 'signing');
  git('commit', '--quiet', '-m', 'remove android project');
  success(check('--tracked-only'));
  const history = check('--range', `${published}..HEAD`);
  assert.notEqual(history.status, 0);
  assert.match(history.stderr, /outgoing commit/);
});

test('bundle CLI records route costs and refuses missing or exceeded required route budgets', t => {
  const directory = fixture(t);
  const app = path.join(directory, 'app');
  const write = (name, contents) => {
    const filename = path.join(app, name);
    fs.mkdirSync(path.dirname(filename), { recursive: true });
    fs.writeFileSync(filename, contents);
  };
  const dashboard = 'src/components/desktop/DesktopDashboard.tsx';
  const settings = 'src/components/desktop/dashboard/SettingsModal.tsx';
  const media = 'src/components/desktop/dashboard/MediaPlayer.tsx';
  const pdf = 'src/components/desktop/dashboard/PdfViewer.tsx';
  const budget = JSON.parse(fs.readFileSync(path.join(scripts, '../app/bundle-budget.json'), 'utf8'));
  budget.routeJavaScriptBudgets = { [dashboard]: 310000 };
  budget.featureChunkBudgets = { [settings]: 160000, [media]: 250000, [pdf]: 510000 };
  write('scripts/check-bundle-budget.cjs', fs.readFileSync(path.join(scripts, '../app/scripts/check-bundle-budget.cjs')));
  write('src/i18n/locales/en.json', '{}');
  const emptyKeysId = crypto.createHash('sha256').update(JSON.stringify([[], []])).digest('hex');
  write('dist/assets/translation-keys-fixture.json', JSON.stringify({format: 'td-keys-v2', id: emptyKeysId, keys: [], shared: []}));
  write('dist/assets/en-fixture.json', JSON.stringify({format: 'td-locale-v2', language: 'en', id: emptyKeysId, values: []}));
  write('dist/assets/main.js', '/* entry */'.padEnd(30, ' '));
  const manifest = { 'src/main.tsx': { isEntry: true, file: 'assets/main.js' } };
  for (const [key, name, bytes] of [[dashboard, 'dashboard', 200], [settings, 'settings', 80], [media, 'media', 120], [pdf, 'pdf', 160]]) {
    write(`dist/assets/${name}.js`, `/* ${name} */`.padEnd(bytes, ' '));
    manifest[key] = { src: key, file: `assets/${name}.js`, imports: key === dashboard ? [settings] : [] };
  }
  write('dist/.vite/manifest.json', JSON.stringify(manifest));
  const runBudget = () => {
    write('bundle-budget.json', JSON.stringify(budget));
    const result = spawnSync(process.execPath, [path.join(app, 'scripts/check-bundle-budget.cjs')], { encoding: 'utf8', timeout: 15_000 });
    assert.ifError(result.error);
    return result;
  };
  success(runBudget());
  const report = JSON.parse(fs.readFileSync(path.join(app, 'dist/bundle-report.json'), 'utf8'));
  assert.equal(report.summary.initialJavaScriptBytes, 30);
  assert.equal(report.routeJavaScript[dashboard].bytes, 280, 'route cost includes static imports');
  assert.equal(report.featureChunks[media].bytes, 120);
  // Actual packed data artifacts must retain every canonical value; shared
  // dictionary bytes participate in the same locale ceilings.
  write('src/i18n/locales/es.json', JSON.stringify({common: {greeting: 'Hola'}}));
  const keys = ['common.greeting'];
  const shared = ['Hola'];
  const keyId = crypto.createHash('sha256').update(JSON.stringify([keys, shared])).digest('hex');
  write('dist/assets/translation-keys-fixture.json', JSON.stringify({format: 'td-keys-v2', id: keyId, keys, shared}));
  write('dist/assets/es-fixture.json', JSON.stringify({format: 'td-locale-v2', language: 'es', id: keyId, values: [0]}));
  write('dist/assets/en-fixture.json', JSON.stringify({format: 'td-locale-v2', language: 'en', id: keyId, values: [null]}));
  success(runBudget());
  const packedReport = JSON.parse(fs.readFileSync(path.join(app, 'dist/bundle-report.json'), 'utf8'));
  assert.equal(packedReport.summary.totalLocaleDataBytes, ['translation-keys-fixture.json', 'en-fixture.json', 'es-fixture.json'].reduce((sum, name) => sum + fs.statSync(path.join(app, 'dist/assets', name)).size, 0));
  write('dist/assets/es-fixture.json', JSON.stringify({format: 'td-locale-v2', language: 'es', id: keyId, values: ['WRONG']}));
  const corrupt = runBudget();
  assert.notEqual(corrupt.status, 0, 'changed translations must not pass a packed-artifact check');
  assert.match(corrupt.stderr, /canonical.*es|es.*canonical/);
  write('dist/assets/es-fixture.json', JSON.stringify({format: 'td-locale-v2', language: 'es', id: keyId, values: [0]}));
  fs.rmSync(path.join(app, 'dist/assets/translation-keys-fixture.json'));
  assert.notEqual(runBudget().status, 0, 'removing the counted key table must fail');
  write('dist/assets/translation-keys-fixture.json', JSON.stringify({format: 'td-keys-v2', id: keyId, keys, shared}));
  success(runBudget());
  write('dist/assets/es-fixture.json', JSON.stringify({common: {greeting: 'Hola'}}));
  fs.rmSync(path.join(app, 'dist/assets/translation-keys-fixture.json'));
  const unpacked = runBudget();
  assert.notEqual(unpacked.status, 0, 'unpacked production languages are incompatible with the production loader');
  assert.match(unpacked.stderr, /Packed language catalogs require/);
  write('dist/assets/es-fixture.json', JSON.stringify({format: 'td-locale-v2', language: 'es', id: keyId, values: [0]}));
  write('dist/assets/translation-keys-fixture.json', JSON.stringify({format: 'td-keys-v2', id: keyId, keys, shared}));
  success(runBudget());
  delete budget.featureChunkBudgets[media];
  const missing = runBudget();
  assert.notEqual(missing.status, 0, 'removing a mandatory media budget must fail');
  assert.match(missing.stderr, /Missing or invalid required feature budget/);
  budget.featureChunkBudgets[media] = 100;
  const tooLarge = runBudget();
  assert.notEqual(tooLarge.status, 0);
  assert.match(tooLarge.stderr, /MediaPlayer.*exceeds/);
  budget.featureChunkBudgets[media] = 250000;
  budget.routeJavaScriptBudgets[dashboard] = 279;
  assert.notEqual(runBudget().status, 0, 'static route imports count against the ceiling');
  delete budget.routeJavaScriptBudgets;
  const noRoutes = runBudget();
  assert.notEqual(noRoutes.status, 0);
  assert.match(noRoutes.stderr, /Missing or invalid required route budget/);
});


test('the locale validation CLI rejects missing plural forms and altered extra-form interpolation', t => {
  const directory = fixture(t);
  const app = path.join(directory, 'app');
  const write = (name, value) => {
    const target = path.join(app, name);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, typeof value === 'string' ? value : JSON.stringify(value));
  };
  for (const name of ['shared.cjs', 'validate-locales.cjs']) {
    write('scripts/i18n/' + name, fs.readFileSync(path.join(scripts, '../app/scripts/i18n', name), 'utf8'));
  }
  const locales = Object.keys(JSON.parse(fs.readFileSync(path.join(scripts, '../app/src/i18n/copied-english-baseline.json'), 'utf8')));
  write('src/i18n/copied-english-baseline.json', Object.fromEntries(locales.map(locale => [locale, 0])));
  write('src/i18n/invariant-allowlist.json', {keys: [], tokens: []});
  const catalogs = {};
  catalogs.en = {common: {items_one: '{{count}} item for {{owner}}', items_other: '{{count}} items for {{owner}}', action_name:'{{action}} {{name}}', action_count:'{{action}} ({{count}})', action_prose:'Delete {{name}}'}};
  for (const locale of locales) {
    catalogs[locale] = {common: Object.fromEntries(new Intl.PluralRules(locale).resolvedOptions().pluralCategories.map(category => ['items_' + category, locale + ' {{count}} ' + category + ' {{owner}}']))};
  }
  for (const locale of locales) Object.assign(catalogs[locale].common,{action_name:'{{action}} {{name}}',action_count:'{{action}} ({{count}})',action_prose:locale+' {{name}}'});
  for (const [locale, catalog] of Object.entries(catalogs)) write('src/i18n/locales/' + locale + '.json', catalog);
  const check = () => {
    const result = spawnSync(process.execPath, [path.join(app, 'scripts/i18n/validate-locales.cjs')], {encoding: 'utf8', timeout: 15_000});
    assert.ifError(result.error);return result;
  };
  success(check());
  for(const [replacement,diagnostic]of [['{{action}} {{wrong}}','variable_mismatch'],['Delete {{name}}','copied_english_regression']]) {
    const key=diagnostic==='variable_mismatch'?'action_name':'action_prose',prior=catalogs.ar.common[key];
    catalogs.ar.common[key]=replacement;write('src/i18n/locales/ar.json',catalogs.ar);const result=check();
    assert.notEqual(result.status,0);assert.match(result.stderr,new RegExp(diagnostic));
    catalogs.ar.common[key]=prior;write('src/i18n/locales/ar.json',catalogs.ar);
  }
  const outcomes = [];
  for (const [locale, key, replacement, diagnostic] of [['ar', 'items_two', null], ['es', 'items_other', null], ['ar', 'items_few', 'ar {{count}} {{unexpected}}'], ['ar', 'items_many', catalogs.en.common.items_other, 'copied_english_regression']]) {
    const prior = catalogs[locale].common[key];
    if (replacement === null) delete catalogs[locale].common[key];else catalogs[locale].common[key] = replacement;
    write('src/i18n/locales/' + locale + '.json', catalogs[locale]);
    const result = check();outcomes.push({locale, key, rejected: result.status !== 0, diagnostic: (result.stderr + result.stdout).includes(diagnostic || key)});
    catalogs[locale].common[key] = prior;write('src/i18n/locales/' + locale + '.json', catalogs[locale]);
  }
  assert.deepEqual(outcomes.map(({locale, key, rejected, diagnostic}) => ({locale, key, rejected, diagnostic})), [
    {locale: 'ar', key: 'items_two', rejected: true, diagnostic: true},
    {locale: 'es', key: 'items_other', rejected: true, diagnostic: true},
    {locale: 'ar', key: 'items_few', rejected: true, diagnostic: true},
    {locale: 'ar', key: 'items_many', rejected: true, diagnostic: true},
  ]);
  for (const [locale, catalog] of Object.entries(catalogs)) {
    catalog.settings = {sync: {title: locale === 'en' ? 'Folder Sync' : locale + ' folder sync'}};
    write('src/i18n/locales/' + locale + '.json', catalog);
  }
  write('src/i18n/invariant-allowlist.json', {keys: ['settings.sync.title'], tokens: []});
  success(check());
  catalogs.ar.settings.sync.title = 'Folder Sync';
  write('src/i18n/locales/ar.json', catalogs.ar);
  const copiedExemption = check();
  assert.notEqual(copiedExemption.status, 0);
  assert.match(copiedExemption.stderr, /copied_english_regression/);
  catalogs.ar.settings.sync.title = 'ar folder sync';
  write('src/i18n/locales/ar.json', catalogs.ar);
  success(check());
});

test('the shipping UI scanner rejects TS toast and confirmation text, templates and conditional JSX', t => {
  const directory = fixture(t);
  const app = path.join(directory, 'app');
  const write = (name, text) => { const target=path.join(app,name);fs.mkdirSync(path.dirname(target),{recursive:true});fs.writeFileSync(target,text); };
  write('scripts/i18n/scan-ui-literals.cjs',fs.readFileSync(path.join(scripts,'../app/scripts/i18n/scan-ui-literals.cjs')));
  write('src/i18n/literal-budget.json',JSON.stringify({maxFindings:0,areaBudgets:{other:{maxFindings:0,patterns:[]}}}));
  write('src/i18n/literal-allowlist.json',JSON.stringify({allowlist:[]}));
  const check=()=>{const result=spawnSync(process.execPath,[path.join(app,'scripts/i18n/scan-ui-literals.cjs')],{cwd:app,env:{...process.env,NODE_PATH:path.join(scripts,'../app/node_modules')},encoding:'utf8',timeout:15_000});assert.ifError(result.error);return result;};
  const cases=[
    ['notice.ts',"toast.error('Cannot upload this file');",'Cannot upload this file'],
    ['notice.ts','toast.success(`Uploaded ${count} files`);','Uploaded'],
    ['notice.ts',"confirm(ready ? 'Remove this file?' : 'Wait for the transfer');",'Remove this file'],
    ['notice.ts',"confirm({title:'Delete the folder',description:'Files will be removed',confirmLabel:'Delete',variant:'danger'});",'Files will be removed'],
    ['notice.tsx',"const view=<p>{failed ? 'Transfer failed' : 'Transfer ready'}</p>;",'Transfer failed'],
    ['notice.tsx','const view=<button aria-label={`Pause ${name}`}>...</button>;','Pause'],
  ];
  const outcomes=[];
  for(const[name,source,expected]of cases){write('src/'+name,source);const result=check();outcomes.push({rejected:result.status!==0,diagnostic:(result.stdout+result.stderr).includes(expected)});fs.rmSync(path.join(app,'src',name));}
  assert.deepEqual(outcomes,cases.map(()=>({rejected:true,diagnostic:true})));
  write('src/translated.ts',"toast.success(t('notifications.saved')); confirm({title:t('files.delete'),variant:'danger'});");
  write('src/translated.tsx',"const view=<button aria-label={t('files.pause')}>{ready ? t('files.ready') : t('files.wait')}</button>;");
  success(check());
});


test('a real Cargo build rejects missing FTS5 and a cached SQLite dependency that ignored the changed environment', t => {
  const directory = fixture(t);
  fs.mkdirSync(path.join(directory, 'src'));
  const probe = path.resolve(scripts, '../app/src-tauri/build/search-support.rs');
  fs.copyFileSync(probe, path.join(directory, 'search-support.rs'));
  fs.writeFileSync(path.join(directory, 'Cargo.toml'), `[package]
name="release-search-build-fixture"
version="0.0.0"
edition="2021"
[build-dependencies]
sqlite={version="=0.37.0",features=["bundled"]}
`);
  fs.writeFileSync(path.join(directory, 'src/main.rs'), 'fn main() {}\n');
  fs.writeFileSync(path.join(directory, 'build.rs'), 'include!("search-support.rs"); fn main() { verify_search_support(); }\n');
  const environment = { ...process.env };
  delete environment.SQLITE_ENABLE_FTS5;
  const cargo = (args, env = environment) => {
    const result = spawnSync('cargo', args, { cwd: directory, env, encoding: 'utf8', timeout: 120_000 });
    assert.ifError(result.error);
    return result;
  };
  const missing = cargo(['build', '--offline']);
  assert.notEqual(missing.status, 0);
  assert.match(missing.stderr, /release blocked/);
  const cached = cargo(['build', '--offline'], { ...environment, SQLITE_ENABLE_FTS5: '1' });
  assert.notEqual(cached.status, 0);
  assert.match(cached.stderr, /SQLite lacks FTS5/);
  success(cargo(['clean', '-p', 'sqlite3-src']));
  success(cargo(['build', '--offline'], { ...environment, SQLITE_ENABLE_FTS5: '1' }));
});


test('release preflight checks local inputs and exactly one read-only health request', async t => {
  const http = require('node:http');
  const { spawn } = require('node:child_process');
  let health = { status: 'ok', price: '5.00', currency: 'USD', max_active_devices: 3, entitlement_public_key: 'public-fixture-key' };
  const requests = [];
  const server = http.createServer((req, res) => {
    requests.push([req.method, req.url]);
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify(health));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  t.after(() => server.close());
  const root = path.resolve(scripts, '..');
  let extraEnv = {};
  const invoke = (...args) => new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [path.join(scripts, 'release-preflight.cjs'), '--allow-unreleased', ...args], { cwd: root, env: { ...process.env, SUPPORTER_SERVICE_URL: '', TELEGRAM_DRIVE_SUPPORTER_SERVICE_URL: '', SUPPORTER_PUBLIC_KEY: '', TELEGRAM_DRIVE_SUPPORTER_PUBLIC_KEY: '', ...extraEnv } });
    let stdout = '', stderr = '';
    child.stdout.on('data', data => { stdout += data; });
    child.stderr.on('data', data => { stderr += data; });
    child.on('error', reject); child.on('close', status => resolve({ status, stdout, stderr }));
  });
  const url = `http://127.0.0.1:${server.address().port}`;
  const healthy = await invoke('--service-url', url, '--public-key', 'public-fixture-key');
  success(healthy); assert.match(healthy.stdout, /PASSED/); assert.deepEqual(requests, [['GET', '/health']]);
  extraEnv = { SUPPORTER_PUBLIC_KEY: 'public-fixture-key', TELEGRAM_DRIVE_SUPPORTER_PUBLIC_KEY: 'different-build-key' };
  const conflicting = await invoke('--service-url', url);
  assert.notEqual(conflicting.status, 0); assert.match(conflicting.stderr, /conflicting/i);
  extraEnv = {};
  const skipped = await invoke('--skip-network');
  assert.equal(skipped.status, 2); assert.match(skipped.stdout, /SKIPPED/); assert.doesNotMatch(skipped.stdout, /PASSED/);
  const keyMismatch = await invoke('--service-url', url, '--public-key', 'other-key');
  assert.notEqual(keyMismatch.status, 0); assert.match(keyMismatch.stderr, /public key/);
  for (const mutation of [{ price: '6.00' }, { currency: 'EUR' }, { max_active_devices: 4 }]) {
    const previous = health; health = { ...health, ...mutation };
    assert.notEqual((await invoke('--service-url', url)).status, 0); health = previous;
  }
  const draft = fixture(t);
  for (const name of ['app/package.json', 'app/package-lock.json', 'app/src-tauri/Cargo.toml',
    'app/src-tauri/Cargo.lock', 'app/src-tauri/tauri.conf.json', 'scripts/release-preflight.cjs']) {
    const destination = path.join(draft, name);
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    fs.copyFileSync(path.join(root, name), destination);
  }
  fs.writeFileSync(path.join(draft, 'CHANGELOG.md'), '## [Unreleased]\n\nExplicit draft fixture.\n');
  const strict = await new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [path.join(draft, 'scripts/release-preflight.cjs'), '--skip-network'], {
      cwd: draft, env: { ...process.env, SUPPORTER_SERVICE_URL: '', TELEGRAM_DRIVE_SUPPORTER_SERVICE_URL: '',
        SUPPORTER_PUBLIC_KEY: '', TELEGRAM_DRIVE_SUPPORTER_PUBLIC_KEY: '' }
    });
    let stderr = ''; child.stderr.on('data', data => { stderr += data; });
    child.on('error', reject); child.on('close', status => resolve({ status, stderr }));
  });
  assert.notEqual(strict.status, 0); assert.match(strict.stderr, /changelog/);
});

test('desktop release tag policy keeps rehearsal releases off latest and validates version/changelog inputs', t => {
  const directory = fixture(t);
  const app = path.join(directory, 'app');
  fs.mkdirSync(path.join(app, 'src-tauri'), { recursive: true });
  fs.writeFileSync(path.join(app, 'package.json'), JSON.stringify({ version: '4.0.0' }));
  fs.writeFileSync(path.join(app, 'src-tauri/Cargo.toml'), '[package]\nname="app"\nversion="4.0.0"\n');
  fs.writeFileSync(path.join(app, 'src-tauri/tauri.conf.json'), JSON.stringify({ version: '4.0.0' }));
  const changelog = path.join(directory, 'CHANGELOG.md');
  fs.writeFileSync(changelog, '## [4.0.0] - 2026-10-02\n\nReviewed release body.\n\n## [3.9.8]\n\nOld body.\n');
  const body = path.join(directory, 'body.md');
  for (const tag of ['v4.0.0', 'v4.0.0-rc.1']) {
    const result = run('release-tag-policy.cjs', '--tag', tag, '--app-root', app, '--changelog', changelog, '--body-output', body);
    success(result); const policy = JSON.parse(result.stdout);
    assert.equal(policy.applicationVersion, '4.0.0');
    assert.equal(policy.prerelease, tag.includes('-'));
    assert.equal(policy.make_latest, tag.includes('-') ? 'false' : 'true');
    assert.match(fs.readFileSync(body, 'utf8'), /Reviewed release body/);
    assert.doesNotMatch(fs.readFileSync(body, 'utf8'), /Old body/);
  }
  for (const tag of ['v4.0', '4.0.0', 'v04.0.0', 'v4.0.0-rc.01', 'v4.0.0-', 'v4.0.0+build', 'v4.0.1-rc.1']) {
    assert.notEqual(run('release-tag-policy.cjs', '--tag', tag, '--app-root', app, '--changelog', changelog).status, 0);
  }
  fs.writeFileSync(changelog, '## [Unreleased]\nDraft\n## [4.0.0]\nFinal\n');
  assert.notEqual(run('release-tag-policy.cjs', '--tag', 'v4.0.0-rc.1', '--app-root', app, '--changelog', changelog).status, 0);
});

test('draft release reruns reuse a paginated draft and await publication policy without redrafting stable releases', async t => {
  const http = require('node:http');
  const { spawn } = require('node:child_process');
  let releases = [{ id: 1, tag_name: 'v3.9.8', draft: false }];
  const writes = [];
  const server = http.createServer((req, res) => {
    let body = ''; req.on('data', data => { body += data; });
    req.on('end', () => {
      const url = new URL(req.url, 'http://fixture');
      res.setHeader('Content-Type', 'application/json');
      if (req.method === 'GET') {
        // One release per page forces real pagination even when a draft is later.
        const page = Number(url.searchParams.get('page') || 1);
        return res.end(JSON.stringify(releases.slice(page - 1, page)));
      }
      const data = JSON.parse(body); writes.push([req.method, data]);
      setTimeout(() => {
        if (req.method === 'POST') { const release = { id: releases.length + 1, ...data }; releases.push(release); res.end(JSON.stringify(release)); }
        else { const id = Number(url.pathname.split('/').pop()); const release = releases.find(value => value.id === id); Object.assign(release, data); res.end(JSON.stringify(release)); }
      }, 50);
    });
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve)); t.after(() => server.close());
  const directory = fixture(t), bodyFile = path.join(directory, 'body.md'); fs.writeFileSync(bodyFile, 'Reviewed notes.\n');
  const invoke = (...args) => new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [path.join(scripts, 'release-draft.cjs'), '--fixture-api', `http://127.0.0.1:${server.address().port}`, '--repo', 'owner/repo', '--tag', 'v4.0.0-rc.1', '--body-file', bodyFile, ...args]);
    let stdout = '', stderr = ''; child.stdout.on('data', data => { stdout += data; }); child.stderr.on('data', data => { stderr += data; });
    child.on('error', reject); child.on('close', status => resolve({ status, stdout, stderr }));
  });
  success(await invoke()); success(await invoke());
  assert.equal(releases.filter(value => value.tag_name === 'v4.0.0-rc.1').length, 1);
  assert.equal(writes.filter(([method]) => method === 'POST').length, 1);
  success(await invoke('--publish'));
  const rehearsal = releases.find(value => value.tag_name === 'v4.0.0-rc.1');
  assert.equal(rehearsal.draft, false); assert.equal(rehearsal.prerelease, true); assert.equal(rehearsal.make_latest, 'false');
  const prior = writes.length; assert.notEqual((await invoke()).status, 0); assert.equal(writes.length, prior);
});

test('packaged startup smoke requires application readiness and reaps the isolated child', async t => {
  const { spawn } = require('node:child_process');
  const directory = fixture(t);
  const application = path.join(directory, 'application.cjs');
  fs.writeFileSync(application, `const fs=require('node:fs'),path=require('node:path');
const root=process.env.TELEGRAM_DRIVE_STARTUP_SMOKE_ROOT;
fs.writeFileSync(path.join(root,'child-pid'),String(process.pid));
if(process.argv[2]==='exit')process.exit(3);
if(process.argv[2]==='ready')fs.writeFileSync(path.join(root,'startup-ready.json'),JSON.stringify({process_id:process.pid,database_ready:true,app_data_ready:true,streaming_runtime_ready:true,app_data_dir:path.join(root,'data'),version:'4.0.0',run_token:process.env.TELEGRAM_DRIVE_STARTUP_SMOKE_TOKEN,profile_identifier:'com.cameronamer.telegramdrive.smoke.'+process.env.TELEGRAM_DRIVE_STARTUP_SMOKE_TOKEN,...Object.fromEntries(['app_data_dir','app_cache_dir','app_config_dir','app_local_data_dir'].map(name=>[name,path.join(root,name,'com.cameronamer.telegramdrive.smoke.'+process.env.TELEGRAM_DRIVE_STARTUP_SMOKE_TOKEN)]))}));
setInterval(()=>{},1000);
`);
  const invoke = (mode, timeout = '2000') => new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [path.join(scripts, 'packaged-startup-smoke.cjs'), '--disposable-user', '--executable', process.execPath, '--args-json', JSON.stringify([application, mode]), '--timeout-ms', timeout]);
    let stdout = '', stderr = ''; child.stdout.on('data', data => { stdout += data; }); child.stderr.on('data', data => { stderr += data; });
    child.on('error', reject); child.on('close', status => resolve({status,stdout,stderr}));
  });
  success(await invoke('ready'));
  assert.notEqual((await invoke('exit')).status, 0);
  const waiting = await invoke('wait', '500');
  assert.notEqual(waiting.status, 0); assert.match(waiting.stderr, /readiness/);
});

test('RC packaging uses the plain application version for the exact upstream Debian and Arch recipe', t => {
  const directory = fixture(t);
  const policy = run('release-tag-policy.cjs', '--tag', 'v4.0.0-rc.1'); success(policy);
  const version = JSON.parse(policy.stdout).applicationVersion;
  const deb = path.join(directory, `Telegram.Drive_${version}_amd64.deb`); fs.writeFileSync(deb, 'fixture binary package');
  const rendered = path.join(directory, 'recipe');
  const result = spawnSync('bash', [path.join(scripts, 'render-arch-pkgbuild.sh'), deb, version, rendered], {encoding:'utf8',timeout:15000}); success(result);
  const recipe = fs.readFileSync(path.join(rendered, 'PKGBUILD'), 'utf8');
  assert.match(recipe, /^pkgver=4\.0\.0$/m); assert.match(recipe, /Telegram\.Drive_4\.0\.0_amd64\.deb/);
  assert.doesNotMatch(recipe, /rc\.1/);
});
