import React from "react";
import ReactDOM from "react-dom/client";
import PresentView from "./components/PresentView";
import { hasTauri } from "./env";
// Order mirrors the main entry (main.tsx -> global.css, App.tsx -> app.css):
// global.css defines the design tokens (:root variables) and the
// [data-theme="dark"] overrides that app.css's rules — and this window's own
// theme toggle — depend on. Without it every var() here is unresolved.
import "./styles/global.css";
import "./styles/app.css";

// Dedicated, lightweight entry for the presentation window (a separate page so
// it loads reliably as its own Tauri window — embedding the full SPA in a second
// window proved flaky). The side comes from the Tauri window label
// (present-a / present-b) or, in the browser preview, the ?side=A|B query.
function resolveSide(): "A" | "B" {
  const q = new URLSearchParams(location.search).get("side");
  if (q === "A" || q === "B") return q;
  if (hasTauri()) {
    const label = (
      window as unknown as {
        __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } };
      }
    ).__TAURI_INTERNALS__?.metadata?.currentWindow?.label;
    if (label === "present-b") return "B";
  }
  return "A";
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <PresentView side={resolveSide()} />
  </React.StrictMode>
);
