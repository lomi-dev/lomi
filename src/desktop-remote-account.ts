import type { AuthState } from "./auth/model";

export function remoteAuthSessionKey(
  state: AuthState | null,
): string | undefined {
  if (
    state?.status !== "signed-in" ||
    state.user === null ||
    state.session === null
  ) {
    return undefined;
  }
  return JSON.stringify([state.user.id, state.session.id]);
}

export function remoteAccountLookupIsCurrent(
  expectedRevision: number,
  expectedSessionKey: string,
  current: AuthState | null,
): boolean {
  return (
    current !== null &&
    current.revision >= expectedRevision &&
    remoteAuthSessionKey(current) === expectedSessionKey
  );
}
