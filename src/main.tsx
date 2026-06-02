import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/global.css";

// The presentation window is a separate page (present.html / present-main.tsx),
// so the main entry just renders the app.
ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
