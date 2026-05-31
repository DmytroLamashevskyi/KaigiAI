import { useEffect } from "react";
import { AppProvider, useApp } from "./state/AppState";
import Sidebar from "./components/Sidebar";
import TranscriptView from "./components/TranscriptView";
import Settings from "./components/Settings";
import PresentBroadcaster from "./present/PresentBroadcaster";
import "./styles/app.css";

function MainArea() {
  const { view } = useApp();
  return (
    <main className="main">
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

export default function App() {
  return (
    <AppProvider>
      <div className="app-shell">
        <Sidebar />
        <MainArea />
      </div>
      <ErrorToast />
      <PresentBroadcaster />
    </AppProvider>
  );
}
