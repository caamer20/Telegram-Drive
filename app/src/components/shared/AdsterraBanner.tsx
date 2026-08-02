import { useCallback, useState, useEffect } from 'react';
import { usePlatform } from '../../hooks/usePlatform';
import { open } from '@tauri-apps/plugin-shell';
import { load } from '@tauri-apps/plugin-store';
import { ExternalLink, X } from 'lucide-react';

interface AdsterraBannerProps {
  visible: boolean;
}

const SMARTLINK_URL = 'https://www.effectivecpmnetwork.com/nk8qy01t0g?key=a6c132f628973ad13b326e57e4a92f40';
const DISMISSED_KEY = 'adBannerDismissed';

/** SmartLink clickable banner for Android. Tapping opens the offerwall in an external browser. */
export default function AdsterraBanner({ visible }: AdsterraBannerProps) {
  const { isAndroid } = usePlatform();
  const [dismissed, setDismissed] = useState(false);
  const [exiting, setExiting] = useState(false);
  const [loaded, setLoaded] = useState(false);

  // Restore persisted dismissal state on mount
  useEffect(() => {
    let cancelled = false;
    load('config.json')
      .then((store) => store.get<boolean>(DISMISSED_KEY))
      .then((wasDismissed) => {
        if (!cancelled && wasDismissed) setDismissed(true);
        if (!cancelled) setLoaded(true);
      })
      .catch(() => setLoaded(true));
    return () => { cancelled = true; };
  }, []);

  const handleClick = useCallback(async (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    try {
      await open(SMARTLINK_URL);
    } catch {
      window.open(SMARTLINK_URL, '_blank', 'noopener,noreferrer');
    }
  }, []);

  const handleDismiss = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    // Persist dismissal to store so it survives app restarts
    load('config.json')
      .then((store) => store.set(DISMISSED_KEY, true).then(() => store.save()))
      .catch(() => {});
    // Trigger fade-out animation, then fully dismiss
    setExiting(true);
    setTimeout(() => setDismissed(true), 300);
  }, []);

  // Don't render until store check completes, or once dismissed.
  // Using !loaded prevents a flash on restart when the banner was previously dismissed.
  if (!isAndroid || !loaded || dismissed) {
    return null;
  }

  const isVisible = visible && !exiting;

  return (
    <div
      id="adsterra-banner-container"
      role="complementary"
      aria-label="Sponsored content"
      className="relative flex w-full justify-center overflow-hidden border border-app-border bg-app-surface-raised shadow-[var(--shadow-raised)] transition-all duration-200 ease-out"
      style={{
        visibility: isVisible ? 'visible' : 'hidden',
        minHeight: isVisible ? 48 : 0,
        maxHeight: isVisible ? 48 : 0,
        height: isVisible ? 48 : 0,
        opacity: isVisible ? 1 : 0,
      }}
    >
      <button
        onClick={handleClick}
        className="quiet-control flex flex-1 items-center justify-center gap-2 px-4 py-2.5 text-metadata font-medium text-app-text-secondary hover:bg-app-hover hover:text-app-text"
      >
        <ExternalLink className="h-3 w-3 text-app-accent" />
        <span className="sponsored-label border-0">Sponsored</span>
      </button>
      <button
        onClick={handleDismiss}
        className="quiet-control absolute end-1 top-1/2 -translate-y-1/2 p-1.5 text-app-text-secondary hover:text-app-text"
        aria-label="Close ad"
      >
        <X className="w-3.5 h-3.5" />
      </button>
    </div>
  );
}
