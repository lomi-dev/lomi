import { layoutPanes, type ChatTab, type Session } from "./model.ts";

export interface ChatOpenCommand {
  type: "open_chat";
  workspaceId: string;
  conversationId: string;
  panelId: string;
  create: boolean;
  notAfterMillis: string;
}

export interface ChatDraftCommand {
  type: "draft_chat";
  workspaceId: string;
  notAfterMillis: string;
  input: {
    workspaceId: string;
    panelId: string;
    conversationId: string;
    text: string;
    expectedDraftRevision: string;
    expectedConversationRevision: string;
  };
}
export interface ChatDraftUpdated {
  workspaceId: string;
  panelId: string;
  conversationId: string;
  draftRevision: string;
  conversationRevision: string;
  textSha256: string;
  totalUtf16: number;
}

export interface ChatSendCommand {
  type: "send_chat";
  workspaceId: string;
  requestId: string;
  userId: string;
  assistantId: string;
  notAfterMillis: string;
  input: {
    workspaceId: string;
    panelId: string;
    conversationId: string;
    connectionId: string;
    model: string;
    expectedDraftRevision: string;
    expectedConversationRevision: string;
  };
}
export interface ChatSent {
  workspaceId: string;
  panelId: string;
  conversationId: string;
  requestId: string;
  userId: string;
  assistantId: string;
  connectionId: string;
  model: string;
  draftRevision: string | null;
  rejection: string | null;
}

export function openAgentChat(
  session: Session,
  projectId: string,
  command: ChatOpenCommand,
  title: string,
): { session: Session; panel: ChatTab } {
  const project = session.projects.find((p) => p.id === projectId);
  const workspace = project?.workspaces.find(
    (w) => w.id === command.workspaceId,
  );
  if (!project || !workspace) throw Error("TARGET_NOT_FOUND");
  const match = workspace.tabs
    .flatMap((tab) =>
      (tab.type === "terminal" ? layoutPanes(tab.layout) : [tab]).map(
        (panel) => ({ tab, panel }),
      ),
    )
    .find(
      ({ panel }) =>
        panel.type === "chat" &&
        panel.conversationId === command.conversationId,
    );
  const existing = match?.panel.type === "chat" ? match.panel : undefined;
  if (existing && existing.id !== command.panelId)
    throw Error("REVISION_CONFLICT");
  const panel: ChatTab = existing ?? {
    type: "chat",
    id: command.panelId,
    conversationId: command.conversationId,
    title,
  };
  if (
    !existing &&
    session.projects.some((p) =>
      p.workspaces.some((w) =>
        w.tabs.some(
          (t) =>
            t.id === panel.id ||
            (t.type === "terminal" &&
              layoutPanes(t.layout).some((child) => child.id === panel.id)),
        ),
      ),
    )
  )
    throw Error("REVISION_CONFLICT");
  return {
    panel,
    session: {
      ...session,
      activeProjectId: projectId,
      projects: session.projects.map((p) =>
        p.id !== projectId
          ? p
          : {
              ...p,
              activeWorkspaceId: workspace.id,
              workspaces: p.workspaces.map((w) =>
                w.id !== workspace.id
                  ? w
                  : {
                      ...w,
                      activeTabId: match?.tab.id ?? panel.id,
                      tabs: existing
                        ? match?.tab.type === "terminal"
                          ? w.tabs.map((tab) =>
                              tab.id === match.tab.id && tab.type === "terminal"
                                ? { ...tab, activePaneId: panel.id }
                                : tab,
                            )
                          : w.tabs
                        : [...w.tabs, panel],
                    },
              ),
            },
      ),
    },
  };
}
