const TOKEN_QUERY = /([?&#]token\s*=\s*)[^&#\s"'<>]*/gi;
const BEARER = /(\bbearer\s+)[^\s"',}\]]+/gi;
const AUTHORIZATION_VALUE = /(["']?\bauthorization\b["']?\s*[:=]\s*["']?)(?:bearer\s+)?[^"',}\]\r\n]+/gi;
const LOOPBACK_STREAM_URL = /https?:\/\/(?:localhost|127\.0\.0\.1):[1-9][0-9]{0,4}\/(?:stream|hls|fmp4)\/[^\s"'<>]*/gi;

function isSensitiveKey(key: string): boolean {
  const normalized = key.replace(/[-_\s]/g, '').toLowerCase();
  return new Set([
    'token',
    'streamtoken',
    'querytoken',
    'accesstoken',
    'refreshtoken',
    'apitoken',
    'authorization',
    'authorizationheader',
    'authorizationtoken',
  ]).has(normalized);
}

export function redactSensitiveText(value: string): string {
  return value
    .replace(AUTHORIZATION_VALUE, '$1[REDACTED]')
    .replace(TOKEN_QUERY, '$1[REDACTED]')
    .replace(BEARER, '$1[REDACTED]')
    .replace(LOOPBACK_STREAM_URL, '[REDACTED_STREAM_URL]');
}

export function redactSensitiveValue(value: unknown, seen = new WeakSet<object>()): unknown {
  if (typeof value === 'string') return redactSensitiveText(value);
  if (value === null || typeof value !== 'object') return value;
  if (seen.has(value)) return '[Circular]';
  seen.add(value);
  if (Array.isArray(value)) return value.map(item => redactSensitiveValue(item, seen));
  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>).map(([key, item]) => [
      key,
      isSensitiveKey(key) ? '[REDACTED]' : redactSensitiveValue(item, seen),
    ]),
  );
}
