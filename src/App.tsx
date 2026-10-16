import { PluginsProvider } from "./plugins/PluginsProvider";
import { lazy, Suspense } from "react";
import { KeybindingsProvider } from "./KeybindingsProvider";
import { ThemeProvider } from "./ThemeProvider";
import { EditorPreferencesProvider } from "./EditorPreferencesProvider";
import ReadyWindow from "./ReadyWindow";
import { useWindowFullscreen } from "./useWindowFullscreen";

import { TerminalPreferencesProvider } from "./TerminalPreferencesProvider";

const Workbench = lazy(() => import("./Workbench"));

export default function App() {
  useWindowFullscreen();
  return (
    <ThemeProvider>
      <PluginsProvider>
        <KeybindingsProvider>
          <EditorPreferencesProvider>
            <TerminalPreferencesProvider>
              <Suspense
                fallback={
                  <main className="empty-message">Opening workspace…</main>
                }
              >
                <Workbench />
                <ReadyWindow />
              </Suspense>
            </TerminalPreferencesProvider>
          </EditorPreferencesProvider>
        </KeybindingsProvider>
      </PluginsProvider>
    </ThemeProvider>
  );
}
