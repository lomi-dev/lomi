import { useEffect, useState } from "react";
import { GitCommitHorizontal, History } from "./icons";
import { api, errorMessage } from "./api";
import type { GitCommitSummary, GitHistoryPage } from "./api";
import { useT } from "./i18n";

export default function GitHistory({
  root,
  path,
  onOpenCommit,
}: {
  root: string;
  path?: string;
  onOpenCommit: (commit: GitCommitSummary) => void;
}) {
  const { t } = useT();
  const [history, setHistory] = useState<GitHistoryPage>();
  const [request, setRequest] = useState<{
    skip: number;
    tips: string[] | null;
  }>({ skip: 0, tips: null });
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  useEffect(() => {
    let current = true;
    void api<GitHistoryPage>("git_history", { root, path, ...request })
      .then((page) => {
        if (current)
          setHistory((previous) => ({
            ...page,
            commits: request.skip
              ? [...(previous?.commits ?? []), ...page.commits]
              : page.commits,
          }));
      })
      .catch((error) => {
        if (current) setError(errorMessage(error));
      })
      .finally(() => {
        if (current) setBusy(false);
      });
    return () => {
      current = false;
    };
  }, [root, path, request]);
  const load = (next: typeof request) => {
    setBusy(true);
    setError("");
    setRequest(next);
  };
  return (
    <div className="git-history">
      <div className="git-history-heading">
        <span title={path}>{path || t("All branches")}</span>
        {history && (
          <span>{t("{count} loaded", { count: history.commits.length })}</span>
        )}
      </div>
      <div className="git-history-scroll">
        <ol className="git-history-list" aria-label={t("Commit history")}>
          {history?.commits.map((commit) => (
            <li key={commit.id}>
              <button
                type="button"
                className="git-history-entry"
                onClick={() => onOpenCommit(commit)}
                title={`${commit.subject}\n${commit.authorName}\n${commit.id}`}
              >
                <GitCommitHorizontal size={15} />
                <span className="git-history-entry-content">
                  <span className="git-history-subject">
                    {commit.subject || t("(no subject)")}
                  </span>
                  <span className="git-history-author">
                    {commit.authorName}
                  </span>
                  <span className="git-history-meta">
                    <code>{commit.shortId}</code>
                    <time
                      dateTime={commit.authoredAt}
                      title={new Date(commit.authoredAt).toLocaleString()}
                    >
                      {new Date(commit.authoredAt).toLocaleDateString()}
                    </time>
                  </span>
                </span>
              </button>
            </li>
          ))}
        </ol>
        {busy && (
          <p className="sidebar-empty" role="status">
            {t("Loading commits…")}
          </p>
        )}
        {error && (
          <div className="git-history-error" role="alert">
            <p>{error}</p>
            <button
              type="button"
              className="button"
              onClick={() => load({ ...request })}
            >
              {t("Retry")}
            </button>
          </div>
        )}
        {!busy && !error && history && !history.commits.length && (
          <div className="git-clean">
            <History size={22} />
            <p>{t("No commits yet.")}</p>
            <span>{t("Your commit history will appear here.")}</span>
          </div>
        )}
        {!busy && !error && history?.hasMore && (
          <button
            type="button"
            className="button git-history-more"
            onClick={() =>
              load({ skip: history.commits.length, tips: history.tips })
            }
          >
            {t("Load more commits")}
          </button>
        )}
      </div>
    </div>
  );
}
