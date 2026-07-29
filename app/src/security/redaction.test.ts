import { describe, expect, it } from 'vitest';
import { redactSensitiveText, redactSensitiveValue } from './redaction';

describe('sensitive stream redaction', () => {
  it('redacts query tokens in any position, fragments, case variants, and encoded values', () => {
    const value = redactSensitiveText(
      'https://x.test/a?foo=1&TOKEN=s%2Fe%3Fc#token=fragment-secret',
    );
    expect(value).not.toContain('s%2Fe%3Fc');
    expect(value).not.toContain('fragment-secret');
    expect(value.match(/\[REDACTED\]/g)).toHaveLength(2);
  });

  it('redacts bearer and quoted authorization representations', () => {
    const value = redactSensitiveText(
      `Authorization: Bearer first-secret {"authorization":"second-secret"} 'AUTHORIZATION'='Bearer third-secret'`,
    );
    expect(value).not.toContain('first-secret');
    expect(value).not.toContain('second-secret');
    expect(value).not.toContain('third-secret');
  });

  it('redacts nested objects and arrays while handling circular values', () => {
    const circular: Record<string, unknown> = {
      playlistUrl: 'http://x.test/hls?token=url-secret',
      authorization: 'header-secret',
      tokenCount: 3,
      items: [{ streamToken: 'nested-secret' }],
    };
    circular.self = circular;
    expect(redactSensitiveValue(circular)).toEqual({
      playlistUrl: 'http://x.test/hls?token=[REDACTED]',
      authorization: '[REDACTED]',
      tokenCount: 3,
      items: [{ streamToken: '[REDACTED]' }],
      self: '[Circular]',
    });
  });

  it('removes complete private loopback stream URLs from diagnostics', () => {
    const redacted = redactSensitiveText(
      'failed http://127.0.0.1:49152/stream/home/7?token=secret',
    );
    expect(redacted).toContain('[REDACTED_STREAM_URL]');
    expect(redacted).not.toContain('/stream/home/7');
    expect(redacted).not.toContain('secret');
  });
});
