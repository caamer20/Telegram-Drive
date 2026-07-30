import { useCallback, useState } from 'react';
import { Images, Loader2 } from 'lucide-react';
import { toast } from 'sonner';
import { openMediaLibrary } from '../../media-library';

export function MediaLibraryButton({ isAndroid }: { isAndroid: boolean }) {
  const [isOpeningMediaLibrary, setIsOpeningMediaLibrary] = useState(false);

  const handleOpenMediaLibrary = useCallback(async () => {
    if (isOpeningMediaLibrary) return;

    setIsOpeningMediaLibrary(true);
    try {
      const result = await openMediaLibrary();
      if (result.exitReason === 'error' || result.error) {
        console.error('[MediaLibrary] Native gallery returned an error', result);
        toast.error(result.error || 'Unable to open media gallery');
      }
    } catch (error) {
      console.error('[MediaLibrary] Failed to open native gallery', error);
      toast.error('Unable to open media gallery');
    } finally {
      setIsOpeningMediaLibrary(false);
    }
  }, [isOpeningMediaLibrary]);

  if (!isAndroid) return null;

  return (
    <button
      type="button"
      onClick={handleOpenMediaLibrary}
      disabled={isOpeningMediaLibrary}
      aria-label="Open media gallery"
      className="p-2 rounded-xl bg-telegram-hover/30 hover:bg-telegram-hover/60 border border-telegram-border/40 text-telegram-subtext transition-all duration-300 disabled:cursor-not-allowed disabled:opacity-60"
    >
      {isOpeningMediaLibrary ? (
        <Loader2 className="w-5 h-5 animate-spin" aria-hidden="true" />
      ) : (
        <Images className="w-5 h-5" aria-hidden="true" />
      )}
    </button>
  );
}
