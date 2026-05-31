import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import PresentView from "./components/PresentView";
import "./styles/global.css";

const presentSide = new URLSearchParams(location.search).get("present");
const isPresent = presentSide === "A" || presentSide === "B";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {isPresent ? <PresentView side={presentSide as "A" | "B"} /> : <App />}
  </React.StrictMode>
);
