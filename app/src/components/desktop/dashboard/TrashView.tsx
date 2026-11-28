import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowLeft, RotateCcw, Trash2, Loader2, Clock3, Folder } from 'lucide-react';
import type { TrashItem, TrashStats } from '../../../types';
import { formatBytes } from '../../../utils';
import { useConfirm } from '../../../context/ConfirmContext';

interface TrashedFolder {
  folder_id: number;
  name: string;
  parent_id: number | null;
  deleted_at: number;
}

interface TrashViewProps {
  onClose: () => void;
  /** Called after a successful restore so the caller can refresh folder data. */
  onRestored?: () => void;
}

function daysUntil(epochSeconds: string): number {
  const expires = Number(epochSeconds) * 1000;
  const diff = expires - Date.now();
  return Math.max(0, Math.ceil(diff / (1000 * 60 * 60 * 24)));
}

/** Soft-deleted files with a 20-day retention window and one-click restore. */
export function TrashView({ onClose, onRestored }: TrashViewProps) {
  const [items, setItems] = useState<TrashItem[]>([]);
  const [folders, setFolders] = useState<TrashedFolder[]>([]);
  const [busyFolder, setBusyFolder] = useState<number | null>(null);
  const [selectedFolders, setSelectedFolders] = useState<number[]>([]);
  const [selectedFiles, setSelectedFiles] = useState<number[]>([]);
  const { confirm } = useConfirm();
  const [stats, setStats] = useState<TrashStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [busyId, setBusyId] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [list, trashStats, folderList] = await Promise.all([
        invoke<TrashItem[]>('cmd_list_trash'),
        invoke<TrashStats>('cmd_get_trash_stats'),
        invoke<TrashedFolder[]>('cmd_list_trashed_folders').catch(() => [] as TrashedFolder[]),
      ]);
      setItems(list);
      setStats(trashStats);
      setFolders(folderList);
    } catch (e) {
      setError(String(e));
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const handleRestore = async (item: TrashItem) => {
    setBusyId(item.trash_message_id);
    try {
      await invoke('cmd_restore_from_trash', { trashMessageId: item.trash_message_id });
      onRestored?.();
      await refresh();
    } catch (e) {
      setError(String(e));
    }
    setBusyId(null);
  };

  const handleRestoreFolder = async (folder: TrashedFolder) => {
    setBusyFolder(folder.folder_id);
    try {
      await invoke('cmd_restore_trashed_folder', { folderId: folder.folder_id });
      onRestored?.();
      await refresh();
    } catch (e) {
      setError(String(e));
    }
    setBusyFolder(null);
  };

  const handleForgetFolder = async (folder: TrashedFolder) => {
    setBusyFolder(folder.folder_id);
    try {
      await invoke('cmd_forget_trashed_folder', { folderId: folder.folder_id });
      await refresh();
    } catch (e) {
      setError(String(e));
    }
    setBusyFolder(null);
  };

  const toggleFolder = (folderId: number) => {
    setSelectedFolders(previous => previous.includes(folderId)
      ? previous.filter(id => id !== folderId)
      : [...previous, folderId]);
  };

  const toggleFile = (trashMessageId: number) => {
    setSelectedFiles(previous => previous.includes(trashMessageId)
      ? previous.filter(id => id !== trashMessageId)
      : [...previous, trashMessageId]);
  };

  const handlePurgeFiles = async () => {
    const ids = selectedFiles.length > 0 ? selectedFiles : items.map(item => item.trash_message_id);
    if (ids.length === 0) return;
    const accepted = await confirm({
      title: 'Delete files permanently',
      message: `${ids.length} file(s) will be removed for good. This cannot be undone.`,
      confirmText: 'Delete permanently',
      variant: 'danger',
    });
    if (!accepted) return;
    setBusyId(-1);
    setError(null);
    try {
      await invoke('cmd_purge_trash_items', { trashMessageIds: ids });
      setSelectedFiles([]);
      onRestored?.();
      await refresh();
    } catch (e) {
      setError(String(e));
    }
    setBusyId(null);
  };

  const handlePurgeFolders = async () => {
    const ids = selectedFolders.length > 0 ? selectedFolders : folders.map(folder => folder.folder_id);
    if (ids.length === 0) return;
    const accepted = await confirm({
      title: 'Delete folders permanently',
      message: `${ids.length} folder(s) and their Telegram channels will be deleted for good. This cannot be undone.`,
      confirmText: 'Delete permanently',
      variant: 'danger',
    });
    if (!accepted) return;
    setBusyFolder(-1);
    setError(null);
    try {
      const purged = await invoke<number[]>('cmd_purge_trashed_folders', { folderIds: ids });
      if (purged.length < ids.length) {
        setError('Telegram rate limit: some folders were kept. Try the rest in a few minutes.');
      }
      setSelectedFolders([]);
      onRestored?.();
      await refresh();
    } catch (e) {
      setError(String(e));
    }
    setBusyFolder(null);
  };

  const handlePurge = async () => {
    try {
      await invoke('cmd_purge_expired_trash');
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-app-canvas">
      <div className="flex items-center justify-between border-b border-telegram-border px-4 py-3">
        <div className="flex items-center gap-3">
          <button
            onClick={onClose}
            className="flex items-center gap-1.5 rounded-lg px-2 py-1.5 text-sm text-telegram-subtext transition hover:bg-telegram-hover hover:text-telegram-text"
          >
            <ArrowLeft className="h-4 w-4" />
            Back
          </button>
          <h2 className="flex items-center gap-2 text-lg font-bold text-telegram-text">
            <Trash2 className="h-5 w-5 text-red-400" />
            Trash
          </h2>
          {stats && (
            <span className="text-xs text-telegram-subtext">
              {stats.count} item{stats.count !== 1 ? 's' : ''} · {formatBytes(stats.total_size)}
            </span>
          )}
        </div>
        <button
          onClick={() => void handlePurge()}
          className="rounded-lg px-3 py-1.5 text-xs font-medium text-telegram-subtext transition hover:bg-telegram-hover hover:text-telegram-text"
        >
          Purge expired
        </button>
      </div>

      {error && <p role="alert" className="px-4 py-2 text-sm text-red-400">{error}</p>}

      <div className="flex-1 overflow-y-auto p-4">
        {folders.length > 0 && (
          <section className="mb-5">
            <div className="mb-2 flex items-center gap-2">
              <h3 className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wide text-telegram-subtext">
                <Folder className="h-3.5 w-3.5 text-amber-400" />
                Folders ({folders.length})
              </h3>
              <button
                type="button"
                onClick={() => setSelectedFolders(selectedFolders.length === folders.length ? [] : folders.map(folder => folder.folder_id))}
                className="rounded-lg px-2 py-0.5 text-xs font-medium text-telegram-subtext hover:bg-telegram-hover hover:text-telegram-text"
              >
                {selectedFolders.length === folders.length ? 'Clear' : 'Select all'}
              </button>
              <button
                type="button"
                disabled={busyFolder === -1}
                onClick={() => void handlePurgeFolders()}
                className="ms-auto flex items-center gap-1 rounded-lg px-2 py-1 text-xs font-semibold text-red-400 hover:bg-red-500/10 disabled:opacity-50"
              >
                {busyFolder === -1 ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Trash2 className="h-3.5 w-3.5" />}
                Delete permanently{selectedFolders.length > 0 ? ` (${selectedFolders.length})` : ' (all)'}
              </button>
            </div>
            <ul className="divide-y divide-telegram-border overflow-hidden rounded-xl border border-telegram-border">
              {folders.map(folder => (
                <li key={folder.folder_id} className="flex items-center gap-3 bg-telegram-surface px-3 py-2">
                  <input
                    type="checkbox"
                    checked={selectedFolders.includes(folder.folder_id)}
                    onChange={() => toggleFolder(folder.folder_id)}
                    aria-label={`Select ${folder.name}`}
                    className="h-3.5 w-3.5 shrink-0 accent-red-500"
                  />
                  <Folder className="h-4 w-4 shrink-0 fill-current text-amber-400" />
                  <span className="min-w-0 flex-1 truncate text-sm text-telegram-text">{folder.name}</span>
                  <button
                    type="button"
                    disabled={busyFolder === folder.folder_id}
                    onClick={() => void handleRestoreFolder(folder)}
                    className="flex items-center gap-1 rounded-lg px-2 py-1 text-xs font-medium text-emerald-400 hover:bg-telegram-hover disabled:opacity-50"
                  >
                    {busyFolder === folder.folder_id ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RotateCcw className="h-3.5 w-3.5" />}
                    Restore
                  </button>
                  <button
                    type="button"
                    disabled={busyFolder === folder.folder_id}
                    onClick={() => void handleForgetFolder(folder)}
                    className="rounded-lg px-2 py-1 text-xs font-medium text-telegram-subtext hover:bg-telegram-hover disabled:opacity-50"
                    title="Remove from this list; the Telegram channel is kept"
                  >
                    Remove from list
                  </button>
                </li>
              ))}
            </ul>
            <p className="mt-1 text-xs text-telegram-subtext">
              Folder deletion is a recycle bin: the Telegram channel is kept, so restoring is instant.
            </p>
          </section>
        )}
        {loading ? (
          <div className="flex items-center justify-center py-16 text-telegram-subtext">
            <Loader2 className="h-5 w-5 animate-spin" />
          </div>
        ) : items.length === 0 ? (
          <div className="py-16 text-center text-telegram-subtext">
            <Trash2 className="mx-auto mb-2 h-10 w-10 opacity-20" />
            <p className="text-sm">Trash is empty.</p>
            <p className="mt-1 text-xs opacity-70">Deleted files stay here for 20 days before they are removed for good.</p>
          </div>
        ) : (
          <section>
            <div className="mb-2 flex items-center gap-2">
              <h3 className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wide text-telegram-subtext">
                <Clock3 className="h-3.5 w-3.5 text-red-400" />
                Files ({items.length})
              </h3>
              <button
                type="button"
                onClick={() => setSelectedFiles(selectedFiles.length === items.length ? [] : items.map(item => item.trash_message_id))}
                className="rounded-lg px-2 py-0.5 text-xs font-medium text-telegram-subtext hover:bg-telegram-hover hover:text-telegram-text"
              >
                {selectedFiles.length === items.length ? 'Clear' : 'Select all'}
              </button>
              <button
                type="button"
                disabled={busyId === -1}
                onClick={() => void handlePurgeFiles()}
                className="ms-auto flex items-center gap-1 rounded-lg px-2 py-1 text-xs font-semibold text-red-400 hover:bg-red-500/10 disabled:opacity-50"
              >
                {busyId === -1 ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Trash2 className="h-3.5 w-3.5" />}
                Delete permanently{selectedFiles.length > 0 ? ` (${selectedFiles.length})` : ' (all)'}
              </button>
            </div>
            <ul className="divide-y divide-telegram-border overflow-hidden rounded-xl border border-telegram-border">
              {items.map(item => (
                <li key={item.trash_message_id} className="flex items-center gap-3 bg-telegram-surface px-3 py-2">
                  <input
                    type="checkbox"
                    checked={selectedFiles.includes(item.trash_message_id)}
                    onChange={() => toggleFile(item.trash_message_id)}
                    aria-label={`Select ${item.file_name}`}
                    className="h-3.5 w-3.5 shrink-0 accent-red-500"
                  />
                  <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-red-500/10 text-red-400">
                    <Trash2 className="h-3.5 w-3.5" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm text-telegram-text">{item.file_name}</p>
                    <p className="text-[11px] text-telegram-subtext">
                      {daysUntil(item.expires_at)} day{daysUntil(item.expires_at) !== 1 ? 's' : ''} left · {formatBytes(item.file_size)}
                    </p>
                  </div>
                  <button
                    type="button"
                    onClick={() => void handleRestore(item)}
                    disabled={busyId === item.trash_message_id}
                    className="flex items-center gap-1 rounded-lg px-2 py-1 text-xs font-medium text-emerald-400 hover:bg-telegram-hover disabled:opacity-50"
                  >
                    {busyId === item.trash_message_id ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RotateCcw className="h-3.5 w-3.5" />}
                    Restore
                  </button>
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>
    </div>
  );
}
