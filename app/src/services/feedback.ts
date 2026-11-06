export type HapticTone = 'selection' | 'success' | 'warning' | 'error';

export function triggerHaptic(tone: HapticTone = 'selection') {
  if (typeof navigator === 'undefined' || typeof navigator.vibrate !== 'function') return;
  const pattern: Record<HapticTone, number | number[]> = {
    selection: 8,
    success: [10, 35, 14],
    warning: [18, 40, 18],
    error: [24, 35, 24, 35, 24],
  };
  try {
    navigator.vibrate(pattern[tone]);
  } catch {
    // Optional device feedback must never prevent the requested action.
  }
}
