import assert from "node:assert/strict";
import test from "node:test";
import type { AuthState, AuthStatus } from "./auth/model";
import {
  remoteAccountLookupIsCurrent,
  remoteAuthSessionKey,
} from "./desktop-remote-account.ts";

function authState(revision: number, status: AuthStatus): AuthState {
  const signedIn = status === "signed-in";
  return {
    revision,
    status,
    user: signedIn
      ? {
          id: "legacy-auth-user-id",
          displayName: "User",
          email: "user@example.invalid",
          githubLogin: null,
          status: "active",
        }
      : null,
    session: signedIn ? { id: "auth-session", expiresAt: "" } : null,
    attempt: null,
    storage: null,
    message: null,
    remoteRevocationConfirmed: null,
  };
}

test("an unchanged signed-in auth revision accepts its account lookup", () => {
  assert.equal(
    remoteAccountLookupIsCurrent(
      4,
      remoteAuthSessionKey(authState(4, "signed-in"))!,
      authState(4, "signed-in"),
    ),
    true,
  );
});

test("same-session refreshes preserve a current lookup", () => {
  assert.equal(
    remoteAccountLookupIsCurrent(
      4,
      remoteAuthSessionKey(authState(4, "signed-in"))!,
      authState(5, "signed-in"),
    ),
    true,
  );
});

test("sign-out invalidates an account lookup started for the prior session", () => {
  assert.equal(
    remoteAccountLookupIsCurrent(
      4,
      remoteAuthSessionKey(authState(4, "signed-in"))!,
      authState(5, "signed-out"),
    ),
    false,
  );
});

test("a later session invalidates a lookup from the previous account", () => {
  const previous = authState(4, "signed-in");
  const replacement = authState(5, "signed-in");
  replacement.session = { id: "replacement-session", expiresAt: "" };

  assert.equal(
    remoteAccountLookupIsCurrent(
      4,
      remoteAuthSessionKey(previous)!,
      replacement,
    ),
    false,
  );
});

test("an older auth event cannot replace a newer session snapshot", () => {
  assert.equal(
    remoteAccountLookupIsCurrent(
      5,
      remoteAuthSessionKey(authState(5, "signed-in"))!,
      authState(4, "signed-in"),
    ),
    false,
  );
});
