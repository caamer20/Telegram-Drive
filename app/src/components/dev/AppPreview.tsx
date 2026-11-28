import { Suspense, lazy } from 'react';
import { installFeaturePreviewMock } from './featurePreviewData';
import { AppProviders } from '../shared/AppProviders';

// Install the full Tauri IPC mock before providers mount so the REAL app tree
// can boot in a plain browser (no Rust backend present).
installFeaturePreviewMock();

const Dashboard = lazy(() =>
  import('../desktop/DesktopDashboard').then(m => ({ default: m.Dashboard })),
);

/**
 * Boots the real application UI (`AppProviders` + `Dashboard`) against mocked
 * IPC. Used by `?app-preview` to inspect the genuine v4.2.0 shell in a browser.
 */
export function AppPreview() {
  return (
    <AppProviders>
      <Suspense fallback={<div className="flex h-screen items-center justify-center bg-app-canvas text-app-text">Loading app…</div>}>
        <Dashboard onLogout={() => undefined} />
      </Suspense>
    </AppProviders>
  );
}
