import { useState } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AuthWizard } from "./components/AuthWizard";
import { Dashboard } from "./components/Dashboard";
import { DriveModeSelector } from "./components/DriveModeSelector";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { UpdateBanner } from "./components/UpdateBanner";
import { VaultWizard } from "./components/VaultWizard";
import { useUpdateCheck } from "./hooks/useUpdateCheck";
import { DriveMode } from "./types";
import "./App.css";

import { Toaster } from "sonner";
import { ConfirmProvider } from "./context/ConfirmContext";
import { ThemeProvider, useTheme } from "./context/ThemeContext";
import { DropZoneProvider } from "./contexts/DropZoneContext";

const queryClient = new QueryClient();

function AppContent() {
  const [isAuthenticated, setIsAuthenticated] = useState(false);
  const [driveMode, setDriveMode] = useState<DriveMode | null>(null);
  const [isVaultUnlocked, setIsVaultUnlocked] = useState(false);
  const { theme } = useTheme();
  const { available, version, downloading, progress, downloadAndInstall, dismissUpdate } = useUpdateCheck();

  const resetSession = () => {
    setIsVaultUnlocked(false);
    setDriveMode(null);
    setIsAuthenticated(false);
  };

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
      {isAuthenticated && driveMode === 'plain' ? (
        <Dashboard driveMode="plain" onLogout={resetSession} />
      ) : isAuthenticated && driveMode === 'vault' && isVaultUnlocked ? (
        <Dashboard driveMode="vault" onLogout={resetSession} />
      ) : isAuthenticated && driveMode === 'vault' ? (
        <VaultWizard
          onUnlock={() => setIsVaultUnlocked(true)}
          onBack={() => {
            setIsVaultUnlocked(false);
            setDriveMode(null);
          }}
        />
      ) : isAuthenticated ? (
        <DriveModeSelector onSelect={setDriveMode} />
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
