const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs'), os = require('node:os'), path = require('node:path');
const { spawnSync } = require('node:child_process');
const { randomBytes } = require('node:crypto');
const cli = path.resolve(__dirname, '../packaged-startup-smoke.cjs');
const app = path.join(__dirname, 'fixtures/startup-smoke-app.cjs');
function journey(t, mode, extra = []) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'td-packaged-cli-e2e-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const evidence = path.join(directory, 'evidence.json');
  const result = spawnSync(process.execPath, [cli, '--disposable-user', '--executable', process.execPath,
    '--args-json', JSON.stringify([app, mode, evidence]), '--timeout-ms', process.platform === 'win32' ? '20000' : '8000', ...extra], { encoding: 'utf8', timeout: 45000 });
  assert.ifError(result.error);
  return { result, evidence, record: fs.existsSync(evidence) && JSON.parse(fs.readFileSync(evidence, 'utf8')) };
}
test('smoke CLI accepts numeric-leading nonce with a valid isolated identity and removes the private profile', t => {
  const token = '0' + randomBytes(16).toString('hex').slice(1);
  const { result, record } = journey(t, 'ready', ['--run-token', token, '--expected-bundle-type', 'nsis']);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(record.token, token);
  assert.equal(record.identifier, `com.cameronamer.telegramdrive.smoke.r${token}`);
  assert.equal(fs.existsSync(record.root), false);
});
test('smoke CLI rejects incorrect runtime package metadata', t => {
  const { result, record } = journey(t, 'wrong-kind', ['--expected-bundle-type', 'nsis']);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /bundle type.*nsis/i);
  assert.equal(fs.existsSync(record.root), false);
});
test('early exit keeps the startup failure and reaps surviving descendants before cleanup', t => {
  const { result, evidence, record } = journey(t, 'orphan');
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /fixture startup failed/);
  const pid = Number(fs.readFileSync(`${evidence}.child`, 'utf8'));
  assert.ok(Number.isSafeInteger(pid) && pid > 1, 'Descendant must report a valid PID');
  if (process.platform === 'win32') {
    const check = spawnSync('tasklist', ['/FI', `PID eq ${pid}`, '/FO', 'CSV', '/NH'], { encoding: 'utf8' });
    assert.equal(check.status, 0, check.stderr); assert.ok(!check.stdout.includes(`"${pid}"`), check.stdout);
  } else {
    // A briefly unreaped Unix zombie cannot run or retain the lock descriptor.
    const check = spawnSync('ps', ['-o', 'stat=', '-p', String(pid)], { encoding: 'utf8' });
    assert.ok(check.status !== 0 || /^Z/.test(check.stdout.trim()), check.stdout);
  }
  fs.rmSync(`${evidence}.child.lock`);
  assert.equal(fs.existsSync(record.root), false);
});
test('readiness deadline fails and cleans up the private launch', t => {
  const { result, record } = journey(t, 'deadline', ['--timeout-ms', process.platform === 'win32' ? '10000' : '250']);
  assert.notEqual(result.status, 0); assert.match(result.stderr, /before deadline/);
  assert.equal(fs.existsSync(record.root), false);
});
test('Windows rejects readiness followed by natural application exit while its descendants are reaped', { skip: process.platform !== 'win32' }, t => {
  const { result, record, evidence } = journey(t, 'ready-exit');
  assert.notEqual(result.status, 0, result.stdout);
  assert.match(result.stderr, /early exit|exited naturally/);
  assert.doesNotMatch(result.stdout, /Startup readiness passed/);
  const pid = Number(fs.readFileSync(`${evidence}.child`, 'utf8'));
  assert.ok(Number.isSafeInteger(pid) && pid > 1, 'Descendant must report a valid PID');
  const check = spawnSync('tasklist', ['/FI', `PID eq ${pid}`, '/FO', 'CSV', '/NH'], { encoding: 'utf8' });
  assert.equal(check.status, 0, check.stderr); assert.ok(!check.stdout.includes(`"${pid}"`), check.stdout);
  fs.rmSync(`${evidence}.child.lock`);
  assert.equal(fs.existsSync(record.root), false);
});
test('Windows smoke preserves the OS KnownFolder profile environment', { skip: process.platform !== 'win32' }, t => {
  const { result, record } = journey(t, 'ready');
  assert.equal(result.status, 0, result.stderr);
  for (const name of ['USERPROFILE', 'APPDATA', 'LOCALAPPDATA']) assert.equal(record[name], process.env[name]);
});
