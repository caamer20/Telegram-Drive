import { describe, expect, it } from 'vitest';

import { consumeWhatsNew } from '../../src/services/updateReliability';

describe('consumeWhatsNew', () => {
  it('does not throw when localStorage is unavailable', () => {
    const originalLocalStorage = window.localStorage;
    const unavailableStorage = {
      getItem: () => {
        throw new Error('storage unavailable');
      },
      setItem: () => {
        throw new Error('storage unavailable');
      },
      removeItem: () => {
        throw new Error('storage unavailable');
      },
      clear: () => {},
    };

    Object.defineProperty(window, 'localStorage', {
      configurable: true,
      value: unavailableStorage,
    });

    try {
      expect(() => consumeWhatsNew('2.3.0')).not.toThrow();
      expect(consumeWhatsNew('2.3.0')).toBeNull();
    } finally {
      Object.defineProperty(window, 'localStorage', {
        configurable: true,
        value: originalLocalStorage,
      });
    }
  });
});
