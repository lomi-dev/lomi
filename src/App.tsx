import { PluginsProvider } from "./plugins/PluginsProvider";
import { useT } from "./i18n";
import { lazy, Suspense, useEffect, useState } from "react";
import { KeybindingsProvider } from "./KeybindingsProvider";
import { ThemeProvider } from "./ThemeProvider";
import { EditorPreferencesProvider } from "./EditorPreferencesProvider";
import ReadyWindow from "./ReadyWindow";
import { loadOnboardingState, onboardingDone } from "./onboarding-state";
import { useWindowFullscreen } from "./useWindowFullscreen";

import { TerminalPreferencesProvider } from "./TerminalPreferencesProvider";

const Workbench = lazy(() => import("./Workbench"));
const Onboarding = lazy(() => import("./Onboarding"));

export default function App() {
  const { t } = useT();
  useWindowFullscreen();
  // Decided after the native copy is merged, so lost webview storage does not
  // bring onboarding back for someone who finished it.
  const [onboarding, setOnboarding] = useState(false);
  useEffect(() => {
    let live = true;
    void loadOnboardingState().then(
      () => live && setOnboarding(!onboardingDone()),
    );
    return () => {
      live = false;
    };
  }, []);
  return (
    <ThemeProvider>
      <PluginsProvider>
        <KeybindingsProvider>
          <EditorPreferencesProvider>
            <TerminalPreferencesProvider>
              <Suspense
                fallback={
                  <main className="empty-message">
                    {t("Opening workspace…")}
                  </main>
                }
              >
                <Workbench />
                <ReadyWindow />
                {onboarding && (
                  <Suspense fallback={null}>
                    <Onboarding onDone={() => setOnboarding(false)} />
                  </Suspense>
                )}
              </Suspense>
            </TerminalPreferencesProvider>
          </EditorPreferencesProvider>
        </KeybindingsProvider>
      </PluginsProvider>
    </ThemeProvider>
  );
}
