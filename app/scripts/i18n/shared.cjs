const fs = require('fs');
const path = require('path');

const LOCALES_DIR = path.join(__dirname, '../../src/i18n/locales');
const INVARIANT_ALLOWLIST_PATH = path.join(__dirname, '../../src/i18n/invariant-allowlist.json');

const SUPPORTED_LOCALES = [
  'en', 'es', 'ru', 'zh-CN', 'fr', 'it', 'ar', 'pt-BR', 'de', 'hi', 'id', 'tr', 'ja', 'ko'
];

function flattenObject(obj, prefix = '') {
  let result = {};
  for (const [key, value] of Object.entries(obj)) {
    const fullKey = prefix ? `${prefix}.${key}` : key;
    if (typeof value === 'object' && value !== null && !Array.isArray(value)) {
      Object.assign(result, flattenObject(value, fullKey));
    } else {
      result[fullKey] = value;
    }
  }
  return result;
}

function loadLocale(locale) {
  const filePath = path.join(LOCALES_DIR, `${locale}.json`);
  if (!fs.existsSync(filePath)) {
    throw new Error(`Locale file not found: ${filePath}`);
  }
  const content = fs.readFileSync(filePath, 'utf8');
  return { raw: JSON.parse(content), flat: flattenObject(JSON.parse(content)) };
}

function loadInvariantAllowlist() {
  if (!fs.existsSync(INVARIANT_ALLOWLIST_PATH)) {
    return { keys: [], tokens: [] };
  }
  return JSON.parse(fs.readFileSync(INVARIANT_ALLOWLIST_PATH, 'utf8'));
}

module.exports = {
  LOCALES_DIR,
  SUPPORTED_LOCALES,
  flattenObject,
  loadLocale,
  loadInvariantAllowlist
};
