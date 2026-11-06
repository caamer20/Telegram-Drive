import { useEffect, useState } from 'react';
import { RetryFailedUploadsButton } from '../../shared/RetryFailedUploadsButton';
import { CheckCircle2, ChevronDown, ChevronUp, Download, Pause, Play, UploadCloud, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { DownloadItem, QueueItem, TelegramFolder } from '../../../types';
import { formatBytes } from '../../../utils';
import { Badge, Button, IconButton, Progress } from '../../ui';
import { stopControlShortcuts } from './useTopBarController';

interface TransferCenterProps {
  openRequest?: number;
  uploads: QueueItem[];
  downloads: DownloadItem[];
  folders?: TelegramFolder[];
  onClearUploads: () => void;
  onCancelUploads: () => void;
  onPauseUploads: () => void;
  onResumeUploads: () => void;
  onCancelUpload: (id: string) => void;
  onRetryUpload: (id: string) => void;
  onRetryFailedUploads: () => void;
  retryingFailedUploads: boolean;
  onClearDownloads: () => void;
  onCancelDownloads: () => void;
  onPauseDownloads: () => void;
  onResumeDownloads: () => void;
  onCancelDownload: (id: string) => void;
  onRetryDownload: (id: string) => void;
}

type TransferStatus = QueueItem['status'] | DownloadItem['status'];

function canCancelTransfer(item: QueueItem | DownloadItem) {
  return !['success', 'error', 'cancelled'].includes(item.status);
}

function QueuePlaybackAction({ paused = false, direction, onClick, header = false }: {
  paused?: boolean;
  direction: 'upload' | 'download';
  onClick: () => void;
  header?: boolean;
}) {
  const { t } = useTranslation();
  const action = t(paused ? 'activity.resume' : 'activity.pause');
  const title = t(direction === 'upload' ? 'activity.uploads' : 'activity.downloads');
  const Icon = paused ? Play : Pause;
  return <button type="button" onClick={onClick} aria-label={`${action} ${title}`} className={`quiet-control flex shrink-0 items-center gap-1.5 text-app-text-secondary ${header ? 'min-h-9 border border-app-border px-3 text-badge font-medium' : 'min-h-8 px-2'}`}><Icon className="h-3.5 w-3.5" aria-hidden="true" />{action}{header && ` ${title}`}</button>;
}

function statusKey(status: TransferStatus) {
  const normalized = status === 'success' ? 'completed' : status === 'error' ? 'failed' : status;
  return `activity.status_${normalized}`;
}

function statusTone(status: TransferStatus): 'neutral' | 'accent' | 'success' | 'warning' | 'danger' {
  if (status === 'success') return 'success';
  if (status === 'error') return 'danger';
  if (['waiting_for_network', 'waiting_for_unlock', 'cooldown'].includes(status)) return 'warning';
  if (['uploading', 'downloading', 'encrypting', 'decrypting', 'verifying'].includes(status)) return 'accent';
  return 'neutral';
}

function fileName(item: QueueItem | DownloadItem) {
  return 'filename' in item ? item.filename : (item.url || item.path).split(/[\\/]/).pop() || item.path;
}

function isTransferStage(status: TransferStatus) {
  return ['uploading', 'downloading', 'encrypting', 'decrypting', 'verifying'].includes(status);
}

function isSkipped(item: QueueItem | DownloadItem) {
  return 'filename' in item && item.downloadOutcome === 'skipped';
}

function TransferProgress({ item, label }: { item: QueueItem | DownloadItem; label: string }) {
  const { t } = useTranslation();
  if (!isTransferStage(item.status)) return null;

  const bytes = 'filename' in item ? item.downloadedBytes : item.uploadedBytes;
  const totalBytes = item.totalBytes;
  const canMeasureBytes = typeof bytes === 'number' && Number.isFinite(bytes)
    && typeof totalBytes === 'number' && Number.isFinite(totalBytes) && totalBytes > 0;
  const canMeasurePercent = item.status !== 'verifying'
    && typeof item.progress === 'number' && Number.isFinite(item.progress);
  const knownProgress = item.status !== 'verifying' && (canMeasureBytes || canMeasurePercent);

  if (!knownProgress) {
    return (
      <div className="mt-2 flex items-center gap-2" data-testid={`transfer-progress-${item.id}`}>
        <span className="h-2 w-2 shrink-0 rounded-full bg-app-accent animate-pulse motion-reduce:animate-none" aria-hidden="true" />
        <span className="text-metadata text-app-text-secondary" role="status">{t('activity.progress_unknown')}</span>
      </div>
    );
  }

  const measured = canMeasureBytes
    ? (bytes / totalBytes) * 100
    : item.progress as number;
  const progress = Math.min(100, Math.max(0, measured));
  const bytesLabel = canMeasureBytes ? `${formatBytes(bytes)} / ${formatBytes(totalBytes)}` : null;
  return (
    <div className="mt-2 space-y-1" data-testid={`transfer-progress-${item.id}`}>
      <div className="flex items-center justify-between gap-3 text-metadata text-app-text-secondary">
        {bytesLabel && <span className="tabular-nums">{bytesLabel}</span>}
        <span className="ms-auto tabular-nums">{t('activity.progress', { progress: Math.round(progress) })}</span>
      </div>
      <Progress value={progress} label={label} className="h-1.5" />
    </div>
  );
}

function TransferRow({
  item,
  direction,
  folders,
  onCancel,
  onRetry,
  retryBusy = false,
}: {
  item: QueueItem | DownloadItem;
  direction: 'upload' | 'download';
  folders: TelegramFolder[];
  onCancel: () => void;
  onRetry: () => void;
  retryBusy?: boolean;
}) {
  const { t } = useTranslation();
  const name = fileName(item);
  const retryable = ['error', 'cancelled', 'waiting_for_unlock'].includes(item.status);
  const retryLabel = t(item.status === 'waiting_for_unlock' ? 'protection_labels.provide_credentials' : 'activity.retry');
  const cancellable = canCancelTransfer(item);
  const destination = item.folderId === null
    ? t('common.saved_messages')
    : folders.find(folder => folder.id === item.folderId)?.name;
  const protection = 'filename' in item ? item.protectionMode : item.protection?.mode;
  const protectionLabel = t(`settings.encryption_mode_${['vault', 'passphrase', 'vault_and_passphrase'].includes(protection || '') ? protection : 'standard'}`);
  const active = isTransferStage(item.status);
  const speed = active && item.speedBytesPerSec && item.speedBytesPerSec > 0
    ? formatBytes(item.speedBytesPerSec)
    : null;

  return (
    <article data-testid={`transfer-row-${item.id}`} className="space-y-2 border-t border-app-border-subtle px-4 py-3 first:border-t-0">
      <div className="flex min-w-0 items-start gap-3">
        <span className={`mt-1 flex h-8 w-8 shrink-0 items-center justify-center rounded-control ${direction === 'upload' ? 'bg-app-selected text-app-accent' : 'bg-app-info/10 text-app-info'}`} aria-hidden="true">
          {direction === 'upload' ? <UploadCloud className="h-4 w-4" /> : <Download className="h-4 w-4" />}
        </span>
        <div className="min-w-0 flex-1">
          <p className="truncate text-ui font-medium text-app-text" title={name}>{name}</p>
          <div className="mt-1 flex flex-wrap items-center gap-2">
            <Badge tone={isSkipped(item) ? 'neutral' : statusTone(item.status)}>{t(isSkipped(item) ? 'downloadCollision.skipped' : statusKey(item.status))}</Badge>
            {speed && <span className="text-metadata tabular-nums text-app-text-secondary">{speed}/s</span>}
          </div>
          {item.error && <p className="mt-1 break-words text-metadata text-app-danger" role={item.status === 'error' ? 'alert' : 'status'}>{item.error}</p>}
        </div>
        <div className="flex shrink-0 items-center gap-1">
          {retryable && <Button size="sm" variant="primary" disabled={retryBusy} aria-label={`${retryLabel}: ${name}`} onClick={onRetry}>{retryLabel}</Button>}
          {cancellable && <IconButton size="sm" label={`${t('activity.cancel')} ${name}`} tone="danger" onClick={onCancel}><X className="h-4 w-4" aria-hidden="true" /></IconButton>}
        </div>
      </div>
      <TransferProgress item={item} label={name} />
      {direction === 'upload' && (destination || protection) && (
        <details className="group rounded-control border border-app-border-subtle bg-app-surface-sunken/25 px-3 py-2">
          <summary className="flex min-h-7 cursor-pointer list-none items-center gap-1.5 text-badge font-medium text-app-text-secondary focus-visible:outline focus-visible:outline-2 focus-visible:outline-app-accent">
            <ChevronDown className="h-3.5 w-3.5 group-open:rotate-180" aria-hidden="true" />
            {t('activity.details')}
          </summary>
          <dl className="mt-2 grid gap-2 border-t border-app-border-subtle pt-2 text-metadata sm:grid-cols-2">
            {destination && <div className="min-w-0"><dt className="text-app-text-tertiary">{t('files.destination_folder')}</dt><dd className="truncate text-app-text" title={destination}>{destination}</dd></div>}
            {protection && <div className="min-w-0"><dt className="text-app-text-tertiary">{t('settings.tab_encryption')}</dt><dd className="truncate text-app-text">{protectionLabel}</dd></div>}
          </dl>
        </details>
      )}
    </article>
  );
}

function TransferQueueSection({
  direction,
  items,
  paused,
  folders,
  onPause,
  onResume,
  onCancelAll,
  onClear,
  onCancel,
  onRetry,
  onRetryFailed,
  retryBusy = false,
}: {
  direction: 'upload' | 'download';
  items: Array<QueueItem | DownloadItem>;
  paused: number;
  folders: TelegramFolder[];
  onPause: () => void;
  onResume: () => void;
  onCancelAll: () => void;
  onClear: () => void;
  onCancel: (id: string) => void;
  onRetry: (id: string) => void;
  onRetryFailed?: () => void;
  retryBusy?: boolean;
}) {
  const { t } = useTranslation();
  const title = t(direction === 'upload' ? 'activity.uploads' : 'activity.downloads');
  const canCancel = items.some(canCancelTransfer);
  const canPause = items.some(item => canCancelTransfer(item) && item.status !== 'paused');
  const canClear = items.some(item => ['success', 'error', 'cancelled'].includes(item.status));
  return (
    <section aria-label={title}>
      <header className="flex flex-wrap items-center justify-between gap-2 border-t border-app-border-subtle bg-app-surface-sunken/25 px-4 py-2.5 first:border-t-0">
        <div className="flex items-center gap-2">
          <h3 className="text-xs font-semibold text-app-text">{title}</h3>
          <Badge>{items.length}</Badge>
        </div>
        <div className="flex flex-wrap items-center gap-3 text-badge">
          {onRetryFailed && <RetryFailedUploadsButton count={items.filter(item => item.status === 'error').length} busy={retryBusy} onRetry={onRetryFailed} />}
          {direction === 'download' && canPause && <QueuePlaybackAction direction={direction} onClick={onPause} />}
          {direction === 'download' && paused > 0 && <QueuePlaybackAction direction={direction} onClick={onResume} paused />}
          {canCancel && <button type="button" onClick={onCancelAll} className="quiet-control min-h-8 px-2 text-app-danger" aria-label={`${t('activity.cancel')} ${title}`}>{t('common.cancel')}</button>}
          <button type="button" onClick={onClear} disabled={!canClear} className="quiet-control min-h-8 px-2 text-app-accent disabled:opacity-45">{t('settings.clear')}</button>
        </div>
      </header>
      {items.length === 0
        ? <p className="px-4 py-5 text-center text-sm text-app-text-secondary">{t('activity.empty')}</p>
        : items.map(item => <TransferRow key={item.id} item={item} direction={direction} folders={folders} onCancel={() => onCancel(item.id)} onRetry={() => onRetry(item.id)} retryBusy={retryBusy} />)}
    </section>
  );
}

export function TransferCenter({
  openRequest = 0,
  uploads,
  downloads,
  folders = [],
  onClearUploads,
  onCancelUploads,
  onPauseUploads,
  onResumeUploads,
  onCancelUpload,
  onRetryUpload,
  onRetryFailedUploads,
  retryingFailedUploads,
  onClearDownloads,
  onCancelDownloads,
  onPauseDownloads,
  onResumeDownloads,
  onCancelDownload,
  onRetryDownload,
}: TransferCenterProps) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(true);
  useEffect(() => {
    if (openRequest > 0) setExpanded(true);
  }, [openRequest]);
  if (uploads.length === 0 && downloads.length === 0) return null;

  const activeUploads = uploads.filter(item => item.status === 'pending' || isTransferStage(item.status)).length;
  const activeDownloads = downloads.filter(item => item.status === 'pending' || isTransferStage(item.status)).length;
  const activeCount = activeUploads + activeDownloads;
  const pausedUploads = uploads.filter(item => item.status === 'paused').length;
  const pausedDownloads = downloads.filter(item => item.status === 'paused').length;
  const pausedCount = pausedUploads + pausedDownloads;
  const items = [...uploads, ...downloads];
  const attentionCount = items.filter(item => ['error', 'waiting_for_unlock'].includes(item.status)).length;
  const aggregateSpeed = items
    .filter(item => isTransferStage(item.status))
    .reduce((sum, item) => sum + (item.speedBytesPerSec || 0), 0);
  const completedCount = items.filter(item => item.status === 'success' && !isSkipped(item)).length;
  const skippedCount = items.filter(isSkipped).length;
  const allComplete = completedCount === items.length;
  const counts = [
    [t('activity.in_progress'), activeCount],
    [t('activity.attention'), attentionCount],
    [t('activity.status_paused'), pausedCount],
    [t('activity.status_waiting_for_network'), items.filter(item => item.status === 'waiting_for_network').length],
    [t('activity.status_cooldown'), downloads.filter(item => item.status === 'cooldown').length],
    [t('activity.status_completed'), completedCount],
    [t('downloadCollision.skipped'), skippedCount],
    [t('activity.status_cancelled'), items.filter(item => item.status === 'cancelled').length],
  ] as const;
  const summary = counts.filter(([, count]) => count > 0).map(([label, count]) => `${label}: ${count}`).join(' · ')
    + (aggregateSpeed > 0 ? ` · ${formatBytes(aggregateSpeed)}/s` : '');

  return (
    <aside data-testid="transfer-center" data-ui-motion onKeyDown={stopControlShortcuts} className={`quiet-raised ui-enter fixed bottom-4 end-4 z-50 flex max-h-[min(560px,calc(100dvh-12rem))] flex-col overflow-hidden ${expanded ? 'w-[min(600px,calc(100vw-2rem))]' : 'w-[min(360px,calc(100vw-2rem))]'}`} aria-label={t('activity.title')}>
      <header className="flex shrink-0 flex-wrap items-center gap-3 border-b border-app-border-subtle px-4 py-3">
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-control bg-app-selected text-app-accent" aria-hidden="true"><UploadCloud className="h-4 w-4" /></div>
        <div className="min-w-0 flex-1">
          <h2 className="text-base font-semibold text-app-text">{t('common.transfers')}</h2>
          <p className="truncate text-metadata text-app-text-secondary" title={summary}>{summary}</p>
        </div>
        {expanded && uploads.some(item => canCancelTransfer(item) && item.status !== 'paused') && <QueuePlaybackAction direction="upload" onClick={onPauseUploads} header />}
        {expanded && pausedUploads > 0 && <QueuePlaybackAction direction="upload" onClick={onResumeUploads} paused header />}
        <IconButton
          size="sm"
          label={`${expanded ? t('common.close') : t('files.open')} ${t('common.transfers')}`}
          aria-expanded={expanded}
          aria-controls={expanded ? 'desktop-transfer-details' : undefined}
          onClick={() => setExpanded(value => !value)}
        >
          {expanded ? <ChevronDown className="h-4 w-4" /> : <ChevronUp className="h-4 w-4" />}
        </IconButton>
      </header>

      {expanded && (
        <div id="desktop-transfer-details" data-ui-motion className="ui-enter min-h-0 flex-1 overflow-y-auto">
          {uploads.length > 0 && <TransferQueueSection direction="upload" items={uploads} paused={pausedUploads} folders={folders} onPause={onPauseUploads} onResume={onResumeUploads} onCancelAll={onCancelUploads} onClear={onClearUploads} onCancel={onCancelUpload} onRetry={onRetryUpload} onRetryFailed={onRetryFailedUploads} retryBusy={retryingFailedUploads} />}
          {downloads.length > 0 && <TransferQueueSection direction="download" items={downloads} paused={pausedDownloads} folders={folders} onPause={onPauseDownloads} onResume={onResumeDownloads} onCancelAll={onCancelDownloads} onClear={onClearDownloads} onCancel={onCancelDownload} onRetry={onRetryDownload} />}
          {allComplete && (
            <div className="flex items-center gap-3 border-t border-app-border-subtle px-4 py-3 text-sm" role="status">
              <CheckCircle2 className="h-5 w-5 shrink-0 text-app-success" aria-hidden="true" />
              <span className="font-medium text-app-text">{t('activity.status_completed')}</span>
            </div>
          )}
        </div>
      )}
    </aside>
  );
}
