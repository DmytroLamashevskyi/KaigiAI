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

function ErrorToast() {
  const { error, dismissError } = useApp();
  useEffect(() => {
    if (!error) return;
    const id = setTimeout(dismissError, 8000);
    return () => clearTimeout(id);
  }, [error, dismissError]);
  if (!error) return null;
  return (
    <div className="toast toast-error" role="alert">
      <span className="toast-icon">⚠</span>
      <span className="toast-msg">{error}</span>
      <button className="toast-close" onClick={dismissError} aria-label="Dismiss">
        ✕
      </button>
    </div>
  );
}

function NoticeToast() {
  const { notice, dismissNotice } = useApp();
  useEffect(() => {
    if (!notice) return;
    const id = setTimeout(dismissNotice, 6000);
    return () => clearTimeout(id);
  }, [notice, dismissNotice]);
  if (!notice) return null;
  return (
    <div className="toast toast-notice" role="status">
      <span className="toast-icon">✓</span>
      <span className="toast-msg">{notice}</span>
      <button className="toast-close" onClick={dismissNotice} aria-label="Dismiss">
        ✕
      </button>
    </div>
  );
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
