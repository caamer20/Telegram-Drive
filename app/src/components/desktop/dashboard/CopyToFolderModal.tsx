import { X, HardDrive, Folder } from 'lucide-react';
import type { TelegramFolder } from '../../../types';

interface CopyToFolderModalProps {
  folders: TelegramFolder[];
  onClose: () => void;
  onSelect: (id: number | null) => void;
  activeFolderId: number | null;
}

/** Folder picker used for "Copy to…". Unlike Move, the source is untouched. */
export function CopyToFolderModal({ folders, onClose, onSelect, activeFolderId }: CopyToFolderModalProps) {
  const sorted = [...folders].sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' }));

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/50 backdrop-blur-sm" onClick={onClose}>
      <div
        className="flex max-h-[80vh] w-80 flex-col overflow-hidden rounded-xl border border-telegram-border bg-telegram-surface shadow-2xl"
        onClick={e => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-telegram-border p-4">
          <h3 className="font-medium text-telegram-text">Copy to Folder</h3>
          <button onClick={onClose} aria-label="Close" className="text-telegram-subtext hover:text-telegram-text">
            <X className="h-5 w-5" />
          </button>
        </div>
        <div className="flex-1 space-y-1 overflow-y-auto p-2">
          {activeFolderId !== null && (
            <button
              onClick={() => onSelect(null)}
              className="flex w-full items-center gap-3 rounded-lg px-3 py-3 text-left text-sm text-telegram-text transition-colors hover:bg-telegram-hover"
            >
              <div className="flex h-8 w-8 items-center justify-center rounded bg-telegram-primary/20 text-telegram-primary">
                <HardDrive className="h-4 w-4" />
              </div>
              <span className="font-medium">Saved Messages</span>
            </button>
          )}

          {sorted.map(f => {
            if (f.id === activeFolderId) return null;
            return (
              <button
                key={f.id}
                onClick={() => onSelect(f.id)}
                className="flex w-full items-center gap-3 rounded-lg px-3 py-3 text-left text-sm text-telegram-text transition-colors hover:bg-telegram-hover"
              >
                <div className="flex h-8 w-8 items-center justify-center rounded bg-telegram-hover text-telegram-text">
                  <Folder className="h-4 w-4" />
                </div>
                <span className="font-medium">{f.name}</span>
              </button>
            );
          })}

          {folders.length === 0 && activeFolderId === null && (
            <div className="p-4 text-center text-xs text-telegram-subtext">No other folders available. Create one first!</div>
          )}
        </div>
      </div>
    </div>
  );
}
