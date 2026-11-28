import { useMemo, useState } from 'react';
import {
    ChevronRight, Folder, HardDrive, Search, X, Check, MoveRight, Copy as CopyIcon,
} from 'lucide-react';
import type { TelegramFolder } from '../../../types';

export interface FolderTreePickerProps {
    folders: TelegramFolder[];
    /** Header text, e.g. "Move to folder" / "Copy to folder". */
    title: string;
    /** Footer button text, e.g. "Move here" / "Copy here". */
    actionLabel: string;
    /** Folder that cannot be a target (the folder being moved/copied). */
    sourceFolderId?: number | null;
    /** Extra blocked ids (descendants of the source folder). */
    blockedIds?: number[];
    /** Currently open folder, marked as "you are here". */
    currentFolderId?: number | null;
    /** `move` shows an arrow icon, `copy` shows a copy icon. */
    kind?: 'move' | 'copy';
    onClose: () => void;
    onSelect: (targetFolderId: number | null) => void;
}

interface TreeRow {
    id: number;
    name: string;
    depth: number;
    hasChildren: boolean;
}

const ROOT_KEY = 'root';

/**
 * Windows-Explorer style destination picker: folders are shown as a tree with
 * expandable parents instead of one flat list, so nested folders keep their
 * hierarchy.
 */
export function FolderTreePicker({
    folders, title, actionLabel, sourceFolderId = null, blockedIds = [], currentFolderId = null,
    kind = 'move', onClose, onSelect,
}: FolderTreePickerProps) {
    const [expanded, setExpanded] = useState<Set<string>>(() => new Set([ROOT_KEY]));
    const [query, setQuery] = useState('');
    const [selected, setSelected] = useState<number | null>(currentFolderId ?? null);

    const blocked = useMemo(() => {
        const set = new Set<number>(blockedIds);
        if (sourceFolderId != null) set.add(sourceFolderId);
        return set;
    }, [blockedIds, sourceFolderId]);

    const childrenOf = useMemo(() => {
        const map = new Map<number, TelegramFolder[]>();
        for (const folder of folders) {
            const parent = folder.parent_id ?? null;
            if (parent == null) continue;
            const list = map.get(parent) ?? [];
            list.push(folder);
            map.set(parent, list);
        }
        for (const list of map.values()) {
            list.sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' }));
        }
        return map;
    }, [folders]);

    const roots = useMemo(
        () => folders
            .filter(folder => !folder.parent_id)
            .slice()
            .sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' })),
        [folders],
    );

    const matching = useMemo(() => {
        const term = query.trim().toLowerCase();
        if (!term) return null;
        const hits = new Set<number>();
        const parents = new Map<number | null, number | null>();
        for (const folder of folders) parents.set(folder.id, folder.parent_id ?? null);
        for (const folder of folders) {
            if (!folder.name.toLowerCase().includes(term)) continue;
            hits.add(folder.id);
            let cursor = parents.get(folder.id) ?? null;
            while (cursor != null && !hits.has(cursor)) {
                hits.add(cursor);
                cursor = parents.get(cursor) ?? null;
            }
        }
        return hits;
    }, [folders, query]);

    const rows = useMemo(() => {
        const output: TreeRow[] = [];
        const walk = (parent: number | null, depth: number) => {
            const list = parent === null ? roots : childrenOf.get(parent) ?? [];
            for (const folder of list) {
                if (blocked.has(folder.id)) continue;
                if (matching && !matching.has(folder.id)) continue;
                const children = (childrenOf.get(folder.id) ?? []).filter(child => !blocked.has(child.id));
                output.push({ id: folder.id, name: folder.name, depth, hasChildren: children.length > 0 });
                const key = String(folder.id);
                const shouldShow = matching ? true : expanded.has(key);
                if (shouldShow && children.length > 0) walk(folder.id, depth + 1);
            }
        };
        walk(null, 0);
        return output;
    }, [roots, childrenOf, blocked, expanded, matching]);

    const toggle = (id: number) => {
        setExpanded(previous => {
            const next = new Set(previous);
            const key = String(id);
            if (next.has(key)) next.delete(key);
            else next.add(key);
            return next;
        });
    };

    const path = useMemo(() => {
        if (selected === null) return ['Saved Messages'];
        const byId = new Map(folders.map(folder => [folder.id, folder]));
        const chain: string[] = [];
        let cursor: number | null = selected;
        const seen = new Set<number>();
        while (cursor != null && !seen.has(cursor)) {
            seen.add(cursor);
            const folder = byId.get(cursor);
            if (!folder) break;
            chain.unshift(folder.name);
            cursor = folder.parent_id ?? null;
        }
        return ['Saved Messages', ...chain];
    }, [folders, selected]);

    const ActionIcon = kind === 'copy' ? CopyIcon : MoveRight;

    return (
        <div className="fixed inset-0 z-[100] flex items-center justify-center bg-app-overlay p-4 backdrop-blur-sm" onClick={onClose}>
            <div
                role="dialog"
                aria-modal="true"
                aria-label={title}
                className="quiet-raised flex max-h-[80vh] w-[min(460px,100%)] flex-col overflow-hidden"
                onClick={event => event.stopPropagation()}
            >
                <header className="flex items-center justify-between border-b border-app-border px-4 py-3">
                    <h3 className="flex items-center gap-2 text-ui font-semibold text-app-text">
                        <ActionIcon className="h-4 w-4 text-app-accent" />
                        {title}
                    </h3>
                    <button type="button" onClick={onClose} aria-label="Close" className="quiet-control p-1 text-app-text-secondary hover:bg-app-hover">
                        <X className="h-4 w-4" />
                    </button>
                </header>

                <div className="border-b border-app-border-subtle px-3 py-2">
                    <div className="relative">
                        <Search className="pointer-events-none absolute start-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-app-text-tertiary" />
                        <input
                            value={query}
                            onChange={event => setQuery(event.target.value)}
                            placeholder="Find a folder…"
                            className="quiet-control h-8 w-full rounded-lg border border-app-border bg-app-surface-sunken/40 ps-8 pe-3 text-ui text-app-text placeholder:text-app-text-tertiary focus:border-app-accent/50 focus:outline-none"
                        />
                    </div>
                </div>

                <div className="min-h-0 flex-1 overflow-y-auto p-2">
                    <button
                        type="button"
                        onClick={() => setSelected(null)}
                        className={`flex h-9 w-full items-center gap-2 rounded-lg px-2 text-start text-ui ${selected === null ? 'bg-app-selected text-app-accent' : 'text-app-text hover:bg-app-hover'}`}
                    >
                        <HardDrive className="h-4 w-4 shrink-0" />
                        <span className="truncate font-medium">Saved Messages</span>
                        {selected === null && <Check className="ms-auto h-4 w-4 shrink-0" />}
                    </button>

                    {rows.map(row => {
                        const isSelected = selected === row.id;
                        const isOpen = expanded.has(String(row.id));
                        return (
                            <div key={row.id} className="flex items-center">
                                <span style={{ width: row.depth * 14 }} className="shrink-0" aria-hidden="true" />
                                <button
                                    type="button"
                                    onClick={() => { if (row.hasChildren) toggle(row.id); }}
                                    disabled={!row.hasChildren}
                                    aria-label={row.hasChildren ? (isOpen ? 'Collapse' : 'Expand') : undefined}
                                    className={`flex h-9 w-5 shrink-0 items-center justify-center rounded text-app-text-tertiary ${row.hasChildren ? 'hover:bg-app-hover' : 'opacity-0'}`}
                                >
                                    <ChevronRight className={`h-3.5 w-3.5 transition-transform ${isOpen ? 'rotate-90' : ''}`} />
                                </button>
                                <button
                                    type="button"
                                    onClick={() => setSelected(row.id)}
                                    className={`flex h-9 min-w-0 flex-1 items-center gap-2 rounded-lg px-2 text-start text-ui ${isSelected ? 'bg-app-selected text-app-accent' : 'text-app-text hover:bg-app-hover'}`}
                                >
                                    <Folder className={`h-4 w-4 shrink-0 ${isSelected ? 'text-app-accent' : 'text-amber-400'}`} />
                                    <span className="truncate">{row.name}</span>
                                    {row.id === currentFolderId && (
                                        <span className="ms-1 shrink-0 rounded bg-app-hover px-1.5 py-0.5 text-badge text-app-text-tertiary">here</span>
                                    )}
                                    {isSelected && <Check className="ms-auto h-4 w-4 shrink-0" />}
                                </button>
                            </div>
                        );
                    })}

                    {rows.length === 0 && (
                        <p className="px-3 py-6 text-center text-metadata text-app-text-secondary">
                            {query ? 'No folder matches that name.' : 'No other folders yet. Create one first.'}
                        </p>
                    )}
                </div>

                <footer className="flex items-center gap-2 border-t border-app-border px-3 py-2.5">
                    <nav aria-label="Destination" className="min-w-0 flex-1 truncate text-badge text-app-text-tertiary" title={path.join(' / ')}>
                        {path.join(' / ')}
                    </nav>
                    <button type="button" onClick={onClose} className="quiet-control h-8 rounded-lg px-3 text-badge font-medium text-app-text-secondary hover:bg-app-hover">
                        Cancel
                    </button>
                    <button
                        type="button"
                        onClick={() => onSelect(selected)}
                        className="quiet-control flex h-8 items-center gap-1.5 rounded-lg bg-app-accent px-3 text-badge font-semibold text-white hover:opacity-90"
                    >
                        <ActionIcon className="h-3.5 w-3.5" />
                        {actionLabel}
                    </button>
                </footer>
            </div>
        </div>
    );
}
