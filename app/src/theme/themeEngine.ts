// ── Theme Engine ────────────────────────────────────────────────────
// Core types and runtime utilities for the custom theme system.

export interface ThemeColorPalette {
  bg: string;
  surface: string;
  primary: string;
  secondary: string;
  text: string;
  subtext: string;
  border: string;
  hover: string;
}

export interface CustomTheme {
  id: string;
  name: string;
  isDark: boolean;
  palette: ThemeColorPalette;
  isBuiltin?: boolean;
}

const STYLE_ID = 'dynamic-theme';

function parseHex(color: string): [number, number, number] | null {
  const value = color.trim().match(/^#([0-9a-f]{6})$/i)?.[1];
  if (!value) return null;
  return [0, 2, 4].map(offset => parseInt(value.slice(offset, offset + 2), 16)) as [number, number, number];
}

function relativeLuminance(color: string): number | null {
  const rgb = parseHex(color);
  if (!rgb) return null;
  const linear = rgb.map(value => {
    const channel = value / 255;
    return channel <= 0.03928 ? channel / 12.92 : Math.pow((channel + 0.055) / 1.055, 2.4);
  });
  return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2];
}

export function contrastRatio(foreground: string, background: string): number {
  const foregroundLuminance = relativeLuminance(foreground);
  const backgroundLuminance = relativeLuminance(background);
  if (foregroundLuminance === null || backgroundLuminance === null) return 1;
  const lighter = Math.max(foregroundLuminance, backgroundLuminance);
  const darker = Math.min(foregroundLuminance, backgroundLuminance);
  return (lighter + 0.05) / (darker + 0.05);
}

function accessibleText(preferred: string, backgrounds: string[], minimum = 4.5): string {
  if (backgrounds.every(background => contrastRatio(preferred, background) >= minimum)) return preferred;
  const candidates = ['#ffffff', '#101114'];
  return candidates.sort((left, right) => (
    Math.min(...backgrounds.map(background => contrastRatio(right, background)))
      - Math.min(...backgrounds.map(background => contrastRatio(left, background)))
  ))[0];
}

export function ensureAccessiblePalette(palette: ThemeColorPalette): ThemeColorPalette {
  const text = accessibleText(palette.text, [palette.bg, palette.surface]);
  const subtext = accessibleText(palette.subtext, [palette.bg, palette.surface]);
  // Translucent legacy borders describe quiet surface separators, not text or
  // focus indicators. Preserve those overlays rather than treating an rgba
  // value as failed hex parsing and turning every separator solid white.
  const border = !parseHex(palette.border) || contrastRatio(palette.border, palette.surface) >= 3
    ? palette.border
    : accessibleText(palette.border, [palette.surface], 3);
  return { ...palette, text, subtext, border };
}

function contrastText(color: string): string {
  // Preserve the existing foreground for CSS colors outside the hex parser.
  if (!parseHex(color)) return '#ffffff';
  const dark = contrastRatio('#07141b', color);
  const light = contrastRatio('#ffffff', color);
  // Mid-tone accents can fall in the gap between off-black and white.
  // Pure black closes that gap without changing the user's saved accent.
  if (Math.max(dark, light) < 4.5) return '#000000';
  return dark > light ? '#07141b' : '#ffffff';
}

/**
 * Inject a `<style>` block that overrides the @theme CSS variables,
 * and toggle the .dark/.light class on <html>.
 */
export function applyTheme(theme: CustomTheme): void {
  const root = document.documentElement;
  root.classList.add('custom-theme');

  // Toggle dark/light class
  if (theme.isDark) {
    root.classList.add('dark');
    root.classList.remove('light');
  } else {
    root.classList.add('light');
    root.classList.remove('dark');
  }

  // Build CSS variable overrides
  const p = ensureAccessiblePalette(theme.palette);
  root.dataset.themeContrastAdjusted = p.text !== theme.palette.text || p.subtext !== theme.palette.subtext || p.border !== theme.palette.border ? 'true' : 'false';
  const accentContrast = contrastText(p.primary);
  const css = `:root.custom-theme {
  --color-app-canvas: ${p.bg};
  --color-app-sidebar: color-mix(in srgb, ${p.surface} 90%, ${p.bg});
  --color-app-surface: ${p.surface};
  --color-app-surface-raised: color-mix(in srgb, ${p.surface} 94%, ${p.text});
  --color-app-surface-sunken: color-mix(in srgb, ${p.bg} 88%, #000000);
  --color-app-accent: ${p.primary};
  --color-app-accent-hover: color-mix(in srgb, ${p.primary} 86%, ${p.text});
  --color-app-accent-soft: color-mix(in srgb, ${p.primary} 15%, transparent);
  --color-app-accent-contrast: ${accentContrast};
  --color-app-text: ${p.text};
  --color-app-text-secondary: ${p.subtext};
  --color-app-text-tertiary: color-mix(in srgb, ${p.subtext} 72%, transparent);
  --color-app-border-subtle: color-mix(in srgb, ${p.border} 62%, transparent);
  --color-app-border: ${p.border};
  --color-app-border-strong: color-mix(in srgb, ${p.border} 72%, ${p.text});
  --color-app-hover: ${p.hover};
  --color-app-selected: color-mix(in srgb, ${p.primary} 12%, transparent);
  --color-app-overlay: rgba(6, 7, 10, ${theme.isDark ? '0.66' : '0.42'});
  --color-telegram-bg: ${p.bg};
  --color-telegram-surface: ${p.surface};
  --color-telegram-primary: ${p.primary};
  --color-telegram-secondary: ${p.secondary};
  --color-telegram-text: ${p.text};
  --color-telegram-subtext: ${p.subtext};
  --color-telegram-border: ${p.border};
  --color-telegram-hover: ${p.hover};
  --color-telegram-glass-bg: ${theme.isDark ? p.surface : '#ffffff'};
  --color-telegram-glass-border: ${theme.isDark ? '#ffffff' : '#000000'};
}`;

  // Replace or create the style element
  let el = document.getElementById(STYLE_ID) as HTMLStyleElement | null;
  if (!el) {
    el = document.createElement('style');
    el.id = STYLE_ID;
    document.head.appendChild(el);
  }
  el.textContent = css;
}

/**
 * Remove the injected style block so the base @theme values take effect.
 */
export function removeCustomTheme(): void {
  document.documentElement.classList.remove('custom-theme');
  delete document.documentElement.dataset.themeContrastAdjusted;
  const el = document.getElementById(STYLE_ID);
  if (el) el.remove();
}

/** Generate a unique ID for user-created themes. */
export function generateThemeId(): string {
  return `custom-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}
