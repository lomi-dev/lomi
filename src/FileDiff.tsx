import { agentDiff, readAgentGit, useAgentGit } from "./agent-git";
import ResourceIcon from "./ResourceIcon";
import { useEffect, useState } from "react";
import { FileDiff as FileDiffIcon, RefreshCw } from "./icons";
import { api, errorMessage } from "./api";
import type { GitFileDiff } from "./api";
import type { DiffTab } from "./model";
import { Patch } from "./CommitDetails";
import { IconButton } from "./ui";

export default function FileDiff({
  tab,
  requestRevision,
  onOpenFile,
}: {
  tab: DiffTab;
  requestRevision: number;
  onOpenFile: () => void;
}) {
  const agentSource = useAgentGit(tab.id);
  const [result, setResult] = useState<GitFileDiff>();
  const [error, setError] = useState("");
  const [revision, setRevision] = useState(0);
  const refresh = () => setRevision((value) => value + 1);
  useEffect(() => {
    let current = true;
    setResult(undefined);
    setError("");
    const load = tab.agentGit
      ? (agentSource && revision === 0
          ? Promise.resolve(agentSource.body)
          : readAgentGit(agentSource)
        ).then(agentDiff)
      : api<GitFileDiff>("git_diff", {
          root: tab.root,
          path: tab.relative,
          staged: tab.staged,
        });
    void load
      .then((result) => {
        if (current) setResult(result);
      })
      .catch((error) => {
        if (current) setError(errorMessage(error));
      });
    return () => {
      current = false;
    };
  }, [
    tab.root,
    tab.relative,
    tab.staged,
    tab.agentGit,
    agentSource,
    revision,
    requestRevision,
  ]);
  return (
    <section
      className="commit-file-diff working-file-diff"
      aria-label={`Changes in ${tab.relative}`}
    >
      <header>
        <ResourceIcon
          path={`${tab.root}/${tab.relative}`}
          size={15}
          fallback={FileDiffIcon}
        />
        <span className="working-diff-path">{tab.relative}</span>
        <button type="button" className="button" onClick={onOpenFile}>
          Open file
        </button>
        <IconButton title="Refresh file changes" onClick={refresh}>
          <RefreshCw size={14} />
        </IconButton>
      </header>
      <div className="working-diff-comparison">
        <span>
          {tab.staged
            ? "Staged changes · HEAD → Index"
            : "Working tree changes · Index → Disk"}
        </span>
        <span className="diff-additions">+ Added</span>
        <span className="diff-deletions">− Removed</span>
        <span>Read-only</span>
      </div>
      {error ? (
        <div className="commit-view-message" role="alert">
          <p>{error}</p>
          <button type="button" className="button" onClick={refresh}>
            Retry diff
          </button>
        </div>
      ) : !result ? (
        <div className="commit-view-message" role="status">
          Loading file changes…
        </div>
      ) : (
        <>
          {result.notice && (
            <p className="commit-diff-notice" role="status">
              {result.notice}
            </p>
          )}
          {result.truncated && (
            <p className="commit-diff-notice" role="status">
              This file exceeds the preview limit. Showing the first 2 MiB or
              20,000 lines.
            </p>
          )}
          <div
            className="commit-patch"
            tabIndex={0}
            role="region"
            aria-label={`Diff for ${tab.relative}`}
          >
            <Patch patch={result.patch} fullFile />
          </div>
        </>
      )}
    </section>
  );
}
