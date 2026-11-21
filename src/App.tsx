import { PluginsProvider } from "./plugins/PluginsProvider";
import { useT } from "./i18n";
import { lazy, Suspense, useState } from "react";
import { KeybindingsProvider } from "./KeybindingsProvider";
import { ThemeProvider } from "./ThemeProvider";
import { EditorPreferencesProvider } from "./EditorPreferencesProvider";
import ReadyWindow from "./ReadyWindow";
import { onboardingDone } from "./onboarding-state";
import { useWindowFullscreen } from "./useWindowFullscreen";

import { TerminalPreferencesProvider } from "./TerminalPreferencesProvider";

const Workbench = lazy(() => import("./Workbench"));
const Onboarding = lazy(() => import("./Onboarding"));

export default function App() {
  const { t } = useT();
  useWindowFullscreen();
  const [onboarding, setOnboarding] = useState(() => !onboardingDone());
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
