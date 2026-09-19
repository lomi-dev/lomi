import { DisclosureSummary } from "./ui";
import ResourceIcon from "./ResourceIcon";
import { useEffect, useMemo, useState } from "react";
import { FileDiff, GitCommitHorizontal } from "./icons";
import { api, errorMessage } from "./api";
import type { GitCommitDetails, GitCommitDiff, GitCommitSummary } from "./api";
import { diffLines } from "./git-diff";
import { useGitFileActions } from "./GitFileActions";

export default function CommitDetails({
  root,
  commitId,
  onOpenFile,
  onOpenCommit,
  onError,
}: {
  root: string;
  commitId: string;
  onOpenFile: (path: string) => void;
  onOpenCommit: (commit: GitCommitSummary) => void;
  onError: (message: string) => void;
}) {
  const [details, setDetails] = useState<GitCommitDetails>();
  const [error, setError] = useState("");
  const [revision, setRevision] = useState(0);
  const [selectedPath, setSelectedPath] = useState<string>();
  const [diff, setDiff] = useState<{
    path: string;
    result?: GitCommitDiff;
    error?: string;
  }>();
  const [diffRevision, setDiffRevision] = useState(0);
  const actions = useGitFileActions({
    root,
    onDiff: setSelectedPath,
    onOpenFile,
    onOpenCommit,
    onError,
  });
  useEffect(() => {
    let current = true;
    setDetails(undefined);
    setError("");
    void api<GitCommitDetails>("git_commit_details", { root, id: commitId })
      .then((details) => {
        if (!current) return;
        setDetails(details);
        setSelectedPath(details.files[0]?.path);
      })
      .catch((error) => {
        if (current) setError(errorMessage(error));
      });
    return () => {
      current = false;
    };
  }, [root, commitId, revision]);
  const selectedFile = details?.files.find(
    (file) => file.path === selectedPath,
  );
  useEffect(() => {
    if (!selectedFile) return;
    let current = true;
    setDiff(undefined);
    void api<GitCommitDiff>("git_commit_diff", {
      root,
      id: commitId,
      path: selectedFile.path,
      originalPath: selectedFile.originalPath,
    })
      .then((result) => {
        if (current) setDiff({ path: selectedFile.path, result });
      })
      .catch((error) => {
        if (current)
          setDiff({ path: selectedFile.path, error: errorMessage(error) });
      });
    return () => {
      current = false;
    };
  }, [root, commitId, selectedFile, diffRevision]);
  if (error)
    return (
      <div className="commit-view-message" role="alert">
        <h2>Unable to load commit</h2>
        <p>{error}</p>
        <button
          type="button"
          className="button"
          onClick={() => setRevision((value) => value + 1)}
        >
          Retry
        </button>
      </div>
    );
  if (!details)
    return (
      <div className="commit-view-message" role="status">
        Loading commit…
      </div>
    );
  const { commit, files } = details;
  const additions = files.reduce((sum, file) => sum + (file.additions ?? 0), 0);
  const deletions = files.reduce((sum, file) => sum + (file.deletions ?? 0), 0);
  const currentDiff = diff?.path === selectedPath ? diff : undefined;
  return (
    <article className="commit-details" aria-label={`Commit ${commit.shortId}`}>
      <header className="commit-details-header">
        <div className="commit-details-title">
          <GitCommitHorizontal size={20} />
          <h1>{commit.subject || "(no subject)"}</h1>
        </div>
        <dl className="commit-metadata">
          <div>
            <dt>Commit</dt>
            <dd>
              <code>{commit.id}</code>
            </dd>
          </div>
          <div>
            <dt>Author</dt>
            <dd>
              {commit.authorName} &lt;{details.authorEmail}&gt;
              <time dateTime={commit.authoredAt}>
                {new Date(commit.authoredAt).toLocaleString()}
              </time>
            </dd>
          </div>
          <div>
            <dt>Committer</dt>
            <dd>
              {details.committerName} &lt;{details.committerEmail}&gt;
              <time dateTime={details.committedAt}>
                {new Date(details.committedAt).toLocaleString()}
              </time>
            </dd>
          </div>
          {!!details.parents.length && (
            <div>
              <dt>{details.parents.length > 1 ? "Parents" : "Parent"}</dt>
              <dd>
                {details.parents.map((parent) => (
                  <code key={parent}>{parent}</code>
                ))}
              </dd>
            </div>
          )}
        </dl>
        <details className="commit-full-message">
          <DisclosureSummary>Full commit message</DisclosureSummary>
          <pre>{details.message}</pre>
        </details>
      </header>
      <div className="commit-files-heading">
        <span>
          {files.length} {files.length === 1 ? "changed file" : "changed files"}
        </span>
        <span className="commit-file-stats">
          <span className="diff-additions">+{additions}</span>
          <span className="diff-deletions">−{deletions}</span>
        </span>
        <span className="commit-comparison">
          {details.parents.length
            ? `Compared with ${details.parents.length > 1 ? "first parent " : "parent "}${details.parents[0].slice(0, 7)}`
            : "Initial commit"}
        </span>
      </div>
      {!files.length ? (
        <div className="commit-view-message">
          This commit has no file changes.
        </div>
      ) : (
        <div className="commit-changes">
          <nav className="commit-file-list" aria-label="Changed files">
            {files.map((file) => (
              <button
                type="button"
                key={file.path}
                className="commit-file-entry"
                aria-current={file.path === selectedPath ? "true" : undefined}
                onClick={() => setSelectedPath(file.path)}
                onContextMenu={(event) =>
                  actions.onContext(event, { path: file.path })
                }
                onKeyDown={(event) => actions.onKey(event, { path: file.path })}
                title={
                  file.originalPath
                    ? `${file.originalPath} → ${file.path}`
                    : file.path
                }
              >
                <span className="commit-file-status" data-status={file.status}>
                  {file.status}
                </span>
                <span className="commit-file-path">{file.path}</span>
                <span className="commit-file-stats">
                  {file.additions === null ? (
                    "Binary"
                  ) : (
                    <>
                      <span className="diff-additions">+{file.additions}</span>
                      <span className="diff-deletions">−{file.deletions}</span>
                    </>
                  )}
                </span>
              </button>
            ))}
          </nav>
          <section className="commit-file-diff" aria-label="File changes">
            <header>
              <ResourceIcon
                path={`${root}/${selectedPath}`}
                size={14}
                fallback={FileDiff}
              />
              <span>
                {selectedFile?.originalPath
                  ? `${selectedFile.originalPath} → ${selectedFile.path}`
                  : selectedPath}
              </span>
            </header>
            {currentDiff?.error ? (
              <div className="commit-view-message" role="alert">
                <p>{currentDiff.error}</p>
                <button
                  type="button"
                  className="button"
                  onClick={() => setDiffRevision((value) => value + 1)}
                >
                  Retry diff
                </button>
              </div>
            ) : !currentDiff?.result ? (
              <div className="commit-view-message" role="status">
                Loading diff…
              </div>
            ) : (
              <>
                {currentDiff.result.truncated && (
                  <p className="commit-diff-notice" role="status">
                    This diff is too large to display in full. Showing the first
                    2 MiB or 20,000 lines.
                  </p>
                )}
                {currentDiff.result.patch ? (
                  <div
                    className="commit-patch"
                    tabIndex={0}
                    role="region"
                    aria-label={`Diff for ${selectedPath}`}
                  >
                    <Patch patch={currentDiff.result.patch} />
                  </div>
                ) : (
                  <div className="commit-view-message">
                    No text changes in this file.
                  </div>
                )}
              </>
            )}
          </section>
        </div>
      )}
      {actions.menu}
      {actions.dialogs}
    </article>
  );
}

export function Patch({
  patch,
  fullFile = false,
}: {
  patch: string;
  fullFile?: boolean;
}) {
  const lines = useMemo(
    () =>
      diffLines(patch).filter(
        (line) =>
          !fullFile ||
          line.oldLine !== undefined ||
          line.newLine !== undefined ||
          line.text.startsWith("\\ No newline"),
      ),
    [patch, fullFile],
  );
  return (
    <pre className="commit-patch-lines">
      <code>
        {lines.map((line, index) => (
          <span className={`diff-line diff-line-${line.kind}`} key={index}>
            <span className="diff-line-number" aria-hidden="true">
              {line.oldLine}
            </span>
            <span className="diff-line-number" aria-hidden="true">
              {line.newLine}
            </span>
            <span className="diff-line-text">{line.text || " "}</span>
          </span>
        ))}
      </code>
    </pre>
  );
}
