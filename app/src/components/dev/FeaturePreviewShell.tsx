import { Component, useState, Suspense, type ReactNode } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { ThemeProvider } from '../../context/ThemeContext';
import { installFeaturePreviewMock } from './featurePreviewData';
import { FeaturePreview } from './FeaturePreview';

// Install the mock IPC before any provider mounts. This component deliberately
// avoids AppProviders: the real startup providers assume a live Tauri backend
// and would push the tree into the error boundary in a plain browser.
installFeaturePreviewMock();

class PreviewBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state: { error: Error | null } = { error: null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  render() {
    if (this.state.error) {
      return (
        <div className="flex h-screen flex-col items-center justify-center gap-3 bg-app-canvas p-8 text-app-text">
          <h1 className="text-lg font-semibold">Preview error</h1>
          <pre className="max-w-2xl overflow-auto rounded-lg bg-black/30 p-4 text-xs text-red-300">
            {String(this.state.error?.stack || this.state.error)}
          </pre>
        </div>
      );
    }
    return this.props.children;
  }
}

export function FeaturePreviewShell() {
  const [queryClient] = useState(() => new QueryClient({
    defaultOptions: { queries: { refetchOnWindowFocus: false, retry: false } },
  }));

  return (
    <PreviewBoundary>
      <ThemeProvider>
        <QueryClientProvider client={queryClient}>
          <Suspense fallback={<div className="h-screen bg-app-canvas" />}>
            <FeaturePreview />
          </Suspense>
        </QueryClientProvider>
      </ThemeProvider>
    </PreviewBoundary>
  );
}
