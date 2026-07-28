export type SupportedLanguage = 'en' | 'es' | 'ru' | 'zh-CN' | 'fr' | 'it' | 'ar' | 'pt-BR' | 'de' | 'hi' | 'id' | 'tr' | 'ja' | 'ko';

export type LanguagePreference = 'system' | SupportedLanguage;

export interface LanguageInfo {
  code: SupportedLanguage;
  nativeLabel: string;
  englishLabel: string;
  dir: 'ltr' | 'rtl';
  numberLocale: string;
  dateLocale: string;
  aliases: string[];
  fontFamily?: string;
}

export const LANGUAGES: LanguageInfo[] = [
  { code: 'en', nativeLabel: 'English', englishLabel: 'English', dir: 'ltr', numberLocale: 'en-US', dateLocale: 'en-US', aliases: ['en'] },
  { code: 'es', nativeLabel: 'Español', englishLabel: 'Spanish', dir: 'ltr', numberLocale: 'es-ES', dateLocale: 'es-ES', aliases: ['es'] },
  { code: 'ru', nativeLabel: 'Русский', englishLabel: 'Russian', dir: 'ltr', numberLocale: 'ru-RU', dateLocale: 'ru-RU', aliases: ['ru'] },
  { code: 'zh-CN', nativeLabel: '简体中文', englishLabel: 'Chinese (Simplified)', dir: 'ltr', numberLocale: 'zh-CN', dateLocale: 'zh-CN', aliases: ['zh', 'zh-CN', 'zh-SG', 'zh-Hans'] },
  { code: 'fr', nativeLabel: 'Français', englishLabel: 'French', dir: 'ltr', numberLocale: 'fr-FR', dateLocale: 'fr-FR', aliases: ['fr'] },
  { code: 'it', nativeLabel: 'Italiano', englishLabel: 'Italian', dir: 'ltr', numberLocale: 'it-IT', dateLocale: 'it-IT', aliases: ['it'] },
  { code: 'ar', nativeLabel: 'العربية', englishLabel: 'Arabic', dir: 'rtl', numberLocale: 'ar', dateLocale: 'ar', aliases: ['ar'] },
  { code: 'pt-BR', nativeLabel: 'Português (Brasil)', englishLabel: 'Portuguese (Brazil)', dir: 'ltr', numberLocale: 'pt-BR', dateLocale: 'pt-BR', aliases: ['pt', 'pt-BR'] },
  { code: 'de', nativeLabel: 'Deutsch', englishLabel: 'German', dir: 'ltr', numberLocale: 'de-DE', dateLocale: 'de-DE', aliases: ['de'] },
  { code: 'hi', nativeLabel: 'हिन्दी', englishLabel: 'Hindi', dir: 'ltr', numberLocale: 'hi-IN', dateLocale: 'hi-IN', aliases: ['hi'] },
  { code: 'id', nativeLabel: 'Bahasa Indonesia', englishLabel: 'Indonesian', dir: 'ltr', numberLocale: 'id-ID', dateLocale: 'id-ID', aliases: ['id', 'in'] },
  { code: 'tr', nativeLabel: 'Türkçe', englishLabel: 'Turkish', dir: 'ltr', numberLocale: 'tr-TR', dateLocale: 'tr-TR', aliases: ['tr'] },
  { code: 'ja', nativeLabel: '日本語', englishLabel: 'Japanese', dir: 'ltr', numberLocale: 'ja-JP', dateLocale: 'ja-JP', aliases: ['ja'] },
  { code: 'ko', nativeLabel: '한국어', englishLabel: 'Korean', dir: 'ltr', numberLocale: 'ko-KR', dateLocale: 'ko-KR', aliases: ['ko'] },
];

export function getLanguageInfo(code: string): LanguageInfo {
  const normalized = code ? code.trim() : 'en';
  const found = LANGUAGES.find(l => l.code === normalized || l.aliases.includes(normalized) || l.aliases.some(a => normalized.startsWith(a + '-')));
  return found || LANGUAGES[0];
}
