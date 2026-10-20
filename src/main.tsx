import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { initializeAppearance } from "./theme/runtime";
import { macOS, native } from "./api";
import { trackRemoteActivity } from "./remote-activity";
import "./styles.css";
import "./content-ui.css";
import "./workflow-ui.css";

window.addEventListener("contextmenu", (event) => event.preventDefault(), {
  capture: true,
});

initializeAppearance();
trackRemoteActivity();
if (native && macOS) document.documentElement.dataset.platform = "macos";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
