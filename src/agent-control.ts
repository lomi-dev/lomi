import type { AndroidManagementApproval } from "./AgentAndroidApproval";
import {
  openAgentChat,
  type ChatOpenCommand,
  type ChatDraftCommand,
  type ChatDraftUpdated,
  type ChatSendCommand,
} from "./agent-chat";
import {
  moveAgentPanel,
  panelMoveIdentities,
  type PanelMove,
  type PanelMoveIdentity,
  type PanelMoveDestination,
} from "./agent-layout";
import {
  agentSessionSignature,
  closeAgentProject,
  closeAgentWorkspace,
  sameAgentSession,
} from "./agent-workspace";
import { useEffect, useRef } from "react";
import {
  useAgentSettingsReader,
  type SettingsReadRequest,
} from "./agent-settings";
import { listen } from "@tauri-apps/api/event";
import { api, native } from "./api";
import type { KeybindingPatch, KeybindingSource } from "./agent-keybindings";
import type {
  EditorReadInput,
  EditorEditsInput,
  EditorSaveInput,
} from "./editor-control";
import {
  stageAgentGit,
  retainAgentGit,
  type PreparedGitView,
  type GitView,
} from "./agent-git";
import { stageAgentPreview, type PreparedEditorFile } from "./agent-preview";
import { isMarkdownFile } from "./markdown";
import type { FileChange } from "./explorer-model";
import {
  loadedEditor,
  closingEditorDocuments,
  assertCleanEditorPaths,
  openEditorDocument,
  retainEditorTabs,
  stageEditorRead,
} from "./editor-service";
import {
  basename,
  active,
  activePanel,
  layoutPanes,
  removePane,
  newFileTab,
  openFileTab,
  openDiffTab,
  openCommitTab,
  updateTab,
  updateFile,
  type FilePreviewView,
  type TerminalTab,
  type BrowserTab,
  type AndroidTab,
  type Session,
} from "./model";
import {
  runningTerminal,
  closeTerminals,
  stageAgentTerminal,
  clearAgentTerminal,
  waitForAgentTerminal,
} from "./terminal-runtime";
import {
  stageAgentBrowser,
  clearAgentBrowser,
  waitForAgentBrowser,
  hasLiveAgentBrowser,
} from "./browser-runtime";

export interface ControlWorkspace {
  id: string;
  projectId: string;
  name: string;
  projectName: string;
  projectPath: string;
}
export interface ControlState {
  supported: boolean;
  helperPath: string | null;
  broker: null | {
    endpoint: {
      instanceId: string;
      endpoint: string;
      brokerSha256: string;
      ipcVersion: number;
    };
    uiReady: boolean;
    terminalProfile: { id: string; revision: string } | null;
    terminalProfiles?: { id: string; revision: string }[];
    workspaces: ControlWorkspace[];
    pending: {
      id: string;
      clientLabel: string;
      certificateSha256: string;
      secondsRemaining: number;
    }[];
    pendingProjectOpens: {
      operationId: string;
      clientLabel: string;
      projectPath: string;
      workspaceName: string;
      scopes: string[];
      requestKey: string;
      secondsRemaining: number;
    }[];
    pendingSettingsUpdates: {
      operationId: string;
      clientLabel: string;
      requestKey: string;
      section: "editor" | "terminal" | "keybinds" | "themes";
      patch?: { type: string };
      before: {
        tabSize?: number;
        insertSpaces?: boolean;
        [key: string]: unknown;
      };
      after: {
        tabSize?: number;
        insertSpaces?: boolean;
        [key: string]: unknown;
      };
      secondsRemaining: number;
    }[];
    pendingAndroidManagement?: AndroidManagementApproval[];
    pendingBrowserUploads?: {
      operationId: string;
      clientLabel: string;
      workspaceId: string;
      panelId: string;
      elementRef: string;
      target: {
        frameId: string;
        origin: string;
        documentUrl: string;
        label: string;
      };
      fileName: string;
      byteLength: number;
      sha256: string;
      secondsRemaining: number;
    }[];
    pendingInstalls: {
      operationId: string;
      clientLabel: string;
      workspaceId: string;
      deviceId: string;
      generation: string;
      title: string;
      relativePath: string;
      artifactId: string;
      sha256: string;
      byteLength: number;
      secondsRemaining: number;
    }[];
    pendingControls: {
      operationId: string;
      clientLabel: string;
      workspaceId: string;
      panelId: string;
      terminalSessionId: string;
      title: string;
      secondsRemaining: number;
    }[];
    sessions: {
      id: string;
      clientLabel: string;
      projectIds: string[];
      workspaceIds: string[];
      scopes: string[];
      terminalProfile?: { id: string; revision: string } | null;
      browserOrigins: string[];
      androidDeviceIds: string[];
      androidPackages: string[];
      chatConversations: string[];
    }[];
  };
}

/** Workbench remains the domain authority. Serial publication prevents a slow
 * native response from restoring an older domain snapshot. Reload drops grants. */
interface UiCommand {
  operationId: string;
  nonce: string;
  uiEpoch: string;
  domainRevision: string;
  projectId: string;
  action:
    | ChatOpenCommand
    | ChatDraftCommand
    | ChatSendCommand
    | {
        type: "open_project";
        workspaceId: string;
        projectId: string;
        projectPath: string;
        newWorkspaceId: string;
        tabId: string;
        name: string;
        requestKey: string;
        notAfterMillis: string;
      }
    | {
        type: "git_open";
        workspaceId: string;
        repositoryRelative: string;
        view: GitView;
        notAfterMillis: string;
      }
    | {
        type: "git_mutate";
        workspaceId: string;
        notAfterMillis: string;
      }
    | {
        type: "files_mutate";
        workspaceId: string;
        projectPath: string;
        notAfterMillis: string;
        input: { operation: { type: string; relativePath: string } };
      }
    | {
        type: "editor_save";
        workspaceId: string;
        projectPath: string;
        notAfterMillis: string;
        input: EditorSaveInput;
      }
    | {
        type: "editor_open";
        workspaceId: string;
        relativePath: string;
        presentation: FilePreviewView;
        projectPath: string;
        notAfterMillis: string;
      }
    | {
        type: "editor_edits";
        workspaceId: string;
        projectPath: string;
        notAfterMillis: string;
        input: EditorEditsInput;
      }
    | {
        type: "update_settings";
        workspaceId: string;
        notAfterMillis: string;
        input: {
          expectedSettingsRevision: string;
          patch:
            | { type: "editor_tab_size"; value: number }
            | { type: "editor_insert_spaces"; value: boolean }
            | KeybindingPatch
            | { type: "theme_builtin"; value: "lomi" | "deepmono" }
            | { type: "theme_appearance"; value: "system" | "light" | "dark" }
            | {
                type: "terminal_field";
                field: string;
                value: string | number | boolean | null;
              };
        };
      }
    | {
        type: "open_settings";
        workspaceId: string;
        page:
          | "keybinds"
          | "themes"
          | "plugins"
          | "editor"
          | "terminal"
          | "about"
          | "chat-ai"
          | "android"
          | "agent-control";
      }
    | { type: "import_artifact"; workspaceId: string }
    | { type: "download_browser"; workspaceId: string }
    | { type: "upload_browser"; workspaceId: string }
    | { type: "export_artifact"; workspaceId: string; notAfterMillis: string }
    | {
        type: "android_launch";
        workspaceId: string;
        panelId: string;
        deviceId: string;
        generation: string;
      }
    | {
        type: "android_input" | "android_input_control";
        workspaceId: string;
        panelId: string;
        deviceId: string;
        generation: string;
        action?: "claim" | "release";
      }
    | {
        type: "android_runtime";
        workspaceId: string;
        panelId: string;
        deviceId: string;
        generation: string | null;
      }
    | {
        type: "create_android";
        workspaceId: string;
        panelId: string;
        deviceId: string;
        title: string;
      }
    | {
        type: "interact_browser";
        workspaceId: string;
        panelId: string;
        browserGeneration: string;
        navigationId: string;
        snapshotId: string;
        elementRef: string;
        interaction:
          | { type: "click" }
          | { type: "fill"; text: string }
          | { type: "key"; key: string }
          | { type: "scroll"; deltaX: number; deltaY: number };
      }
    | {
        type: "navigate_browser";
        workspaceId: string;
        panelId: string;
        browserGeneration: string;
        url: string;
        waitUntil: "commit" | "load";
      }
    | {
        type: "create_browser";
        visible: boolean;
        workspaceId: string;
        panelId: string;
        browserGeneration: string;
        profileId: string;
        url: string;
      }
    | {
        type: "move_panel";
        workspaceId: string;
        movement: PanelMove;
        panels: PanelMoveIdentity[];
        tabOrder: string[];
        focusedPanelId: string | null;
        destination: PanelMoveDestination | null;
      }
    | {
        type: "close_workspace";
        workspaceId: string;
        panels: PanelMoveIdentity[];
        notAfterMillis: string;
      }
    | {
        type: "close_project";
        workspaceId: string;
        workspaces: {
          workspaceId: string;
          panels: PanelMoveIdentity[];
          notAfterMillis: string;
        }[];
        notAfterMillis: string;
      }
    | { type: "rename_workspace"; workspaceId: string; name: string }
    | {
        type: "close_panel";
        workspaceId: string;
        panelId: string;
        tabId: string;
        terminalSessionId: string | null;
        browserGeneration: string | null;
        replacementTabId: string;
      }
    | {
        type: "focus_panel" | "select_workspace";
        workspaceId: string;
        panelId: string;
        tabId: string;
        terminalSessionId: string | null;
        browserGeneration: string | null;
      }
    | {
        type: "create_workspace";
        anchorWorkspaceId: string;
        workspaceId: string;
        tabId: string;
        name: string;
      }
    | {
        type: "create_terminal";
        workspaceId: string;
        panelId: string;
        tabId: string;
        terminalSessionId: string;
        profileId: string;
        cwd: string;
        title: string;
      };
}
interface FileTrashRequest {
  operationId: string;
  nonce: string;
  projectId: string;
  projectPath: string;
  relativePath: string;
  notAfterMillis: string;
}
async function checkAgentPanels(
  panels: readonly (import("./model").Tab | import("./model").LayoutPane)[],
  projectId: string,
  reveal: boolean,
) {
  const phones = panels.filter((p) => p.type === "android");
  if (reveal && phones.length) {
    const { refreshAndroid, androidSnapshot } = await import("./android/state");
    const runtime = await import("./android/runtime");
    await refreshAndroid();
    if (
      phones.some(
        (p) =>
          !p.deviceId ||
          runtime.stopPending(p.deviceId) ||
          (p.startMode !== "manual" &&
            runtime.launchPending(p.id, p.deviceId)) ||
          !androidSnapshot()?.statuses.some(
            (s) =>
              s.deviceId === p.deviceId &&
              s.phase === "running" &&
              s.processAlive,
          ),
      )
    )
      throw Error("UI_NOT_READY");
  }
  const chats = panels.filter((p) => p.type === "chat");
  if (!chats.length) return;
  const module = await import("./chat/chat-runtime");
  for (const panel of chats) {
    const runtime =
      module.existing(panel.conversationId) ??
      (reveal ? module.getChat(panel.conversationId) : undefined);
    if (!runtime) continue;
    await runtime.ready;
    if (runtime.snapshot.loaded?.conversation.origin.projectId !== projectId)
      throw Error("TARGET_NOT_FOUND");
  }
}
export function useAgentControlBridge(
  session: Session | undefined,
  getCurrent: () => Session | undefined,
  setCurrent: (session: Session) => void,
  confirmClose: (panelId: string, terminal: boolean) => Promise<boolean>,
  runFileOperation: <T>(
    action: () => Promise<T>,
    trash?: FileTrashRequest,
  ) => Promise<T>,
  applyFileChange: (change: FileChange, canonicalChange: FileChange) => void,
  confirmGit: (
    request: import("./AgentGitApproval").GitApprovalRequest,
  ) => Promise<boolean>,
  confirmChat: (
    request: import("./AgentChatApproval").ChatApprovalRequest,
  ) => Promise<boolean>,
  confirmWorkspaceClose: (
    ids: ReadonlySet<string>,
    decision: import("./EditorCloseGuard").EditorCloseDecision,
  ) => Promise<boolean>,
  refreshGit: () => void,
  layoutSize: () => { width: number; height: number } | null,
) {
  const readSettings = useAgentSettingsReader();
  const settingsReader = useRef(readSettings);
  settingsReader.current = readSettings;
  const domain = useRef({
    getCurrent,
    setCurrent,
    confirmClose,
    runFileOperation,
    applyFileChange,
    confirmGit,
    confirmChat,
    confirmWorkspaceClose,
    refreshGit,
    layoutSize,
  });
  domain.current = {
    getCurrent,
    setCurrent,
    confirmClose,
    runFileOperation,
    applyFileChange,
    confirmGit,
    confirmChat,
    confirmWorkspaceClose,
    refreshGit,
    layoutSize,
  };
  const publish = useRef<() => void>(() => {});
  useEffect(() => {
    if (!native) return;
    let alive = true;
    let epoch: string | null = null;
    let revision = 0;
    let published: Session | undefined;
    let publishedSignature: string | undefined;
    let queue = Promise.resolve();
    const send = async () => {
      const value = domain.current.getCurrent();
      if (!alive || !epoch || !value) return;
      retainAgentGit(value);
      const projection = {
        uiEpoch: epoch,
        focusedPanelId: active(value)?.tab
          ? (activePanel(active(value)!.tab)?.id ?? null)
          : null,
        panels: value.projects.flatMap((p) =>
          p.workspaces.flatMap((w) =>
            w.tabs.flatMap((t) =>
              (t.type === "terminal" ? layoutPanes(t.layout) : [t]).map(
                (panel) => ({
                  id: panel.id,
                  tabId: t.id,
                  workspaceId: w.id,
                  kind: panel.type,
                  title: t.customTitle ?? t.title,
                  terminalSessionId:
                    panel.type === "terminal"
                      ? (runningTerminal(panel.id)?.sessionId ?? null)
                      : null,
                  chatConversationId:
                    panel.type === "chat" ? panel.conversationId : null,
                  androidDeviceId:
                    panel.type === "android" ? panel.deviceId : null,
                  browserGeneration:
                    panel.type === "browser"
                      ? (panel.automation?.generation ?? null)
                      : null,
                }),
              ),
            ),
          ),
        ),
        workspaces: value.projects.flatMap((project) =>
          project.workspaces.map((workspace) => ({
            id: workspace.id,
            name: workspace.name,
            projectId: project.id,
            projectName: basename(project.path),
            projectPath: project.path,
            activePanelId: (() => {
              const tab = workspace.tabs.find(
                (t) => t.id === workspace.activeTabId,
              );
              return tab ? (activePanel(tab)?.id ?? null) : null;
            })(),
          })),
        ),
      };
      const signature = `${agentSessionSignature(value)}\n${JSON.stringify(projection)}`;
      if (signature !== publishedSignature) {
        await api("agent_control_ui_publish", {
          projection: { ...projection, revision: String(++revision) },
        });
        publishedSignature = signature;
      }
      published = value;
    };
    const enqueue = (register: boolean) => {
      queue = queue
        .then(async () => {
          if (!alive) return;
          if (register || !epoch) {
            epoch = await api<string | null>("agent_control_ui_register");
            revision = 0;
          }
          await send();
        })
        .catch(() => {
          // Native state remains unavailable until a successful fresh snapshot;
          // never publish or disclose a cached snapshot after an error.
          epoch = null;
        });
    };
    publish.current = () => enqueue(false);
    const stop = listen("agent-control-refresh", () => enqueue(true));
    const screens = listen<{
      requestId: string;
      uiEpoch: string;
      workspaceId: string;
      panelId: string;
      terminalSessionId: string;
      minimumParsedSequence: string;
      maxBytes: number;
    }>("agent-control-screen", ({ payload: request }) => {
      if (!alive || epoch !== request.uiEpoch) return;
      const runtime = runningTerminal(request.panelId);
      const screen =
        runtime?.sessionId === request.terminalSessionId
          ? runtime.controlScreen(request.maxBytes)
          : null;
      void api("agent_control_terminal_screen_reply", {
        reply: {
          requestId: request.requestId,
          uiEpoch: request.uiEpoch,
          screen: screen
            ? {
                ...screen,
                workspaceId: request.workspaceId,
                panelId: request.panelId,
                terminalSessionId: request.terminalSessionId,
              }
            : null,
        },
      }).catch(() => {});
    });
    const settings = listen<SettingsReadRequest>(
      "agent-control-settings-read",
      ({ payload: request }) => {
        if (!alive || epoch !== request.uiEpoch) return;
        void (async () => {
          let snapshot: unknown = null;
          let error: string | null = null;
          try {
            if (
              !domain.current
                .getCurrent()
                ?.projects.some(
                  (p) =>
                    p.id === request.projectId &&
                    p.workspaces.some(
                      (w) => w.id === request.input.workspaceId,
                    ),
                )
            )
              throw Error("TARGET_NOT_FOUND");
            snapshot = await settingsReader.current(request.input);
          } catch (e) {
            const code = e instanceof Error ? e.message : "";
            error = [
              "TARGET_NOT_FOUND",
              "UI_NOT_READY",
              "RESOURCE_EXHAUSTED",
              "REVISION_CONFLICT",
            ].includes(code)
              ? code
              : "UI_NOT_READY";
          }
          if (!alive || epoch !== request.uiEpoch) return;
          await api("agent_control_settings_read_reply", {
            reply: {
              requestId: request.requestId,
              uiEpoch: request.uiEpoch,
              snapshot,
              error,
            },
          });
        })().catch(() => {});
      },
    );
    const editors = listen<{
      requestId: string;
      uiEpoch: string;
      projectPath: string;
      projectId: string;
      input: EditorReadInput;
    }>("agent-control-editor-read", ({ payload: request }) => {
      if (!alive || epoch !== request.uiEpoch) return;
      let response: {
        sourcePath: string | null;
        text: unknown;
        error: string | null;
      } = {
        sourcePath: null,
        text: null,
        error: "TARGET_NOT_FOUND",
      };
      try {
        const input = request.input;
        const project = domain.current
          .getCurrent()
          ?.projects.find(
            (p) =>
              p.id === request.projectId &&
              p.workspaces.some((w) => w.id === input.workspaceId),
          );
        const workspace = project?.workspaces.find(
          (w) => w.id === input.workspaceId,
        );
        const panel = workspace?.tabs
          .flatMap((t) =>
            t.type === "terminal"
              ? layoutPanes(t.layout).filter((p) => p.type === "file")
              : t.type === "file"
                ? [t]
                : [],
          )
          .find((p) => p.id === input.panelId);
        if (
          panel?.type === "file" &&
          !panel.untitled &&
          panel.root === project?.path &&
          panel.relative === input.relativePath
        ) {
          const document = loadedEditor(panel);
          if (!document) response.error = "UI_NOT_READY";
          else response = { ...document.readAgentBuffer(input), error: null };
        }
      } catch (e) {
        const code = e instanceof Error ? e.message : "";
        response.error = [
          "TARGET_NOT_FOUND",
          "TARGET_BUSY",
          "STALE_GENERATION",
          "REVISION_CONFLICT",
          "RESOURCE_EXHAUSTED",
        ].includes(code)
          ? code
          : "UI_NOT_READY";
      }
      void api("agent_control_editor_read_reply", {
        reply: {
          requestId: request.requestId,
          uiEpoch: request.uiEpoch,
          ...response,
        },
      }).catch(() => {});
    });
    const commands = listen<UiCommand>(
      "agent-control-command",
      ({ payload: command }) => {
        queue = queue
          .then(async () => {
            if (!alive || epoch !== command.uiEpoch) return;
            const ack = (result: unknown) =>
              api("agent_control_ui_ack", {
                ack: {
                  operationId: command.operationId,
                  nonce: command.nonce,
                  uiEpoch: command.uiEpoch,
                  result,
                },
              });
            const action = command.action;
            const browserRuntime =
              action.type === "navigate_browser" ||
              action.type === "interact_browser" ||
              action.type === "android_input" ||
              action.type === "import_artifact" ||
              action.type === "download_browser";
            if (browserRuntime) await send();
            let before = domain.current.getCurrent();
            if (
              !before ||
              (!browserRuntime &&
                (!sameAgentSession(before, published) ||
                  String(revision) !== command.domainRevision))
            ) {
              await ack({ kind: "failure", code: "REVISION_CONFLICT" });
              return;
            }
            if (
              action.type !== "open_project" &&
              !before.projects.some(
                (p) =>
                  p.id === command.projectId &&
                  p.workspaces.some(
                    (w) =>
                      w.id ===
                      (action.type === "create_workspace"
                        ? action.anchorWorkspaceId
                        : action.workspaceId),
                  ),
              )
            ) {
              await ack({ kind: "failure", code: "TARGET_NOT_FOUND" });
              return;
            }
            await api("agent_control_ui_claim", {
              uiEpoch: command.uiEpoch,
              operationId: command.operationId,
              nonce: command.nonce,
            });
            if (
              !alive ||
              (!browserRuntime &&
                !sameAgentSession(domain.current.getCurrent(), before))
            ) {
              await ack({ kind: "failure", code: "REVISION_CONFLICT" });
              return;
            }
            // Retain viewport changes made while the native claim was pending.
            if (!browserRuntime) before = domain.current.getCurrent()!;
            if (action.type === "send_chat") {
              const target = {
                operationId: command.operationId,
                nonce: command.nonce,
              };
              try {
                const input = action.input;
                const project = before.projects.find(
                  (p) => p.id === command.projectId,
                );
                const workspace = project?.workspaces.find(
                  (w) => w.id === action.workspaceId,
                );
                const panel = workspace?.tabs
                  .flatMap((t) =>
                    t.type === "terminal"
                      ? layoutPanes(t.layout).filter((p) => p.type === "chat")
                      : t.type === "chat"
                        ? [t]
                        : [],
                  )
                  .find((p) => p.id === input.panelId);
                if (
                  panel?.type !== "chat" ||
                  panel.conversationId !== input.conversationId
                )
                  throw Error("TARGET_NOT_FOUND");
                const runtime = (await import("./chat/chat-runtime")).existing(
                  input.conversationId,
                );
                if (!runtime) throw Error("UI_NOT_READY");
                await runtime.ready;
                const matches = () => {
                  const loaded = runtime.snapshot.loaded;
                  return (
                    alive &&
                    sameAgentSession(domain.current.getCurrent(), before) &&
                    Date.now() < Number(action.notAfterMillis) &&
                    !runtime.dirty &&
                    !runtime.snapshot.busy &&
                    loaded?.conversation.origin.projectId ===
                      command.projectId &&
                    String(loaded.conversation.revision) ===
                      input.expectedConversationRevision &&
                    String(loaded.draft.revision) ===
                      input.expectedDraftRevision &&
                    loaded.conversation.config.connectionId ===
                      input.connectionId &&
                    loaded.conversation.config.model === input.model
                  );
                };
                if (!matches()) throw Error("REVISION_CONFLICT");
                const plan = await api<
                  import("./AgentChatApproval").ChatSendPlan
                >("agent_control_chat_send_prepare", target);
                const isActive = async () =>
                  matches() &&
                  runtime.snapshot.text === plan.draftText &&
                  (await api<boolean>("agent_control_chat_send_pending", {
                    ...target,
                    planHash: plan.planHash,
                  }));
                if (
                  !(await domain.current.confirmChat({
                    ...target,
                    plan,
                    isActive,
                  }))
                ) {
                  await api("agent_control_chat_send_decide", {
                    ...target,
                    planHash: plan.planHash,
                    approved: false,
                  }).catch(() => {});
                  return;
                }
                if (!matches() || runtime.snapshot.text !== plan.draftText)
                  throw Error("REVISION_CONFLICT");
                const result = await runtime.sendAgent(
                  {
                    requestId: action.requestId,
                    userId: action.userId,
                    assistantId: action.assistantId,
                    conversationId: input.conversationId,
                    expectedRevision: Number(
                      input.expectedConversationRevision,
                    ),
                    draftRevision: Number(input.expectedDraftRevision),
                    action: "send",
                    targetId: null,
                    text: plan.draftText,
                  },
                  { ...target, planHash: plan.planHash },
                );
                if (!alive) return;
                await ack({ kind: "chat_sent", ...result });
              } catch (error) {
                const code =
                  error instanceof Error ? error.message : String(error);
                await ack({
                  kind: "failure",
                  code: [
                    "CONTROL_REVOKED",
                    "RESOURCE_EXHAUSTED",
                    "TARGET_NOT_FOUND",
                    "SCOPE_DENIED",
                    "TARGET_BUSY",
                    "DEADLINE_EXCEEDED",
                    "REVISION_CONFLICT",
                    "OUTCOME_UNKNOWN",
                    "STORAGE_UNAVAILABLE",
                    "UI_NOT_READY",
                  ].includes(code)
                    ? code
                    : "OUTCOME_UNKNOWN",
                });
              }
              return;
            }
            if (action.type === "draft_chat") {
              try {
                const input = action.input;
                const project = before.projects.find(
                  (p) => p.id === command.projectId,
                );
                const workspace = project?.workspaces.find(
                  (w) => w.id === action.workspaceId,
                );
                const panel = workspace?.tabs
                  .flatMap((t) =>
                    t.type === "terminal"
                      ? layoutPanes(t.layout).filter((p) => p.type === "chat")
                      : t.type === "chat"
                        ? [t]
                        : [],
                  )
                  .find((p) => p.id === input.panelId);
                if (
                  panel?.type !== "chat" ||
                  panel.conversationId !== input.conversationId
                )
                  throw Error("TARGET_NOT_FOUND");
                const runtime = (await import("./chat/chat-runtime")).existing(
                  input.conversationId,
                );
                if (!runtime) throw Error("UI_NOT_READY");
                await runtime.ready;
                if (
                  runtime.snapshot.loaded?.conversation.origin.projectId !==
                  command.projectId
                )
                  throw Error("TARGET_NOT_FOUND");
                let result: ChatDraftUpdated | undefined;
                await runtime.applyAgentDraft(
                  input.expectedDraftRevision,
                  input.expectedConversationRevision,
                  input.text,
                  async () => {
                    if (
                      !alive ||
                      !sameAgentSession(domain.current.getCurrent(), before)
                    )
                      throw Error("REVISION_CONFLICT");
                    if (
                      !Number.isSafeInteger(Number(action.notAfterMillis)) ||
                      Date.now() >= Number(action.notAfterMillis)
                    )
                      throw Error("DEADLINE_EXCEEDED");
                    result = await api<ChatDraftUpdated>(
                      "agent_control_chat_draft",
                      {
                        operationId: command.operationId,
                        nonce: command.nonce,
                      },
                    );
                    if (
                      result.conversationId !== input.conversationId ||
                      result.panelId !== input.panelId ||
                      result.workspaceId !== action.workspaceId
                    )
                      throw Error("OUTCOME_UNKNOWN");
                    return {
                      ...runtime.snapshot.loaded!.draft,
                      text: input.text,
                      revision: Number(result.draftRevision),
                    };
                  },
                );
                if (!alive) return;
                await ack({ kind: "chat_draft_updated", ...result });
              } catch (error) {
                const code =
                  error instanceof Error ? error.message : String(error);
                await ack({
                  kind: "failure",
                  code: [
                    "CONTROL_REVOKED",
                    "RESOURCE_EXHAUSTED",
                    "TARGET_NOT_FOUND",
                    "SCOPE_DENIED",
                    "TARGET_BUSY",
                    "DEADLINE_EXCEEDED",
                    "REVISION_CONFLICT",
                    "OUTCOME_UNKNOWN",
                    "STORAGE_UNAVAILABLE",
                  ].includes(code)
                    ? code
                    : "UI_NOT_READY",
                });
              }
              return;
            }
            if (action.type === "open_chat") {
              try {
                openAgentChat(before, command.projectId, action, "Chat AI");
                const conversation = await api<{
                  conversationId: string;
                  title: string;
                }>("agent_control_chat_open", {
                  operationId: command.operationId,
                  nonce: command.nonce,
                });
                if (!alive || domain.current.getCurrent() !== before)
                  throw Error("REVISION_CONFLICT");
                if (
                  !Number.isSafeInteger(Number(action.notAfterMillis)) ||
                  Date.now() >= Number(action.notAfterMillis)
                )
                  throw Error("DEADLINE_EXCEEDED");
                if (conversation.conversationId !== action.conversationId)
                  throw Error("TARGET_NOT_FOUND");
                const root = before.projects
                  .find((p) => p.id === command.projectId)
                  ?.workspaces.find((w) => w.id === action.workspaceId)
                  ?.tabs.find(
                    (t) =>
                      t.id === action.panelId ||
                      (t.type === "terminal" &&
                        layoutPanes(t.layout).some(
                          (p) => p.id === action.panelId,
                        )),
                  );
                const revealing =
                  root?.type === "terminal"
                    ? layoutPanes(root.layout)
                    : root
                      ? [root]
                      : [];
                if (
                  revealing.some(
                    (p) =>
                      (p.type === "terminal" &&
                        !runningTerminal(p.id)?.sessionId) ||
                      (p.type === "browser" &&
                        (!p.automation ||
                          !hasLiveAgentBrowser(
                            p.id,
                            p.automation.generation,
                          ))) ||
                      p.type === "plugin",
                  )
                )
                  throw Error("UI_NOT_READY");
                await checkAgentPanels(revealing, command.projectId, true);
                if (
                  !alive ||
                  !sameAgentSession(domain.current.getCurrent(), before)
                )
                  throw Error("REVISION_CONFLICT");
                const opened = openAgentChat(
                  before,
                  command.projectId,
                  action,
                  conversation.title,
                );
                domain.current.setCurrent(opened.session);
                const runtime = (await import("./chat/chat-runtime")).getChat(
                  action.conversationId,
                );
                await runtime.ready;
                if (
                  !runtime.snapshot.loaded ||
                  runtime.snapshot.loaded.conversation.origin.projectId !==
                    command.projectId
                )
                  throw Error("UI_NOT_READY");
                if (!alive) return;
                await send();
                await ack({
                  kind: "chat_opened",
                  workspaceId: action.workspaceId,
                  panelId: opened.panel.id,
                  conversationId: action.conversationId,
                  created: action.create,
                });
              } catch (error) {
                const code =
                  error instanceof Error ? error.message : String(error);
                await ack({
                  kind: "failure",
                  code: [
                    "CONTROL_REVOKED",
                    "RESOURCE_EXHAUSTED",
                    "TARGET_NOT_FOUND",
                    "SCOPE_DENIED",
                    "TARGET_BUSY",
                    "UNSUPPORTED_CAPABILITY",
                    "DEADLINE_EXCEEDED",
                    "REVISION_CONFLICT",
                  ].includes(code)
                    ? code
                    : "UI_NOT_READY",
                });
              }
              return;
            }
            if (action.type === "update_settings") {
              try {
                const section =
                  action.input.patch.type === "terminal_field"
                    ? "terminal"
                    : action.input.patch.type.startsWith("keybind")
                      ? "keybinds"
                      : action.input.patch.type.startsWith("theme_")
                        ? "themes"
                        : "editor";
                const snapshot = await settingsReader.current({
                  workspaceId: action.workspaceId,
                  section,
                  offset: 0,
                  limit: 100,
                  expectedRevision: action.input.expectedSettingsRevision,
                });
                if (snapshot.readiness !== "ready")
                  throw Error("UNSUPPORTED_CAPABILITY");
                if (section === "themes" && snapshot.values.safeMode)
                  throw Error("UNSUPPORTED_CAPABILITY");
                if (Date.now() >= Number(action.notAfterMillis))
                  throw Error("DEADLINE_EXCEEDED");
                let keybinds;
                if (section === "keybinds") {
                  const source = await api<KeybindingSource>(
                    "agent_control_settings_source",
                    {
                      operationId: command.operationId,
                      nonce: command.nonce,
                      revision: snapshot.revision,
                    },
                  );
                  const latest = await settingsReader.current({
                    workspaceId: action.workspaceId,
                    section,
                    offset: 0,
                    limit: 100,
                    expectedRevision: snapshot.revision,
                  });
                  if (latest.readiness !== "ready")
                    throw Error("UNSUPPORTED_CAPABILITY");
                  keybinds = settingsReader.current.prepareKeybinds(
                    source,
                    action.input.patch as KeybindingPatch,
                  );
                }
                await api("agent_control_settings_prepare", {
                  operationId: command.operationId,
                  nonce: command.nonce,
                  revision: snapshot.revision,
                  current:
                    keybinds ??
                    (section === "themes"
                      ? {
                          active: snapshot.values.active,
                          appearance: snapshot.values.appearance,
                          fileIcons: snapshot.values.fileIcons,
                          productIcons: snapshot.values.productIcons,
                        }
                      : section === "terminal"
                        ? {
                            appearance: snapshot.values.appearanceOverrides,
                            behavior: snapshot.values.behavior,
                            windowsShell: snapshot.values.windowsShell,
                            agentNotifications:
                              snapshot.values.agentNotifications,
                            alwaysShowTitles: snapshot.values.alwaysShowTitles,
                          }
                        : {
                            tabSize: snapshot.values.tabSize,
                            insertSpaces: snapshot.values.insertSpaces,
                          }),
                });
                // Settings owns the exact approval and native completion. Free
                // the main command queue while the user considers the request.
              } catch (error) {
                const code = error instanceof Error ? error.message : error;
                await ack({
                  kind: "failure",
                  code:
                    typeof code === "string" &&
                    [
                      "CONTROL_REVOKED",
                      "SCOPE_DENIED",
                      "TARGET_NOT_FOUND",
                      "REVISION_CONFLICT",
                      "UNSUPPORTED_CAPABILITY",
                      "STORAGE_UNAVAILABLE",
                      "UI_NOT_READY",
                      "RESOURCE_EXHAUSTED",
                      "DEADLINE_EXCEEDED",
                    ].includes(code)
                      ? code
                      : "OUTCOME_UNKNOWN",
                });
              }
              return;
            }
            if (action.type === "open_settings") {
              try {
                await api("agent_control_settings_open", {
                  operationId: command.operationId,
                  nonce: command.nonce,
                });
              } catch (error) {
                await ack({
                  kind: "failure",
                  code:
                    typeof error === "string" &&
                    [
                      "CONTROL_REVOKED",
                      "SCOPE_DENIED",
                      "TARGET_NOT_FOUND",
                      "REVISION_CONFLICT",
                    ].includes(error)
                      ? error
                      : "OUTCOME_UNKNOWN",
                });
              }
              return;
            }
            if (action.type === "open_project") {
              let committed = false;
              try {
                const ticket = {
                  uiEpoch: command.uiEpoch,
                  operationId: command.operationId,
                  nonce: command.nonce,
                };
                const current = () => {
                  const value = domain.current.getCurrent();
                  if (!alive || !value || !sameAgentSession(value, before))
                    throw Error("REVISION_CONFLICT");
                  if (
                    document.querySelector("dialog[open], [aria-modal='true']")
                  )
                    throw Error("TARGET_BUSY");
                  if (
                    value.projects.some(
                      (p) =>
                        p.id === action.projectId ||
                        p.path === action.projectPath ||
                        p.workspaces.some(
                          (w) => w.id === action.newWorkspaceId,
                        ),
                    )
                  )
                    throw Error("REVISION_CONFLICT");
                  if (Date.now() >= Number(action.notAfterMillis))
                    throw Error("DEADLINE_EXCEEDED");
                  return value;
                };
                current();
                while (
                  !(await api<boolean>(
                    "agent_control_project_open_ready",
                    ticket,
                  ))
                ) {
                  current();
                  await new Promise((resolve) => setTimeout(resolve, 100));
                }
                current();
                await api("agent_control_project_open_commit", ticket);
                committed = true;
                const latest = current();
                const tab = { ...newFileTab(latest), id: action.tabId };
                const workspace = {
                  id: action.newWorkspaceId,
                  name: action.name,
                  activeTabId: tab.id,
                  tabs: [tab],
                };
                domain.current.setCurrent({
                  ...latest,
                  activeProjectId: action.projectId,
                  projects: [
                    ...latest.projects,
                    {
                      id: action.projectId,
                      path: action.projectPath,
                      activeWorkspaceId: workspace.id,
                      workspaces: [workspace],
                    },
                  ],
                });
                await send();
                await ack({
                  kind: "project_opened",
                  anchorWorkspaceId: action.workspaceId,
                  projectId: action.projectId,
                  projectPath: action.projectPath,
                  workspaceId: workspace.id,
                  panelId: tab.id,
                  name: action.name,
                  opened: true,
                });
              } catch (error) {
                const message =
                  error instanceof Error
                    ? error.message
                    : typeof error === "string"
                      ? error
                      : "";
                const code = committed
                  ? "OUTCOME_UNKNOWN"
                  : [
                        "CONTROL_REVOKED",
                        "TARGET_NOT_FOUND",
                        "TARGET_BUSY",
                        "REVISION_CONFLICT",
                        "DEADLINE_EXCEEDED",
                        "SCOPE_DENIED",
                      ].includes(message)
                    ? message
                    : "OUTCOME_UNKNOWN";
                await ack({ kind: "failure", code });
              }
              return;
            }
            if (
              action.type === "close_workspace" ||
              action.type === "close_project"
            ) {
              const target = {
                uiEpoch: command.uiEpoch,
                operationId: command.operationId,
                nonce: command.nonce,
              };
              let committed = false;
              let saveAttempted = false;
              let saved = false;
              let discarded = false;
              let chatSaveAttempted = false;
              try {
                if (document.querySelector("dialog[open], [aria-modal='true']"))
                  throw new Error("TARGET_BUSY");
                domain.current.layoutSize();
                const project = before.projects.find(
                  (p) => p.id === command.projectId,
                )!;
                const targets =
                  action.type === "close_project"
                    ? action.workspaces
                    : [action];
                if (
                  action.type === "close_project" &&
                  (targets.length !== project.workspaces.length ||
                    project.workspaces.some(
                      (w) => !targets.some((t) => t.workspaceId === w.id),
                    ))
                )
                  throw new Error("REVISION_CONFLICT");
                const workspaces = targets.map((target) => {
                  const workspace = project.workspaces.find(
                    (w) => w.id === target.workspaceId,
                  );
                  if (!workspace) throw new Error("TARGET_NOT_FOUND");
                  return workspace;
                });
                if (workspaces.some((w) => w.pluginSidebars?.length))
                  throw new Error("UNSUPPORTED_CAPABILITY");
                const panels = workspaces
                  .flatMap((w) => w.tabs)
                  .flatMap<
                    import("./model").Tab | import("./model").LayoutPane
                  >((t) =>
                    t.type === "terminal" ? layoutPanes(t.layout) : [t],
                  );
                if (
                  panels.some(
                    (p) =>
                      ![
                        "file",
                        "terminal",
                        "browser",
                        "diff",
                        "commit",
                        "chat",
                        "android",
                      ].includes(p.type),
                  )
                )
                  throw new Error("UNSUPPORTED_CAPABILITY");
                const identities = workspaces.flatMap((w) =>
                  panelMoveIdentities(
                    w,
                    (id) => runningTerminal(id)?.sessionId ?? null,
                  ),
                );
                const expected = targets.flatMap((t) => t.panels);
                if (
                  identities.length !== expected.length ||
                  identities.some((p, i) =>
                    Object.keys(p).some(
                      (key) =>
                        p[key as keyof PanelMoveIdentity] !==
                        expected[i][key as keyof PanelMoveIdentity],
                    ),
                  )
                )
                  throw new Error("REVISION_CONFLICT");
                const ids = new Set(panels.map((p) => p.id));
                await checkAgentPanels(panels, command.projectId, false);
                const documents = [
                  ...new Set(
                    panels.flatMap((p) =>
                      p.type === "file"
                        ? [loadedEditor(p)].filter((d) => !!d)
                        : [],
                    ),
                  ),
                ];
                let approvedText = documents.map((d) => d.state.doc);
                const unchanged = () =>
                  alive &&
                  sameAgentSession(domain.current.getCurrent(), before) &&
                  documents.every((d, i) => d.state.doc === approvedText[i]);
                const isActive = async () =>
                  alive &&
                  sameAgentSession(domain.current.getCurrent(), before) &&
                  Date.now() < Number(action.notAfterMillis) &&
                  (await api<boolean>(
                    "agent_control_workspace_close_pending",
                    target,
                  ));
                const confirmed = await domain.current.confirmWorkspaceClose(
                  ids,
                  {
                    description:
                      action.type === "close_project"
                        ? `An agent requested closing ${basename(project.path)} and its ${workspaces.length} workspaces. Saving keeps the project open so the agent can request closing again after the saved changes.`
                        : "An agent requested closing this workspace. Saving keeps it open so the agent can request closing again after the saved changes.",
                    isActive,
                    onSaveStart: async () => {
                      if (!(await isActive()))
                        throw new Error("CONTROL_REVOKED");
                      saveAttempted = true;
                    },
                    onDecision: async (choice) => {
                      if (!(await isActive()))
                        throw new Error("CONTROL_REVOKED");
                      saved = choice === "save";
                      discarded = choice === "discard";
                      approvedText = documents.map((d) => d.state.doc);
                    },
                  },
                );
                const closure = {
                  kind:
                    action.type === "close_project"
                      ? "project_closure"
                      : "workspace_closure",
                  workspaceId: action.workspaceId,
                  projectId: command.projectId,
                  panelIds: identities.map((p) => p.panelId),
                  terminalSessionIds: identities.flatMap((p) =>
                    p.terminalSessionId ? [p.terminalSessionId] : [],
                  ),
                  closed: false,
                  ...(action.type === "close_project"
                    ? { workspaceIds: targets.map((t) => t.workspaceId) }
                    : { projectClosed: false }),
                };
                if (saved) {
                  await ack(closure);
                  return;
                }
                if (!confirmed)
                  throw new Error(
                    saveAttempted ? "OUTCOME_UNKNOWN" : "CONTROL_REVOKED",
                  );
                if (saveAttempted) throw new Error("OUTCOME_UNKNOWN");
                if (
                  !discarded &&
                  closingEditorDocuments(ids).some((d) => d.dirty)
                )
                  throw new Error("TARGET_BUSY");
                if (!unchanged() || !(await isActive()))
                  throw new Error("REVISION_CONFLICT");
                const chatsSaved = panels.some((p) => p.type === "chat")
                  ? await (
                      await import("./chat/chat-service")
                    ).prepareAgentChatClose(ids, () => {
                      chatSaveAttempted = true;
                    })
                  : () => true;
                if (!chatsSaved() || !unchanged() || !(await isActive()))
                  throw new Error("REVISION_CONFLICT");
                await api("agent_control_ui_commit_close", target);
                committed = true;
                if (
                  !alive ||
                  !chatsSaved() ||
                  documents.some((d, i) => d.state.doc !== approvedText[i]) ||
                  closingEditorDocuments(ids).some(
                    (d) => d.dirty && (!discarded || !documents.includes(d)),
                  )
                )
                  throw new Error("REVISION_CONFLICT");
                const removed =
                  action.type === "close_project"
                    ? closeAgentProject(
                        before,
                        domain.current.getCurrent(),
                        command.projectId,
                      )
                    : closeAgentWorkspace(
                        before,
                        domain.current.getCurrent(),
                        command.projectId,
                        action.workspaceId,
                      );
                closeTerminals(
                  panels.filter((p) => p.type === "terminal").map((p) => p.id),
                );
                domain.current.setCurrent(removed);
                await send();
                await ack({
                  ...closure,
                  closed: true,
                  ...(action.type === "close_project"
                    ? {}
                    : {
                        projectClosed: !removed.projects.some(
                          (p) => p.id === command.projectId,
                        ),
                      }),
                });
              } catch (e) {
                const code =
                  e instanceof Error
                    ? e.message
                    : typeof e === "string"
                      ? e
                      : "";
                await ack({
                  kind: "failure",
                  code:
                    (!committed || code === "REVISION_CONFLICT") &&
                    !saveAttempted &&
                    !chatSaveAttempted &&
                    [
                      "TARGET_BUSY",
                      "TARGET_NOT_FOUND",
                      "REVISION_CONFLICT",
                      "CONTROL_REVOKED",
                      "PROTECTED_ORIGIN_TERMINAL",
                      "STALE_GENERATION",
                      "SCOPE_DENIED",
                      "UNSUPPORTED_CAPABILITY",
                    ].includes(code)
                      ? code
                      : "OUTCOME_UNKNOWN",
                }).catch(() => {});
              }
              return;
            }
            if (action.type === "move_panel") {
              let applied = false;
              try {
                if (document.querySelector("dialog[open], [aria-modal='true']"))
                  throw new Error("TARGET_BUSY");
                const size = domain.current.layoutSize();
                const workspace = before.projects
                  .find((p) => p.id === command.projectId)!
                  .workspaces.find((w) => w.id === action.workspaceId)!;
                const identities = panelMoveIdentities(
                  workspace,
                  (id) => runningTerminal(id)?.sessionId ?? null,
                );
                const selected = active(before);
                // Compare fields explicitly: IPC JSON object key order is not contractual.
                if (
                  identities.length !== action.panels.length ||
                  identities.some((p, i) => {
                    const expected = action.panels[i];
                    return Object.keys(p).some(
                      (key) =>
                        p[key as keyof PanelMoveIdentity] !==
                        expected[key as keyof PanelMoveIdentity],
                    );
                  }) ||
                  workspace.tabs.map((t) => t.id).join("\n") !==
                    action.tabOrder.join("\n") ||
                  (selected
                    ? (activePanel(selected.tab)?.id ?? null)
                    : null) !== action.focusedPanelId
                )
                  throw new Error("REVISION_CONFLICT");
                if (action.movement.type === "transfer_tab") {
                  const movement = action.movement;
                  const destination = before.projects
                    .find((p) => p.id === command.projectId)!
                    .workspaces.find(
                      (w) => w.id === movement.targetWorkspaceId,
                    );
                  const expected = action.destination;
                  if (
                    !destination ||
                    !expected ||
                    expected.workspaceId !== destination.id
                  )
                    throw new Error("TARGET_NOT_FOUND");
                  const current = panelMoveIdentities(
                    destination,
                    (id) => runningTerminal(id)?.sessionId ?? null,
                  );
                  if (
                    current.length !== expected.panels.length ||
                    destination.tabs.map((t) => t.id).join("\n") !==
                      expected.tabOrder.join("\n") ||
                    current.some((p, i) =>
                      Object.keys(p).some(
                        (key) =>
                          p[key as keyof PanelMoveIdentity] !==
                          expected.panels[i][key as keyof PanelMoveIdentity],
                      ),
                    )
                  )
                    throw new Error("REVISION_CONFLICT");
                  if (
                    identities.some(
                      (p) =>
                        p.tabId === movement.tabId &&
                        p.kind === "browser" &&
                        (!p.browserGeneration ||
                          !hasLiveAgentBrowser(p.panelId, p.browserGeneration)),
                    )
                  )
                    throw new Error("TARGET_NOT_FOUND");
                }
                if (
                  action.movement.type !== "reorder_tab" &&
                  action.movement.type !== "transfer_tab"
                ) {
                  const movement = action.movement;
                  const relevant =
                    movement.type === "dock_tab"
                      ? new Set([movement.tabId, movement.targetTabId])
                      : new Set(
                          identities
                            .filter((p) => p.panelId === movement.panelId)
                            .map((p) => p.tabId),
                        );
                  if (
                    identities.some(
                      (p) =>
                        relevant.has(p.tabId) &&
                        p.kind === "browser" &&
                        (!p.browserGeneration ||
                          !hasLiveAgentBrowser(p.panelId, p.browserGeneration)),
                    )
                  )
                    throw new Error("TARGET_NOT_FOUND");
                }
                const after = moveAgentPanel(
                  before,
                  command.projectId,
                  action.workspaceId,
                  action.movement,
                  size,
                );
                const movement = action.movement;
                const affected = new Set(
                  movement.type === "dock_tab"
                    ? [movement.tabId, movement.targetTabId]
                    : movement.type === "move_pane"
                      ? identities
                          .filter((p) => p.panelId === movement.panelId)
                          .map((p) => p.tabId)
                      : [movement.tabId],
                );
                await checkAgentPanels(
                  workspace.tabs
                    .filter((t) => affected.has(t.id))
                    .flatMap<
                      import("./model").Tab | import("./model").LayoutPane
                    >((t) =>
                      t.type === "terminal" ? layoutPanes(t.layout) : [t],
                    ),
                  command.projectId,
                  movement.type === "dock_tab" || movement.type === "move_pane",
                );
                if (
                  !alive ||
                  !sameAgentSession(domain.current.getCurrent(), before)
                )
                  throw Error("REVISION_CONFLICT");
                domain.current.setCurrent(after);
                applied = true;
                await send();
                const moved = after.projects
                  .find((p) => p.id === command.projectId)!
                  .workspaces.find((w) => w.id === action.workspaceId)!;
                await ack({
                  kind: "panel_moved",
                  workspaceId: action.workspaceId,
                  movement: action.movement,
                  panels: panelMoveIdentities(
                    moved,
                    (id) => runningTerminal(id)?.sessionId ?? null,
                  ),
                  destination:
                    action.movement.type === "transfer_tab"
                      ? (() => {
                          const movement = action.movement;
                          const destination = after.projects
                            .find((p) => p.id === command.projectId)!
                            .workspaces.find(
                              (w) => w.id === movement.targetWorkspaceId,
                            )!;
                          return {
                            workspaceId: destination.id,
                            tabOrder: destination.tabs.map((t) => t.id),
                            panels: panelMoveIdentities(
                              destination,
                              (id) => runningTerminal(id)?.sessionId ?? null,
                            ),
                          };
                        })()
                      : null,
                });
              } catch (e) {
                const code = !applied && e instanceof Error ? e.message : "";
                await ack({
                  kind: "failure",
                  code: [
                    "TARGET_BUSY",
                    "TARGET_NOT_FOUND",
                    "PANEL_NOT_RENDERABLE",
                    "REVISION_CONFLICT",
                  ].includes(code)
                    ? code
                    : "OUTCOME_UNKNOWN",
                });
              }
              return;
            } else if (action.type === "git_mutate") {
              let committed = false;
              const target = {
                operationId: command.operationId,
                nonce: command.nonce,
              };
              try {
                const plan = await api<
                  import("./AgentGitApproval").GitMutationPlan
                >("agent_control_git_mutation_prepare", target);
                const isActive = async () =>
                  alive &&
                  domain.current.getCurrent() === before &&
                  Date.now() < Number(action.notAfterMillis) &&
                  (await api<boolean>("agent_control_git_mutation_pending", {
                    ...target,
                    planHash: plan.planHash,
                  }));
                if (
                  !(await domain.current.confirmGit({
                    ...target,
                    plan,
                    isActive,
                  }))
                ) {
                  await api("agent_control_git_mutation_decide", {
                    ...target,
                    planHash: plan.planHash,
                    approved: false,
                  }).catch(() => {});
                  return;
                }
                const result = await domain.current.runFileOperation(
                  async () => {
                    if (!alive || domain.current.getCurrent() !== before)
                      throw new Error("REVISION_CONFLICT");
                    if (
                      plan.operation === "discard" ||
                      plan.operation === "pull"
                    )
                      assertCleanEditorPaths(
                        new Set(
                          plan.files.map(
                            (file) =>
                              `${plan.repositoryPath}/${file.relativePath}`,
                          ),
                        ),
                      );
                    const result = await api<Record<string, unknown>>(
                      "agent_control_git_mutation_commit",
                      { ...target, planHash: plan.planHash },
                    );
                    committed = true;
                    return result;
                  },
                );
                domain.current.refreshGit();
                await ack({ kind: "git_mutated", ...result });
              } catch (failure) {
                if (!committed) {
                  const code =
                    typeof failure === "string"
                      ? failure
                      : failure instanceof Error
                        ? failure.message
                        : "OUTCOME_UNKNOWN";
                  if (
                    [
                      "REVISION_CONFLICT",
                      "DEADLINE_EXCEEDED",
                      "TARGET_NOT_FOUND",
                      "TARGET_BUSY",
                      "SCOPE_DENIED",
                      "CONTROL_REVOKED",
                      "RESOURCE_EXHAUSTED",
                      "STORAGE_UNAVAILABLE",
                    ].includes(code)
                  )
                    await ack({ kind: "failure", code });
                }
              }
              return;
            }
            if (action.type === "export_artifact") {
              let committed = false;
              try {
                const result = await domain.current.runFileOperation(
                  async () => {
                    if (!alive || domain.current.getCurrent() !== before)
                      throw new Error("REVISION_CONFLICT");
                    if (
                      !Number.isSafeInteger(Number(action.notAfterMillis)) ||
                      Date.now() >= Number(action.notAfterMillis)
                    )
                      throw new Error("DEADLINE_EXCEEDED");
                    const result = await api<Record<string, unknown>>(
                      "agent_control_artifact_export",
                      {
                        operationId: command.operationId,
                        nonce: command.nonce,
                      },
                    );
                    committed = true;
                    if (result.workspaceId !== action.workspaceId)
                      throw new Error("OUTCOME_UNKNOWN");
                    return result;
                  },
                );
                await send();
                await ack({ kind: "artifact_exported", ...result });
              } catch (e) {
                if (committed) return;
                const code =
                  typeof e === "string"
                    ? e
                    : e instanceof Error
                      ? e.message
                      : "";
                if (
                  [
                    "REVISION_CONFLICT",
                    "DEADLINE_EXCEEDED",
                    "TARGET_NOT_FOUND",
                    "TARGET_BUSY",
                    "SCOPE_DENIED",
                    "CONTROL_REVOKED",
                    "RESOURCE_EXHAUSTED",
                    "STORAGE_UNAVAILABLE",
                  ].includes(code)
                )
                  await ack({ kind: "failure", code });
              }
              return;
            }
            if (action.type === "files_mutate") {
              let committed = false;
              try {
                const result = await domain.current.runFileOperation(
                  async () => {
                    if (!alive || domain.current.getCurrent() !== before)
                      throw new Error("REVISION_CONFLICT");
                    if (
                      !Number.isSafeInteger(Number(action.notAfterMillis)) ||
                      Date.now() >= Number(action.notAfterMillis)
                    )
                      throw new Error("DEADLINE_EXCEEDED");
                    const result = await api("agent_control_files_mutate", {
                      operationId: command.operationId,
                      nonce: command.nonce,
                    });
                    committed = true;
                    const changed = result as FileChange & {
                      workspaceId: string;
                    };
                    if (changed.workspaceId !== action.workspaceId)
                      throw new Error("OUTCOME_UNKNOWN");
                    if (changed.oldPath) {
                      const project = domain.current
                        .getCurrent()
                        ?.projects.find((p) => p.id === command.projectId);
                      if (!project) throw new Error("OUTCOME_UNKNOWN");
                      domain.current.applyFileChange(
                        {
                          oldPath: `${project.path}/${changed.oldPath}`,
                          newPath: changed.newPath
                            ? `${project.path}/${changed.newPath}`
                            : null,
                        },
                        {
                          oldPath: `${action.projectPath}/${changed.oldPath}`,
                          newPath: changed.newPath
                            ? `${action.projectPath}/${changed.newPath}`
                            : null,
                        },
                      );
                    }
                    return result as Record<string, unknown>;
                  },
                  action.input.operation.type === "trash"
                    ? {
                        operationId: command.operationId,
                        nonce: command.nonce,
                        projectId: command.projectId,
                        projectPath: action.projectPath,
                        relativePath: action.input.operation.relativePath,
                        notAfterMillis: action.notAfterMillis,
                      }
                    : undefined,
                );
                await send();
                await ack({ kind: "files_mutated", ...result });
              } catch (e) {
                if (committed) return;
                const code =
                  typeof e === "string"
                    ? e
                    : e instanceof Error
                      ? e.message
                      : "";
                if (
                  [
                    "REVISION_CONFLICT",
                    "DEADLINE_EXCEEDED",
                    "TARGET_NOT_FOUND",
                    "TARGET_BUSY",
                    "SCOPE_DENIED",
                    "CONTROL_REVOKED",
                    "RESOURCE_EXHAUSTED",
                    "STORAGE_UNAVAILABLE",
                  ].includes(code)
                )
                  await ack({ kind: "failure", code });
              }
              return;
            }
            if (action.type === "git_open") {
              let staged: ReturnType<typeof stageAgentGit> | undefined;
              let prepared: PreparedGitView | undefined;
              let published = false;
              try {
                prepared = await api<PreparedGitView>(
                  "agent_control_git_open",
                  { operationId: command.operationId, nonce: command.nonce },
                );
                if (!alive || domain.current.getCurrent() !== before)
                  throw new Error("REVISION_CONFLICT");
                if (
                  !Number.isSafeInteger(Number(action.notAfterMillis)) ||
                  Date.now() >= Number(action.notAfterMillis)
                )
                  throw new Error("DEADLINE_EXCEEDED");
                let next =
                  action.view.type === "diff"
                    ? openDiffTab(
                        before,
                        action.workspaceId,
                        prepared.root,
                        action.view.relativePath,
                        action.view.staged,
                      )
                    : openCommitTab(
                        before,
                        action.workspaceId,
                        prepared.root,
                        action.view.commit,
                        action.view.commit.slice(0, 7),
                      );
                const project = next.projects.find(
                  (p) => p.id === command.projectId,
                )!;
                const workspace = project.workspaces.find(
                  (w) => w.id === action.workspaceId,
                )!;
                const panel = workspace.tabs.find(
                  (t) => t.id === workspace.activeTabId,
                )!;
                if (panel.type !== "diff" && panel.type !== "commit")
                  throw new Error("TARGET_NOT_FOUND");
                next = updateTab(next, panel.id, (tab) =>
                  tab.type === "diff" || tab.type === "commit"
                    ? { ...tab, agentGit: true }
                    : tab,
                );
                next = {
                  ...next,
                  activeProjectId: project.id,
                  projects: next.projects.map((p) =>
                    p.id === project.id
                      ? { ...p, activeWorkspaceId: workspace.id }
                      : p,
                  ),
                };
                staged = stageAgentGit(panel.id, prepared);
                published = true;
                domain.current.setCurrent(next);
                staged.commit();
                await send();
                await ack({
                  kind: "git_opened",
                  workspaceId: action.workspaceId,
                  panelId: panel.id,
                  repositoryRelative: action.repositoryRelative,
                  view: action.view,
                  observationRevision: prepared.observationRevision,
                });
              } catch (e) {
                if (published) return;
                staged?.cancel();
                if (!staged && prepared)
                  void api("agent_control_git_release", {
                    permitId: prepared.permitId,
                  }).catch(() => {});
                const code =
                  typeof e === "string"
                    ? e
                    : e instanceof Error
                      ? e.message
                      : "";
                if (
                  [
                    "REVISION_CONFLICT",
                    "DEADLINE_EXCEEDED",
                    "TARGET_NOT_FOUND",
                    "TARGET_BUSY",
                    "SCOPE_DENIED",
                    "CONTROL_REVOKED",
                    "RESOURCE_EXHAUSTED",
                    "STORAGE_UNAVAILABLE",
                    "UNSUPPORTED_CAPABILITY",
                  ].includes(code)
                )
                  await ack({ kind: "failure", code });
              }
              return;
            }
            if (action.type === "editor_open") {
              let published = false;
              let clear: (() => void) | undefined;
              let preview: ReturnType<typeof stageAgentPreview> | undefined;
              let assetPermit: string | null = null;
              try {
                const file = await api<PreparedEditorFile>(
                  "agent_control_editor_open_file",
                  { operationId: command.operationId, nonce: command.nonce },
                );
                assetPermit = file.assetPermit;
                const project = before.projects.find(
                  (p) => p.id === command.projectId,
                )!;
                const target = {
                  type: "file" as const,
                  id: command.operationId,
                  title: basename(action.relativePath),
                  root: project.path,
                  relative: action.relativePath,
                };
                if (file.body.kind === "text")
                  clear = await stageEditorRead(target, {
                    ...file,
                    ...file.body,
                  });
                if (!alive || domain.current.getCurrent() !== before)
                  throw new Error("REVISION_CONFLICT");
                if (
                  !Number.isSafeInteger(Number(action.notAfterMillis)) ||
                  Date.now() >= Number(action.notAfterMillis)
                )
                  throw new Error("DEADLINE_EXCEEDED");
                let next = openFileTab(
                  before,
                  action.workspaceId,
                  project.path,
                  action.relativePath,
                );
                next = {
                  ...next,
                  activeProjectId: project.id,
                  projects: next.projects.map((p) =>
                    p.id === project.id
                      ? { ...p, activeWorkspaceId: action.workspaceId }
                      : p,
                  ),
                };
                const workspace = next.projects
                  .find((p) => p.id === project.id)!
                  .workspaces.find((w) => w.id === action.workspaceId)!;
                const panel = workspace.tabs
                  .flatMap((t) =>
                    t.type === "terminal"
                      ? layoutPanes(t.layout).filter((p) => p.type === "file")
                      : t.type === "file"
                        ? [t]
                        : [],
                  )
                  .find(
                    (p) =>
                      p.root === project.path &&
                      p.relative === action.relativePath,
                  );
                if (!panel) throw new Error("TARGET_NOT_FOUND");
                if (file.body.kind === "image" && loadedEditor(panel))
                  throw new Error("TARGET_BUSY");
                if (
                  file.body.kind === "image" ||
                  isMarkdownFile(action.relativePath)
                ) {
                  preview = stageAgentPreview(panel.id, file);
                  assetPermit = null;
                  next = updateFile(next, panel.id, (tab) => ({
                    ...tab,
                    agentPreview: true,
                  }));
                }
                next = updateFile(next, panel.id, (tab) => ({
                  ...tab,
                  previewView: action.presentation,
                }));
                published = true;
                domain.current.setCurrent(next);
                retainEditorTabs(next);
                preview?.commit();
                if (file.body.kind === "image") {
                  // Decode the exact prepared derivative before acknowledging the visible panel.
                  const image = new Image();
                  image.src = `data:${file.body.mimeType};base64,${file.body.dataBase64}`;
                  await image.decode();
                  if (
                    image.naturalWidth !== file.body.width ||
                    image.naturalHeight !== file.body.height
                  )
                    throw new Error("UI_NOT_READY");
                  await send();
                  await ack({
                    kind: "editor_previewed",
                    workspaceId: action.workspaceId,
                    panelId: panel.id,
                    relativePath: action.relativePath,
                    diskRevision: file.revision,
                    width: file.body.width,
                    height: file.body.height,
                    originalWidth: file.body.originalWidth,
                    originalHeight: file.body.originalHeight,
                  });
                  return;
                }
                const document = await openEditorDocument(panel);
                const result = document.readAgentBuffer({
                  workspaceId: action.workspaceId,
                  panelId: panel.id,
                  relativePath: action.relativePath,
                  maxChars: 2,
                });
                if (result.sourcePath !== file.path)
                  throw new Error("SCOPE_DENIED");
                await send();
                await ack({
                  kind: "editor_opened",
                  workspaceId: action.workspaceId,
                  panelId: panel.id,
                  relativePath: action.relativePath,
                  documentId: result.text.documentId,
                  bufferRevision: result.text.bufferRevision,
                  diskRevision: result.text.diskRevision,
                  dirty: result.text.dirty,
                  presentation: action.presentation,
                });
              } catch (e) {
                if (!published) {
                  const code =
                    e instanceof Error
                      ? e.message
                      : typeof e === "string"
                        ? e
                        : "UI_NOT_READY";
                  await ack({
                    kind: "failure",
                    code: [
                      "REVISION_CONFLICT",
                      "DEADLINE_EXCEEDED",
                      "TARGET_NOT_FOUND",
                      "SCOPE_DENIED",
                      "TARGET_BUSY",
                      "RESOURCE_EXHAUSTED",
                      "UNSUPPORTED_CAPABILITY",
                    ].includes(code)
                      ? code
                      : "UI_NOT_READY",
                  }).catch(() => {});
                }
              } finally {
                clear?.();
                if (!published) preview?.cancel();
                if (assetPermit)
                  void api("agent_control_preview_release", {
                    permitId: assetPermit,
                  }).catch(() => {});
              }
              return;
            }
            if (
              action.type === "editor_edits" ||
              action.type === "editor_save"
            ) {
              let applied = false;
              try {
                if (
                  !Number.isSafeInteger(Number(action.notAfterMillis)) ||
                  Date.now() >= Number(action.notAfterMillis)
                )
                  throw new Error("DEADLINE_EXCEEDED");
                const project = before.projects.find(
                  (p) => p.id === command.projectId,
                )!;
                const workspace = project.workspaces.find(
                  (w) => w.id === action.workspaceId,
                )!;
                const panel = workspace.tabs
                  .flatMap((t) =>
                    t.type === "terminal"
                      ? layoutPanes(t.layout).filter((p) => p.type === "file")
                      : t.type === "file"
                        ? [t]
                        : [],
                  )
                  .find((p) => p.id === action.input.panelId);
                if (
                  !panel ||
                  panel.untitled ||
                  panel.root !== project.path ||
                  panel.relative !== action.input.relativePath
                )
                  throw new Error("TARGET_NOT_FOUND");
                const document = loadedEditor(panel);
                if (!document) throw new Error("UI_NOT_READY");
                const sourcePath = `${action.projectPath}/${action.input.relativePath}`;
                const result =
                  action.type === "editor_save"
                    ? await document.saveAgent(
                        action.input,
                        sourcePath,
                        command.operationId,
                        command.nonce,
                      )
                    : document.applyAgentEdits(action.input, sourcePath);
                applied = true;
                await ack({
                  kind:
                    action.type === "editor_save"
                      ? "editor_saved"
                      : "editor_edited",
                  ...result,
                });
              } catch (e) {
                if (applied) return;
                const code = e instanceof Error ? e.message : "";
                if (
                  [
                    "DEADLINE_EXCEEDED",
                    "TARGET_NOT_FOUND",
                    "UI_NOT_READY",
                    "TARGET_BUSY",
                    "SCOPE_DENIED",
                    "STALE_GENERATION",
                    "REVISION_CONFLICT",
                    "RESOURCE_EXHAUSTED",
                    "STORAGE_UNAVAILABLE",
                    "UNSUPPORTED_CAPABILITY",
                    "CONTROL_REVOKED",
                  ].includes(code)
                )
                  await ack({ kind: "failure", code });
                // An unexpected error after dispatch is uncertain; leave the receipt unreplayed.
              }
              return;
            }
            if (action.type === "close_panel") {
              const workspace = before.projects
                .find((p) => p.id === command.projectId)!
                .workspaces.find((w) => w.id === action.workspaceId)!;
              const tab = workspace.tabs.find((t) => t.id === action.tabId);
              const panel =
                tab?.type === "terminal"
                  ? layoutPanes(tab.layout).find((p) => p.id === action.panelId)
                  : tab;
              if (
                !panel ||
                (panel.type !== "terminal" &&
                  panel.type !== "chat" &&
                  panel.type !== "android" &&
                  panel.type !== "file" &&
                  panel.type !== "browser" &&
                  !(
                    (panel.type === "diff" || panel.type === "commit") &&
                    panel.agentGit
                  ))
              ) {
                await ack({ kind: "failure", code: "TARGET_NOT_FOUND" });
                return;
              }
              if (
                panel.type === "browser" &&
                (panel.automation?.generation !== action.browserGeneration ||
                  !hasLiveAgentBrowser(panel.id, action.browserGeneration!))
              ) {
                await ack({ kind: "failure", code: "STALE_GENERATION" });
                return;
              }
              if (panel.type === "file" && loadedEditor(panel)?.dirty) {
                await ack({ kind: "failure", code: "TARGET_BUSY" });
                return;
              }
              if (!(await domain.current.confirmClose(panel.id, false))) {
                await ack({ kind: "failure", code: "CONTROL_REVOKED" });
                return;
              }
              let chatsSaved = () => true;
              let chatSaveAttempted = false;
              if (panel.type === "chat") {
                try {
                  await checkAgentPanels([panel], command.projectId, false);
                  chatsSaved = await (
                    await import("./chat/chat-service")
                  ).prepareAgentChatClose(new Set([panel.id]), () => {
                    chatSaveAttempted = true;
                  });
                } catch {
                  await ack({
                    kind: "failure",
                    code: chatSaveAttempted
                      ? "OUTCOME_UNKNOWN"
                      : "STORAGE_UNAVAILABLE",
                  });
                  return;
                }
              }
              if (
                !alive ||
                domain.current.getCurrent() !== before ||
                !chatsSaved() ||
                (panel.type === "file" && loadedEditor(panel)?.dirty)
              ) {
                await ack({
                  kind: "failure",
                  code: chatSaveAttempted
                    ? "OUTCOME_UNKNOWN"
                    : "REVISION_CONFLICT",
                });
                return;
              }
              try {
                await api("agent_control_ui_commit_close", {
                  uiEpoch: command.uiEpoch,
                  operationId: command.operationId,
                  nonce: command.nonce,
                });
              } catch (error) {
                await ack({
                  kind: "failure",
                  code:
                    !chatSaveAttempted &&
                    typeof error === "string" &&
                    [
                      "TARGET_BUSY",
                      "CONTROL_REVOKED",
                      "PROTECTED_ORIGIN_TERMINAL",
                      "STALE_GENERATION",
                      "SCOPE_DENIED",
                      "OUTCOME_UNKNOWN",
                      "STALE_SNAPSHOT",
                      "PANEL_NOT_RENDERABLE",
                      "UNSUPPORTED_CAPABILITY",
                      "REVISION_CONFLICT",
                    ].includes(error)
                      ? error
                      : "OUTCOME_UNKNOWN",
                });
                return;
              }
              if (
                !alive ||
                domain.current.getCurrent() !== before ||
                !chatsSaved() ||
                (panel.type === "file" && loadedEditor(panel)?.dirty)
              ) {
                await ack({ kind: "failure", code: "OUTCOME_UNKNOWN" });
                return;
              }
              const layout =
                tab?.type === "terminal"
                  ? removePane(tab.layout, panel.id)
                  : null;
              let tabs = workspace.tabs.flatMap((t) =>
                t.id !== action.tabId
                  ? [t]
                  : layout && t.type === "terminal"
                    ? [
                        {
                          ...t,
                          layout,
                          activePaneId:
                            t.activePaneId === panel.id
                              ? layoutPanes(layout)[0].id
                              : t.activePaneId,
                        },
                      ]
                    : [],
              );
              let activeTabId = workspace.activeTabId;
              if (!tabs.length || (activeTabId === action.tabId && !layout)) {
                const scratch = {
                  ...newFileTab(before),
                  id: action.replacementTabId,
                };
                tabs = [...tabs, scratch];
                activeTabId = scratch.id;
              }
              if (panel.type === "terminal") closeTerminals([panel.id]);
              domain.current.setCurrent({
                ...before,
                projects: before.projects.map((p) =>
                  p.id === command.projectId
                    ? {
                        ...p,
                        workspaces: p.workspaces.map((w) =>
                          w.id === workspace.id
                            ? { ...w, tabs, activeTabId }
                            : w,
                        ),
                      }
                    : p,
                ),
              });
              await send();
              await ack({
                kind: "panel",
                workspaceId: workspace.id,
                panelId: panel.id,
                focused: false,
                closed: true,
              });
            } else if (
              action.type === "focus_panel" ||
              action.type === "select_workspace"
            ) {
              const workspace = before.projects
                .find((p) => p.id === command.projectId)!
                .workspaces.find((w) => w.id === action.workspaceId)!;
              const tab = workspace.tabs.find((t) => t.id === action.tabId);
              const panels =
                tab?.type === "terminal"
                  ? layoutPanes(tab.layout)
                  : tab
                    ? [tab]
                    : [];
              const target = panels.find((p) => p.id === action.panelId);
              if (
                action.type === "select_workspace" &&
                (workspace.activeTabId !== action.tabId ||
                  !tab ||
                  activePanel(tab)?.id !== action.panelId)
              ) {
                await ack({ kind: "failure", code: "REVISION_CONFLICT" });
                return;
              }
              if (
                !target ||
                (target.type === "browser" &&
                  target.automation?.generation !== action.browserGeneration)
              ) {
                await ack({ kind: "failure", code: "STALE_GENERATION" });
                return;
              }
              if (
                panels.some(
                  (p) =>
                    p.type === "browser" &&
                    (!p.automation ||
                      !hasLiveAgentBrowser(p.id, p.automation.generation)),
                )
              ) {
                await ack({ kind: "failure", code: "TARGET_NOT_FOUND" });
                return;
              }
              try {
                await checkAgentPanels(panels, command.projectId, true);
              } catch {
                await ack({ kind: "failure", code: "TARGET_NOT_FOUND" });
                return;
              }
              if (
                !alive ||
                !sameAgentSession(domain.current.getCurrent(), before)
              ) {
                await ack({ kind: "failure", code: "REVISION_CONFLICT" });
                return;
              }
              domain.current.setCurrent({
                ...before,
                activeProjectId: command.projectId,
                projects: before.projects.map((p) =>
                  p.id === command.projectId
                    ? {
                        ...p,
                        activeWorkspaceId: action.workspaceId,
                        workspaces: p.workspaces.map((w) =>
                          w.id === action.workspaceId
                            ? {
                                ...w,
                                activeTabId: action.tabId,
                                tabs: w.tabs.map((t) =>
                                  t.id === action.tabId && t.type === "terminal"
                                    ? { ...t, activePaneId: action.panelId }
                                    : t,
                                ),
                              }
                            : w,
                        ),
                      }
                    : p,
                ),
              });
              await send();
              await ack({
                kind: "panel",
                workspaceId: action.workspaceId,
                panelId: action.panelId,
                focused: true,
                closed: false,
              });
            } else if (action.type === "rename_workspace") {
              domain.current.setCurrent({
                ...before,
                projects: before.projects.map((p) =>
                  p.id === command.projectId
                    ? {
                        ...p,
                        workspaces: p.workspaces.map((w) =>
                          w.id === action.workspaceId
                            ? { ...w, name: action.name }
                            : w,
                        ),
                      }
                    : p,
                ),
              });
              await send();
              await ack({
                kind: "workspace",
                workspaceId: action.workspaceId,
                name: action.name,
              });
            } else if (action.type === "create_workspace") {
              const tab = { ...newFileTab(before), id: action.tabId };
              const workspace = {
                id: action.workspaceId,
                name: action.name,
                activeTabId: tab.id,
                tabs: [tab],
              };
              domain.current.setCurrent({
                ...before,
                activeProjectId: command.projectId,
                projects: before.projects.map((p) =>
                  p.id === command.projectId
                    ? {
                        ...p,
                        activeWorkspaceId: workspace.id,
                        workspaces: [...p.workspaces, workspace],
                      }
                    : p,
                ),
              });
              await send();
              await ack({
                kind: "workspace",
                workspaceId: workspace.id,
                name: workspace.name,
              });
            } else if (
              action.type === "navigate_browser" ||
              action.type === "interact_browser"
            ) {
              const currentWorkspace = domain.current
                .getCurrent()
                ?.projects.find((p) => p.id === command.projectId)
                ?.workspaces.find((w) => w.id === action.workspaceId);
              const target = currentWorkspace?.tabs
                .flatMap((t) =>
                  t.type === "terminal"
                    ? layoutPanes(t.layout).filter((p) => p.type === "browser")
                    : t.type === "browser"
                      ? [t]
                      : [],
                )
                .find((p) => p.id === action.panelId);
              if (
                !target ||
                target.type !== "browser" ||
                target.automation?.generation !== action.browserGeneration
              ) {
                await ack({ kind: "failure", code: "STALE_GENERATION" });
                return;
              }
              try {
                const result = await api<Record<string, unknown>>(
                  action.type === "navigate_browser"
                    ? "agent_browser_navigate"
                    : "agent_browser_interact",
                  { operationId: command.operationId, nonce: command.nonce },
                );
                await send();
                if (action.type === "interact_browser") {
                  await ack({ kind: "browser_interaction", ...result });
                }
              } catch (error) {
                await send();
                // Navigation receipts are completed natively, including failures.
                if (action.type === "navigate_browser") return;
                await ack({
                  kind: "failure",
                  code:
                    typeof error === "string" &&
                    [
                      "CONTROL_REVOKED",
                      "SCOPE_DENIED",
                      "TARGET_BUSY",
                      "DEADLINE_EXCEEDED",
                      "TARGET_NOT_FOUND",
                      "OUTCOME_UNKNOWN",
                      "STALE_SNAPSHOT",
                      "PANEL_NOT_RENDERABLE",
                      "UNSUPPORTED_CAPABILITY",
                      "REVISION_CONFLICT",
                    ].includes(error)
                      ? error
                      : "OUTCOME_UNKNOWN",
                });
              }
            } else if (
              action.type === "android_input" ||
              action.type === "android_input_control"
            ) {
              if (
                action.action !== "release" &&
                !(await import("./android/runtime")).agentInputReady(
                  action.panelId,
                  action.generation,
                )
              ) {
                await ack({ kind: "failure", code: "PANEL_NOT_RENDERABLE" });
                return;
              }
              void api("agent_android_input", {
                operationId: command.operationId,
                nonce: command.nonce,
              }).catch(() => {});
            } else if (action.type === "upload_browser") {
              try {
                await api("agent_browser_upload_prepare", {
                  operationId: command.operationId,
                  nonce: command.nonce,
                });
              } catch (error) {
                const code = error instanceof Error ? error.message : error;
                await ack({
                  kind: "failure",
                  code:
                    typeof code === "string" &&
                    [
                      "CONTROL_REVOKED",
                      "SCOPE_DENIED",
                      "TARGET_NOT_FOUND",
                      "STALE_GENERATION",
                      "STALE_SNAPSHOT",
                      "PANEL_NOT_RENDERABLE",
                      "REVISION_CONFLICT",
                      "UNSUPPORTED_CAPABILITY",
                      "STORAGE_UNAVAILABLE",
                      "UI_NOT_READY",
                      "RESOURCE_EXHAUSTED",
                      "DEADLINE_EXCEEDED",
                    ].includes(code)
                      ? code
                      : "OUTCOME_UNKNOWN",
                });
              }
            } else if (
              action.type === "import_artifact" ||
              action.type === "download_browser"
            ) {
              void api(
                action.type === "download_browser"
                  ? "agent_browser_download"
                  : "agent_artifact_import",
                {
                  operationId: command.operationId,
                  nonce: command.nonce,
                },
              ).catch(() => {});
            } else if (action.type === "android_launch") {
              void api("agent_android_launch", {
                operationId: command.operationId,
                nonce: command.nonce,
              }).catch(() => {});
            } else if (action.type === "android_runtime") {
              // The retained native task completes its durable receipt, even when
              // the renderer is suspended while the managed guest boots.
              void api("agent_android_runtime", {
                operationId: command.operationId,
                nonce: command.nonce,
              }).catch(() => {});
            } else if (action.type === "create_android") {
              const tab: AndroidTab = {
                type: "android",
                id: action.panelId,
                title: action.title,
                deviceId: action.deviceId,
                startMode: "manual",
              };
              domain.current.setCurrent({
                ...before,
                activeProjectId: command.projectId,
                projects: before.projects.map((p) =>
                  p.id === command.projectId
                    ? {
                        ...p,
                        activeWorkspaceId: action.workspaceId,
                        workspaces: p.workspaces.map((w) =>
                          w.id === action.workspaceId
                            ? {
                                ...w,
                                activeTabId: tab.id,
                                tabs: [...w.tabs, tab],
                              }
                            : w,
                        ),
                      }
                    : p,
                ),
              });
              await send();
              await ack({
                kind: "android_panel",
                workspaceId: action.workspaceId,
                panelId: action.panelId,
                deviceId: action.deviceId,
              });
            } else if (action.type === "create_browser") {
              stageAgentBrowser(
                action.panelId,
                {
                  operationId: command.operationId,
                  nonce: command.nonce,
                },
                action.visible,
              );
              const tab: BrowserTab = {
                type: "browser",
                id: action.panelId,
                title: "Agent browser",
                url: action.url,
                automation: {
                  generation: action.browserGeneration,
                  profileId: action.profileId,
                },
              };
              domain.current.setCurrent({
                ...before,
                activeProjectId: action.visible
                  ? command.projectId
                  : before.activeProjectId,
                projects: before.projects.map((p) =>
                  p.id === command.projectId
                    ? {
                        ...p,
                        activeWorkspaceId: action.visible
                          ? action.workspaceId
                          : p.activeWorkspaceId,
                        workspaces: p.workspaces.map((w) =>
                          w.id === action.workspaceId
                            ? {
                                ...w,
                                activeTabId: action.visible
                                  ? tab.id
                                  : w.activeTabId,
                                tabs: [...w.tabs, tab],
                              }
                            : w,
                        ),
                      }
                    : p,
                ),
              });
              try {
                const page = await waitForAgentBrowser(
                  action.panelId,
                  action.browserGeneration,
                );
                await send();
                await ack({
                  kind: "browser",
                  workspaceId: action.workspaceId,
                  panelId: action.panelId,
                  browserGeneration: action.browserGeneration,
                  profileId: action.profileId,
                  navigationId: page.navigationId,
                  leaseId: null,
                  ready: true,
                  engine: "WKWebView",
                  networkIsolation: "none",
                });
              } catch {
                await send();
                await ack({ kind: "failure", code: "PANEL_NOT_RENDERABLE" });
              } finally {
                clearAgentBrowser(action.panelId);
              }
            } else if (action.type === "create_terminal") {
              stageAgentTerminal(action.panelId, {
                sessionId: action.terminalSessionId,
                operationId: command.operationId,
                nonce: command.nonce,
              });
              const tab: TerminalTab = {
                type: "terminal",
                id: action.tabId,
                title: action.title,
                customTitle: action.title,
                profileId: action.profileId,
                activePaneId: action.panelId,
                layout: {
                  type: "terminal",
                  id: action.panelId,
                  cwd: action.cwd,
                },
              };
              domain.current.setCurrent({
                ...before,
                activeProjectId: command.projectId,
                projects: before.projects.map((p) =>
                  p.id === command.projectId
                    ? {
                        ...p,
                        activeWorkspaceId: action.workspaceId,
                        workspaces: p.workspaces.map((w) =>
                          w.id === action.workspaceId
                            ? {
                                ...w,
                                activeTabId: tab.id,
                                tabs: [...w.tabs, tab],
                              }
                            : w,
                        ),
                      }
                    : p,
                ),
              });
              try {
                await waitForAgentTerminal(
                  action.panelId,
                  action.terminalSessionId,
                );
                await send();
                await ack({
                  kind: "terminal",
                  workspaceId: action.workspaceId,
                  panelId: action.panelId,
                  terminalSessionId: action.terminalSessionId,
                  leaseId: null,
                  ready: true,
                });
              } catch {
                await send();
                await ack({ kind: "failure", code: "PANEL_NOT_RENDERABLE" });
              } finally {
                clearAgentTerminal(action.panelId);
              }
            }
          })
          .catch(() => {
            /* A lost ACK is reconciled as outcome_unknown, never replayed. */
          });
      },
    );
    void Promise.all([stop, commands, screens, editors, settings])
      .then(() => enqueue(true))
      .catch(() => {});
    return () => {
      alive = false;
      publish.current = () => {};
      void stop.then((unlisten) => unlisten()).catch(() => {});
      void commands.then((unlisten) => unlisten()).catch(() => {});
      void screens.then((unlisten) => unlisten()).catch(() => {});
      void editors.then((unlisten) => unlisten()).catch(() => {});
      void settings.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);
  useEffect(() => publish.current(), [session]);
}
