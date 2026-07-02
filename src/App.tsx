import { useEffect } from "react";
import { AppProvider, useApp } from "./state/AppState";
import Sidebar from "./components/Sidebar";
import TranscriptView from "./components/TranscriptView";
import Settings from "./components/Settings";
import SummaryModal from "./components/SummaryModal";
import ExportModal from "./components/ExportModal";
import SetupWizard from "./components/SetupWizard";
import PresentBroadcaster from "./present/PresentBroadcaster";
import "./styles/app.css";

/** Persistent red banner when the current mode is missing configuration, with a
 *  one-click jump into the setup wizard. Hidden while the wizard is open. */
function SetupBanner() {
  const { setupIssues, wizardOpen, openWizard } = useApp();
  if (wizardOpen || setupIssues.length === 0) return null;
  return (
    <div className="setup-banner" role="alert">
      <span className="setup-banner-icon">⚠</span>
      <div className="setup-banner-text">
        <strong>Требуется настройка.</strong>{" "}
        {setupIssues.map((i) => i.message).join("; ")}
      </div>
      <button className="setup-banner-btn" onClick={openWizard}>
        Настроить
      </button>
    </div>
  );
}

function MainArea() {
  const { view } = useApp();
  return (
    <main className="main">
      <SetupBanner />
      {view === "settings" ? <Settings /> : <TranscriptView />}
    </main>
  );
}

/** Auto-dismissing toast shared by the error and notice variants (same markup
 *  and timer, different palette/icon/role/duration). */
const TOAST_VARIANTS = {
  error: { cls: "toast-error", icon: "⚠", role: "alert", ms: 8000 },
  notice: { cls: "toast-notice", icon: "✓", role: "status", ms: 6000 },
} as const;

function Toast({
  variant,
  message,
  onDismiss,
}: {
  variant: keyof typeof TOAST_VARIANTS;
  message: string | null;
  onDismiss: () => void;
}) {
  const { cls, icon, role, ms } = TOAST_VARIANTS[variant];
  useEffect(() => {
    if (!message) return;
    const id = setTimeout(onDismiss, ms);
    return () => clearTimeout(id);
  }, [message, onDismiss, ms]);
  if (!message) return null;
  return (
    <div className={`toast ${cls}`} role={role}>
      <span className="toast-icon">{icon}</span>
      <span className="toast-msg">{message}</span>
      <button className="toast-close" onClick={onDismiss} aria-label="Dismiss">
        ✕
      </button>
    </div>
  );
}

// Thin wrappers so App itself never calls useApp() (it renders AppProvider).
function ErrorToast() {
  const { error, dismissError } = useApp();
  return <Toast variant="error" message={error} onDismiss={dismissError} />;
}

function NoticeToast() {
  const { notice, dismissNotice } = useApp();
  return <Toast variant="notice" message={notice} onDismiss={dismissNotice} />;
}

export default function App() {
  return (
    <AppProvider>
      <div className="app-shell">
        <Sidebar />
        <MainArea />
      </div>
      <ErrorToast />
      <NoticeToast />
      <SummaryModal />
      <ExportModal />
      <SetupWizard />
      <PresentBroadcaster />
    </AppProvider>
  );
}
