import { useEffect, useCallback } from 'react';

interface UseKeyboardShortcutsProps {
    onSelectAll: () => void;
    onDelete: () => void;
    onEscape: () => void;
    onSearch: () => void;
    onEnter?: () => void;
    onDownload?: () => void;
    onShare?: () => void;
    onRename?: () => void;
    onShowShortcuts?: () => void;
    enabled?: boolean;
}

export function useKeyboardShortcuts({
    onSelectAll,
    onDelete,
    onEscape,
    onSearch,
    onEnter,
    onDownload,
    onShare,
    onRename,
    onShowShortcuts,
    enabled = true
}: UseKeyboardShortcutsProps) {

    const handleKeyDown = useCallback((e: KeyboardEvent) => {
        if (!enabled) return;

        // Don't trigger shortcuts when typing in inputs
        const target = e.target as HTMLElement;
        if (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable) {
            // Only allow Escape in inputs
            if (e.key === 'Escape') {
                (target as HTMLInputElement).blur();
                onEscape();
            }
            return;
        }

        const isMod = e.metaKey || e.ctrlKey;

        if (e.key === '?' || (e.key === '/' && e.shiftKey)) {
            e.preventDefault();
            onShowShortcuts?.();
            return;
        }

        // Cmd/Ctrl + A - Select All
        if (isMod && e.key === 'a') {
            e.preventDefault();
            onSelectAll();
            return;
        }

        // Cmd/Ctrl + F - Focus Search
        if (isMod && e.key === 'f') {
            e.preventDefault();
            onSearch();
            return;
        }

        // Delete / Backspace - Delete selected
        if (e.key === 'Delete' || e.key === 'Backspace') {
            e.preventDefault();
            onDelete();
            return;
        }

        // Escape - Clear selection
        if (e.key === 'Escape') {
            e.preventDefault();
            onEscape();
            return;
        }
        // Enter - Open / Preview
        if (e.key === 'Enter') {
            if (target.closest('button, a[href], select, summary, [role="menuitem"]')) return;
            e.preventDefault();
            onEnter?.();
            return;
        }

        // F2 - Rename selected file
        if (e.key === 'F2') {
            e.preventDefault();
            onRename?.();
            return;
        }

        // Ctrl/Cmd + D - Download selected
        if (isMod && e.key.toLowerCase() === 'd') {
            e.preventDefault();
            onDownload?.();
            return;
        }

        // Ctrl/Cmd + Shift + S - Share selected
        if (isMod && e.shiftKey && e.key.toLowerCase() === 's') {
            e.preventDefault();
            onShare?.();
            return;
        }
    }, [enabled, onSelectAll, onDelete, onEscape, onSearch, onEnter, onDownload, onShare, onRename, onShowShortcuts]);

    useEffect(() => {
        window.addEventListener('keydown', handleKeyDown);
        return () => window.removeEventListener('keydown', handleKeyDown);
    }, [handleKeyDown]);
}
