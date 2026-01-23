import { useState } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AuthWizard } from "./components/AuthWizard";
import { Dashboard } from "./components/Dashboard";
import "./App.css";

const queryClient = new QueryClient();

function App() {
  const [isAuthenticated, setIsAuthenticated] = useState(false);

  return (
    <QueryClientProvider client={queryClient}>
      <main className="h-screen w-screen text-telegram-text overflow-hidden selection:bg-telegram-primary/30 relative">
        {isAuthenticated ? (
          <Dashboard />
        ) : (
          <AuthWizard onLogin={() => setIsAuthenticated(true)} />
        )}
      </main>
    </QueryClientProvider>
  );
}

export default App;
