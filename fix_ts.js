const fs = require('fs');

// Patch useMigration.ts
let useMig = fs.readFileSync('app/src/hooks/useMigration.ts', 'utf8');

// The initial replacements I made were partial because they were not global.
useMig = useMig.replace(/setItemProgress/g, 'setActiveProgresses');

// I also need to ensure `setActiveProgresses(null)` in `reset` is fixed because `setActiveProgresses({})` is the correct one.
useMig = useMig.replace(/setActiveProgresses\(null\)/g, 'setActiveProgresses({})');

// In `ItemCompletePayload` listener, it had `setItemProgress(previous => ...)` which I already replaced?
// Wait, I replaced it but maybe it failed? Let's rewrite it.
useMig = useMig.replace(
    /        listen<ItemCompletePayload>\('migration:item-complete', \(e\) => \{[\s\S]*?            setActiveProgresses\(previous =>[\s\S]*?                previous\?\.item_id === e\.payload\.item_id \? null : previous[\s\S]*?            \);[\s\S]*?        \}\)\.then\(retainUnlistener\);/m,
    `        listen<ItemCompletePayload>('migration:item-complete', (e) => {
            void queryClient.invalidateQueries({ queryKey: ['bandwidth'] });
            if (e.payload.status === 'completed_telegram' || e.payload.status === 'completed_local') {
                void queryClient.invalidateQueries({ queryKey: ['files'] });
            }
            setActiveProgresses(prev => {
                const next = { ...prev };
                delete next[e.payload.item_id];
                return next;
            });
        }).then(retainUnlistener);`
);

// In `ItemProgressPayload` listener:
useMig = useMig.replace(
    /                setActiveProgresses\(previous => \{[\s\S]*?                    return acceptProgressEvent\(previous, \{[\s\S]*?                        \.\.\.e\.payload,[\s\S]*?                        timestamp: Date\.now\(\)[\s\S]*?                    \}\);[\s\S]*?                \}\);/m,
    `                setActiveProgresses(prev => {
                    const newProgress = acceptProgressEvent(prev[e.payload.item_id] || null, {
                        ...e.payload,
                        timestamp: Date.now()
                    });
                    return { ...prev, [e.payload.item_id]: newProgress! };
                });`
);

fs.writeFileSync('app/src/hooks/useMigration.ts', useMig);

// Patch OneDriveMigrationPage.tsx
let page = fs.readFileSync('app/src/components/migration/OneDriveMigrationPage.tsx', 'utf8');
page = page.replace(/progress=\{itemProgress\}/g, 'activeProgresses={activeProgresses}');
fs.writeFileSync('app/src/components/migration/OneDriveMigrationPage.tsx', page);

