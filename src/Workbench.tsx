import { useAgentControlBridge } from "./agent-control";
import AgentControlStartup from "./AgentControlStartup";
import { useAgentChatApproval } from "./AgentChatApproval";
import { useAgentGitApproval } from "./AgentGitApproval";
import {
  configureChats,
  retainChats,
  createChat,
  chatAction,
} from "./chat/chat-service";
import {
  updateChat,
  newChatTab,
  chatTabs,
  removeChatConversation,
} from "./model";
import { builtinViews } from "./plugins/builtins";
import { listen } from "@tauri-apps/api/event";
import PluginPanel from "./plugins/PluginPanel";
import { useThemes } from "./ThemeProvider";
import { pluginHost, HostContext } from "./plugins/runtime";
import { setPluginCloseHandler } from "./plugins/PluginsProvider";
import { pluginPanels, updatePluginPanel, newId } from "./model";
import Select from "./Select";
import {
  configureAndroid,
  receiveAndroidOpen,
  retainAndroid,
  stopLastAndroidViews,
  type OpenIntent,
} from "./android/service";
import { newAndroidTab, androidTabs, updateAndroid } from "./model";
import { flushSync } from "react-dom";
import {
  Suspense,
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { Folder, GitBranch, Layers, Settings, Terminal, X } from "./icons";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
  api,
  errorMessage,
  getInfo,
  loadSession,
  native,
  saveSession,
} from "./api";
import useGit from "./useGit";
import { SourceControlState } from "./source-control-state";
import {
  active,
  activePanel,
  browserTabs,
  newBrowserTab,
  updateBrowser,
  addWorkspace,
  filesInTab,
  fileTabs,
  layoutPanes,
  updateFilePosition,
  updateFilePreviewView,
  basename,
  canSplitPane,
  mapLayout,
  mergeTabs,
  moveTab,
  movePane,
  moveSidebar,
  showSidebar,
  toggleSidebar,
  newPane,
  newProject,
  newTab,
  newFileTab,
  openCommitTab,
  openDiffTab,
  openFileTab,
  panes,
  removePane,
  removeTabs,
  removeWorkspace,
  resizeSplit,
  restoreSession,
  splitPane,
  tabsToClose,
  tabTitle,
  updateDirectories,
  updateTab,
  updateFile,
  updateWorkspace,
} from "./model";
import type {
  AppInfo,
  Session,
  ShellProfile,
  SidebarPanel,
  Split,
  TabCloseAction,
  TabDropSide,
  TerminalTab,
  Workspace,
} from "./model";
import {
  closeTerminals,
  configureTerminals,
  observeTerminalContexts,
  runningTerminal,
} from "./terminal-runtime";
import type { TerminalContext } from "./terminal-runtime";
import { dropPaths, terminalAtNativePosition } from "./file-drag";
import { IconButton, Modal, WindowControls } from "./ui";
import Explorer from "./Explorer";
import ProjectSwitcher from "./ProjectSwitcher";
import SourceControl from "./SourceControl";
import Sidebar from "./Sidebar";
import SidebarToggle from "./SidebarToggle";
import Workspaces from "./Workspaces";
import Welcome from "./Welcome";
import { configureBrowsers, retainBrowsers } from "./browser-runtime";
import SplitView from "./SplitView";
import { usePaneMotion } from "./pane-motion";
import { usePointerFocus } from "./usePointerFocus";
import { useWindowZoom, changeWindowZoom } from "./useWindowZoom";
import TabBar from "./TabBar";
import FileEditorStatus from "./FileEditorStatus";
import { useKeybindings } from "./KeybindingsProvider";
import { useTerminalPreferences } from "./TerminalPreferencesProvider";
import { useTerminalTitleReveal } from "./useTerminalTitleReveal";
import { defaultTerminalProfile } from "./terminal-preferences";
import type { ActionId } from "./keybindings";
import CommandPicker from "./plugins/CommandPicker";
import { PluginFills, Slot, SlotProvider } from "./plugins/Slots";
import { commandAvailable } from "./plugins/host";
import {
  actionForEvent,
  isTextInput,
  isZoomAction,
  shortcutTitle,
} from "./keybindings";
import {
  captureEditorPositions,
  editorRevision,
  loadedEditor,
  openEditorDocument,
  pauseEditorFileOperations,
  relocateEditorFiles,
  retainEditorTabs,
  subscribeEditors,
  subscribeEditorSaves,
} from "./editor-service";
import { useCloseGuard } from "./CloseGuard";
import { useUpdater } from "./Updater";
import {
  prepareApplicationClose,
  type ReleaseClosePreparation,
} from "./application-close";
import { useCliIntegrations } from "./CliIntegrations";
import { useAgentNotifications } from "./AgentNotifications";
import {
  absoluteFilePath,
  applyFileChange,
  containsPath,
} from "./explorer-model";
import type { FileChange, FileOperation } from "./explorer-model";
import type { SearchMatch } from "./ProjectSearch";
import "@xterm/xterm/css/xterm.css";

type Dialog =
  | {
      type: "name";
      title: string;
      initial: string;
      submit: (name: string) => void;
    }
  | { type: "confirm"; title: string; text: string; submit: () => void }
  | {
      type: "environment";
      title: string;
      profiles: ShellProfile[];
      selected: string;
      submit: (profileId: string) => void;
    }
  | { type: "preview"; title: string; content: string };
let bootstrap:
  Promise<{ info: AppInfo; saved: unknown; restoreError: string }> | undefined;
function initialize() {
  return (bootstrap ??= (async () => {
    const info = await getInfo();
    await api("reset_terminals");
    try {
      const saved = await loadSession();
      if (
        saved &&
        typeof saved === "object" &&
        "version" in saved &&
        saved.version !== 1 &&
        saved.version !== 2 &&
        saved.version !== 3
      ) {
        throw new Error(
          "This session was saved in an unsupported format. The saved file has been left intact.",
        );
      }
      if (saved) restoreSession(saved, info);
      return { info, saved, restoreError: "" };
    } catch (error) {
      return { info, saved: null, restoreError: errorMessage(error) };
    }
  })());
}

export default function Workbench() {
  const [sourceControlState] = useState(() => new SourceControlState());
  const closeGuard = useCloseGuard();
  const gitApproval = useAgentGitApproval();
  const chatApproval = useAgentChatApproval();
  const theme = useThemes();
  useSyncExternalStore(subscribeEditors, editorRevision);
  const fileOpenRequest = useRef(0);
  const fileOperationBusy = useRef(false);
  const preferences = useKeybindings();
  const [commandPicker, setCommandPicker] = useState(false);
  const [docking, setDocking] = useState<{
    mode: "movePanel" | "dockTab";
    target: string;
    side: TabDropSide;
  } | null>(null);
  const executeBuiltin = useRef<(id: ActionId) => void>(() => {});
  const terminalPreferences = useTerminalPreferences();
  const revealTerminalTitles = useTerminalTitleReveal(
    !terminalPreferences.value.alwaysShowTitles,
  );
  const { bindings } = preferences;
  const [info, setInfo] = useState<AppInfo>();
  const defaultProfileId = info
    ? defaultTerminalProfile(info, terminalPreferences.value.windowsShell)
    : "";
  const [session, renderSession] = useState<Session>();
  const currentSession = useRef<Session>(undefined);
  const workArea = useRef<HTMLDivElement>(null);
  const terminalLayout = useRef<HTMLDivElement>(null);
  const selected = session ? active(session) : undefined;
  const renderPaneLayout = usePaneMotion(workArea);
  const setSession = useCallback(
    (
      update: Session | ((state: Session | undefined) => Session | undefined),
      animate = false,
    ) => {
      const previous = currentSession.current;
      const next = typeof update === "function" ? update(previous) : update;
      // Queued shortcuts must see the updated layout before React renders it.
      currentSession.current = next;
      retainEditorTabs(next);
      retainBrowsers(next);
      retainChats(next);
      retainAndroid(next);
      pluginHost.retainPanels(
        new Set(next ? pluginPanels(next).map((panel) => panel.id) : []),
      );
      const previousSelection = previous ? active(previous) : undefined;
      const nextSelection = next ? active(next) : undefined;
      const sameWorkspace =
        !!previousSelection &&
        !!nextSelection &&
        previousSelection.workspace.id === nextSelection.workspace.id;
      const sameTab =
        sameWorkspace && previousSelection?.tab.id === nextSelection?.tab.id;
      const motion =
        sameTab && previous && next
          ? animate
            ? "panes"
            : previous.sidebar !== next.sidebar ||
                previous.rightSidebar !== next.rightSidebar
              ? "sidebars"
              : false
          : false;
      renderPaneLayout(() => renderSession(next), motion, !sameTab);
    },
    [renderPaneLayout],
  );
  useAgentControlBridge(
    session,
    () => currentSession.current,
    setSession,
    (id, terminal) =>
      closeGuard.confirm(new Set([id]), terminal ? [id] : [], undefined, true),
    async (operation, trash) => {
      if (fileOperationBusy.current || closing.current || updater.busy.current)
        throw new Error("TARGET_BUSY");
      fileOperationBusy.current = true;
      let resume: (() => void) | undefined;
      try {
        let verifyTrashBuffers: (() => void) | undefined;
        if (trash) {
          const current = currentSession.current;
          const targetProject = current?.projects.find(
            (p) => p.id === trash.projectId,
          );
          if (!current || !targetProject) throw new Error("TARGET_NOT_FOUND");
          const path = `${targetProject.path}/${trash.relativePath}`;
          const canonical = `${trash.projectPath}/${trash.relativePath}`;
          if (
            current.projects.some(
              (p) =>
                containsPath(path, p.path) || containsPath(canonical, p.path),
            )
          )
            throw new Error("TARGET_BUSY");
          const files = fileTabs(current).filter(
            (file) =>
              containsPath(path, absoluteFilePath(file)) ||
              containsPath(
                canonical,
                loadedEditor(file)?.path ?? absoluteFilePath(file),
              ),
          );
          const documents = [
            ...new Set(
              files.flatMap((file) => {
                const document = loadedEditor(file);
                return document ? [document] : [];
              }),
            ),
          ];
          const captured = documents.map((document) => ({
            document,
            revision: document.trashRevision(),
            dirty: document.dirty,
          }));
          const check = () => {
            if (currentSession.current !== current)
              throw new Error("REVISION_CONFLICT");
            if (Date.now() >= Number(trash.notAfterMillis))
              throw new Error("DEADLINE_EXCEEDED");
            const loaded = new Set(
              files.map((file) => loadedEditor(file)).filter(Boolean),
            );
            if (
              loaded.size !== captured.length ||
              captured.some(({ document }) => !loaded.has(document))
            )
              throw new Error("REVISION_CONFLICT");
            for (const { document, revision } of captured) {
              const now = document.trashRevision();
              if (
                now.documentId !== revision.documentId ||
                now.path !== revision.path ||
                now.bufferRevision !== revision.bufferRevision
              )
                throw new Error("REVISION_CONFLICT");
            }
          };
          verifyTrashBuffers = check;
          const buffers = captured
            .filter((v) => v.dirty)
            .map(({ revision }) => {
              if (!revision.path.startsWith(`${trash.projectPath}/`))
                throw new Error("SCOPE_DENIED");
              return {
                documentId: revision.documentId,
                relativePath: revision.path.slice(trash.projectPath.length + 1),
                bufferRevision: revision.bufferRevision,
                diskRevision: revision.diskRevision,
              };
            });
          const plan = await api<{ planHash: string; awaitingUser: boolean }>(
            "agent_control_file_trash_prepare",
            {
              operationId: trash.operationId,
              nonce: trash.nonce,
              buffers,
            },
          );
          if (plan.awaitingUser !== buffers.length > 0)
            throw new Error("REVISION_CONFLICT");
          if (plan.awaitingUser) {
            const approve = async (choice: "save" | "discard") => {
              check();
              await api("agent_control_file_trash_decide", {
                operationId: trash.operationId,
                nonce: trash.nonce,
                planHash: plan.planHash,
                approved: choice === "discard",
              });
            };
            if (
              !(await closeGuard.confirm(new Set(files.map((f) => f.id)), [], {
                description: `An agent requested moving “${trash.relativePath}” to Trash. Saving changes will require the agent to request Trash again for the saved version.`,
                onDecision: approve,
                isActive: async () => {
                  check();
                  return await api<boolean>(
                    "agent_control_file_trash_pending",
                    {
                      operationId: trash.operationId,
                      nonce: trash.nonce,
                      planHash: plan.planHash,
                    },
                  );
                },
              }))
            ) {
              await api("agent_control_file_trash_decide", {
                operationId: trash.operationId,
                nonce: trash.nonce,
                planHash: plan.planHash,
                approved: false,
              }).catch(() => {});
              throw new Error("CONTROL_REVOKED");
            }
          }
        }
        resume = await pauseEditorFileOperations();
        verifyTrashBuffers?.();
        return await operation();
      } finally {
        resume?.();
        fileOperationBusy.current = false;
      }
    },
    (result, canonicalChange) => applyDiskFileChange(result, canonicalChange),
    gitApproval.confirm,
    chatApproval.confirm,
    (ids, decision) => {
      if (fileOperationBusy.current || closing.current || updater.busy.current)
        throw new Error("TARGET_BUSY");
      return closeGuard.confirm(ids, [], decision, true);
    },
    () => git.refresh(),
    () => {
      if (
        fileOperationBusy.current ||
        closing.current ||
        updater.busy.current ||
        folderPickerBusy.current
      )
        throw new Error("TARGET_BUSY");
      const container = terminalLayout.current;
      return container?.isConnected
        ? { width: container.clientWidth, height: container.clientHeight }
        : null;
    },
  );
  useEffect(
    () =>
      setPluginCloseHandler(async (owner) => {
        const ids = new Set(
          (currentSession.current ? pluginPanels(currentSession.current) : [])
            .filter((p) => p.owner === owner)
            .map((p) => p.id),
        );
        return closeGuard.confirm(ids, []);
      }),
    [closeGuard.confirm],
  );
  const [focusedPlugin, setFocusedPlugin] = useState<string | null>(null);
  const focusedSidebar = selected?.workspace.pluginSidebars?.find(
    (panel) =>
      panel.id === focusedPlugin &&
      (session?.sidebar === panel.viewType ||
        session?.rightSidebar === panel.viewType),
  );
  const hostContext = {
    projectPath: selected?.project.path ?? null,
    workspaceName: selected?.workspace.name ?? null,
    activePanelId: selected ? (activePanel(selected.tab)?.id ?? null) : null,
    viewType: selected
      ? ((activePanel(selected.tab)?.type === "plugin"
          ? (activePanel(selected.tab) as import("./model").PluginPanel)
              .viewType
          : activePanel(selected.tab)?.type) ?? null)
      : null,
    appearance: theme.snapshot.appearance,
    themeRevision: theme.snapshot.sourceRevision,
  };
  if (focusedSidebar) {
    hostContext.activePanelId = focusedSidebar.id;
    hostContext.viewType = focusedSidebar.viewType;
  }
  useEffect(() => {
    const focus = (event: FocusEvent) => {
      const target = event.target;
      if (
        target instanceof HTMLElement &&
        target.closest(".terminal-stage, .tab-bar") &&
        !target.closest(".sidebar")
      )
        setFocusedPlugin(null);
    };
    document.addEventListener("focusin", focus);
    return () => document.removeEventListener("focusin", focus);
  }, []);
  useEffect(() => {
    pluginHost.setContext(hostContext);
    pluginHost.openView = async (viewType, value) => {
      const state = currentSession.current;
      const selection = state && active(state);
      if (!state || !selection) throw new Error("Open a workspace first.");
      const entry = pluginHost.catalog.entries.find((e) =>
        e.manifest?.contributes?.views?.some((v) => v.id === viewType),
      );
      const view = entry?.manifest?.contributes?.views?.find(
        (v) => v.id === viewType,
      );
      if (!entry || !view) throw new Error("Unknown plugin view.");
      if (view.placement === "sidebar") {
        const existing = selection.workspace.pluginSidebars?.find(
          (p) => p.viewType === viewType,
        );
        const panel = existing ?? {
          type: "plugin" as const,
          id: newId(),
          title: view.title,
          owner: entry.id,
          viewType,
          stateVersion: view.stateVersion,
          state: value,
        };
        const side = state.sidebarSides[viewType as SidebarPanel] ?? "left";
        const next = updateWorkspace(state, selection.workspace.id, (w) => ({
          ...w,
          pluginSidebars: existing
            ? w.pluginSidebars
            : [...(w.pluginSidebars ?? []), panel],
        }));
        setSession(moveSidebar(next, viewType as SidebarPanel, side));
        return panel.id;
      }
      const existing =
        !view.multiple &&
        pluginPanels(state).find(
          (p) =>
            p.viewType === viewType &&
            selection.workspace.tabs.some(
              (t) =>
                t.id === p.id ||
                (t.type === "terminal" &&
                  layoutPanes(t.layout).some((pane) => pane.id === p.id)),
            ),
        );
      const panel = existing || {
        type: "plugin" as const,
        id: newId(),
        title: view.title,
        owner: entry.id,
        viewType,
        stateVersion: view.stateVersion,
        state: value,
      };
      setSession(
        updateWorkspace(state, selection.workspace.id, (w) => {
          const parent = w.tabs.find(
            (t) =>
              t.id === panel.id ||
              (t.type === "terminal" &&
                layoutPanes(t.layout).some((p) => p.id === panel.id)),
          );
          return {
            ...w,
            tabs: parent
              ? w.tabs.map((t) =>
                  t === parent && t.type === "terminal"
                    ? { ...t, activePaneId: panel.id }
                    : t,
                )
              : [...w.tabs, panel],
            activeTabId: parent?.id ?? panel.id,
          };
        }),
      );
      return panel.id;
    };
    if (session) pluginHost.start();
    pluginHost.panelOwner = (id) =>
      currentSession.current
        ? pluginPanels(currentSession.current).find((panel) => panel.id === id)
            ?.owner
        : undefined;
    pluginHost.retainPanels(
      new Set(session ? pluginPanels(session).map((panel) => panel.id) : []),
    );
    pluginHost.theme(theme.snapshot.revision, theme.snapshot.appearance);
  }, [session, theme.snapshot, focusedPlugin]);
  const [error, setError] = useState("");
  const [diffRevision, setDiffRevision] = useState(0);
  useWindowZoom(setError);
  const [restoreError, setRestoreError] = useState("");
  useEffect(
    () =>
      subscribeEditorSaves((id, location) => {
        setSession((state) =>
          state
            ? updateFile(state, id, (file) => {
                const { untitled: _untitled, ...saved } = file;
                return {
                  ...saved,
                  ...location,
                  title: basename(location.relative),
                };
              })
            : state,
        );
      }),
    [setSession],
  );
  const [paneNotice, setPaneNotice] = useState("");
  const cliIntegrations = useCliIntegrations(
    selected ? activePanel(selected.tab)?.id : undefined,
    setError,
    setPaneNotice,
  );
  const agentNotifications = useAgentNotifications(
    session,
    terminalPreferences.ready && terminalPreferences.value.agentNotifications,
    setError,
    setPaneNotice,
  );
  const [projectMenuOpen, setProjectMenuOpen] = useState(false);
  const folderPickerBusy = useRef(false);
  const [browsing, setBrowsing] = useState(false);
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [terminalOverview, setTerminalOverview] = useState(false);
  usePointerFocus(
    preferences.focusFollowsPointer && !terminalOverview,
    terminalLayout,
  );
  const savingEnabled = useRef(false);
  const [autosavePaused, setAutosavePaused] = useState(false);
  const autosaveTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const closing = useRef(false);
  const [stoppingForClose, setStoppingForClose] = useState<
    "stopping" | "cancelling" | null
  >(null);
  const cancelClose = useRef(false);
  const closeDescriptionId = useId();
  const cancelCloseButton = useRef<HTMLButtonElement>(null);
  const requestCancelClose = () => {
    cancelClose.current = true;
    setStoppingForClose("cancelling");
  };
  const prepareAndroidRemoval = async (ids: ReadonlySet<string>) => {
    if (
      !androidTabs(currentSession.current).some(
        (tab) => ids.has(tab.id) && tab.deviceId,
      )
    )
      return true;
    if (closing.current) return false;
    closing.current = true;
    cancelClose.current = false;
    flushSync(() => setStoppingForClose("stopping"));
    try {
      await stopLastAndroidViews(ids, () => currentSession.current);
      return !cancelClose.current;
    } catch (reason) {
      setError(errorMessage(reason));
      return false;
    } finally {
      closing.current = false;
      setStoppingForClose(null);
    }
  };
  useEffect(() => {
    configureAndroid({
      change: (id, change) => {
        if (!closing.current)
          setSession((state) =>
            state ? updateAndroid(state, id, change) : state,
          );
      },
      error: setError,
      session: () => currentSession.current,
      commit: (update) =>
        setSession((state) => (state ? update(state) : state)),
      blocked: () =>
        closing.current || updater.busy.current || fileOperationBusy.current,
    });
  }, [setSession]);
  useEffect(() => {
    if (!native) return;
    const listener = listen<OpenIntent>(
      "android-open-request",
      ({ payload }) => {
        void receiveAndroidOpen(payload).catch((error) =>
          setError(errorMessage(error)),
        );
      },
    );
    return () => {
      void listener.then((stop) => stop()).catch(() => {});
    };
  }, []);
  const prepareClose = async (showProgress = true) => {
    try {
      const release = await prepareApplicationClose(
        closeGuard.confirm,
        async () => {
          // A debounce from an earlier layout must not overtake the final save.
          clearTimeout(autosaveTimer.current);
          flushSync(() => setAutosavePaused(true));
          if (currentSession.current && savingEnabled.current) {
            await saveSession(captureEditorPositions(currentSession.current));
          }
        },
        showProgress
          ? () => {
              cancelClose.current = false;
              // Enter the modal before saving; edits must not race the final session.
              flushSync(() => setStoppingForClose("stopping"));
              return {
                cancelled: () => cancelClose.current,
                release: () => setStoppingForClose(null),
              };
            }
          : undefined,
      );
      if (!release) {
        setAutosavePaused(false);
        return null;
      }
      return async () => {
        try {
          await release();
        } finally {
          setAutosavePaused(false);
        }
      };
    } catch (error) {
      setAutosavePaused(false);
      throw error;
    }
  };
  const updater = useUpdater(!!info, async () => {
    if (closing.current || fileOperationBusy.current) return null;
    return prepareClose(false);
  });
  useEffect(() => {
    if (!native) return;
    const stop = listen("plugin-restart-request", () => {
      void (async () => {
        if (
          closing.current ||
          fileOperationBusy.current ||
          updater.busy.current
        )
          return;
        closing.current = true;
        let release: ReleaseClosePreparation | null = null;
        let restarting = false;
        try {
          release = await prepareClose();
          if (!release) return;
          await api("restart_plugins");
          restarting = true;
        } catch (error) {
          setError(errorMessage(error));
        } finally {
          if (!restarting && release) {
            await release().catch((error) => setError(errorMessage(error)));
          }
          closing.current = false;
        }
      })();
    });
    return () => {
      void stop.then((stop) => stop()).catch(() => {});
    };
  }, [closeGuard.confirm]);
  const git = useGit(selected?.project.path ?? "");
  const closeProjectMenu = useCallback(() => setProjectMenuOpen(false), []);

  useEffect(() => {
    if (!native) return;
    const stop = listen<string>("chat-conversation-deleted", ({ payload }) => {
      setSession((state) =>
        state
          ? removeChatConversation(state, payload, defaultProfileId)
          : state,
      );
    });
    return () => {
      void stop.then((fn) => fn()).catch(() => {});
    };
  }, [setSession, defaultProfileId]);
  useEffect(
    () =>
      configureChats({
        activate: (workspace, panel) =>
          setSession((state) =>
            state
              ? updateWorkspace(state, workspace, (w) => {
                  const parent = w.tabs.find(
                    (t) =>
                      t.id === panel ||
                      (t.type === "terminal" &&
                        layoutPanes(t.layout).some((p) => p.id === panel)),
                  );
                  return parent
                    ? {
                        ...w,
                        activeTabId: parent.id,
                        tabs: w.tabs.map((t) =>
                          t === parent && t.type === "terminal"
                            ? { ...t, activePaneId: panel }
                            : t,
                        ),
                      }
                    : w;
                })
              : state,
          ),
        update: (id, change) =>
          setSession((state) =>
            state ? updateChat(state, id, change) : state,
          ),
        error: setError,
      }),
    [setSession],
  );
  useEffect(() => {
    configureBrowsers(
      (id, change) =>
        setSession((state) =>
          state ? updateBrowser(state, id, change) : state,
        ),
      (id, url) =>
        setSession((state) => {
          if (!state) return state;
          const workspace = state.projects
            .flatMap((project) => project.workspaces)
            .find((workspace) =>
              workspace.tabs.some((tab) =>
                tab.type === "browser"
                  ? tab.id === id
                  : tab.type === "terminal" &&
                    layoutPanes(tab.layout).some((pane) => pane.id === id),
              ),
            );
          if (!workspace || !browserTabs(state).some((tab) => tab.id === id))
            return state;
          const added = newBrowserTab(url);
          return updateWorkspace(state, workspace.id, (current) => ({
            ...current,
            tabs: [...current.tabs, added],
            activeTabId: added.id,
          }));
        }),
      setError,
    );
  }, [setSession]);

  useEffect(() => {
    if (preferences.error) setError(preferences.error);
  }, [preferences.error]);
  useEffect(() => {
    if (!paneNotice) return;
    const timer = setTimeout(() => setPaneNotice(""), 5000);
    return () => clearTimeout(timer);
  }, [paneNotice]);

  useEffect(() => {
    if (!native) return;
    let current = true;
    void initialize()
      .then(({ info, saved, restoreError }) => {
        if (!current) return;
        setInfo(info);
        setSession(restoreSession(saved, info));
        setRestoreError(restoreError);
        savingEnabled.current = !restoreError;
      })
      .catch((error) => {
        if (current) setError(errorMessage(error));
      });
    return () => {
      current = false;
    };
  }, []);
  useEffect(() => {
    configureTerminals(
      (id, cwd) =>
        setSession((state) =>
          state ? updateDirectories(state, { [id]: cwd }) : state,
        ),
      setError,
    );
  }, []);
  useEffect(() => {
    if (!session || !savingEnabled.current || autosavePaused) return;
    const timer = setTimeout(() => {
      const latest = currentSession.current;
      if (latest)
        void saveSession(captureEditorPositions(latest)).catch((error) =>
          setError(`Could not save the session: ${errorMessage(error)}`),
        );
    }, 400);
    autosaveTimer.current = timer;
    return () => clearTimeout(timer);
  }, [session, autosavePaused]);
  useEffect(() => {
    if (!info) return;
    const timer = setInterval(() => {
      void api<Record<string, TerminalContext>>("terminal_contexts")
        .then((contexts) => {
          const directories = observeTerminalContexts(contexts);
          void cliIntegrations.observe(contexts);
          setSession((state) =>
            state ? updateDirectories(state, directories) : state,
          );
        })
        .catch(() => {});
    }, 1000);
    return () => clearInterval(timer);
  }, [info, cliIntegrations.observe]);
  useEffect(() => {
    if (!info) return;
    let current = true;
    const unlisten = getCurrentWindow().onCloseRequested(async (event) => {
      event.preventDefault();
      if (closing.current || updater.busy.current) return;
      closing.current = true;
      let release: ReleaseClosePreparation | null = null;
      let destroyed = false;
      try {
        release = await prepareClose();
        if (!release) return;
        await getCurrentWindow().destroy();
        destroyed = true;
      } catch (error) {
        setError(`Could not close the window: ${errorMessage(error)}`);
      } finally {
        if (!destroyed && release) {
          await release().catch((error) => setError(errorMessage(error)));
        }
        if (!destroyed) closing.current = false;
      }
    });
    void unlisten
      .then((stop) => {
        if (!current) stop();
      })
      .catch((error) => setError(errorMessage(error)));
    return () => {
      current = false;
      void unlisten.then((stop) => stop()).catch(() => {});
    };
  }, [info]);
  useEffect(() => {
    if (!info) return;
    let current = true;
    let target: HTMLElement | null = null;
    const unlisten = getCurrentWebviewWindow().onDragDropEvent(
      ({ payload }) => {
        target?.classList.remove("drop-target");
        if (payload.type === "leave") {
          target = null;
          return;
        }
        target = terminalAtNativePosition(payload.position, info.platform);
        if (payload.type === "drop") {
          void dropPaths(target, payload.paths, setError);
          target = null;
        } else target?.classList.add("drop-target");
      },
    );
    void unlisten
      .then((stop) => {
        if (!current) stop();
      })
      .catch((error) => setError(errorMessage(error)));
    return () => {
      current = false;
      target?.classList.remove("drop-target");
      void unlisten.then((stop) => stop()).catch(() => {});
    };
  }, [info]);
  useEffect(() => {
    if (!native) return;
    void getCurrentWindow()
      .setTitle(
        selected
          ? `${basename(selected.project.path)} — ${selected.workspace.name} — Lomi`
          : "Lomi",
      )
      .catch(() => {});
  }, [selected?.project.path, selected?.workspace.name]);

  const change = (transform: (state: Session) => Session, animate = false) =>
    setSession((state) => (state ? transform(state) : state), animate);
  const addTab = (cwd?: string) =>
    change((state) => {
      const selection = active(state);
      if (!selection) return state;
      const { project, workspace, tab } = selection;
      const added = newTab(
        cwd ?? project.path,
        info?.platform !== "windows" && tab.type === "terminal"
          ? tab.profileId
          : defaultProfileId,
        `Terminal ${workspace.tabs.length + 1}`,
      );
      return updateWorkspace(state, workspace.id, (workspace) => ({
        ...workspace,
        tabs: [...workspace.tabs, added],
        activeTabId: added.id,
      }));
    });
  const addChat = async () => {
    const state = currentSession.current;
    const selection = state && active(state);
    if (!selection) return;
    try {
      const conversation = await createChat(
        selection.project,
        selection.workspace,
      );
      const added = newChatTab(conversation.id, conversation.title);
      change((state) =>
        updateWorkspace(state, selection.workspace.id, (workspace) => ({
          ...workspace,
          tabs: [...workspace.tabs, added],
          activeTabId: added.id,
        })),
      );
    } catch (error) {
      setError(errorMessage(error));
    }
  };
  const closeTab = async (id: string, action: TabCloseAction = "close") => {
    const state = currentSession.current;
    if (!state) return;
    const selection = active(state);
    if (!selection) return;
    const { project, workspace } = selection;
    const tab = workspace.tabs.find((tab) => tab.id === id);
    if (!tab) return;
    const modified = new Set(
      workspace.tabs
        .filter(
          (tab) =>
            filesInTab(tab).some((file) => loadedEditor(file)?.dirty) ||
            (tab.type === "terminal"
              ? layoutPanes(tab.layout).some((p) => pluginHost.isDirty(p.id))
              : pluginHost.isDirty(tab.id)),
        )
        .map((tab) => tab.id),
    );
    const ids = new Set(
      tabsToClose(workspace.tabs, id, action, modified).map((tab) => tab.id),
    );
    const fileIds = new Set(
      workspace.tabs
        .filter((tab) => ids.has(tab.id))
        .flatMap((tab) =>
          tab.type === "terminal"
            ? layoutPanes(tab.layout).map((p) => p.id)
            : [tab.id],
        ),
    );
    if (
      !ids.size ||
      !(await closeGuard.confirm(
        fileIds,
        workspace.tabs.flatMap((tab) =>
          ids.has(tab.id) && tab.type === "terminal"
            ? panes(tab.layout).map((pane) => pane.id)
            : [],
        ),
      ))
    )
      return;
    if (!(await prepareAndroidRemoval(fileIds))) return;
    const current = currentSession.current?.projects
      .find((candidate) => candidate.id === project.id)
      ?.workspaces.find((candidate) => candidate.id === workspace.id);
    if (!current) return;
    closeTerminals(
      current.tabs.flatMap((tab) =>
        ids.has(tab.id) && tab.type === "terminal"
          ? panes(tab.layout).map((pane) => pane.id)
          : [],
      ),
    );
    change((state) =>
      updateWorkspace(state, workspace.id, (workspace) =>
        removeTabs(
          workspace,
          ids,
          project.path,
          info?.platform !== "windows" && tab.type === "terminal"
            ? tab.profileId
            : defaultProfileId,
        ),
      ),
    );
  };
  const openSettings = () => {
    setProjectMenuOpen(false);
    void api("open_settings").catch((error) => setError(errorMessage(error)));
  };
  useEffect(() => {
    if (!session || !info || !preferences.ready || !terminalPreferences.ready)
      return;
    const execute = (action: ActionId) => {
      const selected = currentSession.current && active(currentSession.current);
      const panel = selected && activePanel(selected.tab);
      if (isZoomAction(action)) {
        void changeWindowZoom(action).catch((error) =>
          setError(errorMessage(error)),
        );
        return;
      }
      if (action === "runCommand") {
        document
          .querySelector<HTMLButtonElement>(
            ".terminal-pane.is-active .composer-run:not(:disabled)",
          )
          ?.click();
        return;
      }
      if (action === "commandPicker") {
        setCommandPicker(true);
        return;
      }
      setProjectMenuOpen(false);
      if (action === "openSettings") {
        openSettings();
        return;
      }
      if (action === "toggleWorkspaces") {
        change((state) => toggleSidebar(state, "workspaces"));
        return;
      }
      if (!selected) return;
      const { workspace, tab } = selected;
      const sidebar = workspace.pluginSidebars?.find(
        (panel) => panel.id === pluginHost.context.activePanelId,
      );
      if (sidebar && action === "closeTerminal") {
        void (async () => {
          if (await closeGuard.confirm(new Set([sidebar.id]), []))
            change((state) =>
              updateWorkspace(state, workspace.id, (w) => ({
                ...w,
                pluginSidebars: w.pluginSidebars?.filter(
                  (panel) => panel.id !== sidebar.id,
                ),
              })),
            );
        })();
        return;
      }
      const runtime =
        panel?.type === "terminal" ? runningTerminal(panel.id) : undefined;
      switch (action) {
        case "chatNew":
          void addChat();
          break;
        case "chatFocusInput":
        case "chatStop":
        case "chatHistory":
          if (panel?.type === "chat") chatAction(panel.id, action);
          break;
        case "movePanel":
        case "dockTab": {
          const target =
            action === "movePanel"
              ? tab.type === "terminal"
                ? layoutPanes(tab.layout).find((p) => p.id !== tab.activePaneId)
                    ?.id
                : undefined
              : workspace.tabs.find(
                  (t) => t.type === "terminal" && t.id !== tab.id,
                )?.id;
          if (!target || tab.type === "diff" || tab.type === "commit") {
            setError("There is no compatible docking target.");
            break;
          }
          setDocking({ mode: action, target, side: "right" });
          break;
        }
        case "terminalOverview":
          setTerminalOverview((shown) => !shown);
          break;
        case "saveFile":
          if (panel?.type === "file") {
            const document = loadedEditor(panel);
            void document
              ?.save()
              .catch((error) => document.reportError(errorMessage(error)));
          }
          break;
        case "findFile":
        case "goToLine":
        case "toggleWordWrap":
          if (panel?.type === "file") loadedEditor(panel)?.command(action);
          break;
        case "newTerminal":
        case "splitVertical": {
          if (tab.type !== "terminal") break;
          split(action === "newTerminal" ? "horizontal" : "vertical");
          break;
        }
        case "closeTerminal":
          if (tab.type === "terminal") closePane(tab.activePaneId);
          else closeTab(tab.id);
          break;
        case "searchTerminal":
          runtime?.toggleView("searchOpen");
          break;
        case "commandInput":
          runtime?.toggleView("composerOpen");
          break;
        case "commandBlocks":
          runtime?.toggleView("blocksOpen");
          break;
        case "copyTerminal":
          void runtime?.copy();
          break;
        case "pasteTerminal":
          void runtime?.pasteClipboard();
          break;
        case "changeEnvironment":
          changeEnvironment();
          break;
        case "newTab":
          addTab();
          break;
        case "closeTab":
          closeTab(tab.id);
          break;
        case "toggleExplorer":
          change((state) => toggleSidebar(state, "files"));
          break;
        case "toggleSourceControl":
          change((state) => toggleSidebar(state, "git"));
          break;
        case "nextTab":
        case "previousTab": {
          const index =
            (workspace.tabs.findIndex((candidate) => candidate.id === tab.id) +
              (action === "previousTab" ? -1 : 1) +
              workspace.tabs.length) %
            workspace.tabs.length;
          change((state) =>
            updateWorkspace(state, workspace.id, (workspace) => ({
              ...workspace,
              activeTabId: workspace.tabs[index].id,
            })),
          );
          break;
        }
      }
    };
    executeBuiltin.current = execute;
    const keyboard = (event: KeyboardEvent) => {
      const selected = currentSession.current && active(currentSession.current);
      const panel = selected && activePanel(selected.tab);
      const inEditor =
        event.target instanceof Element &&
        !!event.target.closest(".file-editor");
      const action = actionForEvent(event, bindings);
      if (
        event.defaultPrevented ||
        document.querySelector("dialog[open]") ||
        (event.target instanceof Element &&
          event.target.closest(
            ".tab-context-menu, .editor-status-menu, .sidebar-context-menu, .markdown-preview-menu, .explorer-context-menu",
          )) ||
        (isTextInput(event.target) &&
          !(
            event.target instanceof Element &&
            event.target.closest("[data-android-input]")
          ) &&
          !action?.includes(".") &&
          !inEditor &&
          !(
            panel?.type === "chat" &&
            [
              "chatNew",
              "chatFocusInput",
              "chatStop",
              "chatHistory",
              "closeTerminal",
              "commandPicker",
            ].includes(action ?? "")
          ) &&
          !(
            action === "terminalOverview" &&
            event.target instanceof Element &&
            event.target.closest(".terminal-pane")
          ))
      )
        return;
      if (!action || action === "runCommand" || isZoomAction(action)) return;
      if (action.includes(".")) {
        const entry = pluginHost.catalog.entries.find(
          (e) =>
            e.enabled &&
            e.manifest?.contributes?.commands?.some((c) => c.id === action),
        );
        const command = entry?.manifest?.contributes?.commands?.find(
          (c) => c.id === action,
        );
        if (
          !entry ||
          !command ||
          !commandAvailable(
            command.context,
            pluginHost.context,
            isTextInput(event.target),
          )
        )
          return;
        event.preventDefault();
        event.stopPropagation();
        if (!event.repeat)
          void pluginHost
            .execute(action, isTextInput(event.target))
            .catch((error) => setError(errorMessage(error)));
        return;
      }

      if (
        action === "terminalOverview" &&
        (selected?.tab.type !== "terminal" ||
          !panes(selected.tab.layout).length)
      )
        return;
      const editorAction = [
        "saveFile",
        "findFile",
        "goToLine",
        "toggleWordWrap",
      ].includes(action);
      if (editorAction && panel?.type !== "file") return;
      if (
        ["newTerminal", "splitVertical"].includes(action) &&
        selected?.tab.type !== "terminal"
      )
        return;
      if (
        panel?.type !== "terminal" &&
        [
          "searchTerminal",
          "commandInput",
          "commandBlocks",
          "copyTerminal",
          "pasteTerminal",
          "changeEnvironment",
        ].includes(action)
      )
        return;
      if (
        !selected &&
        action !== "commandPicker" &&
        action !== "openSettings" &&
        action !== "toggleWorkspaces"
      )
        return;
      if (action === "toggleSourceControl" && !git.repositories.length) return;
      if (
        (action === "copyTerminal" || action === "pasteTerminal") &&
        !(
          event.target instanceof Element &&
          event.target.classList.contains("xterm-helper-textarea")
        )
      )
        return;
      // Capture application shortcuts before xterm can forward them to the PTY.
      event.preventDefault();
      event.stopPropagation();
      if (event.repeat) return;
      execute(action);
    };
    window.addEventListener("keydown", keyboard, true);
    return () => window.removeEventListener("keydown", keyboard, true);
  });

  if (!native)
    return (
      <div className="app-shell browser-preview">
        <header className="titlebar">
          <Folder size={16} />
          <span>Lomi</span>
        </header>
        <main className="empty-message">
          <Terminal size={28} />
          <h1>Your development workspace</h1>
          <p>Open the desktop app to use local projects and terminals.</p>
        </main>
      </div>
    );
  if (!session || !info || !preferences.ready || !terminalPreferences.ready)
    return (
      <div className="app-shell">
        <main className="empty-message">
          <Layers size={28} />
          <h1>Lomi</h1>
          <p>{error || "Restoring your workspace…"}</p>
          {error && (
            <button
              className="button"
              onClick={() => {
                bootstrap = undefined;
                window.location.reload();
              }}
            >
              Try again
            </button>
          )}
        </main>
      </div>
    );
  const selectProject = async (
    path: string,
    {
      workspaceId,
      tabId,
      createFile = false,
    }: { workspaceId?: string; tabId?: string; createFile?: boolean } = {},
  ) => {
    setProjectMenuOpen(false);
    try {
      const normalized = await api<string>("validate_directory", { path });
      change((state) => {
        const found = state.projects.find(
          (project) => project.path === normalized,
        );
        if (workspaceId) {
          const workspace = found?.workspaces.find(
            (workspace) => workspace.id === workspaceId,
          );
          if (
            !found ||
            !workspace ||
            (tabId && !workspace.tabs.some((tab) => tab.id === tabId))
          )
            return state;
          return {
            ...state,
            activeProjectId: found.id,
            projects: state.projects.map((project) =>
              project.id === found.id
                ? {
                    ...project,
                    activeWorkspaceId: workspaceId,
                    workspaces: project.workspaces.map((workspace) =>
                      workspace.id === workspaceId && tabId
                        ? { ...workspace, activeTabId: tabId }
                        : workspace,
                    ),
                  }
                : project,
            ),
          };
        }
        const added = found ?? newProject(normalized, defaultProfileId);
        const next = {
          ...state,
          projects: [
            added,
            ...state.projects.filter((project) => project.id !== added.id),
          ],
          activeProjectId: added.id,
        };
        if (!createFile) return next;
        const file = newFileTab(next);
        return updateWorkspace(next, added.activeWorkspaceId, (workspace) => ({
          ...workspace,
          tabs: found ? [...workspace.tabs, file] : [file],
          activeTabId: file.id,
        }));
      });
    } catch (error) {
      setError(errorMessage(error));
    }
  };
  const browse = async (
    intent: "project" | "workspace" | "file" = "project",
  ) => {
    if (folderPickerBusy.current) return;
    folderPickerBusy.current = true;
    setBrowsing(true);
    setProjectMenuOpen(false);
    try {
      const path = await open({
        directory: true,
        multiple: false,
        defaultPath: selected?.project.path ?? info.home,
        title:
          intent === "workspace"
            ? "Choose workspace folder"
            : intent === "file"
              ? "Choose a folder for your new file"
              : "Open Local Folder",
      });
      if (!path) return;
      if (intent === "workspace") {
        const normalized = await api<string>("validate_directory", { path });
        const count =
          currentSession.current?.projects.find(
            (project) => project.path === normalized,
          )?.workspaces.length ?? 0;
        setDialog({
          type: "name",
          title: "New workspace",
          initial: `${basename(normalized)}${count ? ` ${count + 1}` : ""}`,
          submit: (name) =>
            change((state) =>
              addWorkspace(state, normalized, defaultProfileId, name),
            ),
        });
      } else await selectProject(path, { createFile: intent === "file" });
    } catch (error) {
      setError(errorMessage(error));
    } finally {
      folderPickerBusy.current = false;
      setBrowsing(false);
    }
  };
  const projectPicker = (
    <ProjectSwitcher
      projects={session.projects}
      activeProjectId={session.activeProjectId}
      expanded={projectMenuOpen}
      onToggle={() => setProjectMenuOpen(!projectMenuOpen)}
      onClose={closeProjectMenu}
      onSelect={(path) => void selectProject(path)}
      onBrowse={() => void browse()}
    />
  );
  const notice = (error || restoreError) && (
    <div className="notice" role="alert">
      <span>{error || restoreError}</span>
      {restoreError && (
        <button
          className="text-button"
          onClick={() => {
            savingEnabled.current = true;
            setRestoreError("");
            void saveSession(session, true).catch((error) =>
              setError(errorMessage(error)),
            );
          }}
        >
          Save current layout instead
        </button>
      )}
      <IconButton title="Dismiss message" onClick={() => setError("")}>
        <X size={14} />
      </IconButton>
    </div>
  );
  const sidebarOpen = (panel: SidebarPanel) =>
    session.sidebarSides[panel] === "left"
      ? session.sidebar === panel
      : session.rightSidebar === panel;
  const workspaceSide = session.sidebarSides.workspaces;
  const workspaceWidthKey =
    workspaceSide === "left" ? "sidebarWidth" : "rightSidebarWidth";
  const deleteWorkspace = (workspace: Workspace) => {
    setDialog({
      type: "confirm",
      title: "Delete workspace",
      text: `Delete “${workspace.name}” and close all of its tabs? Files in its folder will remain on disk.`,
      submit: async () => {
        const current = () =>
          currentSession.current?.projects
            .flatMap((project) => project.workspaces)
            .find((candidate) => candidate.id === workspace.id);
        const target = current();
        if (
          !target ||
          !(await closeGuard.confirm(
            new Set([
              ...(target.pluginSidebars ?? []).map((panel) => panel.id),
              ...target.tabs.flatMap((tab) =>
                tab.type === "terminal"
                  ? layoutPanes(tab.layout).map((p) => p.id)
                  : [tab.id],
              ),
            ]),
            target.tabs.flatMap((tab) =>
              tab.type === "terminal"
                ? panes(tab.layout).map((pane) => pane.id)
                : [],
            ),
          ))
        )
          return;
        const androidIds = new Set(
          target.tabs.flatMap((tab) =>
            tab.type === "terminal"
              ? layoutPanes(tab.layout).map((pane) => pane.id)
              : [tab.id],
          ),
        );
        if (!(await prepareAndroidRemoval(androidIds))) return;
        const remaining = current();
        if (!remaining) return;
        closeTerminals(
          remaining.tabs.flatMap((tab) =>
            tab.type === "terminal"
              ? panes(tab.layout).map((pane) => pane.id)
              : [],
          ),
        );
        change((state) => removeWorkspace(state, workspace.id));
      },
    });
  };
  const workspacePanel = sidebarOpen("workspaces") && (
    <Sidebar
      key="workspaces"
      side={workspaceSide}
      width={session[workspaceWidthKey]}
      label="Workspaces"
      onResize={(width) =>
        change((state) => ({ ...state, [workspaceWidthKey]: width }))
      }
    >
      <Workspaces
        projects={session.projects}
        activeWorkspaceId={selected?.workspace.id}
        onSelect={(path, id, tabId) =>
          void selectProject(path, { workspaceId: id, tabId })
        }
        onNew={() => void browse("workspace")}
        onRename={(workspace) =>
          setDialog({
            type: "name",
            title: "Rename workspace",
            initial: workspace.name,
            submit: (name) =>
              change((state) =>
                updateWorkspace(state, workspace.id, (workspace) => ({
                  ...workspace,
                  name,
                })),
              ),
          })
        }
        onDelete={deleteWorkspace}
      />
    </Sidebar>
  );
  const workspaceToggle = (
    <div className="status-panel-control" data-side={workspaceSide}>
      <SidebarToggle
        panel="workspaces"
        side={workspaceSide}
        active={sidebarOpen("workspaces")}
        title={shortcutTitle("Toggle workspaces", bindings.toggleWorkspaces)}
        onToggle={() => change((state) => toggleSidebar(state, "workspaces"))}
        onMove={(side) =>
          change((state) => moveSidebar(state, "workspaces", side))
        }
      />
    </div>
  );
  if (!selected)
    return (
      <HostContext.Provider value={hostContext}>
        <SlotProvider>
          <PluginFills />
          <div className="app-shell">
            <header className="titlebar" data-tauri-drag-region>
              {projectPicker}
              <div className="titlebar-space" data-tauri-drag-region />
              <IconButton
                title={shortcutTitle("Settings", bindings.openSettings)}
                onClick={openSettings}
              >
                <Settings size={16} />
              </IconButton>
              <WindowControls onError={setError} />
            </header>
            {notice}
            <div className="work-area" ref={workArea}>
              {workspacePanel}
              <Welcome
                busy={browsing}
                onOpenFolder={() => void browse()}
                onNewFile={() => void browse("file")}
              />
            </div>
            <footer className="statusbar">
              {workspaceToggle}
              <Slot name="statusbar" />
              {cliIntegrations.bar}
              <span className="status-spacer" />
            </footer>
            {dialog && (
              <AppDialog dialog={dialog} onClose={() => setDialog(null)} />
            )}
            {commandPicker && (
              <CommandPicker
                onClose={() => setCommandPicker(false)}
                onBuiltin={(id) => {
                  setCommandPicker(false);
                  executeBuiltin.current(id);
                }}
                onError={setError}
              />
            )}
            {updater.dialog}
            {closeGuard.dialog}
            {gitApproval.dialog}
            {chatApproval.dialog}

            {agentNotifications.dialog}
            <AgentControlStartup />
          </div>
        </SlotProvider>
      </HostContext.Provider>
    );
  const { project, workspace, tab } = selected;
  const panel = activePanel(tab);
  const editorDocument =
    panel?.type === "file" ? loadedEditor(panel) : undefined;
  const profile =
    tab.type === "terminal"
      ? info.profiles.find((profile) => profile.id === tab.profileId)
      : undefined;
  const allPanes = tab.type === "terminal" ? layoutPanes(tab.layout) : [];
  const totalGitChanges = git.repositories.reduce(
    (sum, repository) => sum + repository.changes.length,
    0,
  );
  const sidebarPanels: SidebarPanel[] =
    git.repositories.length ||
    git.errors.length ||
    git.limited ||
    (git.loading && sidebarOpen("git"))
      ? ["files", "git"]
      : ["files"];
  const pluginSidebarViews = workspace.pluginSidebars ?? [];
  const pluginSidebars = pluginSidebarViews
    .filter((panel) => sidebarOpen(panel.viewType as SidebarPanel))
    .map((panel) => {
      const side =
        session.sidebarSides[panel.viewType as SidebarPanel] ?? "left";
      const width = side === "left" ? "sidebarWidth" : "rightSidebarWidth";
      return (
        <Sidebar
          key={panel.id}
          side={side}
          width={session[width]}
          label={panel.title}
          onResize={(value) =>
            change((state) => ({ ...state, [width]: value }))
          }
        >
          <Slot name="sidebar-actions" />
          <PluginPanel
            panel={panel}
            placement="sidebar"
            active
            onFocus={() => {
              setFocusedPlugin(panel.id);
              pluginHost.setContext({
                ...hostContext,
                activePanelId: panel.id,
                viewType: panel.viewType,
              });
            }}
            setState={(value) =>
              change((state) => updatePluginPanel(state, panel.id, value))
            }
            onClose={() =>
              void (async () => {
                if (await closeGuard.confirm(new Set([panel.id]), []))
                  change((state) =>
                    updateWorkspace(state, workspace.id, (w) => ({
                      ...w,
                      pluginSidebars: w.pluginSidebars?.filter(
                        (p) => p.id !== panel.id,
                      ),
                    })),
                  );
              })()
            }
          />
        </Sidebar>
      );
    });
  const pluginSidebarToggles = pluginSidebarViews.map((panel) => (
    <div
      className="status-panel-control"
      data-side={session.sidebarSides[panel.viewType as SidebarPanel] ?? "left"}
      key={panel.id}
    >
      <SidebarToggle
        panel={panel.viewType as SidebarPanel}
        side={session.sidebarSides[panel.viewType as SidebarPanel] ?? "left"}
        active={sidebarOpen(panel.viewType as SidebarPanel)}
        title={panel.title}
        onToggle={() =>
          change((state) =>
            toggleSidebar(state, panel.viewType as SidebarPanel),
          )
        }
        onMove={(side) =>
          change((state) =>
            moveSidebar(state, panel.viewType as SidebarPanel, side),
          )
        }
      />
    </div>
  ));
  const selectTab = (id: string) =>
    change((state) =>
      updateWorkspace(state, workspace.id, (workspace) => ({
        ...workspace,
        activeTabId: id,
      })),
    );
  const modifyTab = (
    transform: (tab: TerminalTab) => TerminalTab,
    animate = false,
  ) =>
    change(
      (state) =>
        updateTab(state, tab.id, (current) =>
          current.type === "terminal" ? transform(current) : current,
        ),
      animate,
    );
  const split = (axis: Split["axis"]) => {
    const state = currentSession.current;
    const selection = state ? active(state) : undefined;
    const container = terminalLayout.current;
    if (!container || selection?.tab.type !== "terminal") return;
    const current = selection.tab;
    const pane = layoutPanes(current.layout).find(
      (pane) => pane.id === current.activePaneId,
    );
    if (!pane) return;
    if (
      !canSplitPane(current.layout, pane.id, axis, {
        width: container.clientWidth,
        height: container.clientHeight,
      })
    ) {
      setPaneNotice(
        "No room for another terminal in this direction. Enlarge the window, hide the sidebar, or resize the panels.",
      );
      return;
    }
    const added = newPane(
      pane.type === "terminal"
        ? (runningTerminal(pane.id)?.getSnapshot().cwd ?? pane.cwd)
        : selection.project.path,
    );
    if (pane.type === "terminal" && pane.profileId !== undefined)
      added.profileId = pane.profileId;
    setPaneNotice("");
    change(
      (state) =>
        updateTab(state, current.id, (tab) => ({
          ...tab,
          layout: splitPane(current.layout, pane.id, axis, added),
          activePaneId: added.id,
        })),
      true,
    );
  };
  const closePane = async (id: string) => {
    const state = currentSession.current;
    if (!state) return;
    const current = active(state)?.tab;
    if (current?.type !== "terminal") return;
    const pane = layoutPanes(current.layout).find((pane) => pane.id === id);
    if (!pane) return;
    if (current.layout.type !== "split") {
      await closeTab(current.id);
      return;
    }
    if (
      !(await closeGuard.confirm(
        new Set([id]),
        pane.type === "terminal" ? [id] : [],
      ))
    )
      return;
    if (!(await prepareAndroidRemoval(new Set([id])))) return;
    if (pane.type === "terminal") closeTerminals([id]);
    change(
      (state) =>
        updateTab(state, current.id, (tab) => {
          if (tab.type !== "terminal") return tab;
          const layout = removePane(tab.layout, id);
          if (!layout) return tab;
          return {
            ...tab,
            layout,
            activePaneId:
              tab.activePaneId === id
                ? layoutPanes(layout)[0].id
                : tab.activePaneId,
          };
        }),
      true,
    );
  };
  const restartPane = (id: string, useProjectDirectory = false) => {
    closeTerminals([id]);
    modifyTab((tab) => {
      let activePaneId = tab.activePaneId;
      const layout = mapLayout(tab.layout, (pane) => {
        if (pane.id !== id) return pane;
        const added = newPane(useProjectDirectory ? project.path : pane.cwd);
        if (pane.type === "terminal" && pane.profileId !== undefined)
          added.profileId = pane.profileId;
        if (activePaneId === id) activePaneId = added.id;
        return added;
      });
      return { ...tab, layout, activePaneId };
    });
  };
  const changeEnvironment = () => {
    if (tab.type !== "terminal") return;
    setDialog({
      type: "environment",
      title: "Change terminal environment",
      profiles: info.profiles,
      selected: tab.profileId,
      submit: (profileId) => {
        closeTerminals(panes(tab.layout).map((pane) => pane.id));
        const layout = mapLayout(tab.layout, () => newPane(project.path));
        modifyTab((tab) => ({
          ...tab,
          profileId,
          layout,
          activePaneId: panel?.type === "file" ? panel.id : panes(layout)[0].id,
        }));
      },
    });
  };
  const openFile = async (
    relative: string,
    root = project.path,
    match?: SearchMatch,
  ) => {
    const request = ++fileOpenRequest.current;
    try {
      const normalized = await api<string>("resolve_editor_file", {
        root,
        relative,
      });
      change((state) => {
        const previous = state.projects
          .flatMap((project) => project.workspaces)
          .find((candidate) => candidate.id === workspace.id)?.activeTabId;
        const opened = openFileTab(state, workspace.id, root, normalized);
        const file = match
          ? opened.projects
              .flatMap((project) => project.workspaces)
              .find((candidate) => candidate.id === workspace.id)
              ?.tabs.flatMap(filesInTab)
              .find(
                (file) => file.root === root && file.relative === normalized,
              )
          : undefined;
        const next = file
          ? updateFilePreviewView(opened, file.id, "editor")
          : opened;
        return request === fileOpenRequest.current || !previous
          ? next
          : updateWorkspace(next, workspace.id, (workspace) => ({
              ...workspace,
              activeTabId: previous,
            }));
      });
      if (match && request === fileOpenRequest.current) {
        const file = currentSession.current?.projects
          .flatMap((project) => project.workspaces)
          .find((candidate) => candidate.id === workspace.id)
          ?.tabs.flatMap(filesInTab)
          .find((file) => file.root === root && file.relative === normalized);
        if (file) {
          const document = await openEditorDocument(file);
          if (
            request === fileOpenRequest.current &&
            active(currentSession.current!)?.workspace.id === workspace.id
          )
            document.selectMatch(match);
        }
      }
    } catch (error) {
      setError(errorMessage(error));
    }
  };
  const applyDiskFileChange = (
    result: FileChange,
    canonicalChange?: FileChange,
  ) => {
    change((state) => {
      const next = applyFileChange(state, result, defaultProfileId);
      relocateEditorFiles(state, next, canonicalChange);
      const kept = new Set(next.projects.map((project) => project.id));
      closeTerminals(
        state.projects
          .filter((project) => !kept.has(project.id))
          .flatMap((project) =>
            project.workspaces.flatMap((workspace) =>
              workspace.tabs.flatMap((tab) =>
                tab.type === "terminal"
                  ? panes(tab.layout).map((pane) => pane.id)
                  : [],
              ),
            ),
          ),
      );
      return next;
    });
    git.refresh();
  };
  const operateFile = async (
    relative: string,
    operation: FileOperation,
  ): Promise<boolean> => {
    if (fileOperationBusy.current)
      throw new Error("Another file operation is in progress.");
    fileOperationBusy.current = true;
    let resume: (() => void) | undefined;
    try {
      const deleting =
        operation.kind === "trash" || operation.kind === "delete";
      const expectedPath = deleting
        ? await api<string>("resolve_project_entry", {
            root: project.path,
            relative,
          })
        : undefined;
      if (expectedPath) {
        const path = expectedPath;
        const ids = new Set(
          fileTabs(currentSession.current!)
            .filter(
              (file) =>
                containsPath(path, absoluteFilePath(file)) ||
                currentSession.current!.projects.some(
                  (project) =>
                    containsPath(path, project.path) &&
                    project.workspaces.some((workspace) =>
                      workspace.tabs
                        .flatMap(filesInTab)
                        .some((view) => view.id === file.id),
                    ),
                ),
            )
            .map((file) => file.id),
        );
        for (const panel of chatTabs(currentSession.current).filter((panel) =>
          currentSession.current!.projects.some(
            (project) =>
              containsPath(path, project.path) &&
              project.workspaces.some((workspace) =>
                workspace.tabs.some(
                  (tab) =>
                    tab.id === panel.id ||
                    (tab.type === "terminal" &&
                      layoutPanes(tab.layout).some((p) => p.id === panel.id)),
                ),
              ),
          ),
        ))
          ids.add(panel.id);
        if (
          !(await closeGuard.confirm(
            ids,
            currentSession
              .current!.projects.filter((project) =>
                containsPath(path, project.path),
              )
              .flatMap((project) =>
                project.workspaces.flatMap((workspace) =>
                  workspace.tabs.flatMap((tab) =>
                    tab.type === "terminal"
                      ? panes(tab.layout).map((pane) => pane.id)
                      : [],
                  ),
                ),
              ),
          ))
        )
          return false;
      }
      resume = await pauseEditorFileOperations();
      const result = await api<FileChange>("file_operation", {
        root: project.path,
        relative,
        operation,
        expectedPath,
      });
      applyDiskFileChange(result);
      return true;
    } finally {
      resume?.();
      fileOperationBusy.current = false;
    }
  };
  const openHistoryCommit = (
    commit: import("./api").GitCommitSummary,
    root: string,
  ) => {
    if (!root) return;
    change((state) =>
      openCommitTab(
        state,
        workspace.id,
        root,
        commit.id,
        `${commit.shortId} · ${commit.subject}`,
      ),
    );
  };
  const diff = (path: string, staged: boolean, root: string) => {
    setDiffRevision((value) => value + 1);
    change((state) => openDiffTab(state, workspace.id, root, path, staged));
  };

  return (
    <HostContext.Provider value={hostContext}>
      <SlotProvider>
        <PluginFills />
        <div className="app-shell">
          <header className="titlebar" data-tauri-drag-region>
            {projectPicker}
            <TabBar
              key={workspace.id}
              tabs={workspace.tabs}
              modified={
                new Set(
                  workspace.tabs
                    .filter(
                      (tab) =>
                        filesInTab(tab).some(
                          (file) => loadedEditor(file)?.dirty,
                        ) ||
                        (tab.type === "terminal"
                          ? layoutPanes(tab.layout).some((p) =>
                              pluginHost.isDirty(p.id),
                            )
                          : pluginHost.isDirty(tab.id)),
                    )
                    .map((tab) => tab.id),
                )
              }
              activeTabId={tab.id}
              newTabTitle={shortcutTitle("New tab", bindings.newTab)}
              onNew={() => addTab()}
              onNewChat={() => void addChat()}
              onNewAndroid={() =>
                change((state) => {
                  const added = newAndroidTab();
                  return updateWorkspace(state, workspace.id, (current) => ({
                    ...current,
                    tabs: [...current.tabs, added],
                    activeTabId: added.id,
                  }));
                })
              }
              onNewBrowser={() =>
                change((state) => {
                  const added = newBrowserTab();
                  return updateWorkspace(state, workspace.id, (current) => ({
                    ...current,
                    tabs: [...current.tabs, added],
                    activeTabId: added.id,
                  }));
                })
              }
              onNewFile={() =>
                change((state) => {
                  const file = newFileTab(state);
                  return updateWorkspace(state, workspace.id, (current) => ({
                    ...current,
                    tabs: [...current.tabs, file],
                    activeTabId: file.id,
                  }));
                })
              }
              onSelect={selectTab}
              onClose={closeTab}
              mergeContainer={terminalLayout}
              onMove={(id, beforeId) =>
                change((state) =>
                  updateWorkspace(state, workspace.id, (current) =>
                    moveTab(current, id, beforeId),
                  ),
                )
              }
              onMerge={(id, targetId, side) => {
                const container = terminalLayout.current;
                if (!container) return;
                change(
                  (state) =>
                    updateWorkspace(state, workspace.id, (current) =>
                      current.activeTabId === targetId
                        ? mergeTabs(current, id, targetId, side, {
                            width: container.clientWidth,
                            height: container.clientHeight,
                          })
                        : current,
                    ),
                  true,
                );
              }}
              onRename={(candidate) =>
                setDialog({
                  type: "name",
                  title: "Rename tab",
                  initial: tabTitle(candidate),
                  submit: (title) =>
                    change((state) =>
                      updateTab(state, candidate.id, (tab) => ({
                        ...tab,
                        customTitle: title,
                      })),
                    ),
                })
              }
            />
            <div className="titlebar-space" data-tauri-drag-region />
            <IconButton
              title={shortcutTitle("Settings", bindings.openSettings)}
              onClick={openSettings}
            >
              <Settings size={16} />
            </IconButton>
            <WindowControls onError={setError} />
          </header>
          {notice}
          <div className="work-area" ref={workArea}>
            {workspacePanel}
            {pluginSidebars}
            {sidebarPanels.filter(sidebarOpen).map((panel) => {
              const side = session.sidebarSides[panel];
              const widthKey =
                side === "left" ? "sidebarWidth" : "rightSidebarWidth";
              return (
                <Sidebar
                  key={panel}
                  side={side}
                  width={session[widthKey]}
                  label={panel === "files" ? "Explorer" : "Source Control"}
                  onResize={(width) =>
                    change((state) => ({ ...state, [widthKey]: width }))
                  }
                >
                  {panel === "files" ? (
                    <Explorer
                      key={project.path}
                      root={project.path}
                      onTerminal={addTab}
                      onOpenFile={(relative, match) =>
                        void openFile(relative, project.path, match)
                      }
                      repositories={git.repositories}
                      onRefreshGit={git.refresh}
                      onOpenCommit={openHistoryCommit}
                      onOperation={operateFile}
                      onError={setError}
                    />
                  ) : (
                    <SourceControl
                      key={project.path}
                      projectRoot={project.path}
                      repositories={git.repositories}
                      state={sourceControlState}
                      errors={git.errors}
                      limited={git.limited}
                      loading={git.loading}
                      onRefresh={git.refresh}
                      onPull={async (root, rebase) => {
                        if (fileOperationBusy.current)
                          throw new Error(
                            "Wait for the current file operation to finish.",
                          );
                        fileOperationBusy.current = true;
                        let resume: (() => void) | undefined;
                        try {
                          resume = await pauseEditorFileOperations();
                          await api("git_pull", { root, rebase });
                        } finally {
                          resume?.();
                          fileOperationBusy.current = false;
                          setDiffRevision((value) => value + 1);
                        }
                      }}
                      onDiff={(root, path, staged) => diff(path, staged, root)}
                      onOpenCommit={(root, commit) =>
                        openHistoryCommit(commit, root)
                      }
                      onOpenFile={(root, path) => void openFile(path, root)}
                      onDiscard={async (root, change) => {
                        if (fileOperationBusy.current)
                          throw new Error(
                            "Wait for the current file operation to finish.",
                          );
                        fileOperationBusy.current = true;
                        let resume: (() => void) | undefined;
                        try {
                          resume = await pauseEditorFileOperations();
                          const path = `${root.replace(/[\\/]$/, "")}/${change.path}`;
                          if (
                            fileTabs(currentSession.current!).some((file) => {
                              const document = loadedEditor(file);
                              return (
                                document?.dirty &&
                                containsPath(path, document.path)
                              );
                            })
                          )
                            throw new Error(
                              "Save or discard unsaved editor changes before discarding Git changes.",
                            );
                          await api("git_discard", { root, change });
                        } finally {
                          resume?.();
                          fileOperationBusy.current = false;
                        }
                      }}
                      onError={setError}
                    />
                  )}
                </Sidebar>
              );
            })}
            <main
              className="terminal-stage"
              id={`panel-${tab.id}`}
              role="tabpanel"
              aria-labelledby={`tab-${tab.id}`}
            >
              {tab.type === "diff" ? (
                <builtinViews.diff
                  key={tab.id}
                  tab={tab}
                  requestRevision={diffRevision}
                  onOpenFile={() => void openFile(tab.relative, tab.root)}
                />
              ) : tab.type === "commit" ? (
                <builtinViews.commit
                  key={tab.id}
                  root={tab.root}
                  commitId={tab.commit}
                  agentPanelId={tab.id}
                  agentRestricted={tab.agentGit === true}
                  onOpenFile={(path) => void openFile(path, tab.root)}
                  onOpenCommit={(commit) => openHistoryCommit(commit, tab.root)}
                  onError={setError}
                />
              ) : tab.type === "android" ? (
                <Suspense fallback={<div role="status">Loading Android…</div>}>
                  <builtinViews.android
                    key={tab.id}
                    tab={tab}
                    onClose={() => void closeTab(tab.id)}
                  />
                </Suspense>
              ) : tab.type === "chat" ? (
                <Suspense
                  fallback={<div role="status">Loading conversation…</div>}
                >
                  <builtinViews.chat
                    tab={tab}
                    onFocus={() => {}}
                    onClose={() => void closeTab(tab.id)}
                  />
                </Suspense>
              ) : tab.type === "browser" ? (
                <builtinViews.browser
                  key={tab.id}
                  tab={tab}
                  onClose={() => void closeTab(tab.id)}
                />
              ) : tab.type === "plugin" ? (
                <PluginPanel
                  panel={tab}
                  active
                  setState={(value) =>
                    change((state) => updatePluginPanel(state, tab.id, value))
                  }
                  onFocus={() => {}}
                  onClose={() => void closeTab(tab.id)}
                />
              ) : tab.type === "file" ? (
                <Suspense
                  fallback={
                    <div className="empty-message" role="status">
                      Loading editor…
                    </div>
                  }
                >
                  <builtinViews.file
                    key={tab.id}
                    tab={tab}
                    onOpenFile={(root, relative) =>
                      void openFile(relative, root)
                    }
                    onPreviewView={(view) =>
                      change((state) =>
                        updateFilePreviewView(state, tab.id, view),
                      )
                    }
                    onPosition={(position) =>
                      change((state) =>
                        updateFilePosition(state, tab.id, position),
                      )
                    }
                  />
                </Suspense>
              ) : (
                <div className="terminal-layout" ref={terminalLayout}>
                  <SplitView
                    key={tab.id}
                    layout={tab.layout}
                    profile={profile}
                    profiles={info.profiles}
                    activePaneId={tab.activePaneId}
                    overview={terminalOverview}
                    revealTitles={revealTerminalTitles}
                    onFocus={(id) => {
                      if (id !== tab.activePaneId)
                        modifyTab((tab) => ({ ...tab, activePaneId: id }));
                    }}
                    onRestart={restartPane}
                    onMove={(id, targetId, side) => {
                      const container = terminalLayout.current;
                      if (!container) return;
                      modifyTab((tab) => {
                        const layout = movePane(
                          tab.layout,
                          id,
                          targetId,
                          side,
                          {
                            width: container.clientWidth,
                            height: container.clientHeight,
                          },
                        );
                        return layout === tab.layout
                          ? tab
                          : { ...tab, layout, activePaneId: id };
                      }, true);
                    }}
                    onClosePane={closePane}
                    onPluginState={(id, value) =>
                      change((state) => updatePluginPanel(state, id, value))
                    }
                    onFilePosition={(id, position) =>
                      change((state) => updateFilePosition(state, id, position))
                    }
                    onPreviewView={(id, view) =>
                      change((state) => updateFilePreviewView(state, id, view))
                    }
                    onOpenFile={(root, relative) =>
                      void openFile(relative, root)
                    }
                    onKeepActivePane={async () => {
                      const kept = allPanes.find(
                        (pane) => pane.id === tab.activePaneId,
                      )!;
                      const removed = allPanes.filter(
                        (pane) => pane.id !== kept.id,
                      );
                      if (
                        !(await closeGuard.confirm(
                          new Set(
                            removed
                              .filter(
                                (pane) =>
                                  pane.type === "file" ||
                                  pane.type === "plugin" ||
                                  pane.type === "chat",
                              )
                              .map((pane) => pane.id),
                          ),
                          removed
                            .filter((pane) => pane.type === "terminal")
                            .map((pane) => pane.id),
                        ))
                      )
                        return;
                      if (
                        !(await prepareAndroidRemoval(
                          new Set(removed.map((pane) => pane.id)),
                        ))
                      )
                        return;
                      closeTerminals(
                        removed
                          .filter((pane) => pane.type === "terminal")
                          .map((pane) => pane.id),
                      );
                      const removedIds = new Set(
                        removed.map((pane) => pane.id),
                      );
                      modifyTab((tab) => {
                        let layout = tab.layout;
                        for (const id of removedIds) {
                          const next = removePane(layout, id);
                          if (next) layout = next;
                        }
                        return { ...tab, layout };
                      });
                      setPaneNotice("");
                    }}
                    onResize={(id, ratio) =>
                      modifyTab((tab) => ({
                        ...tab,
                        layout: resizeSplit(tab.layout, id, ratio),
                      }))
                    }
                  />
                </div>
              )}
              {paneNotice && (
                <div className="pane-limit-notice" role="status">
                  <span>{paneNotice}</span>
                  <IconButton
                    title="Dismiss panel limit"
                    onClick={() => setPaneNotice("")}
                  >
                    <X size={14} />
                  </IconButton>
                </div>
              )}
            </main>
          </div>
          <footer className="statusbar">
            {workspaceToggle}
            {sidebarPanels.map((panel) => (
              <div
                key={panel}
                className="status-panel-control"
                data-side={session.sidebarSides[panel]}
              >
                <SidebarToggle
                  panel={panel}
                  side={session.sidebarSides[panel]}
                  active={sidebarOpen(panel)}
                  title={
                    panel === "files"
                      ? shortcutTitle(
                          "Toggle file explorer",
                          bindings.toggleExplorer,
                        )
                      : shortcutTitle(
                          "Toggle source control",
                          bindings.toggleSourceControl,
                        )
                  }
                  onToggle={() =>
                    change((state) => toggleSidebar(state, panel))
                  }
                  onMove={(side) =>
                    change((state) => moveSidebar(state, panel, side))
                  }
                />
                {panel === "git" && git.repositories.length > 0 && (
                  <>
                    <span className="status-divider" />
                    <button
                      className="branch-status"
                      title="Show source control"
                      onClick={() =>
                        change((state) => showSidebar(state, "git"))
                      }
                    >
                      <GitBranch size={12} />
                      {git.repositories.length === 1
                        ? git.repositories[0].branch
                        : `${git.repositories.length} repositories`}
                      {totalGitChanges > 0 && (
                        <span className="count-badge">{totalGitChanges}</span>
                      )}
                    </button>
                  </>
                )}
              </div>
            ))}
            <div
              className="status-panel-control"
              data-side={session.terminalOverviewSide}
            >
              <SidebarToggle
                panel="terminalOverview"
                side={session.terminalOverviewSide}
                title={shortcutTitle(
                  "Toggle terminal overview",
                  bindings.terminalOverview,
                )}
                active={tab.type === "terminal" && terminalOverview}
                disabled={tab.type !== "terminal" || !panes(tab.layout).length}
                onToggle={() => setTerminalOverview((shown) => !shown)}
                onMove={(side) =>
                  change((state) => ({ ...state, terminalOverviewSide: side }))
                }
              />
            </div>
            {pluginSidebarToggles}
            <Slot name="statusbar" />
            {cliIntegrations.bar}
            <span className="status-spacer" />
            {editorDocument && (
              <FileEditorStatus
                key={editorDocument.path}
                document={editorDocument}
              />
            )}
          </footer>
          {docking && (
            <Modal
              title={
                docking.mode === "movePanel"
                  ? "Move active panel"
                  : "Dock current tab"
              }
              onClose={() => setDocking(null)}
            >
              <form
                className="dialog-form"
                onSubmit={(event) => {
                  event.preventDefault();
                  const element =
                    terminalLayout.current ??
                    document.querySelector<HTMLElement>(".terminal-stage");
                  if (!element) return;
                  const size = {
                    width: element.clientWidth,
                    height: element.clientHeight,
                  };
                  const next =
                    docking.mode === "dockTab"
                      ? mergeTabs(
                          workspace,
                          tab.id,
                          docking.target,
                          docking.side,
                          size,
                        )
                      : tab.type === "terminal"
                        ? (() => {
                            const layout = movePane(
                              tab.layout,
                              tab.activePaneId,
                              docking.target,
                              docking.side,
                              size,
                            );
                            return layout === tab.layout
                              ? workspace
                              : {
                                  ...workspace,
                                  tabs: workspace.tabs.map((t) =>
                                    t.id === tab.id ? { ...tab, layout } : t,
                                  ),
                                };
                          })()
                        : workspace;
                  if (next === workspace) {
                    setError(
                      "The requested docking operation does not fit this layout.",
                    );
                    return;
                  }
                  change(
                    (state) => updateWorkspace(state, workspace.id, () => next),
                    true,
                  );
                  setDocking(null);
                }}
              >
                <label>
                  Target
                  <Select
                    aria-label="Docking target"
                    value={docking.target}
                    onChange={(target) => setDocking({ ...docking, target })}
                    options={
                      docking.mode === "dockTab"
                        ? workspace.tabs
                            .filter(
                              (t) => t.type === "terminal" && t.id !== tab.id,
                            )
                            .map((t) => ({ value: t.id, label: tabTitle(t) }))
                        : tab.type === "terminal"
                          ? layoutPanes(tab.layout)
                              .filter((p) => p.id !== tab.activePaneId)
                              .map((p, index) => ({
                                value: p.id,
                                label: `${p.type} ${index + 1}${"title" in p ? ` · ${p.title}` : ""}`,
                              }))
                          : []
                    }
                  />
                </label>
                <label>
                  Position
                  <Select
                    aria-label="Docking position"
                    value={docking.side}
                    onChange={(side) =>
                      setDocking({ ...docking, side: side as TabDropSide })
                    }
                    options={["left", "right", "top", "bottom"].map(
                      (value) => ({ value, label: value }),
                    )}
                  />
                </label>
                <div className="dialog-actions">
                  <button
                    type="button"
                    className="button"
                    onClick={() => setDocking(null)}
                  >
                    Cancel
                  </button>
                  <button className="button button-primary" type="submit">
                    Dock
                  </button>
                </div>
              </form>
            </Modal>
          )}
          {commandPicker && (
            <CommandPicker
              onClose={() => setCommandPicker(false)}
              onBuiltin={(id) => {
                setCommandPicker(false);
                executeBuiltin.current(id);
              }}
              onError={setError}
            />
          )}
          {dialog && (
            <AppDialog dialog={dialog} onClose={() => setDialog(null)} />
          )}
          {updater.dialog}
          {closeGuard.dialog}
          {gitApproval.dialog}
          {chatApproval.dialog}
          {stoppingForClose && (
            <Modal
              protectTheme
              title="Preparing to close"
              className="close-progress-dialog"
              descriptionId={closeDescriptionId}
              initialFocus={cancelCloseButton}
              onClose={requestCancelClose}
            >
              <div className="close-progress-body" role="status">
                <span className="close-progress-spinner" aria-hidden="true" />
                <div id={closeDescriptionId}>
                  <p>
                    {stoppingForClose === "cancelling"
                      ? "Cancelling close. Waiting for current operations to finish."
                      : "Finishing pending operations before closing."}
                  </p>
                  <p className="close-progress-hint">
                    {stoppingForClose === "cancelling"
                      ? "Stopped phones will stay stopped."
                      : "Android phones will stop safely and keep their data."}
                  </p>
                </div>
              </div>
              <div className="dialog-actions close-progress-actions">
                <button
                  ref={cancelCloseButton}
                  type="button"
                  className="button"
                  disabled={stoppingForClose === "cancelling"}
                  onClick={requestCancelClose}
                >
                  {stoppingForClose === "cancelling"
                    ? "Cancelling…"
                    : "Cancel closing"}
                </button>
              </div>
            </Modal>
          )}

          {agentNotifications.dialog}
          <AgentControlStartup />
        </div>
      </SlotProvider>
    </HostContext.Provider>
  );
}

function AppDialog({
  dialog,
  onClose,
}: {
  dialog: Dialog;
  onClose: () => void;
}) {
  const nameInput = useRef<HTMLInputElement>(null);
  const confirmButton = useRef<HTMLButtonElement>(null);
  const [value, setValue] = useState(
    dialog.type === "name"
      ? dialog.initial
      : dialog.type === "environment"
        ? dialog.selected
        : "",
  );
  return (
    <Modal
      protectTheme={dialog.type === "confirm" || dialog.type === "environment"}
      title={dialog.title}
      tone={
        dialog.type === "confirm" || dialog.type === "environment"
          ? "warning"
          : undefined
      }
      onClose={onClose}
      wide={dialog.type === "preview"}
      initialFocus={
        dialog.type === "name"
          ? nameInput
          : dialog.type === "confirm"
            ? confirmButton
            : undefined
      }
    >
      {dialog.type === "preview" ? (
        <pre className="file-preview">{dialog.content || "(empty file)"}</pre>
      ) : (
        <form
          className="dialog-form"
          onSubmit={(event) => {
            event.preventDefault();
            if (dialog.type === "name") {
              if (!value.trim()) return;
              dialog.submit(value.trim());
            } else if (dialog.type === "environment") {
              if (
                !dialog.profiles.some((profile) => profile.id === value) ||
                value === dialog.selected
              )
                return;
              dialog.submit(value);
            } else dialog.submit();
            onClose();
          }}
        >
          {dialog.type === "name" ? (
            <label>
              Name
              <input
                ref={nameInput}
                value={value}
                onChange={(event) => setValue(event.target.value)}
                maxLength={120}
              />
            </label>
          ) : dialog.type === "environment" ? (
            <>
              <label>
                Terminal environment
                <Select
                  autoFocus
                  aria-label="Terminal environment"
                  value={value}
                  onChange={setValue}
                  options={[
                    ...(!dialog.profiles.some(
                      (profile) => profile.id === dialog.selected,
                    )
                      ? [
                          {
                            value: dialog.selected,
                            label: "Unavailable environment",
                            disabled: true,
                          },
                        ]
                      : []),
                    ...dialog.profiles.map((profile) => ({
                      value: profile.id,
                      label: profile.distro
                        ? profile.name
                        : `Local · ${profile.name}`,
                    })),
                  ]}
                />
              </label>
              <p>
                Changing the environment restarts all terminals in this tab in
                the project folder.
              </p>
            </>
          ) : (
            <p>{dialog.text}</p>
          )}
          <div className="dialog-actions">
            <button type="button" className="button" onClick={onClose}>
              Cancel
            </button>
            <button
              ref={confirmButton}
              className={`button button-primary${dialog.type === "confirm" ? " button-danger" : ""}`}
              disabled={
                dialog.type === "name"
                  ? !value.trim()
                  : dialog.type === "environment" &&
                    (!value || value === dialog.selected)
              }
            >
              {dialog.type === "name"
                ? "Save"
                : dialog.type === "environment"
                  ? "Restart terminals"
                  : "Continue"}
            </button>
          </div>
        </form>
      )}
    </Modal>
  );
}
