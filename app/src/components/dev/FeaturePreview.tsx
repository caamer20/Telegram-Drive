import { useState } from 'react';
import { HomeDashboard } from '../desktop/dashboard/HomeDashboard';
import { StorageStatsModal } from '../desktop/dashboard/StorageStatsModal';
import { TrashView } from '../desktop/dashboard/TrashView';
import { PropertiesModal } from '../desktop/dashboard/PropertiesModal';
import { CopyToFolderModal } from '../desktop/dashboard/CopyToFolderModal';
import { PREVIEW_FOLDERS, PREVIEW_DRIVE_STATS, installFeaturePreviewMock } from './featurePreviewData';

// Install the data-backed IPC mock as soon as this preview chunk loads,
// before the ported screens mount and fire their queries.
installFeaturePreviewMock();

type Tab = 'home' | 'storage' | 'trash' | 'copy' | 'properties';

const TABS: { id: Tab; label: string }[] = [
  { id: 'home', label: 'Home dashboard' },
  { id: 'storage', label: 'Storage stats' },
  { id: 'trash', label: 'Trash' },
  { id: 'copy', label: 'Copy to folder' },
  { id: 'properties', label: 'Properties' },
];

/**
 * Browser-only preview of the ported v1.8.x feature UI. Mounted at
 * `?feature-preview` and backed by mocked IPC data so the screens render
 * without a Tauri/Rust backend.
 */
export function FeaturePreview() {
  const [tab, setTab] = useState<Tab>('home');

  const sampleFile = { id: 42, name: 'holiday_video.mp4', size: 734_003_200, sizeStr: '700 MB', type: 'file' as const, created_at: '2026-11-01 18:20' };

  return (
    <div className="flex h-screen flex-col bg-app-canvas text-app-text">
      <div className="fixed inset-x-0 top-0 z-[500] flex flex-wrap items-center gap-1 border-b border-telegram-border bg-app-canvas/95 px-3 py-2 backdrop-blur">
        <span className="mr-2 text-xs font-semibold uppercase tracking-wider text-telegram-subtext">Feature preview</span>
        {TABS.map(t => (
          <button
            key={t.id}
            onClick={() => setTab(t.id)}
            className={`rounded-lg px-3 py-1.5 text-xs font-medium transition ${tab === t.id ? 'bg-telegram-primary/15 text-telegram-primary' : 'text-telegram-subtext hover:bg-telegram-hover hover:text-telegram-text'}`}
          >
            {t.label}
          </button>
        ))}
        <span className="ml-auto text-[10px] text-telegram-subtext/70">mocked data · no backend</span>
      </div>

      <div className="mt-11 flex min-h-0 flex-1">
        {tab === 'home' && (
          <HomeDashboard
            folders={PREVIEW_FOLDERS}
            onFolderClick={() => undefined}
            onTrashClick={() => setTab('trash')}
            onStorageStatsClick={() => setTab('storage')}
            onRenameFolder={() => undefined}
            onDeleteFolder={() => undefined}
          />
        )}
        {tab === 'trash' && <TrashView onClose={() => setTab('home')} />}
        {tab === 'storage' && (
          <div className="flex flex-1 items-center justify-center text-sm text-telegram-subtext">
            <StorageStatsModal driveStats={PREVIEW_DRIVE_STATS} folders={PREVIEW_FOLDERS} onClose={() => setTab('home')} />
          </div>
        )}
        {tab === 'copy' && (
          <div className="flex flex-1 items-center justify-center text-sm text-telegram-subtext">
            <CopyToFolderModal folders={PREVIEW_FOLDERS} activeFolderId={101} onSelect={() => setTab('home')} onClose={() => setTab('home')} />
          </div>
        )}
        {tab === 'properties' && (
          <div className="flex flex-1 items-center justify-center text-sm text-telegram-subtext">
            <PropertiesModal item={sampleFile} onClose={() => setTab('home')} />
          </div>
        )}
      </div>
    </div>
  );
}
