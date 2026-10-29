import { Channel } from "@tauri-apps/api/core";
import { api } from "./api";
import { layoutPanes, removePane } from "./model";
import type { Layout, LayoutPane, Pane, Session, TerminalTab } from "./model";
import type {
  RpcRequestContract,
  RpcResponseContract,
  LocalStreamEventContract,
  TerminalContextContract,
  WorkspaceContextContract,
} from "../../lomi-remote/packages/remote-contracts/generated/index.js";
import { validateShape } from "@lomi/remote-contracts";
import type { LocalResult as LocalResultContract } from "../../lomi-remote/packages/remote-contracts/generated/local-response.js";
import type {
  LocalAuthProvider,
  LocalStreamAck as LocalStreamAckContract,
} from "../../lomi-remote/packages/remote-contracts/generated/local-request.js";

export type SharedHostWorkspaceSummary = Extract<
  LocalResultContract,
  { type: "workspace" }
>["value"];
export type SharedHostWorkspaceImportReceipt = Extract<
  LocalResultContract,
  { type: "workspaceImported" }
>["value"];
export type SharedHostWorkspaceBinding =
  | {
      state: "missing";
      workspaceId: string;
      previousRootPath?: string;
    }
  | {
      state: "active";
      workspaceId: string;
      canonicalWorkspaceId: string;
      rootPath: string;
      previousRootPath?: string;
    }
  | {
      state: "pending" | "retired";
      workspaceId: string;
      canonicalWorkspaceId: string;
      previousRootPath?: string;
    };
export type SharedHostDesktopUncertaintyWarning = {
  operationId: string;
  requestHash: string;
  message: string;
  createdAtMs: number;
};
export type SharedHostStatus = {
  state:
    "ready" | "administratorInstallRequired" | "unavailable" | "updateRequired";
  hostId: string | null;
  runtimeId: string | null;
  sharing: "disabled" | "enabled" | null;
  message: string | null;
};
export type SharedHostWorkspaceBindingsResponse = {
  bindings: SharedHostWorkspaceBinding[] | null;
  bindingError: string | null;
  uncertaintyWarnings: SharedHostDesktopUncertaintyWarning[];
};
export type SharedHostLoginStatus = Extract<
  LocalResultContract,
  { type: "loginStatus" }
>["value"];
export type SharedHostLoginPresentation = Extract<
  LocalResultContract,
  { type: "loginPresentation" }
>["value"];
export type SharedHostAuthSetup = {
  bootstrapConfigured: boolean;
  providers: LocalAuthProvider[];
};
export type SharedHostAuthProvider = LocalAuthProvider;
export type SharedHostTerminalAlias = {
  desktopPaneId: string;
  desktopTabId: string;
  hostPaneId: string;
  hostTabId: string;
};
export type SharedHostEvent =
  | { type: "frame"; value: LocalStreamEventContract }
  | { type: "transportLost" };

export const sharedHostAliasesUpdatedEvent = "lomi:shared-host-aliases-updated";

type RpcCall = RpcRequestContract["call"];
type RpcMethod = RpcCall["method"];
type SuccessfulRpcResponse = Extract<RpcResponseContract, { type: "result" }>;
type RpcResponseResult = SuccessfulRpcResponse["result"];
type RpcResult<Method extends RpcMethod> = Extract<
  RpcResponseResult,
  { method: Method }
>["params"];
type SharedWorkspaceState = RpcResult<"workspace.get">;
type SharedHostTerminalLayout = SharedWorkspaceState["tabs"][number]["layout"];
export type SharedHostWorkspaceState = SharedWorkspaceState;
export type SharedHostWorkspaceContext = WorkspaceContextContract;
export type SharedHostTerminalContext = TerminalContextContract;
export type SharedHostLease = RpcResult<"terminal.claim">;

export type SharedHostTerminalBinding = {
  workspaceId: string;
  workspaceName: string;
  rootPath: string;
  desktopTabId: string;
  tabName: string;
};
export type SharedHostCreateIntent =
  | { kind: "create" }
  | { kind: "split"; targetPaneId: string; axis: "horizontal" | "vertical" };
const createIntents = new Map<string, SharedHostCreateIntent>();

export function stageSharedHostCreate(
  paneId: string,
  intent: SharedHostCreateIntent,
) {
  createIntents.set(paneId, intent);
}

export function sharedHostCreateIntent(paneId: string) {
  return createIntents.get(paneId);
}

export function finishSharedHostCreate(paneId: string) {
  createIntents.delete(paneId);
}

export async function sharedHostOperationId(...parts: string[]) {
  const input = new TextEncoder().encode(
    `lomi-desktop-operation-v1\0${parts.join("\0")}`,
  );
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", input));
  digest[6] = (digest[6] & 0x0f) | 0x50;
  digest[8] = (digest[8] & 0x3f) | 0x80;
  const hex = [...digest.subarray(0, 16)]
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

export function sharedHostChannel(onEvent: (event: SharedHostEvent) => void) {
  const events = new Channel<SharedHostEvent>();
  events.onmessage = onEvent;
  return events;
}

const sharedWorkspaceImports = new Map<
  string,
  Promise<SharedHostWorkspaceImportReceipt>
>();

export async function importSharedWorkspace(
  workspaceId: string,
  name: string,
  rootPath: string,
) {
  const key = JSON.stringify([workspaceId, name, rootPath]);
  const pending = sharedWorkspaceImports.get(key);
  if (pending) return pending;
  const request = api<SharedHostWorkspaceImportReceipt>(
    "shared_host_import_workspace",
    {
      workspaceId,
      name,
      rootPath,
    },
  );
  sharedWorkspaceImports.set(key, request);
  try {
    return await request;
  } finally {
    if (sharedWorkspaceImports.get(key) === request)
      sharedWorkspaceImports.delete(key);
  }
}

export async function closeSharedHostTerminal(
  workspaceId: string,
  desktopPaneId: string,
) {
  return api<void>("shared_host_close_terminal", {
    workspaceId,
    desktopPaneId,
    operationId: await sharedHostOperationId(
      workspaceId,
      desktopPaneId,
      "close",
    ),
    requestId: crypto.randomUUID(),
    statusRequestId: crypto.randomUUID(),
  });
}

export function detachSharedHostView() {
  return api<void>("shared_host_detach_view");
}

export function sharedHostStatus() {
  return api<SharedHostStatus>("shared_host_status");
}

export function sharedHostAuthSetup() {
  return api<SharedHostAuthSetup>("shared_host_auth_setup");
}

export function configureSharedHostAuth(
  provider: LocalAuthProvider,
  acknowledgeProtectedFile: boolean,
) {
  return api<boolean>("shared_host_auth_configure", {
    provider,
    acknowledgeProtectedFile,
  });
}

export function sharedHostLoginStatus() {
  return api<SharedHostLoginStatus | null>("shared_host_login_status");
}

export function startSharedHostLogin(operationId: string) {
  return api<void>("shared_host_login_start", { operationId });
}

export function sharedHostLoginPresentation() {
  return api<SharedHostLoginPresentation>("shared_host_login_presentation");
}

export function logoutSharedHost() {
  return api<void>("shared_host_logout");
}

export function sharedHostWorkspaces() {
  return api<SharedHostWorkspaceSummary[]>("shared_host_workspaces");
}

export function sharedHostWorkspaceBindings(
  workspaceIds: string[],
  acknowledge?: { operationId: string; requestHash: string },
) {
  return api<SharedHostWorkspaceBindingsResponse>(
    "shared_host_workspace_bindings",
    {
      workspaceIds,
      warningOperationId: acknowledge?.operationId ?? null,
      warningRequestHash: acknowledge?.requestHash ?? null,
    },
  );
}

export function setSharedHostWorkspaceSharing(
  workspaceId: string,
  shared: boolean,
) {
  return api<SharedHostWorkspaceSummary>("shared_host_set_workspace_sharing", {
    workspaceId,
    shared,
  });
}

function terminalPane(pane: LayoutPane): pane is Pane {
  return (pane as { type?: string }).type === "terminal";
}

function privatePanels(layout: Layout): Exclude<LayoutPane, Pane>[] {
  return layoutPanes(layout).filter(
    (pane): pane is Exclude<LayoutPane, Pane> => !terminalPane(pane),
  );
}

function localPanels(
  layout: Layout,
  sharedPaneIds: ReadonlySet<string>,
): LayoutPane[] {
  return layoutPanes(layout).filter(
    (pane) => !terminalPane(pane) || !sharedPaneIds.has(pane.id),
  );
}

/** Merge the canonical terminal tree while retaining desktop-only panels and buffers. */
export function mergeSharedHostWorkspace(
  session: Session,
  workspaceId: string,
  rootPath: string,
  defaultProfileId: string,
  state: SharedHostWorkspaceState,
  aliases: SharedHostTerminalAlias[],
): Session {
  const project = session.projects.find((candidate) =>
    candidate.workspaces.some((workspace) => workspace.id === workspaceId),
  );
  const workspace = project?.workspaces.find(
    (candidate) => candidate.id === workspaceId,
  );
  if (!project || !workspace || project.path !== rootPath) return session;

  const oldTerminalTabs = workspace.tabs.filter(
    (tab): tab is TerminalTab => tab.type === "terminal",
  );
  const oldTerminalTabsById = new Map(
    oldTerminalTabs.map((tab) => [tab.id, tab]),
  );
  const oldTerminalPanesById = new Map(
    oldTerminalTabs.flatMap((tab) =>
      layoutPanes(tab.layout)
        .filter(terminalPane)
        .map((pane) => [pane.id, pane] as const),
    ),
  );
  const mappedDesktopPaneIds = new Set(
    aliases.map((alias) => alias.desktopPaneId),
  );
  const remoteTabs = state.tabs.map((hostTab) => {
    const hostTabId = hostTab.tabId;
    const alias = aliases.find(
      (candidate) => candidate.hostTabId === hostTabId,
    );
    const nonTerminalCollision = workspace.tabs.some(
      (tab) =>
        tab.id === (alias?.desktopTabId ?? hostTabId) &&
        tab.type !== "terminal",
    );
    const localTabId =
      alias?.desktopTabId ??
      (nonTerminalCollision ? `shared-host-tab-${hostTabId}` : hostTabId);
    const oldTab = oldTerminalTabsById.get(localTabId);
    const convert = (
      layout: (typeof hostTab)["layout"],
      path: string,
    ): Layout => {
      if (layout.kind === "pane") {
        const hostPaneId = layout.pane.paneId;
        const paneAlias = aliases.find(
          (candidate) => candidate.hostPaneId === hostPaneId,
        );
        const localPaneId = paneAlias?.desktopPaneId ?? hostPaneId;
        const previous =
          oldTerminalPanesById.get(localPaneId) ??
          oldTerminalPanesById.get(hostPaneId);
        return previous
          ? { ...previous, id: localPaneId }
          : { type: "terminal", id: localPaneId, cwd: "", profileId: "" };
      }
      return {
        type: "split",
        id: `shared-host-${hostTabId}-${path}`,
        axis: layout.axis,
        ratio: 0.5,
        first: convert(layout.first, `${path}0`),
        second: convert(layout.second, `${path}1`),
      };
    };
    let layout = convert(hostTab.layout, "root");
    for (const [index, panel] of (oldTab
      ? localPanels(oldTab.layout, mappedDesktopPaneIds)
      : []
    ).entries()) {
      layout = {
        type: "split",
        id: `shared-host-${hostTabId}-private-${index}-${panel.id}`,
        axis: "horizontal",
        ratio: 0.5,
        first: layout,
        second: panel,
      };
    }
    const terminalPanes = layoutPanes(layout).filter(terminalPane);
    return {
      id: localTabId,
      hostTabId,
      tab: {
        ...(oldTab ?? {}),
        type: "terminal" as const,
        id: localTabId,
        title: hostTab.name,
        customTitle: undefined,
        profileId: oldTab?.profileId || defaultProfileId,
        activePaneId: terminalPanes.some(
          (pane) => pane.id === oldTab?.activePaneId,
        )
          ? oldTab!.activePaneId
          : (terminalPanes[0]?.id ?? ""),
        layout,
      },
    };
  });

  const replacementById = new Map(remoteTabs.map((item) => [item.id, item]));
  const mappedDesktopTabIds = new Set(
    aliases.map((alias) => alias.desktopTabId),
  );
  const tabs: Session["projects"][number]["workspaces"][number]["tabs"] = [];
  let insertedHostTabs = false;
  for (const oldTab of workspace.tabs) {
    if (oldTab.type !== "terminal") {
      tabs.push(oldTab);
      continue;
    }
    const replacement = replacementById.get(oldTab.id);
    const hasHostOnlyPane = layoutPanes(oldTab.layout).some(
      (pane) => terminalPane(pane) && pane.profileId === "",
    );
    const hasMappedHostPane = layoutPanes(oldTab.layout).some(
      (pane) => terminalPane(pane) && mappedDesktopPaneIds.has(pane.id),
    );
    const belongsToHost =
      mappedDesktopTabIds.has(oldTab.id) ||
      replacement ||
      hasHostOnlyPane ||
      hasMappedHostPane;
    if (belongsToHost) {
      if (!insertedHostTabs) {
        tabs.push(...remoteTabs.map((item) => item.tab));
        insertedHostTabs = true;
      }
      if (
        !replacement &&
        (mappedDesktopTabIds.has(oldTab.id) ||
          hasHostOnlyPane ||
          hasMappedHostPane)
      ) {
        if (hasMappedHostPane) {
          let retained: Layout | null = oldTab.layout;
          for (const pane of layoutPanes(oldTab.layout)) {
            if (terminalPane(pane) && mappedDesktopPaneIds.has(pane.id))
              retained = retained ? removePane(retained, pane.id) : null;
          }
          if (retained && layoutPanes(retained).some(terminalPane)) {
            const retainedTerminals =
              layoutPanes(retained).filter(terminalPane);
            tabs.push({
              ...oldTab,
              activePaneId: retainedTerminals.some(
                (pane) => pane.id === oldTab.activePaneId,
              )
                ? oldTab.activePaneId
                : retainedTerminals[0].id,
              layout: retained,
            });
          } else if (retained) {
            tabs.push(...privatePanels(retained));
          }
        } else {
          tabs.push(...privatePanels(oldTab.layout));
        }
      }
      continue;
    }
    tabs.push(oldTab);
  }
  if (!insertedHostTabs) tabs.unshift(...remoteTabs.map((item) => item.tab));

  const tabIds = new Set(tabs.map((tab) => tab.id));
  let activeTabId = workspace.activeTabId;
  if (!tabIds.has(activeTabId)) {
    activeTabId =
      remoteTabs.find((item) =>
        aliases.some(
          (alias) =>
            alias.desktopTabId === workspace.activeTabId &&
            alias.hostTabId === item.hostTabId,
        ),
      )?.id ??
      remoteTabs[0]?.id ??
      tabs[0]?.id ??
      "";
  }
  if (
    activeTabId === workspace.activeTabId &&
    JSON.stringify(tabs) === JSON.stringify(workspace.tabs)
  )
    return session;
  return {
    ...session,
    projects: session.projects.map((candidateProject) =>
      candidateProject.id !== project.id
        ? candidateProject
        : {
            ...candidateProject,
            workspaces: candidateProject.workspaces.map((candidateWorkspace) =>
              candidateWorkspace.id !== workspaceId
                ? candidateWorkspace
                : { ...candidateWorkspace, tabs, activeTabId },
            ),
          },
    ),
  };
}

export function sharedWorkspaceState(workspaceId: string) {
  return api<SharedWorkspaceState>("shared_host_workspace_state", {
    workspaceId,
  });
}

export function sharedHostPaneLocation(
  state: SharedHostWorkspaceState,
  paneId: string,
) {
  const find = (
    layout: SharedHostTerminalLayout,
  ): { paneId: string } | undefined => {
    if (layout.kind === "pane")
      return layout.pane.paneId === paneId
        ? { paneId: layout.pane.paneId }
        : undefined;
    return find(layout.first) ?? find(layout.second);
  };
  for (const tab of state.tabs) {
    if (find(tab.layout)) return { tabId: tab.tabId, paneId };
  }
  return undefined;
}

export function sharedHostMoveMatches(
  state: SharedHostWorkspaceState,
  paneId: string,
  targetTabId: string,
  targetPaneId: string,
  placement: "before" | "after",
) {
  const target = state.tabs.find((tab) => tab.tabId === targetTabId);
  if (!target) return false;
  const visit = (layout: SharedHostTerminalLayout): boolean => {
    if (layout.kind === "pane") return false;
    if (
      layout.axis === "horizontal" &&
      (placement === "before"
        ? layout.first.kind === "pane" &&
          layout.first.pane.paneId === paneId &&
          layout.second.kind === "pane" &&
          layout.second.pane.paneId === targetPaneId
        : layout.second.kind === "pane" &&
          layout.second.pane.paneId === paneId &&
          layout.first.kind === "pane" &&
          layout.first.pane.paneId === targetPaneId)
    )
      return true;
    return visit(layout.first) || visit(layout.second);
  };
  return visit(target.layout);
}

export function validateSharedWorkspaceFrame(json: string) {
  return api<void>("shared_host_validate_workspace_frame", { json });
}

export function acknowledgeSharedHostStream(ack: LocalStreamAckContract) {
  return api<void>("shared_host_stream_ack", {
    stream: ack.stream,
    frame: ack.frame,
  });
}

type WorkspaceStreamAckContract = Extract<
  LocalStreamAckContract,
  { stream: "workspace" }
>["frame"];

export function acknowledgeSharedWorkspaceStream(
  frame: WorkspaceStreamAckContract,
) {
  return acknowledgeSharedHostStream({ stream: "workspace", frame });
}

export function registerSharedShellProfile(
  workspaceId: string,
  shellProfileId: string,
  cwd: string,
) {
  return api<string>("shared_host_register_shell_profile", {
    workspaceId,
    shellProfileId,
    cwd,
  });
}

export function sharedHostTerminalAliases(workspaceId: string) {
  return api<SharedHostTerminalAlias[]>("shared_host_terminal_aliases", {
    workspaceId,
  });
}

export function saveSharedHostTerminalAlias(
  workspaceId: string,
  alias: SharedHostTerminalAlias,
) {
  return api<void>("shared_host_save_terminal_alias", {
    workspaceId,
    ...alias,
  }).then(() => {
    window.dispatchEvent(
      new CustomEvent(sharedHostAliasesUpdatedEvent, {
        detail: { workspaceId },
      }),
    );
  });
}

export function sharedHostRpc<Method extends RpcMethod>(
  call: Extract<RpcCall, { method: Method }>,
  events?: Channel<SharedHostEvent>,
): Promise<RpcResult<Method>> {
  return (async () => {
    const response = await sharedHostRawRpc(call, events);
    if (response.type === "error")
      throw new Error(`Local host request failed (${response.error.code}).`);
    if (response.result.method !== call.method)
      throw new Error("Local host returned an unexpected response.");
    return response.result.params as RpcResult<Method>;
  })();
}

/** Keeps the full canonical RPC response, including effect and operation IDs. */
export async function sharedHostRawRpc(
  call: RpcCall,
  events?: Channel<SharedHostEvent>,
): Promise<RpcResponseContract> {
  const request: RpcRequestContract = {
    v: 2,
    type: "request",
    requestId: crypto.randomUUID(),
    timeoutMs: 30_000,
    call,
  };
  const eventChannel = events ?? new Channel<SharedHostEvent>();
  if (!events) eventChannel.onmessage = () => {};
  const response = await api<RpcResponseContract>("shared_host_rpc", {
    request,
    events: eventChannel,
  });
  if (!validateShape("rpc-response", response))
    throw new Error("Local host returned an invalid response.");
  if (response.requestId !== request.requestId)
    throw new Error("Local host returned a response for a different request.");
  if (response.type === "result" && response.result.method !== call.method)
    throw new Error("Local host returned an unexpected response.");
  return response;
}
