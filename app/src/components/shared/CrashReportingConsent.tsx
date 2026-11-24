import { useRef } from 'react';
import { Bug, ShieldCheck } from 'lucide-react';
import { useSettings } from '../../context/SettingsContext';
import { useModalFocus } from '../../hooks/useModalFocus';

export function CrashReportingConsent() {
    const { settings, setCrashReportingConsent: decide } = useSettings();
    const panelRef = useRef<HTMLDivElement>(null);
    useModalFocus(panelRef, () => decide(false), !settings.crashReportingConsentSeen);

    if (settings.crashReportingConsentSeen) return null;
    return (
        <div className="fixed inset-0 z-[300] flex items-center justify-center bg-app-overlay p-4 backdrop-blur-sm" role="presentation">
            <div
                ref={panelRef}
                role="dialog"
                aria-modal="true"
                aria-labelledby="crash-consent-title"
                tabIndex={-1}
                className="quiet-raised w-[min(460px,calc(100vw-2rem))] p-6"
            >
                <div className="mb-4 flex h-11 w-11 items-center justify-center rounded-xl bg-app-accent/10 text-app-accent">
                    <Bug className="h-5 w-5" />
                </div>
                <h2 id="crash-consent-title" className="text-lg font-semibold text-app-text">Help improve crash reliability?</h2>
                <p className="mt-2 text-sm leading-6 text-app-text-secondary">
                    With your permission, Telegram Drive can send a small technical report only when the app crashes.
                </p>
                <div className="mt-4 flex gap-3 rounded-lg border border-app-border-subtle bg-app-surface-sunken/30 p-3 text-xs leading-5 text-app-text-secondary">
                    <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-app-success" />
                    <span>Reports never include file names, file paths, file contents, Telegram messages, credentials, or personal identifiers. You can turn this off at any time in Settings.</span>
                </div>
                <div className="mt-6 flex justify-end gap-3">
                    <button onClick={() => decide(false)} className="quiet-control px-4 py-2.5 text-sm font-medium text-app-text-secondary hover:text-app-text">
                        No thanks
                    </button>
                    <button onClick={() => decide(true)} className="quiet-control bg-app-accent px-4 py-2.5 text-sm font-medium text-app-accent-contrast hover:bg-app-accent-hover">
                        Allow crash reports
                    </button>
                </div>
            </div>
        </div>
    );
}
