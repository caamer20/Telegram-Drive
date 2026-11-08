import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { AlertTriangle, Eye, FolderInput, Link2, Loader2, Pause, Play, Trash2 } from 'lucide-react';
import { toast } from 'sonner';
import { useTranslation } from 'react-i18next';
import { useSync } from '../../../context/SyncContext';
import type { TelegramFolder } from '../../../types';
import type { SyncDirection, SyncPair, SyncPairStatus, SyncPreview, SyncPreviewRequest } from '../../../types/sync';
import { previewSyncPair } from '../../../services/syncService';
import { userFacingError } from '../../../services/userFacingError';
import { SyncPlanPreview } from './SyncPlanPreview';

const DEFAULT_IGNORES = '.git/\nnode_modules/\n.DS_Store';

export function SyncSettingsPanel() {
  const { t } = useTranslation();
  const { ownerId, settings, pairs, status, setEnabled, setScanner, addPair, updatePair, setPairActive, removePair } = useSync();
  const [folders, setFolders] = useState<TelegramFolder[]>([]);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [channelId, setChannelId] = useState<number | ''>('');
  const [editingId, setEditingId] = useState<number | null>(null);
  const [direction, setDirection] = useState<SyncDirection>('upload_only');
  const [ignoreText, setIgnoreText] = useState(DEFAULT_IGNORES);
  const [propagateDeletions, setPropagateDeletions] = useState(false);
  const [pauseOnConflicts, setPauseOnConflicts] = useState(true);
  const [adoptMatchingFiles, setAdoptMatchingFiles] = useState(false);
  const [activateAfterSave, setActivateAfterSave] = useState(false);
  const [busy, setBusy] = useState(false);
  const [previewBusy, setPreviewBusy] = useState(false);
  const [preview, setPreview] = useState<SyncPreview | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const previewSequence = useRef(0);
  const currentOwner = useRef(ownerId); currentOwner.current = ownerId;
  const formRef = useRef<HTMLDivElement>(null);
  const enabled = settings.data?.enabled ?? false;
  const fastScan = settings.data?.scanner === 'incremental';
  const editingPair = (pairs.data ?? []).find(pair => pair.id === editingId);
  const selectedFolder = useMemo(() => folders.find(folder => folder.id === channelId), [channelId, folders]);
  const draftSignature = JSON.stringify([editingId, selectedPath, channelId, direction, ignoreText, propagateDeletions, pauseOnConflicts, adoptMatchingFiles]);

  useEffect(() => {
    let disposed = false;
    setFolders([]);
    if (!ownerId) return;
    void invoke<TelegramFolder[]>('cmd_get_enriched_folders').then(value => { if (!disposed) setFolders(value); }).catch(() => { if (!disposed) setFolders([]); });
    return () => { disposed = true; };
  }, [ownerId]);
  useEffect(() => {
    previewSequence.current += 1;
    setPreview(null); setPreviewError(null); setPreviewBusy(false);
  }, [draftSignature, ownerId]);
  useEffect(() => () => { previewSequence.current += 1; }, []);

  const resetDraft = () => {
    previewSequence.current += 1;
    setEditingId(null); setSelectedPath(null); setChannelId('');
    setDirection('upload_only'); setIgnoreText(DEFAULT_IGNORES);
    setPropagateDeletions(false); setPauseOnConflicts(true); setAdoptMatchingFiles(false); setActivateAfterSave(false);
    setPreview(null); setPreviewError(null); setPreviewBusy(false);
  };
  useEffect(() => { resetDraft(); setBusy(false); }, [ownerId]);
  const editPair = (pair: SyncPair) => {
    setEditingId(pair.id); setSelectedPath(pair.localPath); setChannelId(pair.channelId);
    setDirection(pair.syncDirection); setIgnoreText(pair.preferences?.ignorePatterns.join('\n') ?? DEFAULT_IGNORES);
    setPropagateDeletions(pair.preferences?.propagateDeletions ?? false);
    setPauseOnConflicts(pair.preferences?.pauseOnConflicts ?? true);
    setAdoptMatchingFiles(pair.preferences?.adoptMatchingFiles ?? false);
    setActivateAfterSave(Boolean(pair.accountOwner && pair.isActive));
    formRef.current?.scrollIntoView?.({ behavior: 'smooth', block: 'nearest' });
  };
  const chooseFolder = async () => {
    const requestOwner = ownerId;
    const selected = await open({ directory: true, multiple: false, title: t('settings.sync.select_folder') });
    if (requestOwner === currentOwner.current && typeof selected === 'string') setSelectedPath(selected);
  };
  const runPreview = async () => {
    if (!ownerId || !selectedPath || channelId === '') return;
    const request: SyncPreviewRequest = {
      pairId: editingId, localPath: selectedPath, channelId, syncDirection: direction,
      preferences: { ignorePatterns: ignoreText.split(/\r?\n/).map(pattern => pattern.trim()).filter(Boolean), propagateDeletions, pauseOnConflicts, adoptMatchingFiles },
    };
    const sequence = ++previewSequence.current;
    setPreviewBusy(true); setPreview(null); setPreviewError(null);
    try {
      const result = await previewSyncPair(request, ownerId);
      if (sequence === previewSequence.current) setPreview(result);
    } catch (error) {
      if (sequence === previewSequence.current) setPreviewError(userFacingError(error, t));
    } finally {
      if (sequence === previewSequence.current) setPreviewBusy(false);
    }
  };
  const savePair = async () => {
    if (!preview) return;
    setBusy(true);
    try {
      if (preview.request.pairId !== null) {
        await updatePair(preview.request, preview.reviewToken, activateAfterSave);
      } else {
        await addPair(preview.request.localPath, preview.request.channelId, selectedFolder?.name, {
          syncDirection: preview.request.syncDirection, preferences: preview.request.preferences,
          previewToken: preview.reviewToken, isActive: activateAfterSave,
        });
      }
      if (ownerId !== currentOwner.current) return;
      resetDraft(); toast.success(t('syncPreview.saved'));
    } catch (error) {
      if (ownerId === currentOwner.current) { setPreview(null); toast.error(userFacingError(error, t)); }
    } finally { if (ownerId === currentOwner.current) setBusy(false); }
  };
  const runAction = async (operation: () => Promise<void>) => {
    setBusy(true);
    try { await operation(); }
    catch (error) { if (ownerId === currentOwner.current) toast.error(userFacingError(error, t)); }
    finally { if (ownerId === currentOwner.current) setBusy(false); }
  };
  const directionLabel = (value: SyncDirection) => value === 'upload_only' ? t('syncPreview.backup_mode') : value === 'download_only' ? t('syncPreview.download_mode') : t('syncPreview.two_way_mode');
  const phaseLabel = (pair: SyncPair, current?: SyncPairStatus) => {
    if (!pair.accountOwner) return t('syncPreview.review_required');
    if (!enabled || !pair.isActive || current?.phase === 'paused') return t('syncPreview.paused');
    if (current?.phase === 'scanning') return t('syncPreview.scanning');
    if (current?.phase === 'syncing') return t('syncPreview.syncing');
    if (current?.phase === 'waiting' && current.waitReason === 'telegram') return t('syncPreview.waiting_telegram');
    return current?.phase === 'ready' ? t('syncPreview.ready') : t('syncPreview.waiting');
  };

  return <section className="space-y-5" aria-labelledby="folder-sync-title">
    <div className="quiet-surface flex items-start justify-between gap-5 p-4">
      <div className="flex gap-3">
        <FolderInput className="mt-0.5 h-5 w-5 shrink-0 text-app-accent" />
        <div><h3 id="folder-sync-title" className="text-sm font-semibold text-app-text">{t('settings.sync.title')}</h3><p className="mt-1 max-w-lg text-xs leading-5 text-app-text-secondary">{t('syncPreview.setup_description')}</p></div>
      </div>
      <button type="button" role="switch" aria-checked={enabled} disabled={busy || settings.isLoading} onClick={() => void runAction(() => setEnabled(!enabled))} className={`relative h-6 w-11 shrink-0 rounded-full transition ${enabled ? 'bg-app-accent' : 'bg-app-border-strong'} disabled:opacity-50`} aria-label={t('settings.sync.toggle')}>
        <span className={`absolute top-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform ${enabled ? 'translate-x-5 rtl:-translate-x-5' : 'translate-x-0.5 rtl:-translate-x-0.5'}`} />
      </button>
    </div>
    {!enabled && <p className="rounded-lg border border-app-accent/25 bg-app-accent/5 p-3 text-xs leading-5 text-app-text-secondary">{t('syncPreview.global_paused')}</p>}
    <div className="quiet-surface p-4">
      <label className="flex items-start gap-2 text-xs leading-5 text-app-text"><input type="checkbox" disabled={busy || settings.isLoading} checked={fastScan} onChange={event => void runAction(() => setScanner(event.target.checked ? 'incremental' : 'full'))} aria-describedby="sync-fast-scan-help" className="mt-1 accent-app-accent" />{t('syncPreview.fast_scan')}</label>
      <p id="sync-fast-scan-help" className="mt-1 text-xs leading-5 text-app-text-tertiary">{t('syncPreview.fast_scan_help')}</p>
    </div>
    <div className="space-y-3">{(pairs.data ?? []).map(pair => {
      const current = status.data?.pairs?.find(value => value.pairId === pair.id);
      return <div key={pair.id} className="quiet-surface space-y-2 p-3">
        <div className="flex items-start gap-3">
          <Link2 className="mt-0.5 h-4 w-4 shrink-0 text-app-accent" />
          <div className="min-w-0 flex-1"><p className="truncate text-sm font-medium text-app-text">{pair.label ?? pair.channelId}</p><p className="break-all text-xs text-app-text-tertiary">{pair.localPath}</p></div>
          <button type="button" disabled={busy} onClick={() => void runAction(async () => { await removePair(pair.id); if (editingId === pair.id) resetDraft(); })} className="quiet-control p-2 text-app-danger disabled:opacity-40" title={t('settings.sync.remove')} aria-label={t('settings.sync.remove')}><Trash2 className="h-4 w-4" /></button>
        </div>
        <p className="text-xs text-app-text-secondary">{directionLabel(pair.syncDirection)} · {pair.preferences?.propagateDeletions ? t('syncPreview.deletions_on') : t('syncPreview.deletions_off')}</p>
        <div className="flex flex-wrap items-center justify-between gap-2">
          <p className={`text-xs ${!pair.accountOwner || current?.lastError ? 'text-app-warning' : 'text-app-text-secondary'}`}>{phaseLabel(pair, current)}{current && current.pendingOps > 0 ? ` · ${t('syncPreview.pending', { count: current.pendingOps })}` : ''}</p>
          <div className="flex gap-2">
            <button type="button" disabled={busy} onClick={() => editPair(pair)} className="quiet-control flex items-center gap-1.5 px-2 py-1 text-xs text-app-accent disabled:opacity-40"><Eye className="h-3.5 w-3.5" />{t('syncPreview.edit')}</button>
            {pair.accountOwner && <button type="button" disabled={busy} onClick={() => void runAction(() => setPairActive(pair.id, !pair.isActive))} className="quiet-control flex items-center gap-1.5 px-2 py-1 text-xs text-app-text-secondary disabled:opacity-40">{pair.isActive ? <Pause className="h-3.5 w-3.5" /> : <Play className="h-3.5 w-3.5" />}{pair.isActive ? t('syncPreview.pause') : t('syncPreview.resume')}</button>}
          </div>
        </div>
        {current?.lastError && enabled && pair.isActive && <p className="text-xs leading-5 text-app-warning">{current.lastError}</p>}
        {current?.lastCheckedAt && <p className="text-xs text-app-text-tertiary">{t('syncPreview.checked_at', { time: new Date(current.lastCheckedAt * 1000).toLocaleString() })}</p>}
      </div>;
    })}</div>

    <div ref={formRef} className="space-y-4 rounded-lg border border-app-border p-4">
      <div className="flex items-center justify-between gap-3">
        <h4 className="text-sm font-semibold text-app-text">{editingPair ? `${t('syncPreview.edit')}: ${editingPair.label ?? editingPair.channelId}` : t('syncPreview.setup_title')}</h4>
        {editingId !== null && <button type="button" disabled={busy} onClick={resetDraft} className="quiet-control px-2 py-1 text-xs text-app-text-secondary">{t('syncPreview.cancel_edit')}</button>}
      </div>
      <div className="grid gap-3 sm:grid-cols-2">
        <button type="button" disabled={busy || editingId !== null} onClick={() => void chooseFolder().catch(error => toast.error(userFacingError(error, t)))} className="quiet-control min-w-0 border border-app-border px-3 py-2 text-start text-xs text-app-text disabled:opacity-60" aria-label={t('settings.sync.select_folder')} title={selectedPath ?? undefined}><span className="block truncate">{selectedPath ?? t('settings.sync.add_folder')}</span></button>
        <select value={channelId} disabled={busy || editingId !== null} onChange={event => setChannelId(event.target.value ? Number(event.target.value) : '')} aria-label={t('settings.sync.select_channel')} className="app-select quiet-control border border-app-border bg-app-surface px-3 py-2 text-xs text-app-text disabled:opacity-60">
          <option value="">{t('settings.sync.select_channel')}</option>{folders.map(folder => <option key={folder.id} value={folder.id}>{folder.name}</option>)}
          {editingPair && !folders.some(folder => folder.id === editingPair.channelId) && <option value={editingPair.channelId}>{editingPair.label ?? editingPair.channelId}</option>}
        </select>
      </div>
      {folders.length === 0 && editingId === null && <p className="flex items-center gap-1.5 text-xs text-app-warning"><AlertTriangle className="h-3.5 w-3.5" />{t('settings.sync.no_channels')}</p>}
      <div>
        <label htmlFor="sync-direction" className="mb-1 block text-xs font-medium text-app-text">{t('syncPreview.mode')}</label>
        <select id="sync-direction" value={direction} disabled={busy} onChange={event => { setDirection(event.target.value as SyncDirection); setPropagateDeletions(false); }} className="app-select quiet-control w-full border border-app-border bg-app-surface px-3 py-2 text-xs text-app-text"><option value="upload_only">{t('syncPreview.backup_mode')}</option><option value="download_only">{t('syncPreview.download_mode')}</option><option value="bidirectional">{t('syncPreview.two_way_mode')}</option></select>
        <p className="mt-2 text-xs leading-5 text-app-text-secondary">{direction === 'upload_only' ? t('syncPreview.backup_description') : direction === 'download_only' ? t('syncPreview.download_description') : t('syncPreview.two_way_description')}</p>
      </div>
      <div className="space-y-2">
        <label className="flex items-start gap-2 text-xs leading-5 text-app-text"><input type="checkbox" disabled={busy} checked={propagateDeletions} onChange={event => setPropagateDeletions(event.target.checked)} className="mt-1 accent-app-accent" />{direction === 'upload_only' ? t('syncPreview.delete_upload') : direction === 'download_only' ? t('syncPreview.delete_download') : t('syncPreview.delete_both')}</label>
        <p className="text-xs leading-5 text-app-text-tertiary">{t('syncPreview.deletion_guard')}</p><p className="text-xs leading-5 text-app-text-secondary">{t('syncPreview.version_notice')}</p>
      </div>
      <div>
        <label htmlFor="sync-ignore-patterns" className="mb-1 block text-xs font-medium text-app-text">{t('syncPreview.ignore_patterns')}</label>
        <textarea id="sync-ignore-patterns" value={ignoreText} disabled={busy} onChange={event => setIgnoreText(event.target.value)} rows={3} className="quiet-control w-full resize-y border border-app-border bg-app-surface px-3 py-2 font-mono text-xs text-app-text" aria-describedby="sync-ignore-help" />
        <p id="sync-ignore-help" className="mt-1 text-xs leading-5 text-app-text-tertiary">{t('syncPreview.ignore_help')}</p>
      </div>
      <div>
        <label className="flex items-start gap-2 text-xs leading-5 text-app-text"><input type="checkbox" disabled={busy} checked={pauseOnConflicts} onChange={event => setPauseOnConflicts(event.target.checked)} className="mt-1 accent-app-accent" />{t('syncPreview.pause_conflicts')}</label>
        <p className="mt-1 text-xs leading-5 text-app-text-tertiary">{t('syncPreview.pause_conflicts_help')}</p><p className="mt-1 text-xs leading-5 text-app-text-tertiary">{t('syncPreview.pause_conditions')}</p>
      </div>
      <div>
        <label className="flex items-start gap-2 text-xs leading-5 text-app-text"><input type="checkbox" disabled={busy} checked={adoptMatchingFiles} onChange={event => setAdoptMatchingFiles(event.target.checked)} aria-describedby="sync-adopt-help" className="mt-1 accent-app-accent" />{t('syncPreview.adopt_matching')}</label>
        <p id="sync-adopt-help" className="mt-1 text-xs leading-5 text-app-text-tertiary">{t('syncPreview.adopt_matching_help')}</p>
      </div>
      <button type="button" disabled={!selectedPath || channelId === '' || busy || previewBusy} onClick={() => void runPreview()} className="quiet-control flex items-center justify-center gap-2 border border-app-accent px-3 py-2 text-xs font-medium text-app-accent disabled:opacity-40">{previewBusy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Eye className="h-4 w-4" />}{previewBusy ? t('syncPreview.preview_loading') : t('syncPreview.preview_button')}</button>
      {previewError && <p role="alert" className="rounded-md border border-app-danger/30 p-3 text-xs leading-5 text-app-danger">{previewError}</p>}
      {preview && <SyncPlanPreview preview={preview} />}
      <label className="flex items-start gap-2 text-xs leading-5 text-app-text"><input type="checkbox" checked={activateAfterSave} disabled={busy} onChange={event => setActivateAfterSave(event.target.checked)} className="mt-1 accent-app-accent" />{t('syncPreview.start_after_save')}</label>
      <div className="flex flex-wrap items-center gap-3">
        <button type="button" disabled={!preview || busy || previewBusy} onClick={() => void savePair()} className="quiet-control flex items-center justify-center gap-2 bg-app-accent px-3 py-2 text-xs font-medium text-white disabled:opacity-40">{busy && <Loader2 className="h-3.5 w-3.5 animate-spin" />}{activateAfterSave ? t('syncPreview.save_active') : t('syncPreview.save_paused')}</button>
        {!preview && <p className="text-xs text-app-text-tertiary">{t('syncPreview.preview_required')}</p>}
      </div>
    </div>
  </section>;
}
