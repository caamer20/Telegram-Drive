const fs = require('fs');

let page = fs.readFileSync('app/src/components/migration/OneDriveMigrationPage.tsx', 'utf8');

page = page.replace(
    /        const unlistenProgress = await listen<ItemProgressPayload>\('migration:item-progress', \(event\) => \{[\s\S]*?        \}\);/m,
    `        const unlistenProgress = await listen<ItemProgressPayload>('migration:item-progress', (event) => {
            if (isMounted) {
                setActiveProgresses(prev => ({
                    ...prev,
                    [event.payload.item_id]: { ...event.payload, timestamp: Date.now() }
                }));
            }
        });
        const unlistenComplete = await listen<any>('migration:item-complete', (event) => {
            if (isMounted) {
                setActiveProgresses(prev => {
                    const next = { ...prev };
                    delete next[event.payload.item_id];
                    return next;
                });
            }
        });`
);

page = page.replace(
    /            unlistenProgress\(\);[\s\S]*?            unlistenStats\(\);/m,
    `            unlistenProgress();
            unlistenComplete();
            unlistenStats();`
);

fs.writeFileSync('app/src/components/migration/OneDriveMigrationPage.tsx', page);
