import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';

import en from './locales/en.json';
import es from './locales/es.json';
import ru from './locales/ru.json';
import zhCN from './locales/zh-CN.json';
import fr from './locales/fr.json';
import it from './locales/it.json';
import ar from './locales/ar.json';
import ptBR from './locales/pt-BR.json';
import de from './locales/de.json';
import hi from './locales/hi.json';
import id from './locales/id.json';
import tr from './locales/tr.json';
import ja from './locales/ja.json';
import ko from './locales/ko.json';

i18n
  .use(initReactI18next)
  .init({
    resources: {
      en: { translation: en },
      es: { translation: es },
      ru: { translation: ru },
      'zh-CN': { translation: zhCN },
      fr: { translation: fr },
      it: { translation: it },
      ar: { translation: ar },
      'pt-BR': { translation: ptBR },
      de: { translation: de },
      hi: { translation: hi },
      id: { translation: id },
      tr: { translation: tr },
      ja: { translation: ja },
      ko: { translation: ko },
    },
    lng: 'en',
    fallbackLng: 'en',
    interpolation: {
      escapeValue: false, // React already safeguards from XSS
    },
    react: {
      useSuspense: false,
    },
  });

export default i18n;
