import { FilePlus2 } from "./icons";
import { useT } from "./i18n";
import LomiSymbol from "./LomiSymbol";
import { AddProjectButton, type SourceKind } from "./project-sources";

export default function Welcome({
  busy,
  onSource,
  onNewFile,
}: {
  busy: boolean;
  onSource: (kind: SourceKind) => void;
  onNewFile: () => void;
}) {
  const { t } = useT();
  return (
    <main className="terminal-stage welcome" aria-label={t("No project open")}>
      <div className="welcome-content">
        <header className="welcome-heading">
          <LomiSymbol size={36} className="welcome-symbol" />
          <h1>{t("What are we building?")}</h1>
          <p>{t("Add a project, or start with a single file.")}</p>
        </header>

        <div className="welcome-buttons" aria-busy={busy}>
          <AddProjectButton busy={busy} onSource={onSource} />
          <button
            type="button"
            className="button"
            aria-disabled={busy}
            onClick={() => !busy && onNewFile()}
          >
            <FilePlus2 size={15} aria-hidden="true" />
            {t("New file")}
          </button>
        </div>
      </div>
    </main>
  );
}
