import {useTranslation} from 'react-i18next';
import { lazy, useEffect, useState, type ReactNode } from 'react';
import { Bug, Cloud, Database, Globe, HardDrive, Megaphone, Shield, Zap, Clipboard, Loader2, RefreshCw, Upload, Download } from 'lucide-react';
import { open } from '@tauri-apps/plugin-shell';
import { toast } from 'sonner';
import type { TFunction } from 'i18next';
import { EncryptionSettingsSection } from '../../../shared/EncryptionSettingsSection';
import { ThemesTab } from '../ThemesTab';
import { useConfirm } from '../../../../context/ConfirmContext';
import type { Settings } from '../../../../types/settings';
import {
  downloadSettingsSync,
  getSettingsSyncStatus,
  uploadSettingsSync,
  type SettingsSyncStatus,
} from '../../../../services/settingsSync';
import i18n from '../../../../i18n';
import { LazyFeatureBoundary } from '../../../shared/LazyFeatureBoundary';

const SupporterSettingsSection = lazy(() => import('./SupporterSettingsSection').then(module => ({ default: module.SupporterSettingsSection })));



interface SettingsTabFrameProps {
  children: ReactNode;
}

export function GeneralSettingsTab({ children }: SettingsTabFrameProps) {
  return <div className="w-full space-y-6">{children}</div>;
}

export function ProxySettingsTab({ children }: SettingsTabFrameProps) {
  return <section className="w-full space-y-3">{children}</section>;
}

export function VpnSettingsTab({ children }: SettingsTabFrameProps) {
  return <section className="w-full space-y-3">{children}</section>;
}

export function WebDavSettingsTab({ children }: SettingsTabFrameProps) {
  return <section className="w-full space-y-4">{children}</section>;
}

export function SharingSettingsTab({ children }: SettingsTabFrameProps) {
  return <section className="w-full space-y-4">{children}</section>;
}

interface PrivacySettingsTabProps {
  crashReportingEnabled: boolean;
  onCrashReportingChange: () => void;
  settings: Settings;
  onSettingsChange: (updates: Partial<Settings>) => void;
  onSettingsSyncEnabledChange: (enabled: boolean) => void;
}

function SettingsSyncSection({
  settings,
  onSettingsChange,
  onEnabledChange,
}: {
  settings: Settings;
  onSettingsChange: (updates: Partial<Settings>) => void;
  onEnabledChange: (enabled: boolean) => void;
}) {
  const { t } = useTranslation();
  const { confirm } = useConfirm();
  const [passphrase, setPassphrase] = useState('');
  const [status, setStatus] = useState<SettingsSyncStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [busyAction, setBusyAction] = useState<'status' | 'upload' | 'download' | null>(null);

  const refreshStatus = async () => {
    setBusyAction('status');
    setStatusError(null);
    try {
      setStatus(await getSettingsSyncStatus());
    } catch (error) {
      setStatus(null);
      setStatusError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusyAction(null);
    }
  };

  useEffect(() => {
    if (settings.telegramSettingsSyncEnabled) void refreshStatus();
  }, [settings.telegramSettingsSyncEnabled]);

  const upload = async () => {
    if (status?.available && !status.current_device) {
      const approved = await confirm({
        title: t('settings_sync_copy.replace_title'),
        message: t('settings_sync_copy.replace_description'),
        confirmText: t('settings_sync_copy.replace'),
      });
      if (!approved) return;
    }
    setBusyAction('upload');
    setStatusError(null);
    try {
      setStatus(await uploadSettingsSync(settings, passphrase));
      toast.success(t('settings_sync_copy.uploaded'));
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setStatusError(message);
      toast.error(message);
    } finally {
      setBusyAction(null);
    }
  };

  const download = async () => {
    const approved = await confirm({
      title: t('settings_sync_copy.apply_title'),
      message: t('settings_sync_copy.apply_description'),
      confirmText: t('settings_sync_copy.apply'),
    });
    if (!approved) return;
    setBusyAction('download');
    setStatusError(null);
    try {
      const restored = await downloadSettingsSync(passphrase);
      onSettingsChange(restored.settings);
      setStatus({
        available: true,
        updated_at: restored.updated_at,
        device_id: restored.device_id,
        current_device: status?.device_id === restored.device_id ? status.current_device : false,
      });
      toast.success(t('settings_sync_copy.downloaded'));
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setStatusError(message);
      toast.error(message);
    } finally {
      setBusyAction(null);
    }
  };

  const passphraseIsValid = passphrase.trim().length >= 12;
  const isBusy = busyAction !== null;

  return (
    <section className="rounded-lg border border-app-border-subtle bg-app-surface-sunken/20 p-4" aria-labelledby="settings-sync-title">
      <div className="flex items-start justify-between gap-4">
        <div className="flex min-w-0 gap-3">
          <Cloud className="mt-0.5 h-5 w-5 shrink-0 text-app-accent" aria-hidden="true" />
          <div>
            <h3 id="settings-sync-title" className="text-sm font-semibold text-app-text">{t('settings_sync_copy.title')}</h3>
            <p className="mt-1 text-xs leading-5 text-app-text-secondary">{t('settings_sync_copy.description')}</p>
          </div>
        </div>
        <button type="button" role="switch" aria-checked={settings.telegramSettingsSyncEnabled} aria-label={t('settings_sync_copy.enable')} onClick={() => { setStatusError(null); setPassphrase(''); onEnabledChange(!settings.telegramSettingsSyncEnabled); }} className={`relative h-6 w-11 shrink-0 rounded-full transition-colors ${settings.telegramSettingsSyncEnabled ? 'bg-app-accent' : 'bg-app-border'}`}><span className={`absolute start-0.5 top-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform ${settings.telegramSettingsSyncEnabled ? 'translate-x-5 rtl:-translate-x-5' : ''}`} /></button>
      </div>

      {settings.telegramSettingsSyncEnabled && (
        <div className="mt-4 space-y-3">
          <div className="rounded-lg border border-app-warning/25 bg-app-warning/5 p-3 text-xs leading-5 text-app-text-secondary">
            <strong className="text-app-text">{t('settings_sync_copy.warning')}</strong> {t('settings_sync_copy.exclusions')}
          </div>
          <label className="block">
            <span className="text-xs font-medium text-app-text">{t('settings_sync_copy.passphrase')}</span>
            <input type="password" value={passphrase} onChange={event => setPassphrase(event.target.value)} minLength={12} autoComplete="new-password" spellCheck={false} placeholder={t('settings_sync_copy.placeholder')} className="mt-1.5 w-full rounded-control border border-app-border bg-app-surface px-3 py-2.5 text-sm text-app-text outline-none focus:border-app-accent" />
          </label>
          <div className="flex flex-wrap items-center gap-2">
            <button type="button" disabled={!passphraseIsValid || isBusy} onClick={() => void upload()} className="quiet-control flex items-center gap-2 px-3 py-2 text-xs font-medium text-app-text disabled:cursor-not-allowed disabled:opacity-50">{busyAction === 'upload' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Upload className="h-3.5 w-3.5" />}{t('settings_sync_copy.upload')}</button>
            <button type="button" disabled={!passphraseIsValid || isBusy || status?.available === false} onClick={() => void download()} className="quiet-control flex items-center gap-2 px-3 py-2 text-xs font-medium text-app-text disabled:cursor-not-allowed disabled:opacity-50">{busyAction === 'download' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Download className="h-3.5 w-3.5" />}{t('settings_sync_copy.download')}</button>
            <button type="button" disabled={isBusy} onClick={() => void refreshStatus()} aria-label={t('settings_sync_copy.refresh')} className="quiet-control p-2 text-app-text-secondary disabled:opacity-50">{busyAction === 'status' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}</button>
          </div>
          {status && (
            <p className="text-xs leading-5 text-app-text-secondary">
              {status.available && status.updated_at
                ? t('settings_sync_copy.latest', { date: new Date(status.updated_at * 1_000).toLocaleString(i18n.language), device: t(status.current_device ? 'settings_sync_copy.this_device' : 'settings_sync_copy.other_device') })
                : t('settings_sync_copy.none')}
            </p>
          )}
          {statusError && <p role="alert" className="text-xs leading-5 text-app-danger">{statusError}</p>}
        </div>
      )}
    </section>
  );
}


export function PrivacySettingsTab({ crashReportingEnabled, onCrashReportingChange, settings, onSettingsChange, onSettingsSyncEnabledChange }: PrivacySettingsTabProps) {
  const { t } = useTranslation();
  return (
    <section className="space-y-4">
      <div className="flex items-start gap-3 rounded-lg border border-app-accent/20 bg-app-accent/5 p-4">
        <Bug className="mt-0.5 h-5 w-5 shrink-0 text-app-accent" />
        <div><h3 className="text-sm font-semibold text-app-text">Crash-only reporting</h3><p className="mt-1 text-xs leading-5 text-app-text-secondary">Optional reports help diagnose unexpected app crashes. Normal usage, analytics, advertising activity, and file operations are never reported.</p></div>
      </div>
      <div className="flex items-center justify-between rounded-lg bg-app-hover/50 p-4">
        <div className="max-w-[75%]"><p className="text-sm font-medium text-app-text">Send anonymous crash reports</p><p className="mt-1 text-xs leading-5 text-app-text-secondary">Never sends file names, paths, contents, Telegram messages, credentials, or personal identifiers. Turning this off also clears reports waiting to be sent.</p></div>
        <button type="button" role="switch" aria-checked={crashReportingEnabled} aria-label="Send anonymous crash reports" onClick={onCrashReportingChange} className={`relative h-6 w-11 rounded-full transition-colors ${crashReportingEnabled ? 'bg-app-accent' : 'bg-app-border'}`}><span className={`absolute start-0.5 top-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform ${crashReportingEnabled ? 'translate-x-5 rtl:-translate-x-5' : ''}`} /></button>
      </div>
      <section className="space-y-3" aria-labelledby="data-usage-title">
        <div><h3 id="data-usage-title" className="text-sm font-semibold text-app-text">Where your data goes</h3><p className="mt-1 text-xs leading-5 text-app-text-secondary">Telegram Drive has no account server of its own. Each destination below is separated by purpose.</p></div>
        <div className="grid gap-3 sm:grid-cols-2">
          {([
            [t('privacy_destinations.device_title'), t('privacy_destinations.device_body'), Database],
            ['Telegram', t('privacy_destinations.telegram_body'), Cloud],
            [t('privacy_destinations.sponsors_title'), t('privacy_destinations.sponsors_body'), Megaphone],
            [t('privacy_destinations.crash_title'), t('privacy_destinations.crash_body'), Bug],
          ] as const).map(([title, description, Icon]) => (
            <div key={title} className="rounded-lg border border-app-border-subtle bg-app-surface-sunken/25 p-3"><Icon className="h-4 w-4 text-app-accent" aria-hidden="true" /><strong className="mt-2 block text-xs text-app-text">{title}</strong><p className="mt-1 text-xs leading-5 text-app-text-secondary">{description}</p></div>
          ))}
        </div>
        <p className="rounded-lg border border-app-border-subtle p-3 text-xs leading-5 text-app-text-secondary"><strong className="text-app-text">Privacy policy summary:</strong> Telegram Drive does not sell personal data, inspect file contents for analytics, or operate a cloud account database. Revoking a share or disabling a local server stops that access immediately.</p>
      </section>
      <SettingsSyncSection settings={settings} onSettingsChange={onSettingsChange} onEnabledChange={onSettingsSyncEnabledChange} />
    </section>
  );
}

export function LicenseSettingsTab() {
  return <section><LazyFeatureBoundary><SupporterSettingsSection /></LazyFeatureBoundary></section>;
}

interface AdvancedSettingsTabProps {
  onOpenApi: () => void;
  onOpenWebDav: () => void;
  onOpenProxy: () => void;
  onOpenVpn: () => void;
}

export function AdvancedSettingsTab({ onOpenApi, onOpenWebDav, onOpenProxy, onOpenVpn }: AdvancedSettingsTabProps) {
    useTranslation();
  return (
    <section className="space-y-4">

      <div className="grid gap-3 sm:grid-cols-2">
        {([
          ['REST API', i18n.t('advanced_copy.rest_description'), Globe, onOpenApi],
          ['WebDAV', i18n.t('advanced_copy.webdav_description'), HardDrive, onOpenWebDav],
          [i18n.t('common.proxy'), i18n.t('advanced_copy.proxy_description'), Shield, onOpenProxy],
          [i18n.t('advanced_copy.vpn_title'), i18n.t('advanced_copy.vpn_description'), Zap, onOpenVpn],
        ] as const).map(([label, description, Icon, action]) => (
          <button key={label} type="button" onClick={action} className="quiet-surface p-4 text-start hover:border-app-accent/30 hover:bg-app-hover"><Icon className="mb-3 h-5 w-5 text-app-accent" /><strong className="block text-sm text-app-text">{label}</strong><span className="mt-1 block text-xs leading-5 text-app-text-secondary">{description}</span></button>
        ))}
      </div>
    </section>
  );
}

export function EncryptionSettingsTab() {
  return <section><EncryptionSettingsSection /></section>;
}

export function ThemeSettingsTab() {
  return <ThemesTab />;
}

interface AboutSettingsTabProps {
  appVersion: string;
  diagnosticsLoading: boolean;
  t: TFunction;
  onCopyDiagnostics: () => void;
}

export function AboutSettingsTab({ appVersion, diagnosticsLoading, t, onCopyDiagnostics }: AboutSettingsTabProps) {
  return (
    <section className="w-full space-y-4">
      <div className="flex flex-col items-center space-y-5 py-6">
        <img src="/logo.svg" className="h-16 w-16 drop-shadow-lg" alt={i18n.t('common.app_title')} />
        <div className="text-center"><h3 className="text-base font-bold text-telegram-text">{i18n.t("common.app_title")}</h3><p className="mt-0.5 text-xs text-telegram-subtext">v{appVersion}</p></div>
        <div className="h-px w-12 bg-telegram-border" />
        <button onClick={onCopyDiagnostics} disabled={diagnosticsLoading} className="flex items-center gap-1.5 rounded-lg border border-telegram-border bg-telegram-hover px-3 py-1.5 text-xs font-medium text-telegram-subtext transition hover:bg-telegram-border/30 hover:text-telegram-text disabled:opacity-50">
          {diagnosticsLoading ? <Loader2 className="h-3 w-3 animate-spin" /> : <Clipboard className="h-3 w-3" />}
          {t('settings.copy_diagnostics')}
        </button>
        <div className="space-y-3 text-center">
          <p className="text-sm font-semibold text-telegram-text">Cameron Amer</p>
          <button onClick={event => { event.preventDefault(); void open('https://www.cameronamer.com'); }} className="flex cursor-pointer items-center justify-center gap-1.5 text-xs text-telegram-primary transition-colors hover:text-telegram-primary/80"><Globe className="h-3.5 w-3.5" />www.cameronamer.com</button>
          <button onClick={event => { event.preventDefault(); void open('https://github.com/caamer20/telegram-drive'); }} className="flex cursor-pointer items-center justify-center gap-1.5 text-xs text-telegram-primary transition-colors hover:text-telegram-primary/80">
            <svg className="h-3.5 w-3.5" viewBox="0 0 24 24" fill="currentColor"><path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z" /></svg>
            github.com/caamer20/telegram-drive
          </button>
        </div>
        <p className="max-w-[280px] text-center text-[11px] leading-relaxed text-telegram-subtext">{t('settings.tagline')}</p>
      </div>
    </section>
  );
}
