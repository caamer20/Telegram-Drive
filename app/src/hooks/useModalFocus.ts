import { RefObject, useLayoutEffect, useRef } from 'react';

const FOCUSABLE = [
    'button:not([disabled])',
    '[href]',
    'input:not([disabled])',
    'select:not([disabled])',
    'textarea:not([disabled])',
    'summary',
    '[tabindex]:not([tabindex="-1"])',
].join(',');

interface ModalEntry {
    container: HTMLElement;
    returnFocus: HTMLElement | null;
    parent?: ModalEntry;
}
const modalStack: ModalEntry[] = [];
const pendingReturns: ModalEntry[] = [];
const topModal = () => modalStack[modalStack.length - 1];
const available = (element: HTMLElement) => element.isConnected
    && element.getClientRects().length > 0
    && !element.closest('[inert], [hidden], [aria-hidden="true"]');
const controls = (container: HTMLElement) => Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(available);

/**
 * Gives desktop dialogs predictable keyboard behaviour: initial focus,
 * focus trapping, Escape-to-close, and restoration to the invoking control.
 */
export function useModalFocus(
    containerRef: RefObject<HTMLElement | null>,
    onClose: () => void,
    enabled = true,
    suspended = false,
) {
    const onCloseRef = useRef(onClose);
    onCloseRef.current = onClose;
    const suspendedRef = useRef(suspended);
    suspendedRef.current = suspended;
    // A parent dialog may temporarily pause while a child owns focus. Keep
    // its original trigger across that handoff, rather than capturing body.
    const session = useRef<ModalEntry | null>(null);
    useLayoutEffect(() => {
        if (!enabled) return;
        const container = containerRef.current;
        if (!container) return;
        if (session.current?.container !== container) {
            const active = document.activeElement as HTMLElement | null;
            const replaced = pendingReturns[pendingReturns.length - 1];
            session.current = {
                container,
                returnFocus: active && active !== document.body && available(active) ? active : replaced?.returnFocus ?? active,
                parent: topModal() ?? replaced?.parent,
            };
        }
        const entry = session.current;
        modalStack.push(entry);
        const focusable = controls(container);
        const preferred = container.querySelector<HTMLElement>('[data-modal-autofocus]');
        if (!suspendedRef.current) (preferred && available(preferred) ? preferred : focusable[0] ?? container).focus({ preventScroll: true });

        const handleKeyDown = (event: KeyboardEvent) => {
            if (topModal() !== entry || suspendedRef.current) return;
            if (event.key === 'Escape') {
                event.preventDefault();
                event.stopImmediatePropagation();
                onCloseRef.current();
                return;
            }
            if (event.key !== 'Tab') return;

            const current = controls(container);
            if (current.length === 0) {
                event.preventDefault();
                container.focus();
                return;
            }
            const first = current[0];
            const last = current[current.length - 1];
            if (!container.contains(document.activeElement)) {
                event.preventDefault();
                (event.shiftKey ? last : first).focus();
            } else if (event.shiftKey && document.activeElement === first) {
                event.preventDefault();
                last.focus();
            } else if (!event.shiftKey && document.activeElement === last) {
                event.preventDefault();
                first.focus();
            }
        };

        document.addEventListener('keydown', handleKeyDown, true);
        return () => {
            document.removeEventListener('keydown', handleKeyDown, true);
            const index = modalStack.lastIndexOf(entry);
            if (index !== -1) modalStack.splice(index, 1);
            pendingReturns.push(entry);
            // React removes/activates related dialogs in the same commit.
            // Let the new top owner register before deciding where to return.
            queueMicrotask(() => {
                const pendingIndex = pendingReturns.indexOf(entry);
                if (pendingIndex !== -1) pendingReturns.splice(pendingIndex, 1);
                let owner: ModalEntry | undefined = entry;
                let target = entry.returnFocus;
                while (target && !available(target) && owner?.parent) {
                    owner = owner.parent;
                    target = owner.returnFocus;
                }
                const top = topModal();
                if (top && (!target || !top.container.contains(target))) return;
                if (target && available(target)) target.focus({ preventScroll: true });
                if (!container.isConnected || !top) session.current = null;
            });
        };
    }, [containerRef, enabled]);
}
