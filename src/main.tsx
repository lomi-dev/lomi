import { lazy, StrictMode, Suspense } from "react";
import { createRoot } from "react-dom/client";
import { isRemoteV2ViewEntry, readRemoteV2ViewReference } from "../../remote-web/src/remote-v2-view-reference";
import { initializeAppearance } from "./theme/runtime";
import { macOS, native } from "./api";
import { trackRemoteActivity } from "./remote-activity";
import "./styles.css";
import "./content-ui.css";
import "./workflow-ui.css";
import "./dialog-ui.css";

const App = lazy(() => import("./App"));
const DesktopRemoteV2Client = lazy(() =>
  import("./desktop-v2-client").then(({ DesktopRemoteV2Client }) => ({
    default: DesktopRemoteV2Client,
  })),
);
const remoteV2View = isRemoteV2ViewEntry(location.search);

window.addEventListener("contextmenu", (event) => event.preventDefault(), {
  capture: true,
});

initializeAppearance();
if (!remoteV2View) trackRemoteActivity();
if (native && macOS) document.documentElement.dataset.platform = "macos";

const initialTarget = remoteV2View
  ? readRemoteV2ViewReference(location.search)
  : undefined;

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Suspense fallback={<main className="empty-message">Opening workspace…</main>}>
      {remoteV2View ? (
        <DesktopRemoteV2Client available={native} entry="workspace" initialTarget={initialTarget} />
      ) : (
        <App />
      )}
    </Suspense>
  </StrictMode>,
);
