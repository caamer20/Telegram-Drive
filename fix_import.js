const fs = require('fs');

// Patch useMigration.ts
let useMig = fs.readFileSync('app/src/hooks/useMigration.ts', 'utf8');
useMig = useMig.replace(
    /import \{ acceptProgressEvent, mergeActivity \} from '\.\.\/components\/migration\/transferState';/g,
    `import { mergeActivity } from '../components/migration/transferState';`
);
fs.writeFileSync('app/src/hooks/useMigration.ts', useMig);
