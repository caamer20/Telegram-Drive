import { PieChart, Pie, Cell, ResponsiveContainer, Tooltip } from 'recharts';
import {
  X, Database, HardDrive, FolderTree, PieChart as PieIcon,
} from 'lucide-react';
import type { TelegramFolder, DriveStats } from '../../../types';
import { formatBytes } from '../../../utils';

interface StorageStatsModalProps {
  driveStats: DriveStats | undefined;
  folders: TelegramFolder[];
  onClose: () => void;
}

interface Slice { name: string; value: number; count: number; color: string }

const COLORS = ['#a78bfa', '#fb7185', '#fbbf24', '#38bdf8', '#94a3b8'];

/** Storage analytics: donut composition, per-type breakdown and folder bars. */
export function StorageStatsModal({ driveStats, folders, onClose }: StorageStatsModalProps) {
  const global = driveStats?.global;
  const folderStats = driveStats?.folders ?? {};
  const totalSize = global?.total_size || 0;

  const slices: Slice[] = global ? [
    { name: 'Images', value: global.image_size, count: global.image_count, color: COLORS[0] },
    { name: 'Videos', value: global.video_size, count: global.video_count, color: COLORS[1] },
    { name: 'Audio', value: global.music_size, count: global.music_count, color: COLORS[2] },
    { name: 'Documents', value: global.document_size, count: global.document_count, color: COLORS[3] },
    { name: 'Others', value: global.app_size + global.exe_size + global.other_size, count: global.app_count + global.exe_count + global.other_count, color: COLORS[4] },
  ].filter(s => s.value > 0) : [];

  const folderBars = folders
    .filter(f => !f.parent_id)
    .map(f => ({ name: f.name, size: folderStats[f.id.toString()]?.total_size || 0, files: folderStats[f.id.toString()]?.total_files || 0 }))
    .sort((a, b) => b.size - a.size)
    .slice(0, 8);
  const maxFolder = Math.max(1, ...folderBars.map(f => f.size));

  const tiles = [
    { label: 'Total files', value: (global?.total_files || 0).toLocaleString(), icon: Database, grad: 'from-blue-500/20 to-blue-500/5', color: 'text-blue-300' },
    { label: 'Total size', value: formatBytes(totalSize), icon: HardDrive, grad: 'from-violet-500/20 to-violet-500/5', color: 'text-violet-300' },
    { label: 'Folders', value: String(folders.filter(f => !f.parent_id).length), icon: FolderTree, grad: 'from-emerald-500/20 to-emerald-500/5', color: 'text-emerald-300' },
  ];

  return (
    <div className="fixed inset-0 z-[150] flex items-center justify-center bg-black/60 p-4 backdrop-blur-md" onClick={onClose}>
      <div
        onClick={e => e.stopPropagation()}
        className="glass max-h-[88vh] w-full max-w-3xl overflow-y-auto rounded-3xl border border-telegram-border bg-telegram-surface shadow-2xl"
      >
        {/* Header */}
        <div className="sticky top-0 z-10 flex items-center justify-between border-b border-telegram-border bg-telegram-surface/95 px-6 py-4 backdrop-blur">
          <div className="flex items-center gap-3">
            <div className="rounded-xl bg-telegram-primary/15 p-2 text-telegram-primary">
              <PieIcon className="h-5 w-5" />
            </div>
            <div>
              <h2 className="text-lg font-bold text-telegram-text">Storage Analytics</h2>
              <p className="text-xs text-telegram-subtext">How your drive space is being used</p>
            </div>
          </div>
          <button onClick={onClose} aria-label="Close" className="rounded-lg p-1.5 text-telegram-subtext transition hover:bg-telegram-hover hover:text-telegram-text">
            <X className="h-5 w-5" />
          </button>
        </div>

        <div className="space-y-6 p-6">
          {/* Tiles */}
          <div className="grid grid-cols-3 gap-3">
            {tiles.map(t => {
              const Icon = t.icon;
              return (
                <div key={t.label} className={`rounded-2xl border border-telegram-border bg-gradient-to-br ${t.grad} p-4`}>
                  <Icon className={`mb-2 h-5 w-5 ${t.color}`} />
                  <p className="text-xl font-extrabold tabular-nums text-telegram-text">{t.value}</p>
                  <p className="text-[10px] uppercase tracking-widest text-telegram-subtext">{t.label}</p>
                </div>
              );
            })}
          </div>

          {/* Donut + legend */}
          <div className="grid grid-cols-1 gap-6 md:grid-cols-2">
            <div className="relative rounded-2xl border border-telegram-border bg-telegram-hover/40 p-4">
              <div className="relative">
                <ResponsiveContainer width="100%" height={230}>
                  <PieChart>
                    <Pie data={slices} cx="50%" cy="50%" innerRadius={72} outerRadius={100} paddingAngle={3} dataKey="value" stroke="none">
                      {slices.map((s, i) => <Cell key={i} fill={s.color} />)}
                    </Pie>
                    <Tooltip
                      contentStyle={{ background: 'rgba(20,20,24,0.95)', border: '1px solid rgba(255,255,255,0.1)', borderRadius: 12, fontSize: 12 }}
                      formatter={(value) => formatBytes(Number(value))}
                    />
                  </PieChart>
                </ResponsiveContainer>
                <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center">
                  <span className="text-[10px] uppercase tracking-widest text-telegram-subtext">Used</span>
                  <span className="text-xl font-extrabold tabular-nums text-telegram-text">{formatBytes(totalSize)}</span>
                </div>
              </div>
            </div>

            <div className="space-y-3">
              {slices.map(s => {
                const pct = totalSize > 0 ? (s.value / totalSize) * 100 : 0;
                return (
                  <div key={s.name} className="rounded-xl border border-telegram-border bg-telegram-hover/30 p-3">
                    <div className="flex items-center justify-between text-sm">
                      <span className="flex items-center gap-2 font-medium text-telegram-text">
                        <span className="h-2.5 w-2.5 rounded-full" style={{ background: s.color }} />
                        {s.name}
                      </span>
                      <span className="tabular-nums text-telegram-text">{formatBytes(s.value)}</span>
                    </div>
                    <div className="mt-2 flex items-center gap-2">
                      <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-white/10">
                        <div className="h-full rounded-full transition-all" style={{ width: `${pct}%`, background: s.color }} />
                      </div>
                      <span className="w-16 text-right text-[11px] tabular-nums text-telegram-subtext">{pct.toFixed(1)}% · {s.count.toLocaleString()}</span>
                    </div>
                  </div>
                );
              })}
              {slices.length === 0 && (
                <p className="py-8 text-center text-sm text-telegram-subtext">No storage data yet.</p>
              )}
            </div>
          </div>

          {/* Folder bars */}
          {folderBars.length > 0 && (
            <div>
              <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold text-telegram-text">
                <FolderTree className="h-4 w-4 text-telegram-primary" />
                Largest folders
              </h3>
              <div className="space-y-2.5">
                {folderBars.map(f => (
                  <div key={f.name} className="flex items-center gap-3">
                    <span className="w-28 shrink-0 truncate text-xs font-medium text-telegram-text" title={f.name}>{f.name}</span>
                    <div className="h-2.5 flex-1 overflow-hidden rounded-full bg-telegram-border/40">
                      <div className="h-full rounded-full bg-gradient-to-r from-telegram-primary to-fuchsia-500 transition-all" style={{ width: `${(f.size / maxFolder) * 100}%` }} />
                    </div>
                    <span className="w-20 shrink-0 text-right text-[11px] tabular-nums text-telegram-subtext">{formatBytes(f.size)}</span>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
