import { useSyncExternalStore } from "react";
import { api, type GitCommitDetails, type GitFileDiff } from "./api";
import type { Session } from "./model";

export type GitView =
  | { type: "diff"; relativePath: string; staged: boolean }
  | { type: "commit"; commit: string };
type NativeDiff = {
  kind: "diff";
  patch: string;
  notice: "binary" | "untracked" | null;
};
type NativeCommit = {
  kind: "commit";
  commit: string;
  author: string;
  authorEmail: string;
  authoredAt: string;
  committer: string;
  committerEmail: string;
  committedAt: string;
  parents: string[];
  message: string;
  omittedEntries: number;
  files: { relativePath: string; status: string }[];
};
export type GitViewBody = NativeDiff | NativeCommit;
export type PreparedGitView = {
  root: string;
  permitId: string;
  observationRevision: string;
  body: GitViewBody;
};
const sources = new Map<string, PreparedGitView>();
const listeners = new Set<() => void>();
const notify = () => {
  for (const listener of listeners) listener();
};
const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};
const release = (source?: PreparedGitView) => {
  if (source)
    void api("agent_control_git_release", { permitId: source.permitId }).catch(
      () => {},
    );
};
export const useAgentGit = (id?: string) =>
  useSyncExternalStore(subscribe, () => (id ? sources.get(id) : undefined));
export function stageAgentGit(id: string, source: PreparedGitView) {
  const previous = sources.get(id);
  const bytes = [...sources].reduce(
    (total, [key, value]) =>
      total + (key === id ? 0 : JSON.stringify(value).length * 2),
    JSON.stringify(source).length * 2,
  );
  if ((!previous && sources.size >= 64) || bytes > 16 * 1024 * 1024) {
    release(source);
    throw new Error("RESOURCE_EXHAUSTED");
  }
  sources.set(id, source);
  notify();
  return {
    commit: () => release(previous),
    cancel: () => {
      if (sources.get(id) !== source) return;
      if (previous) sources.set(id, previous);
      else sources.delete(id);
      release(source);
      notify();
    },
  };
}
export function retainAgentGit(session: Session) {
  const retained = new Set(
    session.projects.flatMap((p) =>
      p.workspaces.flatMap((w) =>
        w.tabs
          .filter(
            (t) => (t.type === "diff" || t.type === "commit") && t.agentGit,
          )
          .map((t) => t.id),
      ),
    ),
  );
  let changed = false;
  for (const [id, source] of sources)
    if (!retained.has(id)) {
      sources.delete(id);
      release(source);
      changed = true;
    }
  if (changed) notify();
}
const reads = new Map<string, Promise<GitViewBody>>();
let queue: Promise<unknown> = Promise.resolve();
export function readAgentGit(
  source: PreparedGitView | undefined,
  relative?: string,
): Promise<GitViewBody> {
  if (!source)
    return Promise.reject(
      new Error(
        "This agent Git view is no longer available. Ask the connected agent to open it again.",
      ),
    );
  const key = `${source.permitId}:${relative ?? ""}`;
  const pending = reads.get(key);
  if (pending) return pending;
  if (reads.size >= 64)
    return Promise.reject(new Error("Too many pending Git reads."));
  const read = queue.then(() =>
    api<GitViewBody>("agent_control_git_read", {
      permitId: source.permitId,
      relative: relative ?? null,
    }),
  );
  reads.set(key, read);
  queue = read.catch(() => {});
  void read
    .finally(() => {
      if (reads.get(key) === read) reads.delete(key);
    })
    .catch(() => {});
  return read;
}
export function agentDiff(body: GitViewBody): GitFileDiff {
  if (body.kind !== "diff")
    throw new Error("This Git view has different content.");
  const lines = body.patch.split("\n");
  const truncated = lines.length > 20000;
  return {
    patch: truncated ? lines.slice(0, 20000).join("\n") : body.patch,
    truncated,
    notice:
      body.notice === "binary"
        ? "Binary file: line-by-line changes are unavailable."
        : body.notice === "untracked"
          ? "This file is untracked. Git does not include it in a working-tree diff."
          : null,
  };
}
export function agentCommit(body: GitViewBody): GitCommitDetails {
  if (body.kind !== "commit")
    throw new Error("This Git view has different content.");
  return {
    commit: {
      id: body.commit,
      shortId: body.commit.slice(0, 7),
      subject: body.message.split("\n")[0] ?? "",
      authorName: body.author,
      authoredAt: body.authoredAt,
    },
    authorEmail: body.authorEmail,
    committerName: body.committer,
    committerEmail: body.committerEmail,
    committedAt: body.committedAt,
    parents: body.parents,
    message: body.message,
    files: body.files.map((f) => ({
      path: f.relativePath,
      originalPath: null,
      status: f.status,
      additions: null,
      deletions: null,
    })),
    statisticsUnavailable: true,
    omittedEntries: body.omittedEntries,
  };
}
