import { lazy, StrictMode, Suspense, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import {
  isRemoteV2ViewEntry,
  readRemoteV2ViewReference,
} from "../../remote-web/src/remote-v2-view-reference";
import { initializeAppearance } from "./theme/runtime";
import { api, macOS, native } from "./api";
import { trackRemoteActivity } from "./remote-activity";
import { SettingsNotice } from "./settings-ui";
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

function RemoteV2ViewEntry() {
  const [capability, setCapability] = useState<
    "loading" | "available" | "unavailable"
  >(native ? "loading" : "unavailable");

  useEffect(() => {
    if (!native) return;
    let current = true;
    void api<{ workspaceBackend: "local" | "shared-host" }>(
      "desktop_runtime_info",
    )
      .then((info) => {
        if (current)
          setCapability(
            info.workspaceBackend === "shared-host"
              ? "available"
              : "unavailable",
          );
      })
      .catch(() => {
        if (current) setCapability("unavailable");
      });
    return () => {
      current = false;
    };
  }, []);

  if (capability === "loading")
    return <SettingsNotice>Checking remote workspace support…</SettingsNotice>;
  if (capability === "unavailable")
    return (
      <SettingsNotice>
        Remote is disabled in this version of Lomi.
      </SettingsNotice>
    );
  return (
    <DesktopRemoteV2Client
      available
      entry="workspace"
      initialTarget={initialTarget}
    />
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Suspense
      fallback={<main className="empty-message">Opening workspace…</main>}
    >
      {remoteV2View ? <RemoteV2ViewEntry /> : <App />}
    </Suspense>
  </StrictMode>,
);
