const fs = require('fs');

// Patch useMigration.ts
let useMig = fs.readFileSync('app/src/hooks/useMigration.ts', 'utf8');

useMig = useMig.replace(
    /const \[itemProgress, setItemProgress\] = useState<ItemProgressPayload \| null>\(null\);/m,
    `const [activeProgresses, setActiveProgresses] = useState<Record<number, ItemProgressPayload>>({});`
);

useMig = useMig.replace(
    /    const reset = useCallback\(\(\) => \{[\s\S]*?        setItemProgress\(null\);/m,
    `    const reset = useCallback(() => {
        setJobs([]);
        setCurrentJobDetail(null);
        setActiveProgresses({});`
);

useMig = useMig.replace(
    /        listen<ItemProgressPayload>\('migration:item-progress', \(e\) => \{[\s\S]*?            if \(now - lastProgressTime > 200 \|\| e\.payload\.percent === 100\) \{[\s\S]*?                lastProgressTime = now;[\s\S]*?                setItemProgress\(previous => \{[\s\S]*?                    return acceptProgressEvent\(previous, \{[\s\S]*?                        \.\.\.e\.payload,[\s\S]*?                        timestamp: Date\.now\(\)[\s\S]*?                    \}\);[\s\S]*?                \}\);[\s\S]*?            \}[\s\S]*?        \}\)\.then\(retainUnlistener\);/m,
    `        listen<ItemProgressPayload>('migration:item-progress', (e) => {
            const now = Date.now();
            if (now - lastProgressTime > 200 || e.payload.percent === 100) {
                lastProgressTime = now;
                setActiveProgresses(prev => {
                    const newProgress = acceptProgressEvent(prev[e.payload.item_id] || null, {
                        ...e.payload,
                        timestamp: Date.now()
                    });
                    return { ...prev, [e.payload.item_id]: newProgress! };
                });
            }
        }).then(retainUnlistener);`
);

useMig = useMig.replace(
    /        listen<ItemCompletePayload>\('migration:item-complete', \(e\) => \{[\s\S]*?            setItemProgress\(previous =>[\s\S]*?                previous\?\.item_id === e\.payload\.item_id \? null : previous[\s\S]*?            \);[\s\S]*?        \}\)\.then\(retainUnlistener\);/m,
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

useMig = useMig.replace(
    /        itemProgress,/m,
    `        activeProgresses,`
);

fs.writeFileSync('app/src/hooks/useMigration.ts', useMig);

// Patch ProgressPanel.tsx
let progPanel = fs.readFileSync('app/src/components/migration/ProgressPanel.tsx', 'utf8');

progPanel = progPanel.replace(
    /    progress: ItemProgressPayload \| null;/m,
    `    activeProgresses: Record<number, ItemProgressPayload>;`
);

progPanel = progPanel.replace(
    /export const ProgressPanel: React\.FC<ProgressPanelProps> = \(\{[\s\S]*?    progress,[\s\S]*?\}\) => \{/m,
    `export const ProgressPanel: React.FC<ProgressPanelProps> = ({
    detail,
    activeProgresses,
    cooldown,
    onStart,
    onStop,
    onRetryAllFailed,
}) => {`
);

progPanel = progPanel.replace(
    /            \{\/\* Current Item Progress \*\/\}[\s\S]*?            \{progress && \([\s\S]*?                <div className="p-4 bg-blue-950\/20 border border-blue-900\/40 rounded-xl space-y-2">[\s\S]*?                <\/div>\n            \)\}/m,
    `            {/* Current Active Progresses */}
            {Object.values(activeProgresses).length > 0 && (
                <div className="space-y-3">
                    {Object.values(activeProgresses).map(progress => (
                        <div key={progress.item_id} className="p-4 bg-blue-950/20 border border-blue-900/40 rounded-xl space-y-2">
                            <div className="flex justify-between items-center text-xs">
                                <span className="font-semibold text-blue-300 truncate max-w-[70%]" title={progress.item_name}>
                                    {progress.phase === 'downloading' ? t('migration.phase_downloading', 'Downloading') :
                                     progress.phase === 'processing' ? t('migration.phase_processing', 'Processing') :
                                     t('migration.phase_uploading', 'Uploading')}: {progress.item_name}
                                </span>
                                <span className="text-blue-400 font-mono font-bold">{progress.percent}%</span>
                            </div>
                            <div className="w-full h-2 bg-slate-950 rounded-full overflow-hidden">
                                <div
                                    className="h-full bg-blue-500 transition-all duration-200"
                                    style={{ width: \`\${progress.percent}%\` }}
                                />
                            </div>
                            <div className="flex justify-between text-[11px] text-slate-400 font-mono">
                                <span>{formatBytes(progress.bytes_done)} / {formatBytes(progress.bytes_total)}</span>
                            </div>
                        </div>
                    ))}
                </div>
            )}`
);

fs.writeFileSync('app/src/components/migration/ProgressPanel.tsx', progPanel);

// Patch JobDetailsPage (or wherever ProgressPanel is used)
// We need to find where ProgressPanel is used. Let's find it.
