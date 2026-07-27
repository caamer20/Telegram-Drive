const fs = require('fs');

let page = fs.readFileSync('app/src/components/migration/OneDriveMigrationPage.tsx', 'utf8');

page = page.replace(
    /        itemProgress,/m,
    `        activeProgresses,`
);

page = page.replace(
    /                                    progress=\{itemProgress\}/m,
    `                                    activeProgresses={activeProgresses}`
);

fs.writeFileSync('app/src/components/migration/OneDriveMigrationPage.tsx', page);
