const fs = require('fs');

// Patch OneDriveMigrationPage.tsx
let page = fs.readFileSync('app/src/components/migration/OneDriveMigrationPage.tsx', 'utf8');
page = page.replace(
    /        itemProgress,/g,
    `        activeProgresses,`
);
page = page.replace(
    /        progress,/g,
    `        activeProgresses,`
);
page = page.replace(
    /activeProgresses=\{itemProgress\}/g,
    `activeProgresses={activeProgresses}`
);
page = page.replace(
    /activeProgresses=\{progress\}/g,
    `activeProgresses={activeProgresses}`
);
fs.writeFileSync('app/src/components/migration/OneDriveMigrationPage.tsx', page);

// Patch useMigration.ts
let useMig = fs.readFileSync('app/src/hooks/useMigration.ts', 'utf8');
useMig = useMig.replace(/import \{ acceptProgressEvent \} from '\.\.\/utils';\n/g, '');
fs.writeFileSync('app/src/hooks/useMigration.ts', useMig);
