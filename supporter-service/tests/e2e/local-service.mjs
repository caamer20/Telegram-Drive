import assert from 'node:assert/strict';
import { createHash, createPrivateKey, createPublicKey, generateKeyPairSync, randomBytes, sign, verify } from 'node:crypto';
import { mkdtemp, readFile, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { unstable_splitSqlQuery } from 'wrangler';
import { Miniflare, convertV4MiniflareOptions, Log, LogLevel } from 'miniflare';

export const terms = '2026-08-11';
export const day = 86_400;
export const hash = value => createHash('sha256').update(value).digest('base64url');
export function device() {
  const pair = generateKeyPairSync('ed25519');
  return { ...pair, encoded: pair.publicKey.export({ format: 'jwk' }).x };
}

// Only the external PayPal transport is simulated. Production routing, payment
// verification, cryptography and D1 transactions all execute inside workerd.
export class PayPalSandbox {
  orders = new Map();
  calls = [];
  unavailable = false;
  verification = 'SUCCESS';
  verificationUnavailable = false;
  verificationBodies = [];
  expectedRawEvent;
  reset() {
    this.orders.clear();
    this.calls.length = 0;
    this.unavailable = false;
    this.verification = 'SUCCESS';
    this.verificationUnavailable = false;
    this.verificationBodies.length = 0;
    this.expectedRawEvent = undefined;
  }
  complete(orderId) {
    const order = this.orders.get(orderId);
    assert.ok(order, 'The order must first be created through the HTTP checkout');
    order.status = 'COMPLETED';
    order.purchase_units[0].payments = { captures: [{
      id: `CAPTURE${orderId}`, status: 'COMPLETED', amount: { value: '5.00', currency_code: 'USD' },
      payee: { merchant_id: 'e2e-merchant' },
    }] };
    return order;
  }
  async fetch(request) {
    const url = new URL(request.url);
    const raw = await request.text();
    const body = raw ? (raw.startsWith('{') ? JSON.parse(raw) : raw) : undefined;
    this.calls.push({ method: request.method, path: url.pathname, body, requestId: request.headers.get('paypal-request-id') });
    assert.equal(url.origin, 'https://api-m.sandbox.paypal.com', 'No external/live payment traffic is allowed');
    if (this.unavailable) return Response.json({ error: 'local-provider-outage' }, { status: 503 });
    if (url.pathname === '/v1/oauth2/token') {
      assert.equal(request.headers.get('authorization'), `Basic ${Buffer.from('e2e-client:e2e-secret').toString('base64')}`);
      return Response.json({ access_token: 'e2e-access-token', expires_in: 3600 });
    }
    assert.equal(request.headers.get('authorization'), 'Bearer e2e-access-token');
    if (url.pathname === '/v2/checkout/orders' && request.method === 'POST') {
      assert.equal(body.intent, 'CAPTURE');
      assert.deepEqual(body.purchase_units[0].amount, { currency_code: 'USD', value: '5.00' });
      assert.equal(body.payment_source.paypal.experience_context.shipping_preference, 'NO_SHIPPING');
      const id = `ORDER${this.orders.size + 1}`;
      const order = {
        id, status: 'CREATED', purchase_units: [{ ...body.purchase_units[0], payee: { merchant_id: 'e2e-merchant' } }],
        // Sensitive provider metadata must never be persisted by the Worker.
        payer: { email_address: 'payer-private@example.invalid', name: { given_name: 'Zoë 測試' } },
        links: [{ href: `https://www.sandbox.paypal.com/checkoutnow?token=${id}`, rel: 'payer-action', method: 'GET' }],
      };
      this.orders.set(id, order);
      return Response.json(order, { status: 201 });
    }
    const match = url.pathname.match(/^\/v2\/checkout\/orders\/([^/]+)(\/capture)?$/);
    if (match) {
      const order = this.orders.get(match[1]);
      if (!order) return Response.json({ error: 'ORDER_NOT_FOUND' }, { status: 404 });
      if (match[2]) {
        assert.equal(request.method, 'POST');
        assert.equal(request.headers.get('paypal-request-id'), `telegram-drive-capture-${order.id}`);
        if (order.status === 'COMPLETED') return Response.json({ error: 'ORDER_ALREADY_CAPTURED' }, { status: 422 });
        this.complete(order.id);
        // Real capture responses may omit canonical order identity metadata.
        return Response.json({ id: order.id, status: 'COMPLETED' });
      }
      assert.equal(request.method, 'GET');
      return Response.json(order);
    }
    if (url.pathname === '/v1/notifications/verify-webhook-signature') {
      this.verificationBodies.push(raw);
      assert.equal(body.webhook_id, 'e2e-webhook');
      if (this.expectedRawEvent !== undefined) assert.ok(raw.endsWith(`\"webhook_event\":${this.expectedRawEvent}}`), 'Webhook verification must preserve original event bytes');
      if (this.verificationUnavailable) return Response.json({ error: 'verification-outage' }, { status: 503 });
      return Response.json({ verification_status: this.verification });
    }
    assert.fail(`Unexpected outbound request: ${request.method} ${url.pathname}`);
  }
}

export async function startService({ escrow, restoredSql, paypal = new PayPalSandbox() } = {}) {
  const project = dirname(dirname(dirname(fileURLToPath(import.meta.url))));
  const directory = await mkdtemp(join(tmpdir(), 'td-supporter-http-e2e-'));
  const privateKey = escrow ? createPrivateKey({ key: JSON.parse(escrow.signing), format: 'jwk' }) : generateKeyPairSync('ed25519').privateKey;
  const signing = { privateKey, publicKey: createPublicKey(privateKey) };
  const signingJwk = signing.privateKey.export({ format: 'jwk' });
  // Retain compatibility with the key-generator label used by existing installs.
  signingJwk.alg = 'Ed25519';
  const env = {
    PAYPAL_ENVIRONMENT: 'sandbox', PAYPAL_CLIENT_ID: 'e2e-client', PAYPAL_CLIENT_SECRET: 'e2e-secret',
    PAYPAL_MERCHANT_ID: 'e2e-merchant', PAYPAL_WEBHOOK_ID: 'e2e-webhook', PUBLIC_ORIGIN: 'https://supporter.example.invalid',
    SUPPORTER_PRICE: '5.00', SUPPORTER_CURRENCY: 'USD', MAX_ACTIVE_DEVICES: '3', TERMS_VERSION: terms,
    ENTITLEMENT_TTL_DAYS: '30', OFFLINE_GRACE_DAYS: '7', ENTITLEMENT_SIGNING_JWK: JSON.stringify(signingJwk),
    RECOVERY_LOOKUP_KEY: escrow?.lookup ?? randomBytes(32).toString('base64url'), RECOVERY_ENCRYPTION_KEY: escrow?.encryption ?? randomBytes(32).toString('base64url'),
  };
  let runtime;
  let origin;
  let db;
  const logs = [];
  const bundle = await build({ entryPoints: [join(project, 'src/index.ts')], bundle: true, write: false, format: 'esm', platform: 'browser', target: 'es2022' });
  async function boot() {
    runtime = new Miniflare(convertV4MiniflareOptions({
      name: 'supporter-http-e2e', modules: true, script: bundle.outputFiles[0].text, compatibilityDate: '2026-08-10',
      host: '127.0.0.1', port: 0, bindings: env, d1Databases: { DB: 'supporter-http-e2e-local' },
      rootPath: directory, resourcePersistencePath: join(directory, 'state'), resourceTmpPath: join(directory, 'runtime'),
      unsafeDevRegistryPath: join(directory, 'registry'), unsafeRegisterWorker: false, unsafeTriggerHandlers: true, cf: false, telemetry: { enabled: false },
      log: new Log(LogLevel.NONE), handleStructuredLogs: entry => logs.push(JSON.stringify(entry)),
      outboundService: request => paypal.fetch(request),
    }));
    origin = await runtime.ready;
    db = await runtime.getD1Database('DB');
  }
  async function sql(query, params = []) {
    const statement = db.prepare(query);
    return (await (params.length ? statement.bind(...params) : statement).all()).results;
  }
  try {
    await boot();
    if (restoredSql !== undefined) {
      const statements = unstable_splitSqlQuery(restoredSql);
      await db.batch(statements.map(statement => db.prepare(statement)));
    } else {
      for (const migration of (await readdir(join(project, 'migrations'))).filter(name => name.endsWith('.sql')).sort()) {
        // Migrations currently contain no triggers or quoted semicolons.
        const statements = (await readFile(join(project, 'migrations', migration), 'utf8')).split(';').map(value => value.trim()).filter(Boolean);
        await db.batch(statements.map(statement => db.prepare(statement)));
      }
    }
  } catch (error) {
    await runtime?.dispose();
    await rm(directory, { recursive: true, force: true });
    throw error;
  }
  return {
    env, paypal, logs, sql,
    async exportSql() {
      // Wrangler's local D1 export operation, against this fixture's own binding.
      const [lines] = await db.prepare('PRAGMA miniflare_d1_export(?,?,?);').bind(false, false).raw();
      return lines.join('\n');
    },
    async request(path, { method = 'GET', body, raw, bearer, headers = {} } = {}) {
      return fetch(new URL(path, origin), {
        method, headers: { ...(body !== undefined || raw !== undefined ? { 'content-type': 'application/json' } : {}), ...(bearer ? { authorization: `Bearer ${bearer}` } : {}), ...headers },
        body: raw ?? (body === undefined ? undefined : JSON.stringify(body)), signal: AbortSignal.timeout(15_000), redirect: 'manual',
      });
    },
    async reset() {
      await db.batch(['activation_challenges', 'entitlement_devices', 'checkout_claims', 'entitlements', 'webhook_events'].map(table => db.prepare(`DELETE FROM ${table}`)));
      paypal.reset();
      logs.length = 0;
    },
    async restart() { await runtime.dispose(); await boot(); },
    async close() { await runtime.dispose(); await rm(directory, { recursive: true, force: true }); },
    verifyToken(token, expectedDevice) {
      const parts = token.split('.');
      assert.equal(parts.length, 3);
      const [header, payload, signature] = parts;
      assert.ok(verify(null, Buffer.from(`${header}.${payload}`), signing.publicKey, Buffer.from(signature, 'base64url')));
      assert.deepEqual(JSON.parse(Buffer.from(header, 'base64url').toString()), { alg: 'EdDSA', typ: 'TD-SUPPORTER', kid: 'v1' });
      const claims = JSON.parse(Buffer.from(payload, 'base64url').toString());
      assert.equal(claims.iss, 'telegram-drive-supporter');
      assert.equal(claims.aud, 'telegram-drive-desktop');
      assert.equal(claims.terms_version, terms);
      assert.equal(claims.device_key_hash, hash(expectedDevice.encoded));
      assert.equal(claims.expires_at - claims.issued_at, 30 * day);
      assert.equal(claims.offline_until - claims.expires_at, 7 * day);
      return claims;
    },
    // Import/compatibility fixtures use an independent Node signer, never a
    // production crypto helper. Every acceptance/rejection still crosses HTTP.
    historicalToken(claims, header = { alg: 'EdDSA', typ: 'TD-SUPPORTER', kid: 'v1' }) {
      const input = `${Buffer.from(JSON.stringify(header)).toString('base64url')}.${Buffer.from(JSON.stringify(claims)).toString('base64url')}`;
      return `${input}.${sign(null, Buffer.from(input), createPrivateKey({ key: signing.privateKey.export({ format: 'jwk' }), format: 'jwk' })).toString('base64url')}`;
    },
  };
}
