import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage, native } from "./api";
import { canShareRemotely } from "./auth/model";
import { useAuthState } from "./auth/useAuthState";
import type { Session, ShellProfile } from "./model";
import {
  runningTerminal,
  subscribeTerminalLifecycle,
  terminalFor,
} from "./terminal-runtime";
import {
  WorkspacePublisher,
  workspaceTerminals,
} from "./remote-workspace-domain";
import type { RemoteState } from "./remote-workspace-domain";
export type { RemoteState } from "./remote-workspace-domain";

export function useRemoteWorkspaces(
  session: Session | undefined,
  current: { current: Session | undefined },
  profiles: ShellProfile[] | undefined,
) {
  const auth = useAuthState();
  const available = canShareRemotely(auth.state);
  const [state, setState] = useState<RemoteState>();
  const [error, setError] = useState("");
  const [busy, setBusy] = useState<string>();
  const acting = useRef(false);
  const profileRef = useRef(profiles);
  profileRef.current = profiles;
  const publisher = useRef<WorkspacePublisher | undefined>(undefined);
  const blocked = useRef(new Set<string>());
  const preparing = useRef(new Set<string>());
  const accept = useRef<(next: RemoteState) => void>(() => {});
  accept.current = (next) => {
    if (!next?.workspaces) return;
    setState(next);
    for (const workspace of next.workspaces) {
      if (workspace.shared && !blocked.current.has(workspace.id))
        publisher.current?.desired.add(workspace.id);
      else if (!preparing.current.has(workspace.id))
        publisher.current?.desired.delete(workspace.id);
    }
  };
  useEffect(() => {
    if (!native) return;
    let alive = true;
    const engine = new WorkspacePublisher({
      session: () => current.current,
      profiles: () => profileRef.current ?? [],
      sessionId: (id) => runningTerminal(id)?.sessionId ?? null,
      start: (pane, profile) => terminalFor(pane, profile).startInBackground(),
      begin: () => api("remote_begin_workspace_sync"),
      sync: async (epoch, revision, workspaces) => {
        const next = await api<RemoteState>("remote_sync_workspaces", {
          epoch,
          revision,
          workspaces,
        });
        if (alive) accept.current(next);
      },
    });
    publisher.current = engine;
    const publish = () => {
      if (current.current && profileRef.current)
        void engine.notify().catch((e) => {
          if (alive) setError(errorMessage(e));
        });
    };
    const unsubscribe = subscribeTerminalLifecycle(publish);
    const refresh = async () => {
      try {
        const next = await api<RemoteState>("remote_get_state");
        if (alive) {
          accept.current(next);
          publish();
        }
      } catch (e) {
        if (alive) setError(errorMessage(e));
      }
    };
    void refresh();
    const timer = setInterval(() => void refresh(), 2000);
    const listener = listen<RemoteState>("lomi-remote-state", (event) => {
      if (alive) {
        accept.current(event.payload);
        publish();
      }
    });
    return () => {
      alive = false;
      engine.stop();
      publisher.current = undefined;
      unsubscribe();
      clearInterval(timer);
      void listener.then((unlisten) => unlisten()).catch(() => {});
    };
  }, [current]);
  useEffect(() => {
    if (session && profiles)
      void publisher.current?.notify().catch((e) => setError(errorMessage(e)));
  }, [session, profiles]);
  const share = async (workspaceId: string, shared: boolean) => {
    if (!canShareRemotely(auth.current.current)) return;
    if (shared && !state?.qualified) {
      setError(
        "Remote hosting currently requires Lomi on a Mac with Apple silicon.",
      );
      return;
    }
    if (acting.current) return;
    const engine = publisher.current;
    if (!engine) {
      setError(
        native
          ? "Remote sharing is not ready. Try again shortly."
          : "Remote sharing requires the Lomi desktop app.",
      );
      return;
    }
    acting.current = true;
    setBusy(workspaceId);
    setError("");
    try {
      if (shared) {
        blocked.current.delete(workspaceId);
        const selected = new Set([...engine.desired, workspaceId]);
        const inventory = workspaceTerminals(current.current).filter((w) =>
          selected.has(w.id),
        );
        if (
          inventory.length > 32 ||
          inventory.reduce((sum, w) => sum + w.terminals.length, 0) > 32
        )
          throw new Error(
            "Remote supports up to 32 shared workspaces and 32 active desktop terminals in total, including terminals in unshared workspaces.",
          );
        preparing.current.add(workspaceId);
        engine.desired.add(workspaceId);
        await engine.notify();
        if (publisher.current !== engine)
          throw new Error(
            "Remote sharing was interrupted. Try sharing the workspace again.",
          );
        if (
          !workspaceTerminals(current.current).some((w) => w.id === workspaceId)
        )
          throw new Error("The workspace was closed.");
      } else {
        blocked.current.add(workspaceId);
        engine.desired.delete(workspaceId);
      }
      if (!canShareRemotely(auth.current.current))
        throw new Error("Sign in to use remote sharing.");
      const next = await api<RemoteState>("remote_share_workspace", {
        workspaceId,
        shared,
      });
      if (
        shared &&
        !next?.workspaces?.some(
          (w) => w.id === workspaceId && w.shared === true,
        )
      )
        throw new Error(
          "Remote sharing was not confirmed. Try sharing the workspace again.",
        );
      accept.current(next);
      void engine.notify().catch((e) => setError(errorMessage(e)));
    } catch (e) {
      setError(errorMessage(e));
      if (shared) engine.desired.delete(workspaceId);
    } finally {
      acting.current = false;
      preparing.current.delete(workspaceId);
      setBusy(undefined);
    }
  };
  const resume = async () => {
    if (
      !canShareRemotely(auth.current.current) ||
      !state?.qualified ||
      acting.current
    )
      return;
    acting.current = true;
    setBusy("resume");
    setError("");
    try {
      accept.current(await api<RemoteState>("remote_resume"));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      acting.current = false;
      setBusy(undefined);
    }
  };
  return { state, error, busy, share, resume, available };
}
