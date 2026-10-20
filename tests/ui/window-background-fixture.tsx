import { createRoot } from "react-dom/client";
import { ThemeProvider, useThemes } from "../../src/ThemeProvider";
import "../../src/styles.css";

function Fixture() {
  const themes = useThemes();
  (window as any).themeBackgroundFixture = themes;
  return <main className="app-shell">Theme background fixture</main>;
}

createRoot(document.getElementById("root")!).render(
  <ThemeProvider>
    <Fixture />
  </ThemeProvider>,
);
