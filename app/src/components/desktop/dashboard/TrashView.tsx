import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowLeft, RotateCcw, Trash2, Loader2, Clock3 } from 'lucide-react';
import type { TrashItem, TrashStats } from '../../../types';
import { formatBytes } from '../../../utils';

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
  const [stats, setStats] = useState<TrashStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [busyId, setBusyId] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [list, trashStats] = await Promise.all([
        invoke<TrashItem[]>('cmd_list_trash'),
        invoke<TrashStats>('cmd_get_trash_stats'),
      ]);
      setItems(list);
      setStats(trashStats);
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
          <ul className="mx-auto max-w-3xl space-y-2">
            {items.map(item => (
              <li
                key={item.trash_message_id}
                className="flex items-center gap-3 rounded-xl border border-telegram-border bg-telegram-hover/40 p-3"
              >
                <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-red-500/10 text-red-400">
                  <Trash2 className="h-4 w-4" />
                </div>
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-medium text-telegram-text">{item.file_name}</p>
                  <p className="flex items-center gap-1 text-[11px] text-telegram-subtext">
                    <Clock3 className="h-3 w-3" />
                    {daysUntil(item.expires_at)} day{daysUntil(item.expires_at) !== 1 ? 's' : ''} left · {formatBytes(item.file_size)}
                  </p>
                </div>
                <button
                  onClick={() => void handleRestore(item)}
                  disabled={busyId === item.trash_message_id}
                  className="flex items-center gap-1.5 rounded-lg bg-telegram-primary/10 px-3 py-1.5 text-xs font-medium text-telegram-primary transition hover:bg-telegram-primary/20 disabled:opacity-50"
                >
                  {busyId === item.trash_message_id ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RotateCcw className="h-3.5 w-3.5" />}
                  Restore
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
