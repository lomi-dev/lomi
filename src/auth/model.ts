import { t } from "../i18n";

export type AuthStatus =
  | "unavailable"
  | "signed-out"
  | "authorizing"
  | "checking"
  | "signed-in"
  | "offline"
  | "storage-locked"
  | "error";

export type AuthStorage = "persistent" | "session";

export interface AuthState {
  revision: number;
  status: AuthStatus;
  user: {
    id: string;
    displayName: string;
    email: string;
    githubLogin: string | null;
    status: string;
  } | null;
  session: { id: string; expiresAt: string } | null;
  attempt: {
    id: string;
    expiresAt: string;
  } | null;
  storage: AuthStorage | null;
  message: string | null;
  remoteRevocationConfirmed: boolean | null;
}

export interface AccountSession {
  id: string;
  kind: "desktop" | "browser" | "unknown";
  createdAt: string;
  updatedAt: string;
  expiresAt: string;
  isCurrent: boolean;
}

export type RevokeOutcome = "revoked" | "reauth-required";

export const unavailableState: AuthState = {
  revision: 0,
  status: "unavailable",
  user: null,
  session: null,
  attempt: null,
  storage: null,
  message: null,
  remoteRevocationConfirmed: null,
};

export function canShareRemotely(state: AuthState | null): boolean {
  return Boolean(state?.status === "signed-in" && state.user && state.session);
}

export function newestAuthState(
  current: AuthState | null,
  incoming: AuthState,
): AuthState | null {
  if (
    !Number.isSafeInteger(incoming.revision) ||
    incoming.revision < 0 ||
    (current !== null && incoming.revision <= current.revision)
  ) {
    return current;
  }
  return incoming;
}

export function authStatusLabel(status: AuthStatus): string {
  switch (status) {
    case "unavailable":
      return t("Unavailable");
    case "signed-out":
      return t("Signed out");
    case "authorizing":
      return t("Waiting for browser");
    case "checking":
      return t("Checking account…");
    case "signed-in":
      return t("Signed in");
    case "offline":
      return t("Connection unavailable");
    case "storage-locked":
      return t("Credential store locked");
    case "error":
      return t("Needs attention");
  }
}
