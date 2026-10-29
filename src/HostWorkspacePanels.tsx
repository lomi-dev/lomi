import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import type { CSSProperties } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import type {
  RpcRequestContract,
  RpcResponseContract,
} from "@lomi/remote-contracts";
import { validateShape } from "@lomi/remote-contracts";
import { ExplorerView } from "../../remote-web/src/ExplorerView";
import { RemoteV2GitPanel } from "../../remote-web/src/RemoteV2GitPanel";
import type { RemoteV2GitCall } from "../../remote-web/src/RemoteV2GitPanel";
import {
  createExplorerPendingStore,
  Explorer,
} from "../../remote-web/src/explorer";
import type {
  ExplorerTransport,
  RpcResponse,
  WatchFrame,
  WorkspaceContext,
} from "../../remote-web/src/explorer";
import "../../remote-web/src/remote-v2-embed.css";
import {
  acknowledgeSharedHostStream,
  importSharedWorkspace,
  sharedHostChannel,
  sharedHostRawRpc,
  sharedWorkspaceState,
} from "./shared-host";
import type { SharedHostEvent, SharedHostWorkspaceState } from "./shared-host";

type RpcCall = RpcRequestContract["call"];
type Capability = Extract<
  Extract<RpcResponseContract, { type: "result" }>["result"],
  { method: "workspace.capabilities" }
>["params"]["capabilities"][number];
type Panel = "files" | "git";

interface Props {
  panel: Panel;
  workspaceId: string;
  workspaceName: string;
  rootPath: string;
}

interface Authority {
  workspace: SharedHostWorkspaceState;
  capabilities: ReadonlySet<Capability>;
}

const invalidationRevisions = new Map<string, number>();
const invalidationListeners = new Map<string, Set<() => void>>();

function subscribeInvalidation(workspaceId: string, listener: () => void) {
  const listeners = invalidationListeners.get(workspaceId) ?? new Set();
  listeners.add(listener);
  invalidationListeners.set(workspaceId, listeners);
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) invalidationListeners.delete(workspaceId);
  };
}

function invalidationRevision(workspaceId: string) {
  return invalidationRevisions.get(workspaceId) ?? 0;
}

function invalidateWorkspace(workspaceId: string) {
  invalidationRevisions.set(workspaceId, invalidationRevision(workspaceId) + 1);
  invalidationListeners.get(workspaceId)?.forEach((listener) => listener());
}

class HostRpcError extends Error {
  constructor(
    message: string,
    readonly code: string,
  ) {
    super(message);
    this.name = "HostRpcError";
  }
}

function capabilityList(response: RpcResponseContract): Capability[] {
  if (response.type === "error") {
    throw new HostRpcError(response.error.message, response.error.code);
  }
  if (response.result.method !== "workspace.capabilities") {
    throw new Error(
      "The local host returned an unexpected capability response.",
    );
  }
  return response.result.params.capabilities;
}

async function inspectWorkspace(
  workspaceId: string,
  workspaceName: string,
  rootPath: string,
  importLocalWorkspace: boolean,
): Promise<Authority> {
  if (importLocalWorkspace) {
    await importSharedWorkspace(workspaceId, workspaceName, rootPath);
  }
  // This invokes WorkspaceInspect afresh. Its context is the authority for all
  // following file and Git calls in this panel instance.
  const workspace = await sharedWorkspaceState(workspaceId);
  const response = await sharedHostRawRpc({
    method: "workspace.capabilities",
    params: { context: workspace.context },
  });
  return { workspace, capabilities: new Set(capabilityList(response)) };
}

class LocalHostExplorerTransport implements ExplorerTransport {
  private readonly frameHandlers = new Set<(frame: WatchFrame) => void>();
  private readonly closeHandlers = new Set<(reason: Error) => void>();
  private closed = false;
  private closeReason = new Error("LOCAL_HOST_TRANSPORT_CLOSED");
  private events?: ReturnType<typeof sharedHostChannel>;

  constructor(
    private readonly onFilesystemChange: () => void,
    private readonly onTransportLost: (reason: Error) => void,
  ) {}

  async call(call: { method: string; params: unknown }): Promise<RpcResponse> {
    if (this.closed) throw this.closeReason;
    const isWatch = call.method === "files.watch";
    const isMutation =
      call.method === "files.create" ||
      call.method === "files.mkdir" ||
      call.method === "files.rename" ||
      call.method === "files.remove";
    let watchEvents: ReturnType<typeof sharedHostChannel> | undefined;
    if (isWatch) {
      // A resync opens a new watch. Retire the old callback before asking the
      // host to replace that binding, so its expected TransportLost event does
      // not close the new watch as well.
      if (this.events) this.events.onmessage = () => {};
      watchEvents = sharedHostChannel((event: SharedHostEvent) => {
        if (this.events !== watchEvents || this.closed) return;
        if (event.type === "transportLost") {
          this.fail(new Error("The local file watch connection was lost."));
          return;
        }
        if (event.value.stream !== "files") return;
        if (!validateShape("file-watch-frame", event.value.frame)) {
          this.fail(
            new Error("The local file watch returned an invalid frame."),
          );
          return;
        }
        const frame = event.value.frame as WatchFrame;
        if (frame.type !== "ack") this.onFilesystemChange();
        for (const handler of this.frameHandlers) handler(frame);
      });
      this.events = watchEvents;
    }
    try {
      const response = await sharedHostRawRpc(call as RpcCall, watchEvents);
      if (isWatch && response.type === "error") this.retireWatch(watchEvents);
      if (isMutation) this.onFilesystemChange();
      return response as RpcResponse;
    } catch (error) {
      if (isWatch) this.retireWatch(watchEvents);
      if (isMutation) this.onFilesystemChange();
      throw error;
    }
  }

  async sendWatchFrame(frame: WatchFrame): Promise<void> {
    if (this.closed) throw this.closeReason;
    if (frame.type !== "ack") return;
    await acknowledgeSharedHostStream({
      stream: "files",
      frame: frame.data,
    });
  }

  onWatchFrame(handler: (frame: WatchFrame) => void): () => void {
    if (this.closed) return () => {};
    this.frameHandlers.add(handler);
    return () => this.frameHandlers.delete(handler);
  }

  onClose(handler: (reason: Error) => void): () => void {
    if (this.closed) {
      handler(this.closeReason);
      return () => {};
    }
    this.closeHandlers.add(handler);
    return () => this.closeHandlers.delete(handler);
  }

  close(): void {
    this.fail(new Error("The local file watch was closed."), false);
  }

  private fail(reason: Error, notifyTransportLoss = true): void {
    if (this.closed) return;
    this.closed = true;
    this.closeReason = reason;
    if (this.events) this.events.onmessage = () => {};
    this.events = undefined;
    this.frameHandlers.clear();
    for (const handler of this.closeHandlers) {
      try {
        handler(reason);
      } catch {
        // A panel cleanup handler must not keep sibling cleanup from running.
      }
    }
    this.closeHandlers.clear();
    if (notifyTransportLoss) this.onTransportLost(reason);
  }

  private retireWatch(events?: ReturnType<typeof sharedHostChannel>): void {
    if (!events) return;
    events.onmessage = () => {};
    if (this.events === events) this.events = undefined;
  }
}

function PanelNotice({
  children,
  action,
  label,
}: {
  children: string;
  action?: () => void;
  label?: string;
}) {
  return (
    <div
      className="host-workspace-panel-notice flex items-center justify-between gap-2 border-b border-line bg-surface px-2 py-2 text-[12px] text-secondary"
      role="status"
    >
      <span>{children}</span>
      {action && (
        <button className="button" type="button" onClick={action}>
          {label ?? "Retry"}
        </button>
      )}
    </div>
  );
}

function FilesView({
  explorer,
  canWrite,
  reconnect,
}: {
  explorer: Explorer;
  canWrite: boolean;
  reconnect: () => void;
}) {
  const state = useSyncExternalStore(explorer.subscribe, explorer.getSnapshot);
  return (
    <div className="host-workspace-files flex min-h-0 min-w-0 flex-1 flex-col">
      {state.status === "error" && (
        <PanelNotice action={reconnect} label="Reconnect">
          {state.error ?? "The file connection is unavailable."}
        </PanelNotice>
      )}
      <ExplorerView explorer={explorer} canWrite={canWrite} />
    </div>
  );
}

export default function HostWorkspacePanels({
  panel,
  workspaceId,
  workspaceName,
  rootPath,
}: Props) {
  const [authority, setAuthority] = useState<Authority>();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [explorer, setExplorer] = useState<Explorer>();
  const [fileTransportLost, setFileTransportLost] = useState(false);
  const authorityRef = useRef<Authority | undefined>(undefined);
  const explorerRef = useRef<Explorer | undefined>(undefined);
  const transportRef = useRef<LocalHostExplorerTransport | undefined>(
    undefined,
  );
  const generation = useRef(0);
  const windowLabel = getCurrentWebviewWindow().label;
  const operationScope = authority
    ? `desktop-owner:${windowLabel}:local-host:${workspaceId}:${authority.workspace.context.workspaceId}`
    : undefined;

  const loadAuthority = useCallback(
    async (importLocalWorkspace: boolean) => {
      const token = ++generation.current;
      setLoading(true);
      setError("");
      try {
        const next = await inspectWorkspace(
          workspaceId,
          workspaceName,
          rootPath,
          importLocalWorkspace,
        );
        if (generation.current !== token) return undefined;
        authorityRef.current = next;
        setAuthority(next);
        return next;
      } catch (cause) {
        if (generation.current !== token) return undefined;
        setError(
          cause instanceof Error
            ? cause.message
            : "The local workspace service is unavailable.",
        );
        setAuthority(undefined);
        authorityRef.current = undefined;
        return undefined;
      } finally {
        if (generation.current === token) setLoading(false);
      }
    },
    [rootPath, workspaceId, workspaceName],
  );

  useEffect(() => {
    setAuthority(undefined);
    authorityRef.current = undefined;
    setFileTransportLost(false);
    void loadAuthority(true);
    return () => {
      generation.current++;
      const currentExplorer = explorerRef.current;
      explorerRef.current = undefined;
      currentExplorer?.dispose();
      transportRef.current?.close();
      transportRef.current = undefined;
    };
  }, [loadAuthority]);

  const subscribe = useCallback(
    (listener: () => void) => subscribeInvalidation(workspaceId, listener),
    [workspaceId],
  );
  const getInvalidation = useCallback(
    () => invalidationRevision(workspaceId),
    [workspaceId],
  );
  const gitInvalidation = useSyncExternalStore(
    subscribe,
    getInvalidation,
    getInvalidation,
  );

  const connectFiles = useCallback(
    async (freshAuthority?: Authority) => {
      const nextAuthority = freshAuthority ?? (await loadAuthority(false));
      if (!nextAuthority || !nextAuthority.capabilities.has("files.read"))
        return;
      const oldExplorer = explorerRef.current;
      explorerRef.current = undefined;
      oldExplorer?.dispose();
      const oldTransport = transportRef.current;
      transportRef.current = undefined;
      oldTransport?.close();
      setExplorer(undefined);
      setFileTransportLost(false);

      const transport = new LocalHostExplorerTransport(
        () => invalidateWorkspace(workspaceId),
        () => {
          if (transportRef.current !== transport) return;
          transportRef.current = undefined;
          const previous = explorerRef.current;
          explorerRef.current = undefined;
          previous?.dispose();
          setExplorer(undefined);
          setFileTransportLost(true);
          // Re-read authority without importing or replaying any mutation.
          void loadAuthority(false);
        },
      );
      const instance = new Explorer(
        transport,
        () => {
          const current = authorityRef.current;
          if (!current)
            throw new Error("The local workspace context is unavailable.");
          return current.workspace.context as WorkspaceContext;
        },
        () => crypto.randomUUID(),
        createExplorerPendingStore(
          `desktop-owner:${windowLabel}:local-host:${workspaceId}:${nextAuthority.workspace.context.workspaceId}`,
        ),
      );
      transportRef.current = transport;
      explorerRef.current = instance;
      setExplorer(instance);
      void instance.open(nextAuthority.workspace.context.workspaceId);
    },
    [loadAuthority, windowLabel, workspaceId],
  );

  useEffect(() => {
    if (
      panel !== "files" ||
      !authority?.capabilities.has("files.read") ||
      fileTransportLost
    ) {
      return;
    }
    void connectFiles(authority);
    return () => {
      const current = explorerRef.current;
      explorerRef.current = undefined;
      current?.dispose();
      const transport = transportRef.current;
      transportRef.current = undefined;
      transport?.close();
    };
  }, [authority, connectFiles, fileTransportLost, panel]);

  const retryAuthority = useCallback(() => {
    void loadAuthority(!fileTransportLost).then((next) => {
      if (next && panel === "files" && fileTransportLost) {
        setFileTransportLost(false);
      }
    });
  }, [fileTransportLost, loadAuthority, panel]);

  const retryFiles = useCallback(async () => {
    const next = await loadAuthority(false);
    if (next) {
      setFileTransportLost(false);
      if (explorerRef.current) return;
      // The effect installs the new single watch binding from the fresh context.
    }
  }, [loadAuthority]);

  const context = authority?.workspace.context;
  const canReadFiles = authority?.capabilities.has("files.read") ?? false;
  const canWriteFiles = authority?.capabilities.has("files.write") ?? false;
  const canReadGit = authority?.capabilities.has("git.read") ?? false;
  const canWriteGit = authority?.capabilities.has("git.write") ?? false;
  const canUseGitNetwork = authority?.capabilities.has("git.network") ?? false;
  const invokeGit = useMemo(
    () => (call: RemoteV2GitCall) => sharedHostRawRpc(call),
    [],
  );

  const content = (() => {
    if (loading)
      return <PanelNotice>Loading local workspace state…</PanelNotice>;
    if (!authority || !context) {
      return (
        <PanelNotice action={retryAuthority}>
          {error || "The local workspace service is unavailable."}
        </PanelNotice>
      );
    }
    if (panel === "files") {
      if (!canReadFiles) {
        return (
          <PanelNotice>
            File read access is unavailable for this workspace.
          </PanelNotice>
        );
      }
      if (fileTransportLost || !explorer) {
        return (
          <PanelNotice action={retryFiles} label="Reconnect">
            The file connection is unavailable. Workspace state was refreshed
            without replaying changes.
          </PanelNotice>
        );
      }
      return (
        <FilesView
          explorer={explorer}
          canWrite={canWriteFiles}
          reconnect={() => void retryFiles()}
        />
      );
    }
    return (
      <div className="flex min-h-0 min-w-0 flex-1 overflow-auto">
        <RemoteV2GitPanel
          context={context}
          invoke={invokeGit}
          canRead={canReadGit}
          canWrite={canWriteGit}
          canNetwork={canUseGitNetwork}
          active
          invalidation={gitInvalidation}
          operationScope={operationScope}
          onMutation={() => invalidateWorkspace(workspaceId)}
        />
      </div>
    );
  })();

  return (
    <div
      className="host-workspace-panel flex h-full min-h-0 min-w-0 flex-1 flex-col"
      style={
        {
          "--lomi-remote-color-background": "var(--color-surface)",
          "--lomi-remote-color-foreground": "var(--color-surface-text)",
          "--lomi-remote-color-surface": "var(--color-surface-container)",
          "--lomi-remote-color-subtle": "var(--color-surface-container-high)",
          "--lomi-remote-color-raised":
            "var(--color-surface-container-highest)",
          "--lomi-remote-color-line": "var(--color-outline)",
          "--lomi-remote-color-muted": "var(--color-muted-text)",
          "--lomi-remote-color-secondary": "var(--color-surface-variant-text)",
          "--lomi-remote-color-lime": "var(--color-primary)",
          "--lomi-remote-radius-control": "var(--radius-control)",
        } as CSSProperties
      }
    >
      {content}
    </div>
  );
}
