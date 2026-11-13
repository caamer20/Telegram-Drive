import { Component, createContext, lazy, type ReactNode, Suspense, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { useModalFocus } from '../hooks/useModalFocus';
import { useSettings } from './SettingsContext';
import { useEncryption } from '../hooks/useEncryption';
import type { UploadProtectionIntent } from '../types';
import type { UploadProtectionPreference } from '../types/settings';
import i18n from '../i18n';

export type UploadChoice = 'store' | 'protect';
export type UploadProtectionSelection = Readonly<Pick<UploadProtectionIntent, 'mode' | 'protectMetadata'>>;
interface UploadScope {
    signal: AbortSignal;
    isCurrent: () => boolean;
}
interface UploadRequest extends UploadScope {
    count: number;
    id: number;
    resolve: (selection: UploadProtectionSelection | null) => void;
    snapshot?: { preference: UploadProtectionPreference; protection: UploadProtectionSelection };
}
interface UploadChoiceContextValue {
    chooseUploadProtection: (count: number, scope: UploadScope) => Promise<UploadProtectionSelection | null>;
}
const UploadChoiceContext = createContext<UploadChoiceContextValue | null>(null);
const Dialog = lazy(() => import('../components/shared/UploadProtectionDialog'));

class UploadDialogBoundary extends Component<{ children: ReactNode; fallback: ReactNode }, { failed: boolean }> {
    state = { failed: false };
    static getDerivedStateFromError() { return { failed: true }; }
    render() { return this.state.failed ? this.props.fallback : this.props.children; }
}

function PendingUploadDialog({ failed = false, retry }: { failed?: boolean; retry: () => void }) {
    return <div>
        <p id="upload-choice-title" role={failed ? 'alert' : 'status'} className="text-sm text-app-text">{i18n.t(failed ? 'upload_choice_copy.load_failed' : 'common.loading')}</p>
        {failed && <button type="button" onClick={retry} className="quiet-control mt-4 min-h-11 px-4 text-sm text-app-accent">{i18n.t('upload_choice_copy.reload_app')}</button>}
    </div>;
}

export function UploadChoiceProvider({ children }: { children: ReactNode }) {
    const { settings, isLoaded } = useSettings();
    const encryption = useEncryption();
    const environmentRef = useRef({ settings, isLoaded, encryption });
    environmentRef.current = { settings, isLoaded, encryption };
    const [request, setRequest] = useState<UploadRequest | null>(null);
    const requestRef = useRef(request);
    const nextId = useRef(0);
    const panelRef = useRef<HTMLDivElement>(null);
    const finish = useCallback((selection: UploadProtectionSelection | null, expectedId?: number) => {
        if (expectedId !== undefined && requestRef.current?.id !== expectedId) return;
        const pending = requestRef.current;
        pending?.resolve(pending.isCurrent() && !pending.signal.aborted ? selection : null);
        requestRef.current = null;
        setRequest(null);
    }, []);
    useModalFocus(panelRef, () => finish(null), !!request);
    useEffect(() => () => { requestRef.current?.resolve(null); requestRef.current = null; }, []);
    const prepare = useCallback((pending: UploadRequest) => {
        if (requestRef.current?.id !== pending.id) return;
        if (!pending.isCurrent() || pending.signal.aborted) { finish(null, pending.id); return; }
        const environment = environmentRef.current;
        if (!environment.isLoaded) { setRequest(pending); return; }
        // A failed initial read leaves the default "ask" preference, requiring an
        // explicit choice. Save/proxy-migration failures do not erase known settings.
        const { settings, encryption } = environment;
        const preference = settings.uploadProtectionPreference;
        const protection: UploadProtectionSelection = {
            mode: settings.encryptionDefaultMode === 'standard' ? 'vault' : settings.encryptionDefaultMode,
            protectMetadata: settings.encryptionProtectMetadata,
        };
        if (preference === 'standard') {
            finish({ ...protection, mode: 'standard' }, pending.id);
            return;
        }
        if (preference === 'encrypted' && encryption.capabilityState === 'ready'
            && encryption.capabilities?.upload_enabled === true
            && (protection.mode === 'passphrase' || encryption.vaultStatus?.is_unlocked)) {
            finish(protection, pending.id);
            return;
        }
        const prepared = { ...pending, snapshot: { preference, protection } };
        requestRef.current = prepared;
        setRequest(prepared);
    }, [finish]);
    useEffect(() => {
        const pending = requestRef.current;
        if (isLoaded && pending && !pending.snapshot) prepare(pending);
    }, [isLoaded, prepare]);
    const chooseUploadProtection = useCallback((count: number, scope: UploadScope) => new Promise<UploadProtectionSelection | null>((resolve) => {
        // Supersede even when the new request can complete without a dialog.
        finish(null);
        if (scope.signal.aborted || !scope.isCurrent()) { resolve(null); return; }
        const id = ++nextId.current;
        const abort = () => finish(null, id);
        const nextRequest: UploadRequest = { count, id, ...scope, resolve: selection => {
            scope.signal.removeEventListener('abort', abort);
            resolve(selection);
        } };
        requestRef.current = nextRequest;
        scope.signal.addEventListener('abort', abort, { once: true });
        prepare(nextRequest);
    }), [finish, prepare]);
    const reloading = useRef(false);
    const retry = (requestId: number) => {
        if (reloading.current || requestRef.current?.id !== requestId) return;
        reloading.current = true;
        finish(null, requestId);
        window.location.reload();
    };
    return <UploadChoiceContext.Provider value={{ chooseUploadProtection }}>
        {children}
        {request && <div className="fixed inset-0 z-[250] flex items-center justify-center bg-app-overlay p-4 backdrop-blur-sm">
            <div ref={panelRef} role="dialog" aria-modal="true" aria-labelledby="upload-choice-title" tabIndex={-1} className="quiet-raised max-h-[calc(100dvh-2rem)] w-[min(520px,calc(100vw-2rem))] overflow-y-auto p-5">
                <UploadDialogBoundary key={request.id} fallback={<PendingUploadDialog failed retry={() => retry(request.id)} />}>
                    <Suspense fallback={<PendingUploadDialog retry={() => retry(request.id)} />}>
                        {request.snapshot ? <Dialog
                            count={request.count}
                            protectionMode={request.snapshot.protection.mode}
                            protectOnly={request.snapshot.preference === 'encrypted'}
                            isCurrent={() => requestRef.current?.id === request.id && request.isCurrent() && !request.signal.aborted}
                            onFinish={choice => finish(choice === null ? null : {
                                ...request.snapshot!.protection,
                                mode: choice === 'store' ? 'standard' : request.snapshot!.protection.mode,
                            }, request.id)}
                        /> : <PendingUploadDialog retry={() => retry(request.id)} />}
                    </Suspense>
                </UploadDialogBoundary>
                <div className="mt-4 flex justify-end"><button type="button" onClick={() => finish(null, request.id)} className="quiet-control min-h-11 px-4 text-sm text-app-text-secondary">{i18n.t('common.cancel')}</button></div>
            </div>
        </div>}

    </UploadChoiceContext.Provider>;
}

export function useUploadChoice() {
    const value = useContext(UploadChoiceContext);
    if (!value) throw new Error('useUploadChoice must be used within UploadChoiceProvider');
    return value;
}
