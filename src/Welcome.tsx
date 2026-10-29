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
          <span className="welcome-brandmark" aria-hidden="true">
            <img src="/app-icon.png" width="36" height="36" alt="" />
          </span>
          <div>
            <h1>Welcome to Lomi</h1>
            <p>Open a project to work with your files and terminals.</p>
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
            <FolderOpen size={20} aria-hidden="true" />
            <span className="welcome-action-copy">
              <strong id="welcome-open-folder">
                Open folder or repository
              </strong>
              <span id="welcome-open-folder-description">
                Explore local files and open a terminal.
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
            <FilePlus2 size={20} aria-hidden="true" />
            <span className="welcome-action-copy">
              <strong id="welcome-new-file">Create a new file</strong>
              <span id="welcome-new-file-description">
                Choose a folder and start with a blank file.
              </span>
            </span>
            <ArrowRight size={16} aria-hidden="true" />
          </button>
        </div>
      </div>
    </main>
  );
}
