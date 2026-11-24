import assert from 'node:assert/strict';
import { createHash, randomBytes, sign } from 'node:crypto';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { gzipSync, gunzipSync } from 'node:zlib';
import { device, startService, terms } from './local-service.mjs';

const digest = bytes => createHash('sha256').update(bytes).digest('hex');

async function body(response, expected = 200) {
  assert.equal(response.status, expected);
  return response.json();
}

async function checkout(worker) {
  const owner = device();
  const claim = await body(await worker.request('/v1/checkout', { method: 'POST', body: {
    device_public_key: owner.encoded, terms_version: terms, terms_accepted: true,
  } }), 201);
  return { ...claim, owner, order: new URL(claim.approval_url).searchParams.get('token') };
}

const poll = (worker, claim) => worker.request(`/v1/checkout/${claim.claim_id}/status`, { bearer: claim.claim_secret });
const activate = (worker, code, owner) => worker.request('/v1/activate', { method: 'POST', body: {
  recovery_code: code, device_public_key: owner.encoded, terms_accepted: true, terms_version: terms,
} });

async function inventory(worker) {
  const schema = await worker.sql("SELECT name, type, sql FROM sqlite_master WHERE type IN ('table','trigger','index') AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_cf_%' ORDER BY name");
  const rows = {};
  for (const { name, type } of schema) {
    if (type !== 'table') continue;
    const contents = await worker.sql(`SELECT * FROM "${name.replaceAll('"', '""')}"`);
    rows[name] = { count: contents.length, digest: digest(contents.map(row => JSON.stringify(row)).sort().join('\n')) };
  }
  return { schema, rows };
}

function unpack(archive, checksum) {
  assert.equal(digest(archive) + '  supporter.sql.gz\n', checksum, 'Backup checksum mismatch');
  return gunzipSync(archive).toString();
}

test('a fresh D1 and independently recovered keys preserve paid, pending and revoked purchases', { timeout: 120_000 }, async t => {
  const archives = await mkdtemp(join(tmpdir(), 'td-synthetic-d1-'));
  const custody = await mkdtemp(join(tmpdir(), 'td-synthetic-custody-'));
  t.after(() => Promise.all([archives, custody].map(dir => rm(dir, { recursive: true, force: true }))));
  const source = await startService();
  let sourceClosed = false;
  t.after(() => sourceClosed ? undefined : source.close());
  const escrow = { signing: source.env.ENTITLEMENT_SIGNING_JWK, lookup: source.env.RECOVERY_LOOKUP_KEY, encryption: source.env.RECOVERY_ENCRYPTION_KEY };
  const keys = join(custody, 'fixture-keys.json');
  await writeFile(keys, JSON.stringify(escrow), { mode: 0o600 });
  const pinned = JSON.parse(escrow.signing).x;
  const paid = await checkout(source);
  const refunded = await checkout(source);
  const pending = await checkout(source);
  source.paypal.complete(paid.order);
  source.paypal.complete(refunded.order);
  const receipt = await body(await poll(source, paid));
  const revoked = await body(await poll(source, refunded));
  const event = { id: 'restore-drill-refund', event_type: 'PAYMENT.CAPTURE.REFUNDED', resource: {
    id: 'restore-refund', links: [{ rel: 'up', href: 'https://api-m.sandbox.paypal.com/v2/payments/captures/CAPTURE' + refunded.order }],
  } };
  const notify = worker => worker.request('/v1/paypal/webhook', { method: 'POST', body: event, headers: {
    'paypal-auth-algo': 'SHA256withRSA', 'paypal-cert-url': 'https://api-m.sandbox.paypal.com/v1/notifications/certs/E2ECERT',
    'paypal-transmission-id': 'drill-fixture', 'paypal-transmission-sig': 'ZmFrZQ==', 'paypal-transmission-time': new Date().toISOString(),
  } });
  await body(await notify(source));
  const before = await inventory(source);
  assert.equal(before.rows.entitlements.count, 2);
  assert.equal(before.rows.checkout_claims.count, 3);
  assert.equal(before.rows.webhook_events.count, 1);
  assert.deepEqual((await source.sql('SELECT DISTINCT status FROM entitlements ORDER BY status')).map(row => row.status), ['active', 'revoked']);

  const sql = await source.exportSql();
  for (const secret of [escrow.lookup, escrow.encryption, JSON.parse(escrow.signing).d, receipt.recovery_code]) {
    assert.equal(sql.includes(secret), false, 'A secret entered the database backup');
  }
  const gz = gzipSync(sql, { level: 9 });
  const filename = join(archives, 'supporter.sql.gz');
  await writeFile(filename, gz, { mode: 0o600 });
  await writeFile(filename + '.sha256', digest(gz) + '  supporter.sql.gz\n', { mode: 0o600 });
  const checksum = await readFile(filename + '.sha256', 'utf8');
  assert.throws(() => unpack(gz.subarray(1), checksum), /checksum mismatch/);
  const restoredSql = unpack(await readFile(filename), checksum);
  await source.close();
  sourceClosed = true;
  const recovered = JSON.parse(await readFile(keys, 'utf8'));
  const restored = await startService({ restoredSql, escrow: recovered, paypal: source.paypal });
  t.after(() => restored.close());
  assert.deepEqual(await inventory(restored), before);
  assert.equal((await body(await restored.request('/health'))).entitlement_public_key, pinned);
  const retry = await body(await poll(restored, paid));
  assert.ok(retry.recovery_code === receipt.recovery_code, 'Recovery encryption key did not recover the undelivered receipt');
  const challenge = await body(await restored.request('/v1/challenge', { method: 'POST', bearer: receipt.entitlement_token }));
  const signature = sign(null, Buffer.from(`telegram-drive-supporter-refresh:${challenge.challenge_id}:${challenge.nonce}`), paid.owner.privateKey);
  const renewed = await body(await restored.request('/v1/refresh', { method: 'POST', body: {
    entitlement_token: receipt.entitlement_token, challenge_id: challenge.challenge_id, nonce: challenge.nonce, signature: signature.toString('base64url'),
  } }));
  assert.equal(source.verifyToken(renewed.entitlement_token, paid.owner).entitlement_id, source.verifyToken(receipt.entitlement_token, paid.owner).entitlement_id);
  for (let i = 0; i < 2; i++) {
    const next = device();
    source.verifyToken((await body(await activate(restored, receipt.recovery_code, next))).entitlement_token, next);
  }
  assert.equal((await activate(restored, receipt.recovery_code, device())).status, 409);
  assert.equal((await activate(restored, revoked.recovery_code, device())).ok, false);
  await body(await notify(restored));
  assert.equal((await body(await poll(restored, pending))).status, 'pending');
  source.paypal.complete(pending.order);
  assert.equal((await body(await poll(restored, pending))).status, 'completed');
  assert.equal(source.paypal.orders.size, 3);

  const wrong = await startService({ restoredSql, escrow: { ...recovered, lookup: randomBytes(32).toString('base64url') } });
  t.after(() => wrong.close());
  assert.equal((await activate(wrong, receipt.recovery_code, device())).ok, false);
  assert.deepEqual(await inventory(wrong), before);
  wrong.env.RECOVERY_LOOKUP_KEY = recovered.lookup;
  wrong.env.RECOVERY_ENCRYPTION_KEY = randomBytes(32).toString('base64url');
  await wrong.restart();
  const unreadable = await poll(wrong, paid);
  assert.equal(unreadable.ok, false, 'A wrong recovery encryption key cannot deliver a receipt');
  assert.ok((await wrong.sql('SELECT recovery_ciphertext FROM checkout_claims WHERE id = ?', [paid.claim_id]))[0].recovery_ciphertext);
  assert.equal(digest(await readFile(filename)), digest(gz));
});
