const fs = require('fs');
const path = require('path');
const zlib = require('zlib');
const crypto = require('crypto');

const appRoot = path.resolve(__dirname, '..');
const distRoot = path.join(appRoot, 'dist');
const manifestPath = path.join(distRoot, '.vite', 'manifest.json');
const budgetPath = path.join(appRoot, 'bundle-budget.json');
const reportPath = path.join(distRoot, 'bundle-report.json');

function fail(message) {
  console.error(`[bundle] ${message}`);
  process.exitCode = 1;
}

if (!fs.existsSync(manifestPath)) {
  throw new Error(`Missing ${manifestPath}. Run the production build before bundle:check.`);
}

const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
const budget = JSON.parse(fs.readFileSync(budgetPath, 'utf8'));
// These surfaces must retain their reviewed ceilings. Removing a configured
// budget must not silently turn off enforcement for a shipping route.
const requiredRoutes = ['src/components/desktop/DesktopDashboard.tsx'];
const requiredFeatures = [
  'src/components/desktop/dashboard/SettingsModal.tsx',
  'src/components/desktop/dashboard/MediaPlayer.tsx',
  'src/components/desktop/dashboard/PdfViewer.tsx',
];
for (const [kind, references, configured] of [
  ['route', requiredRoutes, budget.routeJavaScriptBudgets],
  ['feature', requiredFeatures, budget.featureChunkBudgets],
]) {
  for (const reference of references) {
    const ceiling = configured?.[reference];
    if (!Number.isSafeInteger(ceiling) || ceiling <= 0) {
      throw new Error(`Missing or invalid required ${kind} budget: ${reference}`);
    }
  }
}

const assetFiles = fs.readdirSync(path.join(distRoot, 'assets'))
  .filter((file) => /\.(js|css|json)$/.test(file))
  .sort();

const assets = assetFiles.map((file) => {
  const body = fs.readFileSync(path.join(distRoot, 'assets', file));
  return {
    file: `assets/${file}`,
    type: file.endsWith('.js') ? 'javascript' : file.endsWith('.css') ? 'css' : 'locale-data',
    bytes: body.length,
    gzipBytes: zlib.gzipSync(body, { level: 9 }).length,
    brotliBytes: zlib.brotliCompressSync(body).length,
  };
});

const manifestByFile = new Map(Object.values(manifest).map((entry) => [entry.file, entry]));
const entryFiles = Object.values(manifest).filter((entry) => entry.isEntry).map((entry) => entry.file);
const initialFiles = new Set();

function collectInitial(file) {
  if (initialFiles.has(file)) return;
  initialFiles.add(file);
  const entry = manifestByFile.get(file);
  for (const imported of entry?.imports || []) {
    const importedFile = manifest[imported]?.file;
    if (importedFile) collectInitial(importedFile);
  }
}

for (const file of entryFiles) collectInitial(file);

const javascript = assets.filter((asset) => asset.type === 'javascript');
const css = assets.filter((asset) => asset.type === 'css');
const localeData = assets.filter((asset) => asset.type === 'locale-data');
const summary = {
  initialJavaScriptBytes: javascript.filter((asset) => initialFiles.has(asset.file)).reduce((sum, asset) => sum + asset.bytes, 0),
  maxJavaScriptChunkBytes: Math.max(0, ...javascript.map((asset) => asset.bytes)),
  totalJavaScriptBytes: javascript.reduce((sum, asset) => sum + asset.bytes, 0),
  totalCssBytes: css.reduce((sum, asset) => sum + asset.bytes, 0),
  totalLocaleDataBytes: localeData.reduce((sum, asset) => sum + asset.bytes, 0),
  maxLocaleDataBytes: Math.max(0, ...localeData.map((asset) => asset.bytes)),
};

function resolveManifestKey(reference) {
  if (manifest[reference]) return reference;
  const expectedName = path.basename(reference, path.extname(reference));
  const matches = Object.entries(manifest)
    .filter(([, entry]) => entry.src === reference || entry.name === expectedName)
    .map(([key]) => key);
  if (matches.length !== 1) {
    throw new Error(`Bundle budget reference ${reference} resolved to ${matches.length} manifest entries.`);
  }
  return matches[0];
}

function manifestJavaScriptBytes(manifestReference, excludeFiles = new Set()) {
  const manifestKey = resolveManifestKey(manifestReference);
  const files = new Set();
  const visitedKeys = new Set();
  function collect(key) {
    if (visitedKeys.has(key)) return;
    visitedKeys.add(key);
    const entry = manifest[key];
    if (!entry) throw new Error(`Manifest entry ${manifestReference} imports missing key: ${key}`);
    if (entry.file?.endsWith('.js') && !excludeFiles.has(entry.file)) files.add(entry.file);
    for (const imported of entry.imports || []) collect(imported);
  }
  collect(manifestKey);
  return {
    bytes: javascript
      .filter((asset) => files.has(asset.file))
      .reduce((sum, asset) => sum + asset.bytes, 0),
    files: [...files].sort(),
  };
}

const routeJavaScript = Object.fromEntries(
  Object.keys(budget.routeJavaScriptBudgets || {}).map((key) => [
    key,
    manifestJavaScriptBytes(key, initialFiles),
  ]),
);
const featureChunks = Object.fromEntries(
  Object.keys(budget.featureChunkBudgets || {}).map((key) => {
    const manifestKey = resolveManifestKey(key);
    const asset = javascript.find((candidate) => candidate.file === manifest[manifestKey].file);
    if (!asset) throw new Error(`Bundle budget entry ${key} did not produce a JavaScript chunk.`);
    return [key, { file: asset.file, bytes: asset.bytes }];
  }),
);

const report = {
  schemaVersion: 1,
  summary,
  budgets: budget,
  initialFiles: [...initialFiles].sort(),
  routeJavaScript,
  featureChunks,
  assets,
};
fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);

const checks = [
  ['initial JavaScript', summary.initialJavaScriptBytes, budget.maxInitialJavaScriptBytes],
  ['largest JavaScript chunk', summary.maxJavaScriptChunkBytes, budget.maxJavaScriptChunkBytes],
  ['total JavaScript', summary.totalJavaScriptBytes, budget.maxTotalJavaScriptBytes],
  ['total CSS', summary.totalCssBytes, budget.maxTotalCssBytes],
  ['total locale data', summary.totalLocaleDataBytes, budget.maxTotalLocaleDataBytes],
  ['largest locale catalog', summary.maxLocaleDataBytes, budget.maxLocaleDataBytes],
];

const expectedCatalogs = fs.readdirSync(path.join(appRoot, 'src/i18n/locales'))
  .filter(file => file.endsWith('.json'));
const dictionaries = localeData.filter(asset => /^assets\/translation-keys-[^/]+\.json$/.test(asset.file));
const catalogs = localeData.filter(asset => !dictionaries.includes(asset));
const packedCatalogs = catalogs.filter(asset => JSON.parse(fs.readFileSync(path.join(distRoot, asset.file), 'utf8')).format === 'td-locale-v2');
if (catalogs.length !== expectedCatalogs.length) {
  fail(`Expected ${expectedCatalogs.length} locally bundled language catalogs, found ${catalogs.length}.`);
}
if (expectedCatalogs.length && (packedCatalogs.length !== expectedCatalogs.length || dictionaries.length !== 1)) {
  fail('Packed language catalogs require exactly one counted shared key table.');
}
if (!packedCatalogs.length && dictionaries.length) fail('A language key table has no packed catalogs.');
if (packedCatalogs.length && dictionaries.length === 1) {
  const table = JSON.parse(fs.readFileSync(path.join(distRoot, dictionaries[0].file), 'utf8'));
  // Reconstruct v3 key positions independently of the browser decoder. Older
  // release artifacts and assurance fixtures still use the flat v2 key list.
  const grouped = table.format === 'td-keys-v3' && Array.isArray(table.groups)
    && table.groups.every(group => Array.isArray(group) && group.length > 1 && group.every(part => typeof part === 'string'));
  const keys = grouped
    ? table.groups.flatMap(([prefix, ...names]) => names.map(name => prefix ? `${prefix}.${name}` : name))
    : table.format === 'td-keys-v2' ? table.keys : undefined;
  const validTable = Array.isArray(keys)
    && keys.every(key => typeof key === 'string' && key && key.split('.').every(part => part && !['__proto__', 'prototype', 'constructor'].includes(part)))
    && Array.isArray(table.shared) && table.shared.every(value => typeof value === 'string')
    && new Set(keys).size === keys.length
    && table.id === crypto.createHash('sha256').update(JSON.stringify([keys, table.shared])).digest('hex');
  if (!validTable) fail('Invalid packed language key table.');
  else {
    function flatten(value, prefix = '', result = {}) {
      for (const [name, entry] of Object.entries(value)) {
        const key = prefix ? `${prefix}.${name}` : name;
        if (typeof entry === 'string') result[key] = entry;
        else flatten(entry, key, result);
      }
      return result;
    }
    for (const filename of expectedCatalogs) {
      const matching = packedCatalogs.filter(asset => path.basename(asset.file).startsWith(filename.slice(0, -5) + '-'));
      if (matching.length !== 1) { fail(`Packed canonical language ${filename} resolved to ${matching.length} assets.`); continue; }
      const packet = JSON.parse(fs.readFileSync(path.join(distRoot, matching[0].file), 'utf8'));
      if (packet.language !== filename.slice(0, -5) || packet.id !== table.id || !Array.isArray(packet.values) || packet.values.length !== keys.length || packet.values.some(value => value !== null && typeof value !== 'string' && !(Number.isSafeInteger(value) && value >= 0 && value < table.shared.length))) {
        fail(`Packed canonical language ${filename} has an invalid shape.`); continue;
      }
      const source = flatten(JSON.parse(fs.readFileSync(path.join(appRoot, 'src/i18n/locales', filename), 'utf8')));
      const actual = Object.fromEntries(keys.flatMap((key, index) => packet.values[index] === null ? [] : [[key, typeof packet.values[index] === 'number' ? table.shared[packet.values[index]] : packet.values[index]]]));
      if (Object.keys(source).length !== Object.keys(actual).length || Object.entries(source).some(([key, value]) => actual[key] !== value)) {
        fail(`Packed canonical language ${filename} differs from its source.`);
      }
    }
  }
}
// Dictionary bytes remain in totalLocaleDataBytes and maxLocaleDataBytes. The
// decoder remains in normal initial/total JavaScript accounting.



for (const [label, actual, maximum] of checks) {
  const status = actual <= maximum ? 'PASS' : 'FAIL';
  console.log(`[${status}] ${label}: ${actual} / ${maximum} bytes`);
  if (actual > maximum) fail(`${label} exceeds its reviewed budget.`);
}

for (const [key, maximum] of Object.entries(budget.routeJavaScriptBudgets || {})) {
  const actual = routeJavaScript[key].bytes;
  const status = actual <= maximum ? 'PASS' : 'FAIL';
  console.log(`[${status}] route JavaScript ${key}: ${actual} / ${maximum} bytes`);
  if (actual > maximum) fail(`${key} route JavaScript exceeds its reviewed budget.`);
}

for (const [key, maximum] of Object.entries(budget.featureChunkBudgets || {})) {
  const actual = featureChunks[key].bytes;
  const status = actual <= maximum ? 'PASS' : 'FAIL';
  console.log(`[${status}] feature chunk ${key}: ${actual} / ${maximum} bytes`);
  if (actual > maximum) fail(`${key} feature chunk exceeds its reviewed budget.`);
}

console.log(`[bundle] Wrote ${path.relative(appRoot, reportPath)}`);
