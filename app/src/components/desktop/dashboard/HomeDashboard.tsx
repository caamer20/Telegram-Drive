import { useEffect, useMemo, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { invoke } from '@tauri-apps/api/core';
import { motion, AnimatePresence } from 'framer-motion';
import {
  HardDrive, Image as ImageIcon, Film, FileText, Trash2, Folder,
  File as FileIcon, FileQuestion, Music, ArrowUpDown, ArrowUp, ArrowDown, Share2,
  Pencil, Info, Globe, Database, Layers, Palette, Check,
} from 'lucide-react';
import type { TelegramFolder, DriveStats, TrashStats, SharedFolderInfo } from '../../../types';
import { formatBytes } from '../../../utils';
import { PropertiesModal } from './PropertiesModal';

type FolderSortField = 'name' | 'items' | 'size';
type SortDirection = 'asc' | 'desc';

const FOLDER_COLORS = ['#3b82f6', '#8b5cf6', '#ec4899', '#f59e0b', '#22c55e', '#06b6d4', '#ef4444', '#64748b', '#f97316', '#14b8a6'];

interface HomeDashboardProps {
  folders: TelegramFolder[];
  onFolderClick: (folderId: number | null, folderName: string) => void;
  onTrashClick: () => void;
  onStorageStatsClick: () => void;
  onRenameFolder: (folderId: number, currentName: string) => void;
  onDeleteFolder: (folderId: number, name: string) => void;
  onShareFolder?: (folderId: number) => void;
  onClose?: () => void;
  selectedIds?: number[];
  onToggleSelect?: (folderId: number) => void;
}

interface CategoryDef {
  key: string;
  label: string;
  icon: typeof HardDrive;
  count: number;
  size: number;
  grad: string;
  iconColor: string;
  dot: string;
}

/**
 * Modern landing dashboard: a storage "hero", KPI tiles per media family with a
 * composition bar, and a sortable folder grid. All numbers come from
 * `cmd_get_drive_stats` / `cmd_get_trash_stats` / `cmd_list_shared_folders`.
 */
export function HomeDashboard({
  folders, onFolderClick, onTrashClick, onStorageStatsClick,
  onRenameFolder, onDeleteFolder, onShareFolder, onClose, selectedIds = [], onToggleSelect,
}: HomeDashboardProps) {
  const [sortField, setSortField] = useState<FolderSortField>('name');
  const [sortDirection, setSortDirection] = useState<SortDirection>('asc');
  const [folderMenu, setFolderMenu] = useState<{ x: number; y: number; folder: TelegramFolder } | null>(null);
  const [showPropertiesFolder, setShowPropertiesFolder] = useState<TelegramFolder | null>(null);
  const [showColorPicker, setShowColorPicker] = useState(false);
  const queryClient = useQueryClient();
  const { data: folderColors } = useQuery({
    queryKey: ['folderColors'],
    queryFn: () => invoke<Record<string, string>>('cmd_get_folder_colors'),
  });
  const setFolderColor = async (folderId: number, color: string | null) => {
    try {
      await invoke('cmd_set_folder_color', { folderId, color });
      await queryClient.invalidateQueries({ queryKey: ['folderColors'] });
    } catch { /* colour is best-effort */ }
  };

  useEffect(() => {
    if (!onClose) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const { data: driveStats } = useQuery({
    queryKey: ['driveStats'],
    queryFn: () => invoke<DriveStats>('cmd_get_drive_stats'),
    refetchInterval: 30000,
  });
  const { data: trashStats } = useQuery({
    queryKey: ['trashStats'],
    queryFn: () => invoke<TrashStats>('cmd_get_trash_stats'),
    refetchInterval: 30000,
  });
  const { data: sharedFolders } = useQuery({
    queryKey: ['sharedFolders'],
    queryFn: () => invoke<SharedFolderInfo[]>('cmd_list_shared_folders'),
    refetchInterval: 30000,
  });

  const global = driveStats?.global;
  const totalSize = global?.total_size || 0;
  const totalFiles = global?.total_files || 0;

  const categories: CategoryDef[] = useMemo(() => [
    { key: 'images', label: 'Images', icon: ImageIcon, count: global?.image_count || 0, size: global?.image_size || 0, grad: 'from-violet-500/25 to-violet-500/5', iconColor: 'text-violet-300', dot: 'bg-violet-400' },
    { key: 'videos', label: 'Videos', icon: Film, count: global?.video_count || 0, size: global?.video_size || 0, grad: 'from-rose-500/25 to-rose-500/5', iconColor: 'text-rose-300', dot: 'bg-rose-400' },
    { key: 'audio', label: 'Audio', icon: Music, count: global?.music_count || 0, size: global?.music_size || 0, grad: 'from-amber-500/25 to-amber-500/5', iconColor: 'text-amber-300', dot: 'bg-amber-400' },
    { key: 'docs', label: 'Documents', icon: FileText, count: global?.document_count || 0, size: global?.document_size || 0, grad: 'from-sky-500/25 to-sky-500/5', iconColor: 'text-sky-300', dot: 'bg-sky-400' },
    { key: 'others', label: 'Others', icon: FileQuestion, count: (global?.app_count || 0) + (global?.exe_count || 0) + (global?.other_count || 0), size: (global?.app_size || 0) + (global?.exe_size || 0) + (global?.other_size || 0), grad: 'from-slate-500/25 to-slate-500/5', iconColor: 'text-slate-300', dot: 'bg-slate-400' },
  ], [global]);

  const sharedStats = useMemo(() => {
    if (!sharedFolders) return { count: 0, total_size: 0 };
    return sharedFolders.reduce(
      (acc, s) => ({ count: acc.count + 1, total_size: acc.total_size + Number(s.totalSize) }),
      { count: 0, total_size: 0 },
    );
  }, [sharedFolders]);

  const sortedFolders = useMemo(() => {
    return folders.filter(f => !f.parent_id).slice().sort((a, b) => {
      const sa = driveStats?.folders[a.id.toString()];
      const sb = driveStats?.folders[b.id.toString()];
      let cmp = 0;
      if (sortField === 'name') cmp = a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' });
      else if (sortField === 'items') cmp = (sa?.total_files || 0) - (sb?.total_files || 0);
      else cmp = (sa?.total_size || 0) - (sb?.total_size || 0);
      return sortDirection === 'asc' ? cmp : -cmp;
    });
  }, [folders, driveStats, sortField, sortDirection]);

  // Per-folder totals that include nested folders, so a parent represents the
  // data its children hold as well.
  const aggregates = useMemo(() => {
    const children = new Map<number, number[]>();
    for (const f of folders) {
      const parent = f.parent_id ?? null;
      if (parent == null) continue;
      const list = children.get(parent) ?? [];
      list.push(f.id);
      children.set(parent, list);
    }
    const cache = new Map<number, { files: number; size: number; folders: number }>();
    const compute = (id: number, seen: Set<number>): { files: number; size: number; folders: number } => {
      const cached = cache.get(id);
      if (cached) return cached;
      if (seen.has(id)) return { files: 0, size: 0, folders: 0 };
      seen.add(id);
      const own = driveStats?.folders[String(id)];
      let files = own?.total_files ?? 0;
      let size = own?.total_size ?? 0;
      let folders = 0;
      for (const childId of children.get(id) ?? []) {
        const child = compute(childId, seen);
        files += child.files;
        size += child.size;
        folders += 1 + child.folders;
      }
      const result = { files, size, folders };
      cache.set(id, result);
      return result;
    };
    const map = new Map<number, { files: number; size: number; folders: number }>();
    for (const f of folders) map.set(f.id, compute(f.id, new Set<number>()));
    return map;
  }, [folders, driveStats]);

  const handleSort = (field: FolderSortField) => {
    if (sortField === field) setSortDirection(d => (d === 'asc' ? 'desc' : 'asc'));
    else { setSortField(field); setSortDirection('asc'); }
  };

  const SortIcon = ({ field }: { field: FolderSortField }) => {
    if (sortField !== field) return <ArrowUpDown className="h-3 w-3 opacity-40" />;
    return sortDirection === 'asc'
      ? <ArrowUp className="h-3 w-3 text-telegram-primary" />
      : <ArrowDown className="h-3 w-3 text-telegram-primary" />;
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-app-canvas">
      <div className="flex-1 overflow-y-auto p-6" onClick={() => setFolderMenu(null)}>
        <div className="mx-auto max-w-[1400px] space-y-6">

          {/* Hero */}
          <motion.div
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            className="relative overflow-hidden rounded-3xl border border-telegram-border bg-gradient-to-br from-telegram-primary/15 via-telegram-surface to-telegram-surface p-6 shadow-xl"
          >
            <div className="pointer-events-none absolute -right-16 -top-16 h-48 w-48 rounded-full bg-telegram-primary/20 blur-3xl" />
            <div className="relative flex flex-wrap items-end justify-between gap-6">
              <div>
                <p className="text-[11px] font-semibold uppercase tracking-widest text-telegram-subtext">Total used</p>
                <p className="mt-1 text-4xl font-extrabold tabular-nums text-telegram-text">{formatBytes(totalSize)}</p>
                <p className="mt-1 text-sm text-telegram-subtext">{totalFiles.toLocaleString()} files across {folders.length} folders</p>
              </div>
              <div className="flex gap-3">
                <button
                  onClick={onStorageStatsClick}
                  className="rounded-xl border border-telegram-border bg-telegram-hover/60 px-4 py-3 text-left transition hover:border-telegram-primary/40 hover:bg-telegram-hover"
                >
                  <Database className="mb-1 h-4 w-4 text-telegram-primary" />
                  <p className="text-lg font-bold tabular-nums text-telegram-text">{totalFiles.toLocaleString()}</p>
                  <p className="text-[10px] uppercase tracking-wide text-telegram-subtext">Total files</p>
                </button>
                <button
                  onClick={onTrashClick}
                  className="rounded-xl border border-red-500/20 bg-red-500/5 px-4 py-3 text-left transition hover:border-red-500/40 hover:bg-red-500/10"
                >
                  <Trash2 className="mb-1 h-4 w-4 text-red-400" />
                  <p className="text-lg font-bold tabular-nums text-telegram-text">{trashStats?.count ?? 0}</p>
                  <p className="text-[10px] uppercase tracking-wide text-telegram-subtext">In trash</p>
                </button>
              </div>
            </div>

            {/* Composition bar */}
            <div className="relative mt-6">
              <div className="flex h-2.5 w-full overflow-hidden rounded-full bg-telegram-border/40">
                {categories.map(c => (
                  c.size > 0 ? (
                    <div
                      key={c.key}
                      className={`${c.dot} h-full transition-all`}
                      style={{ width: `${totalSize > 0 ? (c.size / totalSize) * 100 : 0}%` }}
                      title={`${c.label}: ${formatBytes(c.size)}`}
                    />
                  ) : null
                ))}
              </div>
              <div className="mt-3 flex flex-wrap gap-x-5 gap-y-1">
                {categories.map(c => (
                  <span key={c.key} className="flex items-center gap-1.5 text-[11px] text-telegram-subtext">
                    <span className={`h-2 w-2 rounded-full ${c.dot}`} />
                    {c.label}
                    <span className="font-semibold text-telegram-text/80">{totalSize > 0 ? ((c.size / totalSize) * 100).toFixed(0) : 0}%</span>
                  </span>
                ))}
              </div>
            </div>
          </motion.div>

          {/* KPI grid */}
          <section>
            <h2 className="mb-3 text-xs font-semibold uppercase tracking-widest text-telegram-subtext">By file type</h2>
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-5">
              {categories.map((c, i) => {
                const Icon = c.icon;
                const pct = totalSize > 0 ? (c.size / totalSize) * 100 : 0;
                return (
                  <motion.button
                    key={c.key}
                    initial={{ opacity: 0, y: 10 }}
                    animate={{ opacity: 1, y: 0 }}
                    transition={{ delay: i * 0.04 }}
                    onClick={onStorageStatsClick}
                    className={`group relative overflow-hidden rounded-2xl border border-telegram-border bg-gradient-to-br ${c.grad} p-4 text-left transition hover:border-telegram-primary/30 hover:shadow-lg`}
                  >
                    <div className="flex items-center justify-between">
                      <div className={`rounded-xl bg-white/5 p-2 ${c.iconColor} ring-1 ring-white/10 transition-transform group-hover:scale-110`}>
                        <Icon className="h-5 w-5" />
                      </div>
                      <span className="text-[11px] font-semibold tabular-nums text-telegram-subtext">{pct.toFixed(1)}%</span>
                    </div>
                    <p className="mt-3 text-sm font-semibold text-telegram-text">{c.label}</p>
                    <p className="text-xl font-bold tabular-nums text-telegram-text">{formatBytes(c.size)}</p>
                    <p className="text-[11px] text-telegram-subtext">{c.count.toLocaleString()} items</p>
                    <div className="mt-2 h-1 w-full overflow-hidden rounded-full bg-white/10">
                      <div className={`${c.dot} h-full rounded-full transition-all`} style={{ width: `${pct}%` }} />
                    </div>
                  </motion.button>
                );
              })}
            </div>
          </section>

          {/* Trash + Shared compact */}
          <section className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <button
              onClick={onTrashClick}
              className="group flex items-center gap-4 rounded-2xl border border-red-500/20 bg-telegram-surface p-4 text-left transition hover:border-red-500/40 hover:bg-red-500/5"
            >
              <div className="rounded-xl bg-red-500/10 p-3 text-red-400 transition group-hover:bg-red-500 group-hover:text-white">
                <Trash2 className="h-5 w-5" />
              </div>
              <div className="min-w-0 flex-1">
                <p className="text-sm font-semibold text-telegram-text">Trash</p>
                <p className="text-xs text-telegram-subtext">{trashStats?.count ?? 0} items · {trashStats ? formatBytes(trashStats.total_size) : '…'} · 20-day retention</p>
              </div>
              <Trash2 className="h-4 w-4 text-telegram-subtext transition group-hover:translate-x-0.5 group-hover:text-red-400" />
            </button>
            <div className="flex items-center gap-4 rounded-2xl border border-telegram-border bg-telegram-surface p-4">
              <div className="rounded-xl bg-telegram-primary/10 p-3 text-telegram-primary">
                <Globe className="h-5 w-5" />
              </div>
              <div className="min-w-0 flex-1">
                <p className="text-sm font-semibold text-telegram-text">Shared folders</p>
                <p className="text-xs text-telegram-subtext">{sharedStats.count} shared · {sharedStats.count > 0 ? formatBytes(sharedStats.total_size) : 'nothing shared yet'}</p>
              </div>
            </div>
          </section>

          {/* Folders */}
          <section>
            <div className="mb-3 flex items-center justify-between">
              <div className="flex items-center gap-4">
                <h2 className="flex items-center gap-2 text-xs font-semibold uppercase tracking-widest text-telegram-subtext">
                  <Layers className="h-3.5 w-3.5" />
                  My folders
                </h2>
                <div className="flex items-center gap-1.5 text-[11px] text-telegram-subtext">
                  <button onClick={() => handleSort('name')} className={`flex items-center gap-1 rounded-md px-1.5 py-1 transition hover:text-telegram-text ${sortField === 'name' ? 'bg-telegram-hover font-bold text-telegram-primary' : ''}`}>Name <SortIcon field="name" /></button>
                  <button onClick={() => handleSort('items')} className={`flex items-center gap-1 rounded-md px-1.5 py-1 transition hover:text-telegram-text ${sortField === 'items' ? 'bg-telegram-hover font-bold text-telegram-primary' : ''}`}>Items <SortIcon field="items" /></button>
                  <button onClick={() => handleSort('size')} className={`flex items-center gap-1 rounded-md px-1.5 py-1 transition hover:text-telegram-text ${sortField === 'size' ? 'bg-telegram-hover font-bold text-telegram-primary' : ''}`}>Size <SortIcon field="size" /></button>
                </div>
              </div>
            </div>

            <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6">
              <motion.button
                initial={{ opacity: 0, scale: 0.96 }}
                animate={{ opacity: 1, scale: 1 }}
                onClick={() => onFolderClick(null, 'Saved Messages')}
                className="group flex flex-col gap-3 rounded-2xl border border-telegram-border bg-telegram-surface p-4 text-left transition hover:-translate-y-0.5 hover:border-telegram-primary/40 hover:shadow-lg"
              >
                <div className="flex items-center gap-2.5 overflow-hidden">
                  <div className="shrink-0 rounded-xl bg-gradient-to-br from-blue-500/25 to-blue-500/5 p-2 text-blue-300 transition-transform group-hover:scale-110">
                    <Folder className="h-5 w-5 fill-current" />
                  </div>
                  <h3 className="truncate text-sm font-semibold text-telegram-text">Saved Messages</h3>
                </div>
                <div className="flex items-center justify-between text-[11px] text-telegram-subtext">
                  <span className="font-bold tabular-nums text-telegram-text/80">{driveStats?.folders['null']?.total_files || 0} items</span>
                  <span className="font-medium tabular-nums">{formatBytes(driveStats?.folders['null']?.total_size || 0)}</span>
                </div>
              </motion.button>

              {sortedFolders.map((folder, index) => {
                const fStats = driveStats?.folders[folder.id.toString()];
                const accent = folderColors?.[folder.id.toString()];
                const isSelected = selectedIds.includes(folder.id);
                const childCount = folders.filter(f => f.parent_id === folder.id).length;
                const agg = aggregates.get(folder.id) ?? { files: fStats?.total_files ?? 0, size: fStats?.total_size ?? 0, folders: childCount };
                return (
                  <motion.button
                    key={folder.id}
                    initial={{ opacity: 0, scale: 0.96 }}
                    animate={{ opacity: 1, scale: 1 }}
                    transition={{ delay: index * 0.03 }}
                    onClick={() => onFolderClick(folder.id, folder.name)}
                    onContextMenu={e => { e.preventDefault(); e.stopPropagation(); setShowColorPicker(false); setFolderMenu({ x: e.clientX, y: e.clientY, folder }); }}
                    className={`group relative flex flex-col gap-3 rounded-2xl border bg-telegram-surface p-4 text-left transition hover:-translate-y-0.5 hover:shadow-lg ${isSelected ? 'border-telegram-primary bg-telegram-primary/5 ring-1 ring-telegram-primary/40' : 'border-telegram-border hover:border-telegram-primary/40'}`}
                  >
                    {onToggleSelect && (
                      <button
                        type="button"
                        aria-label={isSelected ? 'Deselect folder' : 'Select folder'}
                        aria-pressed={isSelected}
                        onClick={e => { e.stopPropagation(); onToggleSelect(folder.id); }}
                        className={`absolute end-2 top-2 z-10 flex h-5 w-5 items-center justify-center rounded-md border transition ${isSelected ? 'border-telegram-primary bg-telegram-primary text-white opacity-100' : 'border-white/30 bg-black/30 text-transparent opacity-0 group-hover:opacity-100'}`}
                      >
                        <Check className="h-3.5 w-3.5" />
                      </button>
                    )}
                    <div className="flex items-center gap-2.5 overflow-hidden">
                      <div
                        className={`shrink-0 rounded-xl p-2 transition-transform group-hover:scale-110 ${accent ? '' : 'bg-gradient-to-br from-telegram-primary/25 to-telegram-primary/5 text-telegram-primary'}`}
                        style={accent ? { color: accent, backgroundColor: `${accent}22` } : undefined}
                      >
                        <Folder className="h-5 w-5 fill-current" />
                      </div>
                      <h3 className="truncate text-sm font-semibold text-telegram-text" title={folder.name}>{folder.name}</h3>
                      {folder.is_public && (
                        <span className="ml-auto shrink-0 rounded bg-green-500/15 px-1.5 py-0.5 text-[8px] font-bold uppercase text-green-400">Public</span>
                      )}
                    </div>
                    <div className="flex items-center justify-between text-[11px] text-telegram-subtext">
                      <span className="flex items-center gap-2.5 font-bold tabular-nums text-telegram-text/80">
                        <span className="inline-flex items-center gap-1" title={`${agg.files} files`}>
                          <FileIcon className="h-3 w-3 text-sky-400" />{agg.files}
                        </span>
                        <span className="inline-flex items-center gap-1" title={`${agg.folders} folders`}>
                          <Folder className="h-3 w-3 fill-current text-amber-400" />{agg.folders}
                        </span>
                      </span>
                      <span className="font-medium tabular-nums">{formatBytes(agg.size)}</span>
                    </div>
                  </motion.button>
                );
              })}
            </div>

            {folders.length === 0 && (
              <div className="py-12 text-center text-telegram-subtext">
                <Folder className="mx-auto mb-2 h-10 w-10 opacity-20" />
                <p className="text-sm">No custom folders found. Click "New Folder" to create one.</p>
              </div>
            )}
          </section>
        </div>

        {/* Folder context menu */}
        <AnimatePresence>
          {folderMenu && (
            <motion.div
              initial={{ opacity: 0, scale: 0.95 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.95 }}
              style={{ top: folderMenu.y, left: folderMenu.x }}
              className="fixed z-[100] w-48 overflow-hidden rounded-xl border border-telegram-border bg-telegram-surface py-1.5 shadow-2xl"
              onClick={e => e.stopPropagation()}
            >
              <button onClick={() => { onRenameFolder(folderMenu.folder.id, folderMenu.folder.name); setFolderMenu(null); }} className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs font-medium text-telegram-text hover:bg-telegram-hover">
                <Pencil className="h-3.5 w-3.5" /> Rename Folder
              </button>
              <button onClick={() => setShowColorPicker(v => !v)} className="flex w-full items-center justify-between gap-2 px-3 py-2 text-left text-xs font-medium text-telegram-text hover:bg-telegram-hover">
                <span className="flex items-center gap-2"><Palette className="h-3.5 w-3.5 text-fuchsia-400" /> Change Color</span>
                <span className="text-[10px] opacity-50">{showColorPicker ? '▾' : '▸'}</span>
              </button>
              {showColorPicker && (
                <div className="grid grid-cols-5 gap-1.5 border-y border-telegram-border/50 bg-telegram-hover/30 px-3 py-2">
                  {FOLDER_COLORS.map(hex => {
                    const active = folderColors?.[folderMenu.folder.id.toString()] === hex;
                    return (
                      <button
                        key={hex}
                        title={hex}
                        onClick={() => { void setFolderColor(folderMenu.folder.id, hex); setFolderMenu(null); }}
                        className="flex aspect-square items-center justify-center rounded-md ring-1 ring-white/10 transition hover:scale-110"
                        style={{ backgroundColor: hex }}
                      >
                        {active && <Check className="h-3 w-3 text-white" />}
                      </button>
                    );
                  })}
                  <button onClick={() => { void setFolderColor(folderMenu.folder.id, null); setFolderMenu(null); }} className="col-span-5 mt-1 rounded-md bg-telegram-hover px-2 py-1 text-[10px] text-telegram-subtext hover:text-telegram-text">Reset color</button>
                </div>
              )}
              {onShareFolder && (
                <button onClick={() => { onShareFolder(folderMenu.folder.id); setFolderMenu(null); }} className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs font-medium text-telegram-text hover:bg-telegram-hover">
                  <Share2 className="h-3.5 w-3.5" /> Share Folder
                </button>
              )}
              <button onClick={() => { setShowPropertiesFolder(folderMenu.folder); setFolderMenu(null); }} className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs font-medium text-telegram-text hover:bg-telegram-hover">
                <Info className="h-3.5 w-3.5 text-blue-400" /> Properties
              </button>
              <div className="my-1 h-px bg-telegram-border" />
              <button onClick={() => { onDeleteFolder(folderMenu.folder.id, folderMenu.folder.name); setFolderMenu(null); }} className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs font-medium text-red-400 hover:bg-red-500/10">
                <Trash2 className="h-3.5 w-3.5" /> Delete Folder
              </button>
            </motion.div>
          )}
        </AnimatePresence>

        {showPropertiesFolder && (
          <PropertiesModal item={showPropertiesFolder} onClose={() => setShowPropertiesFolder(null)} />
        )}
      </div>
    </div>
  );
}
