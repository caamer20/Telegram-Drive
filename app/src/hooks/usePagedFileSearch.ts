import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import type { TelegramFile } from '../types';

export interface SearchPage<Cursor> {
    files: TelegramFile[];
    next: Cursor | null;
    complete: boolean;
    offline?: boolean;
    total?: number;
}
type SearchView = {
    key: string | null;
    results: TelegramFile[];
    busy: boolean;
    error: boolean;
    more: boolean;
    complete: boolean;
    offline?: boolean;
    total?: number;
};
const idle: SearchView = { key: null, results: [], busy: false, error: false, more: false, complete: false };

export function usePagedFileSearch<Cursor>(
    key: string | null,
    readPage: (cursor: Cursor | null, current: () => boolean) => Promise<SearchPage<Cursor>>,
) {
    const [view, setView] = useState(idle);
    const read = useRef(readPage);
    read.current = readPage;
    const currentKey = useRef(key);
    currentKey.current = key;
    const action = useRef<() => void>(() => {});

    useEffect(() => {
        if (!key) { setView(idle); return; }
        let generation = 0;
        let pending = false;
        let next: Cursor | null = null;
        const rows = new Map<string, TelegramFile>();
        const load = async () => {
            if (pending || currentKey.current !== key) return;
            const ticket = generation;
            const current = () => ticket === generation && currentKey.current === key;
            pending = true;
            setView(prev => ({ ...prev, key, busy: true, error: false }));
            try {
                const page = await read.current(next, current);
                if (!current()) return;
                for (const file of page.files) rows.set(JSON.stringify([file.folder_id ?? null, file.id]), file);
                next = page.next;
                setView({ key, results: [...rows.values()], busy: false, error: false,
                    more: next !== null, complete: page.complete, offline: page.offline, total: page.total });
            } catch (error) {
                if (!current()) return;
                if (String(error).includes('SEARCH_CHANGED') || String(error).includes('ACCOUNT_CHANGED')) {
                    next = null;
                    rows.clear();
                }
                setView(prev => ({ ...prev, results: [...rows.values()], busy: false, error: true, complete: false }));
            } finally {
                if (current()) pending = false;
            }
        };
        const reset = () => {
            generation++;
            pending = false;
            next = null;
            rows.clear();
            setView({ ...idle, key, busy: true });
        };
        reset();
        action.current = () => { void load(); };
        let timer = window.setTimeout(action.current, 350);
        let disposed = false;
        const refresh = () => {
            if (disposed || currentKey.current !== key) return;
            clearTimeout(timer);
            reset();
            timer = window.setTimeout(action.current, 0);
        };
        const listeners = ['vault-locked', 'vault-unlocked', 'file-inventory-changed'].map(event => listen(event, refresh));
        return () => {
            disposed = true;
            generation++;
            clearTimeout(timer);
            action.current = () => {};
            listeners.forEach(value => void value.then(dispose => dispose()).catch(() => undefined));
        };
    }, [key]);
    const visible = key && view.key === key ? view : { ...idle, busy: Boolean(key) };
    return { ...visible, enabled: key !== null, request: () => action.current() };
}
