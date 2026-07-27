const fs = require('fs');

// Patch OneDriveMigrationPage.tsx
let page = fs.readFileSync('app/src/components/migration/OneDriveMigrationPage.tsx', 'utf8');
page = page.replace(/progress=\{progress\}/g, 'activeProgresses={activeProgresses}');
fs.writeFileSync('app/src/components/migration/OneDriveMigrationPage.tsx', page);

// Remove acceptProgressEvent from useMigration.ts since it's unused
let useMig = fs.readFileSync('app/src/hooks/useMigration.ts', 'utf8');
useMig = useMig.replace(/import \{ acceptProgressEvent \} from '\.\.\/utils';\n/g, '');
useMig = useMig.replace(/const newProgress = acceptProgressEvent\([\s\S]*?\);/g, `const newProgress = { ...e.payload, timestamp: e.payload.timestamp ?? now };`);
fs.writeFileSync('app/src/hooks/useMigration.ts', useMig);
