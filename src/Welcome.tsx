import { ArrowRight, FilePlus2, FolderOpen } from "./icons";

export default function Welcome({
  busy,
  onOpenFolder,
  onNewFile,
}: {
  busy: boolean;
  onOpenFolder: () => void;
  onNewFile: () => void;
}) {
  return (
    <main className="terminal-stage welcome" aria-label="No project open">
      <div className="welcome-content">
        <header className="welcome-heading">
          <img src="/app-icon.svg" width="44" height="44" alt="" />
          <div>
            <h1>Welcome to Lomi</h1>
            <p>A place for your files, terminal, and next idea.</p>
          </div>
        </header>

        <div className="welcome-actions" aria-busy={busy}>
          <button
            type="button"
            className="welcome-action welcome-action-primary"
            aria-labelledby="welcome-open-folder"
            aria-describedby="welcome-open-folder-description"
            aria-disabled={busy}
            onClick={(event) => {
              event.currentTarget.focus({ preventScroll: true });
              onOpenFolder();
            }}
          >
            <FolderOpen size={22} aria-hidden="true" />
            <span className="welcome-action-copy">
              <strong id="welcome-open-folder">
                Open folder or repository
              </strong>
              <span id="welcome-open-folder-description">
                Pick a local folder to explore its files and open a terminal.
              </span>
            </span>
            <ArrowRight size={16} aria-hidden="true" />
          </button>
          <button
            type="button"
            className="welcome-action"
            aria-labelledby="welcome-new-file"
            aria-describedby="welcome-new-file-description"
            aria-disabled={busy}
            onClick={(event) => {
              event.currentTarget.focus({ preventScroll: true });
              onNewFile();
            }}
          >
            <FilePlus2 size={22} aria-hidden="true" />
            <span className="welcome-action-copy">
              <strong id="welcome-new-file">Create a new file</strong>
              <span id="welcome-new-file-description">
                Choose a folder, then start with a blank file. Save when ready.
              </span>
            </span>
            <ArrowRight size={16} aria-hidden="true" />
          </button>
        </div>

        <ol className="welcome-steps" role="list" aria-label="Getting started">
          <li>
            <span className="welcome-step-number" aria-hidden="true">
              1
            </span>
            <span>Choose your folder</span>
          </li>
          <li>
            <span className="welcome-step-number" aria-hidden="true">
              2
            </span>
            <span>Edit files or run commands</span>
          </li>
          <li>
            <span className="welcome-step-number" aria-hidden="true">
              3
            </span>
            <span>Return to your saved layout</span>
          </li>
        </ol>
      </div>
    </main>
  );
}
