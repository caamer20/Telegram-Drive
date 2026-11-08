import { AlertTriangle, CheckCircle2, Clock3, CloudOff, RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useSync } from '../../../context/SyncContext';

export function SyncStatusBadge({ collapsed = false }: { collapsed?: boolean }) {
  const { t } = useTranslation();
  const { status } = useSync();
  const value = status.data;
  const waiting = value?.pairs.some(pair => pair.phase === 'waiting');
  const waitingForTelegram = value?.pairs.some(pair => pair.phase === 'waiting' && pair.waitReason === 'telegram');
  const state = !value?.enabled || value.activePairs === 0 ? 'disabled'
    : value.conflicts > 0 || value.lastError ? 'warning'
      : value.running ? 'syncing' : waiting ? 'waiting' : 'synced';
  const details = {
    disabled: { label: t('sync.status.disabled'), icon: CloudOff, className: 'text-app-text-tertiary' },
    warning: { label: t('sync.status.conflicts'), icon: AlertTriangle, className: 'text-app-warning' },
    syncing: { label: t('sync.status.syncing'), icon: RefreshCw, className: 'text-app-accent' },
    waiting: { label: t(waitingForTelegram ? 'syncPreview.waiting_telegram' : 'syncPreview.waiting'), icon: Clock3, className: 'text-app-text-secondary' },
    synced: { label: t('sync.status.synced'), icon: CheckCircle2, className: 'text-app-success' },
  }[state];
  const Icon = details.icon;
  return (
    <button type="button" onClick={() => window.dispatchEvent(new CustomEvent('telegram-drive-open-settings', { detail: { tab: 'sync' } }))} className={`quiet-control flex h-8 w-full items-center ${collapsed ? 'justify-center' : 'gap-2 px-2'} text-xs ${details.className}`} title={details.label}>
      <Icon className={`h-4 w-4 ${state === 'syncing' ? 'animate-spin' : ''}`} />
      {!collapsed && <span>{details.label}</span>}
    </button>
  );
}
