const fs = require('fs');
const path = require('path');
const { SUPPORTED_LOCALES, loadLocale, loadInvariantAllowlist } = require('./shared.cjs');

const ALLOWED_PLURAL_SUFFIXES = {
  ar: ['_zero', '_one', '_two', '_few', '_many', '_other'],
  ru: ['_one', '_few', '_many', '_other'],
  en: ['_one', '_other'],
  es: ['_one', '_other'],
  fr: ['_one', '_other'],
  it: ['_one', '_other'],
  pt_BR: ['_one', '_other'],
  de: ['_one', '_other'],
  hi: ['_one', '_other'],
  id: ['_other'],
  tr: ['_one', '_other'],
  ja: ['_other'],
  ko: ['_other'],
  'zh-CN': ['_other']
};

function extractVariables(text) {
  if (typeof text !== 'string') return [];
  const matches = text.match(/\{\{([^}]+)\}\}/g);
  return matches ? matches.map(m => m.trim()).sort() : [];
}

function validate() {
  const errors = [];
  const warnings = [];
  const invariantAllowlist = loadInvariantAllowlist();
  const allowKeys = new Set(invariantAllowlist.keys || []);
  const allowTokens = new Set(invariantAllowlist.tokens || []);

  let en;
  try {
    en = loadLocale('en');
  } catch (err) {
    console.error(`[en] [json_invalid] Failed to load en.json: ${err.message}`);
    process.exit(1);
  }

  const enKeys = Object.keys(en.flat);

  for (const locale of SUPPORTED_LOCALES) {
    if (locale === 'en') continue;

    let target;
    try {
      target = loadLocale(locale);
    } catch (err) {
      errors.push(`[${locale}] [json_invalid] ${err.message}`);
      continue;
    }

    const targetKeys = new Set(Object.keys(target.flat));
    const validSuffixes = ALLOWED_PLURAL_SUFFIXES[locale] || ALLOWED_PLURAL_SUFFIXES[locale.replace('-', '_')] || ['_one', '_other'];

    // Check missing / unexpected keys
    for (const key of enKeys) {
      const enVal = en.flat[key];
      const targetVal = target.flat[key];

      // Handle plural suffix matching
      const basePluralMatch = key.match(/^(.*)_(one|other)$/);
      if (basePluralMatch) {
        const baseKey = basePluralMatch[1];
        const isOneKey = key.endsWith('_one');
        const isOtherKey = key.endsWith('_other');

        // Check required forms for this locale
        for (const suffix of validSuffixes) {
          const pluralKey = `${baseKey}${suffix}`;
          if (!targetKeys.has(pluralKey) && (suffix === '_one' || suffix === '_other' || suffix === '_two')) {
            // If it's missing, record unless it's a non-required plural variant
            if (suffix === '_two' && locale === 'ar') {
              warnings.push(`[${locale}] [plural_missing] ${pluralKey} — Arabic requires _two form`);
            }
          }
        }
      }

      if (targetVal === undefined) {
        // If not found, check if it has plural variants
        if (!basePluralMatch) {
          errors.push(`[${locale}] [key_missing] ${key} — missing from locale resource`);
        }
      } else {
        // 3. Type check & 4. empty value
        if (typeof targetVal !== 'string') {
          errors.push(`[${locale}] [type_mismatch] ${key} — expected string, got ${typeof targetVal}`);
        } else if (targetVal.trim() === '') {
          errors.push(`[${locale}] [empty_value] ${key} — translation is empty or whitespace`);
        } else if (targetVal.includes('__TODO_TRANSLATE__')) {
          errors.push(`[${locale}] [draft_marker] ${key} — contains unverified draft marker`);
        }

        // 7. Interpolation variables
        const enVars = extractVariables(enVal);
        const targetVars = extractVariables(targetVal);

        // Section 6.2: Approved locale-specific plural forms may omit {{count}} when number is not spoken naturally
        const isPluralKey = /_(zero|one|two|few|many|other)$/.test(key);
        const enVarsFiltered = isPluralKey ? enVars.filter(v => v !== '{{count}}') : enVars;
        const targetVarsFiltered = isPluralKey ? targetVars.filter(v => v !== '{{count}}') : targetVars;

        if (JSON.stringify(enVarsFiltered) !== JSON.stringify(targetVarsFiltered)) {
          errors.push(`[${locale}] [variable_mismatch] ${key} — expected variables [${enVars.join(', ')}], found [${targetVars.join(', ')}]`);
        }

        // 10. Copied English check
        if (enVal === targetVal && typeof enVal === 'string' && enVal.trim().length > 0) {
          if (!allowKeys.has(key) && !allowTokens.has(enVal)) {
            warnings.push(`[${locale}] [copied_english] ${key} — value "${enVal}" is identical to English`);
          }
        }
      }
    }

    // Check extra keys
    for (const key of targetKeys) {
      if (!en.flat[key]) {
        // Check if extra key is valid plural form suffix like _two in Arabic
        const basePluralMatch = key.match(/^(.*)(_zero|_two|_few|_many)$/);
        if (basePluralMatch) {
          const baseKey = basePluralMatch[1];
          if (en.flat[`${baseKey}_one`] || en.flat[`${baseKey}_other`]) {
            continue; // valid plural form
          }
        }
        errors.push(`[${locale}] [unexpected_key] ${key} — not found in English base resource`);
      }
    }
  }

  if (warnings.length > 0) {
    console.log(`--- i18n Validation Warnings (${warnings.length}) ---`);
    for (const w of warnings) {
      console.log(w);
    }
  }

  if (errors.length > 0) {
    console.error(`\n=== i18n Validation Errors (${errors.length}) ===`);
    for (const err of errors) {
      console.error(err);
    }
    process.exit(1);
  }

  console.log('[PASS] All locale files passed structure, variables, and type validation.');
}

validate();
