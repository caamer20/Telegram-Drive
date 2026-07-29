import { describe, expect, it } from 'vitest';
import { redactSensitiveText, redactSensitiveValue } from './redaction';

describe('sensitive stream redaction', () => {
  it('redacts query and bearer credentials', () => {
    const value = redactSensitiveText('http://127.0.0.1/x?token=secret Authorization: Bearer secret');
    expect(value).not.toContain('secret');
    expect(value.match(/\[REDACTED\]/g)).toHaveLength(2);
  });

  it('redacts nested diagnostic fields', () => {
    expect(redactSensitiveValue({ playlistUrl: 'http://x?token=secret', authorization: 'secret' }))
      .toEqual({ playlistUrl: 'http://x?token=[REDACTED]', authorization: '[REDACTED]' });
  });
});
