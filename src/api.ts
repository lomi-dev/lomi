import { invoke, isTauri } from "@tauri-apps/api/core";
import type { AppInfo, Session } from "./model";

export const native = isTauri();
export const macOS = navigator.platform.startsWith("Mac");
export const windows = navigator.platform.startsWith("Win");
export const api = invoke;
export const errorMessage = (error: unknown) =>
  error &&
  typeof error === "object" &&
  "message" in error &&
  typeof error.message === "string"
    ? error.message
    : String(error);

export interface FileEntry {
  name: string;
  relativePath: string;
  path: string;
  isDirectory: boolean;
  isSymlink: boolean;
}
export interface GitChange {
  path: string;
  originalPath: string | null;
  index: string;
  worktree: string;
}
export interface GitStatus {
  root: string;
  branch: string;
  changes: GitChange[];
}
export interface GitRepositoryScan {
  repositories: GitStatus[];
  errors: { root: string; message: string }[];
  limited: boolean;
}
export interface GitCommitSummary {
  id: string;
  shortId: string;
  subject: string;
  authorName: string;
  authoredAt: string;
}
export interface GitHistoryPage {
  commits: GitCommitSummary[];
  tips: string[];
  hasMore: boolean;
}
export interface GitCommitFile {
  path: string;
  originalPath: string | null;
  status: string;
  additions: number | null;
  deletions: number | null;
}
export interface GitCommitDetails {
  commit: GitCommitSummary;
  authorEmail: string;
  committerName: string;
  committerEmail: string;
  committedAt: string;
  parents: string[];
  message: string;
  files: GitCommitFile[];
}
export interface GitCommitDiff {
  patch: string;
  truncated: boolean;
}
export interface GitFileDiff extends GitCommitDiff {
  notice: string | null;
}

export const getInfo = () => api<AppInfo>("app_info");
export const loadSession = () => api<unknown>("load_session");
let pendingSave = Promise.resolve();
export function saveSession(data: Session, recovery = false) {
  pendingSave = pendingSave
    .catch(() => {})
    .then(() => api("save_session", { data, recovery }));
  return pendingSave;
}
