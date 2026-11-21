import { FilePlus2 } from "./icons";
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
  return (
    <main className="terminal-stage welcome" aria-label="No project open">
      <div className="welcome-content">
        <header className="welcome-heading">
          <LomiSymbol size={36} className="welcome-symbol" />
          <h1>What are we building?</h1>
          <p>Add a project, or start with a single file.</p>
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
            New file
          </button>
        </div>
      </div>
    </main>
  );
}
