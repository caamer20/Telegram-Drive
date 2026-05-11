import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { load } from "@tauri-apps/plugin-store";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AuthWizard } from "./components/AuthWizard";
import { Dashboard } from "./components/Dashboard";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { UpdateBanner } from "./components/UpdateBanner";
import { useUpdateCheck } from "./hooks/useUpdateCheck";
import "./App.css";

import { Toaster } from "sonner";
import { ConfirmProvider } from "./context/ConfirmContext";
import { ThemeProvider, useTheme } from "./context/ThemeContext";
import { DropZoneProvider } from "./contexts/DropZoneContext";

const queryClient = new QueryClient();

function AppContent() {
  const [isAuthenticated, setIsAuthenticated] = useState(false);
  const [checkingSession, setCheckingSession] = useState(true);

  const { theme } = useTheme();

  const {
    available,
    version,
    downloading,
    progress,
    downloadAndInstall,
    dismissUpdate,
  } = useUpdateCheck();

  useEffect(() => {
    const checkSession = async () => {
      try {
        const store = await load("config.json");

        const savedId = await store.get<string>("api_id");

        if (!savedId) {
          setCheckingSession(false);
          return;
        }

        const apiId = parseInt(savedId);

        if (isNaN(apiId)) {
          setCheckingSession(false);
          return;
        }

        await invoke("cmd_connect", { apiId });

        const ok = await invoke<boolean>("cmd_check_connection");

        if (ok) {
          setIsAuthenticated(true);
        }
      } catch (err) {
        console.error(err);
      } finally {
        setCheckingSession(false);
      }
    };

    checkSession();
  }, []);

  if (checkingSession) {
    return (
      <main className="h-screen w-screen flex items-center justify-center">
        Checking saved session...
      </main>
    );
  }

  return (
    <main className="h-screen w-screen text-telegram-text overflow-hidden selection:bg-telegram-primary/30 relative">
      <UpdateBanner
        available={available}
        version={version}
        downloading={downloading}
        progress={progress}
        onUpdate={downloadAndInstall}
        onDismiss={dismissUpdate}
      />

      <Toaster theme={theme} position="bottom-center" />

      {isAuthenticated ? (
        <Dashboard onLogout={() => setIsAuthenticated(false)} />
      ) : (
        <AuthWizard onLogin={() => setIsAuthenticated(true)} />
      )}
    </main>
  );
}

function App() {
  return (
    <ErrorBoundary>
      <ThemeProvider>
        <QueryClientProvider client={queryClient}>
          <ConfirmProvider>
            <DropZoneProvider>
              <AppContent />
            </DropZoneProvider>
          </ConfirmProvider>
        </QueryClientProvider>
      </ThemeProvider>
    </ErrorBoundary>
  );
}

export default App;