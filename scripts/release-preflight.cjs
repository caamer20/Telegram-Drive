#!/usr/bin/env node
// Read-only release inputs and supporter compatibility gate.
const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const root = path.resolve(__dirname, '..');
const options = { service: process.env.TELEGRAM_DRIVE_SUPPORTER_SERVICE_URL || process.env.SUPPORTER_SERVICE_URL,
  key: process.env.TELEGRAM_DRIVE_SUPPORTER_PUBLIC_KEY || process.env.SUPPORTER_PUBLIC_KEY };
for (let i = 2; i < process.argv.length; i++) {
  const flag = process.argv[i];
  if (flag === '--skip-network') options.skip = true;
  else if (flag === '--allow-unreleased') options.unreleased = true;
  else if (['--service-url', '--public-key', '--verify-attestation', '--attestation-repo'].includes(flag)) {
    const value = process.argv[++i];
    if (!value || value.startsWith('--')) throw new Error(`Missing value for ${flag}`);
    options[{ '--service-url': 'service', '--public-key': 'key', '--verify-attestation': 'artifact', '--attestation-repo': 'repo' }[flag]] = value;
  } else throw new Error(`Unknown flag ${flag}`);
}
const read = name => fs.readFileSync(path.join(root, name), 'utf8');
const json = name => JSON.parse(read(name));
function processCheck(command, args) {
  const result = spawnSync(command, args, { cwd: root, encoding: 'utf8', timeout: 60_000 });
  if (result.error || result.status !== 0) throw new Error(result.error?.message || result.stderr || result.stdout || `${command} failed`);
  process.stdout.write(result.stdout);
  process.stderr.write(result.stderr);
}
async function main() {
  for (const [short, build, flag] of [['SUPPORTER_SERVICE_URL', 'TELEGRAM_DRIVE_SUPPORTER_SERVICE_URL', '--service-url'], ['SUPPORTER_PUBLIC_KEY', 'TELEGRAM_DRIVE_SUPPORTER_PUBLIC_KEY', '--public-key']]) {
    if (!process.argv.includes(flag) && process.env[short] && process.env[build] && process.env[short] !== process.env[build]) throw new Error(`Conflicting ${short} and ${build} aliases`);
  }
  const version = json('app/package.json').version;
  const cargo = read('app/src-tauri/Cargo.toml').match(/\[package\]([\s\S]*?)(?=\n\[|$)/)?.[1].match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  if (!version || cargo !== version || json('app/src-tauri/tauri.conf.json').version !== version) throw new Error('Application versions disagree');
  const lockedApp = read('app/src-tauri/Cargo.lock').match(/\[\[package\]\]\nname = "app"\nversion = "([^"]+)"/)?.[1];
  if (lockedApp !== version) throw new Error('Cargo lockfile application version disagrees');
  if (json('app/package-lock.json').version !== version || json('app/package-lock.json').packages[''].version !== version) throw new Error('npm lockfile version disagrees');
  const heading = read('CHANGELOG.md').match(/^## \[([^\]]+)\]/m)?.[1];
  if (heading !== version && !(options.unreleased && heading === 'Unreleased')) throw new Error(`First changelog heading must be ${version}; use --allow-unreleased only for local readiness`);
  const links = [...read('README.md').matchAll(/releases\/download\/v([^/]+)\//g)].map(match => match[1]);
  if (!links.length || links.some(value => value !== version)) throw new Error('README desktop download links disagree with application version');
  for (const name of ['.cargo/config.toml', 'app/package-lock.json', 'app/src-tauri/Cargo.lock', 'app/src-tauri/build.rs', 'app/src-tauri/build/search-support.rs', 'app/src-tauri/tauri.windows.release.conf.json']) {
    if (!fs.statSync(path.join(root, name)).isFile()) throw new Error(`Missing release build input ${name}`);
  }
  console.log(`[preflight] Version ${version}, changelog, README and build inputs agree${heading === 'Unreleased' ? ' (local readiness; owner must finalize changelog before tag)' : ''}.`);
  for (const [script, args] of [['check-baseline-expiry.cjs', ['--warn-days', '30']], ['check-test-policy.cjs', []], ['check-app-security.cjs', []], ['check-android-publication.cjs', ['--tracked-only']]]) {
    processCheck(process.execPath, [path.join(root, 'scripts', script), ...args]);
  }
  if (options.artifact) {
    if (!options.repo || !/^[\w.-]+\/[\w.-]+$/.test(options.repo)) throw new Error('--attestation-repo OWNER/REPO is required for verification');
    if (!fs.statSync(options.artifact).isFile()) throw new Error('Attestation artifact is missing');
    processCheck('gh', ['attestation', 'verify', path.resolve(options.artifact), '--repo', options.repo]);
    console.log('[preflight] Artifact attestation verified.');
  } else console.log('[preflight] Artifact attestation UNRUN: supply --verify-attestation FILE --attestation-repo OWNER/REPO after artifact generation.');
  if (options.skip) {
    console.log('[preflight] SKIPPED supporter /health; INCOMPLETE, release readiness is not passed.');
    process.exitCode = 2; return;
  }
  if (!options.service) throw new Error('Provide --service-url or SUPPORTER_SERVICE_URL; use --skip-network for explicit incomplete local checks');
  const url = new URL(options.service);
  const loopback = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname);
  if (!(options.unreleased && loopback && url.protocol === 'http:')) {
    return require('./check-supporter-release.cjs').verifySupporterRelease(options.service, options.key);
  }
  if (url.username || url.password || url.search || url.hash) throw new Error('Local fixture URL must omit credentials, query and fragment');
  url.pathname = `${url.pathname.replace(/\/$/, '').replace(/\/health$/, '')}/health`;
  // Redirects are refused: a preflight makes exactly one GET and never follows another endpoint.
  const response = await fetch(url, { method: 'GET', redirect: 'error', signal: AbortSignal.timeout(10_000) });
  if (!response.ok) throw new Error(`Supporter health HTTP ${response.status}`);
  const health = await response.json();
  if (health.status !== 'ok' || health.price !== '5.00' || health.currency !== 'USD' || health.max_active_devices !== 3) throw new Error('Supporter health violates the $5.00 USD lifetime / three-device configuration');
  if (options.key && health.entitlement_public_key !== options.key) throw new Error('Supporter public key does not equal the configured build public key');
  if (typeof health.entitlement_public_key !== 'string' || !health.entitlement_public_key) throw new Error('Supporter public key is missing');
  console.log(`[preflight] PASSED input and read-only health checks${options.key ? ', including build public key' : '; build key comparison UNRUN (no configured key supplied)'}. Packaged builds, live acceptance and CI remain separate gates.`);
}
main().catch(error => { console.error(`[preflight] FAILED: ${error.message}`); process.exitCode = 1; });
