import { X, Info, Calendar, HardDrive, Hash, Type, FileText, Folder, CheckCircle2 } from 'lucide-react';
import { motion } from 'framer-motion';
import type { TelegramFile, TelegramFolder } from '../../../types';
import { formatBytes } from '../../../utils';

interface PropertiesModalProps {
  item?: TelegramFile | TelegramFolder;
  items?: (TelegramFile | TelegramFolder)[];
  onClose: () => void;
}

function isFolder(item: TelegramFile | TelegramFolder): item is TelegramFolder {
  return 'is_public' in item;
}

/** Read-only "Properties" sheet for a single file/folder or a bulk selection. */
export function PropertiesModal({ item, items, onClose }: PropertiesModalProps) {
  const isBulk = !!items && items.length > 1;
  const effectiveItems = items ?? (item ? [item] : []);

  if (effectiveItems.length === 0) return null;

  const firstItem = effectiveItems[0];
  const folderSelected = isFolder(firstItem);

  const title = isBulk
    ? `${effectiveItems.length} Items Selected`
    : folderSelected ? 'Folder Properties' : 'File Properties';

  const totalSize = effectiveItems.reduce((acc, curr) => {
    if ('size' in curr && typeof curr.size === 'number') return acc + curr.size;
    return acc;
  }, 0);

  const createdAt = !isBulk && !folderSelected ? firstItem.created_at : undefined;

  return (
    <div className="fixed inset-0 z-[200] flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm" onClick={onClose}>
      <motion.div
        initial={{ opacity: 0, scale: 0.9, y: 20 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        className="w-full max-w-md overflow-hidden rounded-2xl border border-telegram-border bg-telegram-surface shadow-2xl"
        onClick={e => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-telegram-border bg-telegram-hover/30 p-4">
          <div className="flex items-center gap-2">
            <Info className="h-5 w-5 text-telegram-primary" />
            <h3 className="font-semibold text-telegram-text">{title}</h3>
          </div>
          <button
            onClick={onClose}
            aria-label="Close"
            className="rounded-full p-1.5 text-telegram-subtext transition-colors hover:bg-white/10 hover:text-telegram-text"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        <div className="space-y-6 p-6">
          <div className="flex items-center gap-4">
            <div className="rounded-2xl bg-telegram-primary/10 p-4 text-telegram-primary">
              {isBulk ? <CheckCircle2 className="h-10 w-10" /> : folderSelected ? <Folder className="h-10 w-10 fill-current" /> : <FileText className="h-10 w-10" />}
            </div>
            <div className="min-w-0">
              <h4 className="mb-1 truncate text-lg font-bold text-telegram-text">
                {isBulk ? `${effectiveItems.length} Selected Items` : firstItem.name}
              </h4>
              <p className="text-sm text-telegram-subtext">
                {isBulk ? 'Bulk Selection' : folderSelected ? 'Telegram Channel' : 'File'}
              </p>
            </div>
          </div>

          <div className="grid grid-cols-1 gap-4">
            {!isBulk && <PropertyItem icon={Hash} label="ID" value={firstItem.id.toString()} />}
            {!isBulk && !folderSelected && 'type' in firstItem && firstItem.type && (
              <PropertyItem icon={Type} label="Type" value={firstItem.type} />
            )}
            {totalSize > 0 && <PropertyItem icon={HardDrive} label="Total Size" value={formatBytes(totalSize)} />}
            {folderSelected && totalSize === 0 && (
              <PropertyItem icon={HardDrive} label="Total Size" value="Check storage in Telegram" />
            )}
            {createdAt && <PropertyItem icon={Calendar} label="Created At" value={createdAt} />}

            {!isBulk && folderSelected && (
              <>
                <PropertyItem icon={Type} label="Visibility" value={firstItem.is_public ? 'Public' : 'Private'} />
                {firstItem.username && <PropertyItem icon={Hash} label="Username" value={`@${firstItem.username}`} />}
              </>
            )}
          </div>
        </div>

        <div className="border-t border-telegram-border bg-telegram-hover/10 p-4">
          <button
            onClick={onClose}
            className="w-full rounded-xl bg-telegram-primary py-2.5 font-semibold text-white transition-all hover:bg-telegram-primary/90 active:scale-[0.98]"
          >
            Close
          </button>
        </div>
      </motion.div>
    </div>
  );
}

function PropertyItem({ icon: Icon, label, value }: { icon: typeof Info; label: string; value: string }) {
  return (
    <div className="flex items-center gap-3 rounded-xl border border-telegram-border/50 bg-telegram-hover/30 p-3">
      <div className="rounded-lg border border-telegram-border/50 bg-telegram-surface p-2">
        <Icon className="h-4 w-4 text-telegram-subtext" />
      </div>
      <div>
        <p className="mb-0.5 text-[10px] font-bold uppercase tracking-wider text-telegram-subtext">{label}</p>
        <p className="select-text text-sm font-medium text-telegram-text">{value}</p>
      </div>
    </div>
  );
}
