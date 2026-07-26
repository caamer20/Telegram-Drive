export type SupportedLanguage = 'en' | 'es' | 'ru' | 'zh-CN' | 'fr' | 'ar' | 'pt-BR' | 'de' | 'hi' | 'id' | 'tr' | 'ja' | 'ko' | 'it';

export interface LanguageInfo {
    code: SupportedLanguage;
    nativeLabel: string;
    englishLabel: string;
    dir: 'ltr' | 'rtl';
}

export const LANGUAGES: LanguageInfo[] = [
    { code: 'en', nativeLabel: 'English', englishLabel: 'English', dir: 'ltr' },
    { code: 'es', nativeLabel: 'Español', englishLabel: 'Spanish', dir: 'ltr' },
    { code: 'ru', nativeLabel: 'Русский', englishLabel: 'Russian', dir: 'ltr' },
    { code: 'zh-CN', nativeLabel: '简体中文', englishLabel: 'Chinese (Simplified)', dir: 'ltr' },
    { code: 'fr', nativeLabel: 'Français', englishLabel: 'French', dir: 'ltr' },
    { code: 'ar', nativeLabel: 'العربية', englishLabel: 'Arabic', dir: 'rtl' },
    { code: 'pt-BR', nativeLabel: 'Português (Brasil)', englishLabel: 'Portuguese (Brazil)', dir: 'ltr' },
    { code: 'de', nativeLabel: 'Deutsch', englishLabel: 'German', dir: 'ltr' },
    { code: 'hi', nativeLabel: 'हिन्दी', englishLabel: 'Hindi', dir: 'ltr' },
    { code: 'id', nativeLabel: 'Bahasa Indonesia', englishLabel: 'Indonesian', dir: 'ltr' },
    { code: 'tr', nativeLabel: 'Türkçe', englishLabel: 'Turkish', dir: 'ltr' },
    { code: 'ja', nativeLabel: '日本語', englishLabel: 'Japanese', dir: 'ltr' },
    { code: 'ko', nativeLabel: '한국어', englishLabel: 'Korean', dir: 'ltr' },
    { code: 'it', nativeLabel: 'Italiano', englishLabel: 'Italian', dir: 'ltr' },
];
