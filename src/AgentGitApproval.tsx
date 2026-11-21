import { useCallback, useEffect, useId, useRef, useState } from "react";
import { api, errorMessage } from "./api";
import { useT } from "./i18n";
import { Modal } from "./ui";

export interface GitMutationPlan {
  planHash: string;
  repositoryPath: string;
  operation:
    "stage" | "unstage" | "commit" | "fetch" | "push" | "discard" | "pull";
  head: string | null;
  branch: string | null;
  files: { relativePath: string; sha256: string | null; byteLength: number }[];
  commit: {
    message: string;
    finalNewlineAdded: boolean;
    author: { name: string; email: string };
    committer: { name: string; email: string };
    changes: {
      relativePath: string;
      oldObject: string;
      newObject: string;
      status: string;
    }[];
  } | null;
  network: {
    remote: string;
    reference: string;
    destination: string;
    location: string;
    previousCommit: string | null;
  } | null;
  pull: {
    target: {
      remote: string;
      reference: string;
      sourceCommit: string;
      expectedRemoteCommit: string;
      mode: "ff_only" | "rebase";
    };
    network: { location: string; destination: string };
    replayCommits: string[];
    affectedPaths: string[];
  } | null;
  discard:
    | {
        relativePath: string;
        indexObject: string;
        indexMode: string;
        patch: string;
      }[]
    | null;
  push: {
    target: {
      remote: string;
      reference: string;
      sourceCommit: string;
      expectedRemoteCommit: string | null;
    };
    location: string;
  } | null;
}
export interface GitApprovalRequest {
  operationId: string;
  nonce: string;
  plan: GitMutationPlan;
  isActive: () => Promise<boolean>;
}

export function useAgentGitApproval() {
  const { t } = useT();
  const [request, setRequest] = useState<GitApprovalRequest>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const resolve = useRef<((approved: boolean) => void) | undefined>(undefined);
  const deciding = useRef(false);
  const cancel = useRef<HTMLButtonElement>(null);
  const description = useId();
  const finish = useCallback((approved: boolean) => {
    resolve.current?.(approved);
    resolve.current = undefined;
    setRequest(undefined);
  }, []);
  useEffect(
    () => () => {
      resolve.current?.(false);
      resolve.current = undefined;
    },
    [],
  );
  const confirm = useCallback((next: GitApprovalRequest) => {
    if (resolve.current) return Promise.resolve(false);
    setError("");
    setBusy(false);
    deciding.current = false;
    return new Promise<boolean>((done) => {
      resolve.current = done;
      setRequest(next);
    });
  }, []);
  useEffect(() => {
    if (!request) return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    const check = async () => {
      if (!deciding.current) {
        const active = await request.isActive().catch(() => false);
        if (stopped) return;
        if (!active && !deciding.current) {
          finish(false);
          return;
        }
      }
      if (!stopped) timer = setTimeout(() => void check(), 500);
    };
    void check();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [request, finish]);
  const decide = async (approved: boolean) => {
    if (!request || deciding.current) return;
    deciding.current = true;
    setBusy(true);
    try {
      if (approved && !(await request.isActive()))
        throw new Error(
          t(
            "The Git request is no longer current. Cancel and request a fresh preview.",
          ),
        );
      await api("agent_control_git_mutation_decide", {
        operationId: request.operationId,
        nonce: request.nonce,
        planHash: request.plan.planHash,
        approved,
      });
      finish(approved);
    } catch (failure) {
      if (!approved) finish(false);
      else setError(errorMessage(failure));
    } finally {
      deciding.current = false;
      setBusy(false);
    }
  };
  const plan = request?.plan;
  return {
    confirm,
    dialog: request && plan && (
      <Modal
        protectTheme
        className="agent-git-approval"
        tone="warning"
        title={
          plan.operation === "pull"
            ? t("Pull this branch for the agent?")
            : plan.operation === "discard"
              ? t("Discard working changes for the agent?")
              : plan.operation === "push"
                ? t("Push this commit for the agent?")
                : plan.operation === "fetch"
                  ? t("Fetch this branch for the agent?")
                  : plan.operation === "commit"
                    ? t("Create this commit for the agent?")
                    : plan.operation === "stage"
                      ? t("Stage files for this agent?")
                      : t("Unstage files for this agent?")
        }
        descriptionId={description}
        initialFocus={cancel}
        closeDisabled={busy}
        onClose={() => {
          if (!busy) void decide(false);
        }}
      >
        <div className="dialog-form dialog-layout" aria-busy={busy}>
          <div className="dialog-content agent-git-details">
            <p id={description}>
              {plan.operation === "pull"
                ? t(
                    "Fetch the listed branch and integrate only the approved commit. A changed remote stops before integration. A conflict stays in the repository for you to resolve; Lomi does not stash or roll back your work.",
                  )
                : plan.operation === "discard"
                  ? t(
                      "Replace these working files with their staged versions. Discarded working changes are not saved in Git or Trash. Staged changes are kept; unsaved editor buffers must be resolved first.",
                    )
                  : plan.operation === "push"
                    ? t(
                        "Publish this exact commit to the listed remote branch. The request is limited to creating a branch or moving it forward from the expected commit.",
                      )
                    : plan.operation === "fetch"
                      ? t(
                          "Contact this remote and update the listed local tracking branch. This fetch does not merge, prune, fetch tags or recurse into submodules.",
                        )
                      : plan.operation === "commit"
                        ? t(
                            "Commit exactly the staged changes listed below. Newer disk versions and unsaved editor changes are not included.",
                          )
                        : plan.operation === "stage"
                          ? t(
                              "Stage the listed file versions from disk. Unsaved editor changes are not included. Configured filters may transform the staged content.",
                            )
                          : t(
                              "Remove the listed changes from the staging area. Working files are kept.",
                            )}
            </p>
            <p>
              {t(
                "Git may run this repository’s configured hooks, helpers or filters with your account’s access. Approve only if you trust the repository and its Git configuration.",
              )}
            </p>
            <p className="agent-git-repository">{plan.repositoryPath}</p>
            <p>
              {plan.branch ?? t("Detached HEAD")} ·{" "}
              {plan.head ? plan.head.slice(0, 12) : t("No commits yet")}
            </p>
            {plan.pull && (
              <>
                <p>
                  <strong>{plan.pull.target.remote}</strong>
                  <br />
                  {plan.pull.network.location}
                </p>
                <p>
                  {t("Remote branch:")}{" "}
                  <code>{plan.pull.target.reference}</code>
                  <br />
                  {t("Current commit:")}{" "}
                  <code>{plan.pull.target.sourceCommit}</code>
                  <br />
                  {t("Approved incoming commit:")}{" "}
                  <code>{plan.pull.target.expectedRemoteCommit}</code>
                </p>
                <p>
                  {plan.pull.target.mode === "ff_only"
                    ? t(
                        "Fast-forward only: local history will not be rewritten.",
                      )
                    : t(
                        "Rebase: replay these local commits on the incoming commit. Their IDs may change; equivalent or empty changes may be dropped. Other branches are kept.",
                      )}
                </p>
                {plan.pull.replayCommits.length > 0 && (
                  <ul aria-label={t("Commits to rebase")}>
                    {plan.pull.replayCommits.map((commit) => (
                      <li key={commit}>
                        <code>{commit}</code>
                      </li>
                    ))}
                  </ul>
                )}
                <p>
                  {t(
                    "The working files below may change. Unsaved editor buffers must be resolved first.",
                  )}
                </p>
              </>
            )}
            {plan.network && (
              <>
                <p>
                  <strong>{plan.network.remote}</strong>
                  <br />
                  {plan.network.location}
                </p>
                <p>
                  {t("Remote branch:")} <code>{plan.network.reference}</code>
                  <br />
                  {t("Local tracking branch:")}{" "}
                  <code>{plan.network.destination}</code>
                  <br />
                  {t("Current tracking commit:")}{" "}
                  <code>
                    {plan.network.previousCommit ?? t("Not fetched yet")}
                  </code>
                </p>
                <p>
                  {t(
                    "Credentials are omitted from this preview. Git may use your configured credentials and transports to connect.",
                  )}
                </p>
              </>
            )}
            {plan.push && (
              <>
                <p>
                  <strong>{plan.push.target.remote}</strong>
                  <br />
                  {plan.push.location}
                </p>
                <p>
                  {t("Remote branch:")}{" "}
                  <code>{plan.push.target.reference}</code>
                  <br />
                  {t("Expected remote commit:")}{" "}
                  <code>
                    {plan.push.target.expectedRemoteCommit ??
                      t("Branch must not exist")}
                  </code>
                  <br />
                  {t("Commit to publish:")}{" "}
                  <code>{plan.push.target.sourceCommit}</code>
                </p>
                <p>
                  {t(
                    "A changed remote rejects the update. Tags and other branches are excluded. Credentials are omitted from this preview.",
                  )}
                </p>
              </>
            )}
            {plan.commit && (
              <>
                <p>
                  {t("Author:")} {plan.commit.author.name} &lt;
                  {plan.commit.author.email}&gt;
                  <br />
                  {t("Committer:")} {plan.commit.committer.name} &lt;
                  {plan.commit.committer.email}&gt;
                </p>
                <label className="field">
                  <span>{t("Commit message")}</span>
                  <textarea
                    className="input agent-git-message"
                    rows={4}
                    readOnly
                    value={plan.commit.message}
                  />
                </label>
                {plan.commit.finalNewlineAdded && (
                  <p>
                    {t("Git adds a final newline; the preview includes it.")}
                  </p>
                )}
                <ul
                  className="agent-git-files"
                  aria-label={t("Staged changes")}
                >
                  {plan.commit.changes.map((change) => (
                    <li key={change.relativePath}>
                      <strong>{change.relativePath}</strong>
                      <span>
                        {change.status === "D"
                          ? t("Delete")
                          : change.status === "A"
                            ? t("Add")
                            : t("Modify")}
                      </span>
                      <code title={t("Previous object")}>
                        {change.oldObject}
                      </code>
                      <code title={t("Approved staged object")}>
                        {change.newObject}
                      </code>
                    </li>
                  ))}
                </ul>
              </>
            )}
            {!plan.commit && !plan.network && !plan.push && (
              <ul className="agent-git-files">
                {plan.files.map((file) => (
                  <li key={file.relativePath}>
                    <strong>{file.relativePath}</strong>
                    <span>
                      {file.sha256
                        ? t("{bytes} bytes", {
                            bytes: file.byteLength.toLocaleString(),
                          })
                        : t("Deleted from disk")}
                    </span>
                    {file.sha256 && (
                      <code title={t("SHA-256 of the approved disk version")}>
                        {file.sha256}
                      </code>
                    )}
                  </li>
                ))}
              </ul>
            )}
            {plan.discard?.map((file) => (
              <section
                key={file.relativePath}
                aria-label={t("Changes to discard in {path}", {
                  path: file.relativePath,
                })}
              >
                <p>
                  <strong>{file.relativePath}</strong>
                  <br />
                  {t("Restore staged object:")} <code>{file.indexObject}</code>
                  <br />
                  {t("File mode:")} <code>{file.indexMode}</code>
                </p>
                <textarea
                  className="agent-git-message"
                  aria-label={t("Working diff for {path}", {
                    path: file.relativePath,
                  })}
                  readOnly
                  value={file.patch}
                />
              </section>
            ))}
            <p>
              {t(
                "Changes to these files or the repository state require a new approval.",
              )}
            </p>
            {error && <p role="alert">{error}</p>}
          </div>
          <div className="dialog-actions">
            <button
              ref={cancel}
              className="button"
              type="button"
              disabled={busy}
              onClick={() => void decide(false)}
            >
              {t("Cancel")}
            </button>
            <button
              className="button button-primary"
              type="button"
              disabled={busy}
              onClick={() => void decide(true)}
            >
              {plan.operation === "pull"
                ? t("Pull this branch")
                : plan.operation === "discard"
                  ? t("Discard working changes")
                  : plan.operation === "push"
                    ? t("Push this commit")
                    : plan.operation === "fetch"
                      ? t("Fetch this branch")
                      : plan.operation === "commit"
                        ? t("Create this commit")
                        : plan.operation === "stage"
                          ? t("Stage these files")
                          : t("Unstage these files")}
            </button>
          </div>
        </div>
      </Modal>
    ),
  };
}
