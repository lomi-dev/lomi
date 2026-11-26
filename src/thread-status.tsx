import { useEffect, useRef, useSyncExternalStore } from "react";
import { api, native } from "./api";
import { msg, t } from "./i18n";
import { activePanel, active, layoutPanes } from "./model";
import type { Session } from "./model";
import { listenThreadStatus, threadsStatus } from "./ai-chat/thread-runtime";
import type { RunState } from "./ai-chat/types";

/** What a chat thread shows in the chrome; "unseen" = finished while hidden. */
export type ThreadBadge = "running" | "waiting" | "unseen";

let badges: Readonly<Record<string, ThreadBadge>> = {};
const listeners = new Set<() => void>();
const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};
function setBadge(threadId: string, badge: ThreadBadge | undefined) {
  if (badges[threadId] === badge) return;
  const { [threadId]: _, ...rest } = badges;
  badges = badge ? { ...rest, [threadId]: badge } : rest;
  for (const listener of listeners) listener();
}

const strength: Record<ThreadBadge, number> = {
  unseen: 1,
  running: 2,
  waiting: 3,
};
const labels: Record<ThreadBadge, string> = {
  running: msg("Working"),
  waiting: msg("Waiting for your approval or answer"),
  unseen: msg("Reply finished"),
};

/** Strongest badge among the given threads (null ids are ignored). */
export function ThreadStatusDot({
  threadIds,
}: {
  threadIds: Array<string | null>;
}) {
  const current = useSyncExternalStore(subscribe, () => badges);
  let badge: ThreadBadge | undefined;
  for (const id of threadIds) {
    const next = id ? current[id] : undefined;
    if (next && (!badge || strength[next] > strength[badge])) badge = next;
  }
  if (!badge) return null;
  return (
    <span
      className="thread-status-dot"
      data-state={badge}
      role="img"
      aria-label={t(labels[badge])}
      title={t(labels[badge])}
    />
  );
}

function threadTitle(session: Session, threadId: string) {
  for (const project of session.projects)
    for (const workspace of project.workspaces)
      for (const tab of workspace.tabs)
        for (const item of tab.type === "terminal"
          ? layoutPanes(tab.layout)
          : [tab])
          if (item.type === "thread" && item.threadId === threadId)
            return item.customTitle ?? item.title;
}

function visibleThread(session: Session | undefined) {
  const selected = session && active(session);
  const panel = selected && activePanel(selected.tab);
  return document.hasFocus() && panel?.type === "thread"
    ? panel.threadId
    : null;
}
function clearSeen(session: Session | undefined) {
  const id = visibleThread(session);
  if (id && badges[id] === "unseen") setBadge(id, undefined);
}

/** One app-level listener: updates badges and notifies when a hidden thread needs you. */
export function useThreadStatus(session: Session | undefined) {
  const latest = useRef(session);
  latest.current = session;
  // Viewing the tab (or refocusing the window) clears the finished marker.
  useEffect(() => {
    clearSeen(session);
  }, [session]);
  useEffect(() => {
    const onFocus = () => clearSeen(latest.current);
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);
  useEffect(() => {
    const states = new Map<string, RunState>();
    const apply = (threadId: string, state: RunState, initial: boolean) => {
      const previous = states.get(threadId) ?? "idle";
      states.set(threadId, state);
      const visible = visibleThread(latest.current) === threadId;
      if (state === "idle")
        setBadge(
          threadId,
          previous !== "idle" && !initial && !visible ? "unseen" : undefined,
        );
      else setBadge(threadId, state);
      if (initial || visible || state === previous) return;
      const finished = state === "idle" && previous === "running";
      if (state !== "waiting" && !finished) return;
      // The thread title comes from user text; keep it out of OS notifications.
      if (native && latest.current && threadTitle(latest.current, threadId))
        void api("notify_thread", {
          kind: finished ? "finished" : "attention",
          title: t("Lomi chat"),
          body: finished ? t("Reply finished") : t("Needs your approval"),
        }).catch(() => {});
    };
    let alive = true;
    const stop = listenThreadStatus(
      (s) => alive && apply(s.threadId, s.state, false),
    );
    for (const project of latest.current?.projects ?? [])
      for (const workspace of project.workspaces)
        void threadsStatus(project.path, workspace.id)
          .then((all) => {
            if (alive)
              for (const s of all)
                if (!states.has(s.threadId)) apply(s.threadId, s.state, true);
          })
          .catch(() => {});
    return () => {
      alive = false;
      void stop.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);
}
