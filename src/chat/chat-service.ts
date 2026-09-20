import { api, errorMessage } from "../api";
import { basename, chatTabs, layoutPanes, newId } from "../model";
import type { ChatTab, Project, Session, Workspace } from "../model";
import type { Conversation } from "./types";
let retained: ChatTab[] = [];
let session: Session | undefined;
let host:
  | {
      update: (
        id: string,
        change: Partial<Pick<ChatTab, "conversationId" | "title">>,
      ) => void;
      error: (message: string) => void;
      activate: (workspace: string, panel: string) => void;
    }
  | undefined;
let runtimeModule:
  Pick<typeof import("./chat-runtime"), "existing" | "retain"> | undefined;
export function registerChatRuntime(value: NonNullable<typeof runtimeModule>) {
  runtimeModule = value;
}
export function configureChats(value: NonNullable<typeof host>) {
  host = value;
  return () => {
    if (host === value) host = undefined;
  };
}
export function retainChats(value?: Session) {
  session = value;
  retained = chatTabs(value);
  runtimeModule?.retain(new Set(retained.map((t) => t.conversationId)));
  if (runtimeModule)
    void api("chat_retain", {
      conversations: [...new Set(retained.map((t) => t.conversationId))],
    }).catch(reportChatError);
}
export const retainedChatIds = () =>
  new Set(retained.map((t) => t.conversationId));
export function hasActiveChatRequests() {
  return retained.some(
    (tab) => runtimeModule?.existing(tab.conversationId)?.snapshot.busy,
  );
}
export function conversationTitle(id: string, title: string) {
  for (const tab of retained)
    if (tab.conversationId === id && tab.title !== title)
      host?.update(tab.id, { title });
}
export async function closeChatViews(panelIds?: ReadonlySet<string>) {
  const removed = retained.filter((t) => !panelIds || panelIds.has(t.id));
  const remaining = retained.filter((t) => panelIds && !panelIds.has(t.id));
  const ids = [...new Set(removed.map((t) => t.conversationId))].filter(
    (id) => !remaining.some((t) => t.conversationId === id),
  );
  if (!ids.length && panelIds) return;
  for (const id of ids) {
    const entry = runtimeModule?.existing(id);
    if (entry?.snapshot.busy) await entry.stop();
  }
  for (const id of ids) await runtimeModule?.existing(id)?.flush();
  await api("chat_close", { conversations: ids, all: !panelIds });
}
export async function createChat(project: Project, workspace: Workspace) {
  return api<Conversation>("chat_main", {
    input: {
      action: "create",
      id: newId(),
      origin: {
        projectId: project.id,
        projectName: basename(project.path),
        workspaceId: workspace.id,
        workspaceName: workspace.name,
      },
    },
  });
}
export async function replaceChat(tab: ChatTab, conversation?: Conversation) {
  if (conversation && session) {
    const workspace = session.projects
      .flatMap((p) => p.workspaces)
      .find((w) =>
        w.tabs.some(
          (t) =>
            t.id === tab.id ||
            (t.type === "terminal" && contains(t.layout, tab.id)),
        ),
      );
    const conversationId = conversation.id;
    const existing = workspace?.tabs
      .flatMap((t) =>
        t.type === "terminal"
          ? layoutPanes(t.layout).filter((p): p is ChatTab => p.type === "chat")
          : t.type === "chat"
            ? [t]
            : [],
      )
      .find((p) => p.conversationId === conversationId);
    if (workspace && existing) {
      host?.activate(workspace.id, existing.id);
      return;
    }
  }
  await closeChatViews(new Set([tab.id]));
  if (!conversation) {
    const project = session?.projects.find((p) =>
      p.workspaces.some((w) =>
        w.tabs.some(
          (t) =>
            t.id === tab.id ||
            (t.type === "terminal" && contains(t.layout, tab.id)),
        ),
      ),
    );
    const workspace = project?.workspaces.find((w) =>
      w.tabs.some(
        (t) =>
          t.id === tab.id ||
          (t.type === "terminal" && contains(t.layout, tab.id)),
      ),
    );
    if (!project || !workspace)
      throw Error("The chat panel is no longer in a workspace.");
    conversation = await createChat(project, workspace);
  }
  host?.update(tab.id, {
    conversationId: conversation.id,
    title: conversation.title,
  });
}
function contains(layout: import("../model").Layout, id: string): boolean {
  return layout.type === "split"
    ? contains(layout.first, id) || contains(layout.second, id)
    : layout.id === id;
}
export function chatAction(id: string, action: string) {
  document
    .querySelector<HTMLElement>(`[data-chat-pane-id="${CSS.escape(id)}"]`)
    ?.dispatchEvent(new CustomEvent("chat-action", { detail: action }));
}
export function reportChatError(error: unknown) {
  host?.error(errorMessage(error));
}

const activities = new Map<string, string>();
const activityListeners = new Set<() => void>();
let activityRevision = 0;
export const chatActivityRevision = () => activityRevision;
export const subscribeChatActivity = (listener: () => void) => {
  activityListeners.add(listener);
  return () => {
    activityListeners.delete(listener);
  };
};
export function chatActivity(id: string, status?: string) {
  if (status === undefined) return activities.get(id) ?? "";
  if ((activities.get(id) ?? "") === status) return status;
  if (status) activities.set(id, status);
  else activities.delete(id);
  activityRevision++;
  activityListeners.forEach((listener) => listener());
  return status;
}
