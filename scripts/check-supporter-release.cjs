// Verify the public inputs that the candidate compiles into its native client.
// The release process alone contacts the configured service.
const fs = require('node:fs');
const path = require('node:path');

async function verifySupporterRelease(service = '', key = '') {
  if (/[\x00-\x20\x7f]/.test(service) || !/^https:\/\/[^/@?#\\\s]+$/.test(service)) {
    throw new Error('Provide the supporter HTTPS origin without credentials, path, trailing slash, query, or fragment.');
  }
  const url = new URL('/health', new URL(service));
  const bytes = Buffer.from(key, 'base64url');
  if (bytes.length !== 32 || bytes.toString('base64url') !== key) {
    throw new Error('Provide the exact 32-byte Ed25519 build public key in canonical unpadded base64url.');
  }

  const source = fs.readFileSync(path.join(__dirname, '../app/src-tauri/src/commands/supporter_desktop.rs'), 'utf8');
  const terms = source.match(/^const TERMS_VERSION:\s*&str\s*=\s*"([^"]+)";/m)?.[1];
  const publishedTerms = fs.readFileSync(path.join(__dirname, '../SUPPORTER_TERMS.md'), 'utf8')
    .match(/^Terms version: (\S+)\s*$/m)?.[1];
  if (!terms || terms !== publishedTerms) throw new Error('The native client and published supporter terms must specify the same version.');

  const response = await fetch(url, { method: 'GET', redirect: 'error', signal: AbortSignal.timeout(10_000) });
  if (!response.ok) throw new Error(`Supporter health returned HTTP ${response.status}.`);
  const health = await response.json();
  const contract = { status: 'ok', price: '5.00', currency: 'USD', max_active_devices: 3, terms_version: terms, entitlement_public_key: key };
  for (const [name, expected] of Object.entries(contract)) {
    if (health?.[name] !== expected) throw new Error(`Supporter health disagrees with the release contract for ${name}.`);
  }
  console.log('Verified supporter service, signing key, $5.00 USD lifetime terms and three-device allowance.');
}

module.exports = { verifySupporterRelease };

if (require.main === module) {
  verifySupporterRelease(process.env.TELEGRAM_DRIVE_SUPPORTER_SERVICE_URL, process.env.TELEGRAM_DRIVE_SUPPORTER_PUBLIC_KEY).catch(error => {
    console.error(`Supporter release verification failed: ${error.message}`);
    process.exitCode = 1;
  });
}
