const TOKEN_QUERY = /([?&]token=)[^&#\s"']*/gi;
const BEARER = /(bearer\s+)[A-Za-z0-9._~+\/-]+/gi;

export function redactSensitiveText(value: string): string {
  return value
    .replace(TOKEN_QUERY, '$1[REDACTED]')
    .replace(BEARER, '$1[REDACTED]');
}
export function redactSensitiveValue(value: unknown, seen = new WeakSet<object>()): unknown {
  if (typeof value === 'string') return redactSensitiveText(value);
  if (value === null || typeof value !== 'object') return value;
  if (seen.has(value)) return '[Circular]';
  seen.add(value);
  if (Array.isArray(value)) return value.map(item => redactSensitiveValue(item, seen));
  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>).map(([key, item]) => [
      /token|authorization/i.test(key) ? key : key,
      /token|authorization/i.test(key) ? '[REDACTED]' : redactSensitiveValue(item, seen),
    ]),
  );
}
