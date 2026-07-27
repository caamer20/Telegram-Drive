const fs = require('fs');

let page = fs.readFileSync('app/src/components/migration/OneDriveMigrationPage.tsx', 'utf8');

page = page.replace(
    /    const \[progress, setProgress\] = useState<ItemProgressPayload \| null>\(null\);/m,
    `    const [activeProgresses, setActiveProgresses] = useState<Record<number, ItemProgressPayload>>({});`
);

page = page.replace(
    /                setProgress\(event\.payload\);/m,
    `                setActiveProgresses(prev => ({
                    ...prev,
                    [event.payload.item_id]: { ...event.payload, timestamp: Date.now() }
                }));`
);

page = page.replace(
    /activeProgresses=\{activeProgresses\}/g,
    `activeProgresses={activeProgresses}`
);

fs.writeFileSync('app/src/components/migration/OneDriveMigrationPage.tsx', page);
