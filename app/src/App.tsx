import React, { useState, useEffect, Suspense } from "react";
import { invoke } from "@tauri-apps/api/core";
import { load } from "@tauri-apps/plugin-store";
import { AppProviders } from "./components/shared/AppProviders";
import { ErrorBoundary } from "./components/shared/ErrorBoundary";
import { UpdateBanner } from "./components/shared/UpdateBanner";
import { useUpdates } from "./context/UpdateContext";
import { usePlatform } from "./hooks/usePlatform";
import "./App.css";

const DesktopDashboard = React.lazy(() => import("./components/desktop/DesktopDashboard").then(m => ({ default: m.Dashboard })));
const AuthWizard = React.lazy(() => import("./components/shared/AuthWizard").then(m => ({ default: m.AuthWizard })));
const AdGateway = React.lazy(() => import("./components/shared/AdGateway").then(m => ({ default: m.AdGateway })));
// Vite requires a fully static import path for dynamic imports so it can
// perform static analysis and code-splitting. Template literals with
// variables prevent Vite from resolving the module at build time.
const DesignGallery = import.meta.env.DEV
  ? React.lazy(() => import("./components/dev/DesignGallery"))
  : null;
const AccessibilityAudit = import.meta.env.DEV
  ? React.lazy(() => import("./components/dev/AccessibilityAudit"))
  : null;
const AccessibilityFixtures = import.meta.env.DEV
  ? React.lazy(() => import("./components/dev/AccessibilityFixtures"))
  : null;

import { Toaster, toast } from "sonner";
import { useTheme } from "./context/ThemeContext";
import { CrashReportingConsent } from "./components/shared/CrashReportingConsent";
import { configureCrashTelemetry } from "./services/crashTelemetry";
import { TelegramCooldownBanner } from "./components/shared/TelegramCooldownBanner";
const WhatsNewDialog = React.lazy(() => import("./components/shared/WhatsNewDialog").then(m => ({ default: m.WhatsNewDialog })));
import { useSettings } from "./context/SettingsContext";
import { useTranslation } from "react-i18next";

import { getLanguageInfo } from "./i18n/languages";
import { resolveLanguagePreference } from "./i18n/resolveLanguage";
import { version as appVersion } from "../package.json";
import { consumeWhatsNew, type WhatsNewDetails } from "./services/updateReliability";
import { useTvSpatialNavigation } from "./hooks/useTvSpatialNavigation";
import { ensureLanguageResource } from "./i18n";
import { publishNativeLanguage } from "./services/nativeLanguage";
import { useSupporter } from "./context/SupporterContext";
import { shouldShowSponsorContent } from "./services/supporterVisibility";

type AuthStatus = "loading" | "authenticated" | "unauthenticated" | "sponsor-check" | "ad-gateway";

const AD_GATEWAY_PASSED_KEY = "ad_gateway_passed";

let languageApplyQueue: Promise<void> = Promise.resolve();

interface StartupProgress {
  stage: "initial" | "local" | "session" | "empty" | "invalid" | "connect" | "account" | "sponsor";
  percent: number;
}

function AppContent() {
  const [authStatus, setAuthStatus] = useState<AuthStatus>("loading");
  const [startupProgress, setStartupProgress] = useState<StartupProgress>({
    stage: "initial",
    percent: 8,
  });
  const [whatsNew, setWhatsNew] = useState<WhatsNewDetails | null>(() => consumeWhatsNew(appVersion));
  const { theme } = useTheme();
  const { available, version, downloading, progress, phase, managedByPackageManager, downloadAndInstall, dismissUpdate } = useUpdates();
  const { isTelevision } = usePlatform();
  useTvSpatialNavigation(isTelevision);
  const { settings, isLoaded, persistenceStatus, retryPersistence } = useSettings();
  const { status: supporterStatus } = useSupporter();
  const { i18n, t } = useTranslation();

  useEffect(() => {
    if (!isLoaded) return;
    configureCrashTelemetry(settings.crashReportingEnabled);
  }, [isLoaded, settings.crashReportingEnabled]);

  // Handle active language and RTL direction changes
  useEffect(() => {
    if (!isLoaded) return;
    const activeLang = resolveLanguagePreference(settings.language);
    const info = getLanguageInfo(activeLang);
    let cancelled = false;
    void ensureLanguageResource(activeLang)
      .then(() => {
        const operation = languageApplyQueue.then(async () => {
          if (cancelled) return;
          await i18n.changeLanguage(activeLang);
          if (cancelled) return;
          document.documentElement.lang = activeLang;
          document.documentElement.dir = info.dir;
          await publishNativeLanguage(activeLang);
        });
        // Serialize IPC as well as language application, including across remounts.
        // An older dispatched language completes before the next valid language.
        languageApplyQueue = operation.catch(() => undefined);
        return operation;
      })
      .catch(() => {
        if (import.meta.env.DEV) console.error(`[i18n] Unable to load ${activeLang}`);
      });
    return () => { cancelled = true; };
  }, [settings.language, isLoaded, i18n]);

  useEffect(() => {
    document.documentElement.classList.toggle('tv-mode', isTelevision);
    return () => document.documentElement.classList.remove('tv-mode');
  }, [isTelevision]);

  // On mount: check for a saved session and auto-restore it.
  // This is the SINGLE source of truth for the initial connection.
  // useTelegramConnection (inside Dashboard) no longer calls cmd_connect on mount.
  useEffect(() => {
    const checkSession = async () => {
      try {
        setStartupProgress({ stage: "local", percent: 18 });
        await invoke("cmd_get_startup_health");
        setStartupProgress({ stage: "session", percent: 38 });
        const store = await load("config.json");
        const savedId = await store.get<string>("api_id");
        const legacyApiHash = await store.get<string>("api_hash");
        if (legacyApiHash) {
          try {
            await invoke('cmd_store_api_hash', { apiHash: legacyApiHash });
            await store.delete('api_hash');
            await store.save();
          } catch {
            // Preserve the only working copy and retry migration next launch.
            // Session restoration needs only the public API ID.
          }
        }

        if (!savedId) {
          setStartupProgress({ stage: "empty", percent: 100 });
          setAuthStatus("unauthenticated");
          return;
        }

        const apiId = parseInt(savedId, 10);
        if (isNaN(apiId)) {
          setStartupProgress({ stage: "invalid", percent: 100 });
          setAuthStatus("unauthenticated");
          return;
        }

        // Initialize the client with the saved API ID
        setStartupProgress({ stage: "connect", percent: 58 });
        await invoke("cmd_connect", { apiId });

        // Verify the session is still valid with Telegram servers
        setStartupProgress({ stage: "account", percent: 82 });
        const ok = await invoke<boolean>("cmd_check_connection");
        if (ok) {
          setStartupProgress({ stage: "sponsor", percent: 96 });
          setAuthStatus("sponsor-check");
        } else {
          setAuthStatus("unauthenticated");
        }
      } catch (err) {
        console.warn("Session restore failed, showing login:", err);
        // Session file is corrupt or revoked — clean up and show login
        try {
          const store = await load("config.json");
          await store.delete("api_id");
          await store.save();
        } catch {
          // best-effort cleanup
        }
        setAuthStatus("unauthenticated");
      }
    };

    checkSession();
  }, []);

  // Resolve supporter access before deciding whether the one-time sponsor
  // gateway is eligible. This prevents any sponsor surface from flashing
  // while a lifetime entitlement or offline-grace token is still loading.
  useEffect(() => {
    if (authStatus !== "sponsor-check" || supporterStatus.state === "loading") return;

    let cancelled = false;
    const finishSponsorCheck = async () => {
      if (!shouldShowSponsorContent(supporterStatus)) {
        if (!cancelled) setAuthStatus("authenticated");
        return;
      }

      try {
        const store = await load("config.json");
        const gatewayPassed = await store.get<boolean>(AD_GATEWAY_PASSED_KEY);
        if (!cancelled) setAuthStatus(gatewayPassed ? "authenticated" : "ad-gateway");
      } catch {
        if (!cancelled) setAuthStatus("ad-gateway");
      }
    };

    void finishSponsorCheck();
    return () => { cancelled = true; };
  }, [authStatus, supporterStatus.ad_free, supporterStatus.state]);

  // Show thank-you toast when user enters the app after clicking the ad
  useEffect(() => {
    if (authStatus !== "authenticated") return;

    const showThanks = async () => {
      try {
        const store = await load("config.json");
        const shouldThank = await store.get<boolean>("ad_click_thanks");
        if (shouldThank) {
          await store.delete("ad_click_thanks");
          await store.save();
          toast.success(t('ads.thanks'), {
            duration: 3000,
            style: {
              background: "rgba(255,255,255,0.08)",
              border: "1px solid rgba(255,255,255,0.1)",
            },
          });
        }
      } catch {
        // Non-critical
      }
    };

    // Small delay to let the dashboard finish mounting
    const timer = setTimeout(showThanks, 600);
    return () => clearTimeout(timer);
  }, [authStatus]);

  // Warm-up screen driven by actual Rust health and Telegram session steps.
  if (authStatus === "loading" || authStatus === "sponsor-check") {
    const visibleProgress: StartupProgress = authStatus === "sponsor-check"
      ? { stage: "sponsor", percent: 96 }
      : startupProgress;
    return (
      <main className="h-screen w-screen flex items-center justify-center bg-telegram-bg">
        <div className="flex w-full max-w-sm flex-col items-center gap-5 px-8" role="status" aria-live="polite">
          <img src="/logo.svg" className="w-16 h-16" alt={i18n.t("common.app_title")} />
          <div className="w-full text-center">
            <p className="text-sm font-semibold text-telegram-text">{t(`runtime.startup_${visibleProgress.stage}_label`)}</p>
            <p className="mt-1 text-xs text-telegram-subtext">{t(`runtime.startup_${visibleProgress.stage}_detail`)}</p>
          </div>
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-white/10" aria-label={t('runtime.startup_percent',{percent:visibleProgress.percent})}>
            <div className="h-full rounded-full bg-telegram-primary transition-[width] duration-300" style={{ width: `${visibleProgress.percent}%` }} />
          </div>
        </div>
      </main>
    );
  }

  return (
    <main className="absolute inset-0 text-telegram-text overflow-hidden selection:bg-telegram-primary/30">
      <UpdateBanner
        available={available}
        version={version}
        downloading={downloading}
        progress={progress}
        phase={phase}
        managedByPackageManager={managedByPackageManager}
        onUpdate={downloadAndInstall}
        onDismiss={dismissUpdate}
      />
      <Toaster theme={theme} position="bottom-center" />
      <TelegramCooldownBanner />
      {persistenceStatus === 'error' && (
        <div className="fixed inset-x-4 top-4 z-[400] mx-auto flex max-w-xl items-center justify-between gap-3 rounded-lg border border-app-danger/30 bg-app-surface-raised px-4 py-3 text-sm text-app-text shadow-xl" role="alert">
          <span>{t('common.operation_failed')}</span>
          <button type="button" onClick={() => void retryPersistence().catch(() => undefined)} className="quiet-control shrink-0 bg-app-accent px-3 py-1.5 font-medium text-app-accent-contrast">
            {t('common.retry')}
          </button>
        </div>
      )}
      {whatsNew && <Suspense fallback={null}><WhatsNewDialog details={whatsNew} onClose={() => setWhatsNew(null)} /></Suspense>}
      {isLoaded && <CrashReportingConsent />}
      {authStatus === "ad-gateway" && (
        <Suspense fallback={<div className="h-screen bg-telegram-bg" />}>
          <AdGateway onContinue={() => setAuthStatus("authenticated")} />
        </Suspense>
      )}
      {authStatus === "authenticated" && (
        <Suspense fallback={
          <div className="h-screen w-screen flex flex-col items-center justify-center bg-telegram-bg">
            <div className="animate-spin rounded-full h-8 w-8 border-t-2 border-b-2 border-telegram-primary"></div>
          </div>
        }>
          <ErrorBoundary>
            <DesktopDashboard onLogout={() => setAuthStatus("unauthenticated")} />
          </ErrorBoundary>
        </Suspense>
      )}
      {authStatus === "unauthenticated" && (
        <Suspense fallback={<div className="h-screen bg-telegram-bg" />}>
          <AuthWizard onLogin={() => setAuthStatus("sponsor-check")} />
        </Suspense>
      )}
    </main>
  );
}


function App() {
  const showDesignGallery = Boolean(
    DesignGallery && typeof window !== 'undefined' && new URLSearchParams(window.location.search).has('design-gallery')
  );
  const showAccessibilityFixture = Boolean(
    AccessibilityFixtures && typeof window !== 'undefined' && new URLSearchParams(window.location.search).has('a11y-fixture')
  );

  return (
    <AppProviders>
      {AccessibilityAudit && (
        <Suspense fallback={null}><AccessibilityAudit /></Suspense>
      )}
      {showAccessibilityFixture && AccessibilityFixtures ? (
        <Suspense fallback={<div className="h-screen bg-app-canvas" />}>
          <AccessibilityFixtures />
        </Suspense>
      ) : showDesignGallery && DesignGallery ? (
        <Suspense fallback={<div className="h-screen bg-app-canvas" />}>
          <DesignGallery />
        </Suspense>
      ) : (
        <AppContent />
      )}
    </AppProviders>
  );
}

export default App;
