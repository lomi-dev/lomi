import type { CliAgent } from "./cli-agents";
import { Channel } from "@tauri-apps/api/core";
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Terminal } from "@xterm/xterm";
import {
  applyPatch,
  applySnapshot,
  setInputEnabled,
} from "@lomi/terminal-renderer-experimental";
import {
  TerminalStreamAssembler,
  TerminalStreamResyncError,
} from "@lomi/terminal-renderer-experimental/stream";
import type { AssembledTerminalValue } from "@lomi/terminal-renderer-experimental/stream";
import type { IDisposable, IMarker } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { WebLinksAddon } from "@xterm/addon-web-links";
import type { WebglAddon } from "@xterm/addon-webgl";
import { api, errorMessage, macOS, windows } from "./api";
import { newId } from "./model";
import type { Pane, ShellProfile } from "./model";
import { inputChunks } from "./terminal-utils";
import { isAgentTerminalState } from "./remote-workspace-domain";
import {
  parseOsc9Progress,
  parseTerminalProgramStatus,
  terminalProgramStatusActivity,
} from "./cli-activity";
import type { CliActivity, TerminalProgramStatus } from "./cli-activity";
import { classifyTerminalInput } from "./terminal-input-source";
import {
  createAgentNotificationGate,
  emitAgentNotification,
  parseAgentSignal,
} from "./agent-notifications";
import type { AgentSignal } from "./agent-notifications";
import {
  loadTerminalFonts,
  terminalAppearance,
  terminalSearchColors,
  themeAppliedEvent,
} from "./theme/runtime";
import {
  closeSharedHostTerminal as closeHostTerminal,
  detachSharedHostView,
  finishSharedHostCreate,
  importSharedWorkspace,
  registerSharedShellProfile,
  saveSharedHostTerminalAlias,
  sharedHostCreateIntent,
  sharedHostOperationId,
  sharedHostRpc,
  sharedHostTerminalAliases,
  sharedWorkspaceState,
} from "./shared-host";
import type {
  SharedHostEvent,
  SharedHostLease,
  SharedHostTerminalBinding,
  SharedHostTerminalContext,
} from "./shared-host";

export interface CommandBlock {
  id: string;
  command: string;
  started: number;
  finished?: number;
  exitCode?: number;
  marker?: IMarker;
}
interface Snapshot {
  status: "starting" | "running" | "exited" | "error";
  title: string;
  titleBusy: boolean;
  programStatus: TerminalProgramStatus | null;
  accountLabel: string | null;
  agentSignal: AgentSignal | null;
  foregroundProgram: string;
  agentControlled: boolean;
  observedWorkingDirectory: string | null;
  error: string | null;
  cwd: string;
  renderer: "WebGL" | "DOM";
  blocks: CommandBlock[];
  searchResult: string;
  searchOpen: boolean;
  composerOpen: boolean;
  blocksOpen: boolean;
}
export interface TitleProcess {
  cli: CliAgent;
  pid: number;
}

export interface TerminalAgentState {
  cli: CliAgent;
  sessionId: string;
  process: TitleProcess | null;
  activity: CliActivity | null;
}

export interface TerminalContext {
  cwd: string | null;
  foregroundProgram: string | null;
  titleCli: TitleProcess | null;
  agentControlled?: boolean;
}
type SharedHostTab =
  import("./shared-host").SharedHostWorkspaceState["tabs"][number];
type SharedHostLayout = SharedHostTab["layout"];
type SharedHostPane = Extract<SharedHostLayout, { kind: "pane" }>["pane"];
function locateHostPane(
  state: import("./shared-host").SharedHostWorkspaceState,
  paneId: string,
): { tabId: string; pane: SharedHostPane } | undefined {
  const find = (
    layout: SharedHostLayout,
    tabId: string,
  ): { tabId: string; pane: SharedHostPane } | undefined => {
    if (layout.kind === "pane")
      return layout.pane.paneId === paneId
        ? { tabId, pane: layout.pane }
        : undefined;
    return find(layout.first, tabId) ?? find(layout.second, tabId);
  };
  for (const tab of state.tabs) {
    const found = find(tab.layout, tab.tabId);
    if (found) return found;
  }
  return undefined;
}
type DirectoryListener = (id: string, cwd: string) => void;
let directoryListener: DirectoryListener = () => {};
let reportError: (message: string) => void = () => {};
let webglModule: Promise<typeof import("@xterm/addon-webgl")> | undefined;
let terminalRenderer: "webgl" | "dom" = "webgl";

export function configureTerminalRenderer(renderer: "webgl" | "dom") {
  terminalRenderer = renderer;
}

export function configureTerminals(
  onDirectory: DirectoryListener,
  onError: (message: string) => void,
) {
  directoryListener = onDirectory;
  reportError = onError;
}

interface AgentTerminalStart {
  sessionId: string;
  operationId: string;
  nonce: string;
}
const agentStarts = new Map<string, AgentTerminalStart>();
export function stageAgentTerminal(panelId: string, start: AgentTerminalStart) {
  if (agentStarts.has(panelId) || runtimes.has(panelId))
    throw new Error("Terminal already exists.");
  agentStarts.set(panelId, start);
  terminalLifecycleChanged();
}
export const clearAgentTerminal = (panelId: string) =>
  (() => {
    const cleared = agentStarts.delete(panelId);
    if (cleared) terminalLifecycleChanged();
    return cleared;
  })();
export async function waitForAgentTerminal(panelId: string, sessionId: string) {
  const deadline = performance.now() + 20000;
  while (performance.now() < deadline) {
    const runtime = runtimes.get(panelId);
    if (runtime) {
      if (runtime.sessionId !== sessionId)
        throw new Error("Terminal generation changed.");
      const status = runtime.getSnapshot().status;
      if (status === "running") return;
      if (status === "error" || status === "exited")
        throw new Error("Terminal could not start.");
    }
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error("Terminal panel could not render.");
}

export class TerminalRuntime {
  sessionId: string;
  readonly agentStarted: boolean;
  private readonly agentStart?: AgentTerminalStart;
  readonly terminal: Terminal;
  readonly fitAddon = new FitAddon();
  readonly searchAddon = new SearchAddon();
  readonly host = document.createElement("div");
  private webgl?: WebglAddon;
  private observer?: ResizeObserver;
  private frame = 0;
  private firstRender?: IDisposable;
  private receivedOutput = false;
  private revealed = false;
  private revealAnimation?: Animation;
  private rendererReady = false;
  private measuredFont?: string;
  private opened = false;
  private attached = false;
  private rendererGeneration = 0;
  private rendererPromise?: Promise<void>;
  private disposed = false;
  private nativeCloseComplete = false;
  private startPromise?: Promise<void>;
  private input = Promise.resolve();
  private pasteOutput?: string[];
  private pendingResize?: { cols: number; rows: number };
  private resizing = false;
  private unacknowledged = 0;
  private controlStreamSequence = 0n;
  private controlParsedSequence = 0n;
  private ackTimer: ReturnType<typeof setTimeout> | undefined;
  private listeners = new Set<() => void>();
  private snapshot: Snapshot;
  private promptEnd?: { marker: IMarker; column: number };
  private atPrompt = false;
  private activeBlock?: CommandBlock;
  private nextCommand?: string;
  private eof = false;
  private exitCode: number | null | undefined;
  private activityOverride: CliActivity | null | undefined;
  private activityOverrideSource:
    "agent-signal" | "progress" | "notification" | "title" | undefined;
  private hostBinding?: SharedHostTerminalBinding;
  private hostTerminal?: SharedHostTerminalContext;
  private hostSubscriptionId?: string;
  private hostAssembler?: TerminalStreamAssembler;
  private hostEvents?: Channel<SharedHostEvent>;
  private hostLease?: SharedHostLease["lease"];
  private hostInputId = 0n;
  private hostInputSeq = 0n;
  private hostResizeSeq = 0n;
  private hostRenderedThroughSeq = "0";
  private hostFocused = false;
  private hostSnapshotReady = false;
  private hostPendingFrames: string[] = [];
  private hostPendingFrameBytes = 0;
  private hostLeaseTimer?: ReturnType<typeof setTimeout>;
  private hostResizePending?: { cols: number; rows: number };
  private hostResizing = false;
  private hostAckChain: Promise<void> = Promise.resolve();

  constructor(
    readonly paneId: string,
    readonly profile: ShellProfile,
    cwd: string,
    private readonly cliLaunch?: CliAgent,
    private readonly accountId?: string,
    hostBinding?: SharedHostTerminalBinding,
  ) {
    this.agentStart = agentStarts.get(paneId);
    agentStarts.delete(paneId);
    this.agentStarted = Boolean(this.agentStart);
    this.hostBinding =
      this.agentStarted || cliLaunch || accountId ? undefined : hostBinding;
    this.sessionId = this.agentStart?.sessionId ?? newId();
    this.snapshot = {
      status: "starting",
      title: "",
      titleBusy: false,
      programStatus: null,
      accountLabel: null,
      agentSignal: null,
      foregroundProgram: "",
      agentControlled: false,
      observedWorkingDirectory: null,
      error: null,
      cwd,
      renderer: "DOM",
      blocks: [],
      searchResult: "",
      searchOpen: false,
      composerOpen: false,
      blocksOpen: false,
    };
    this.terminal = new Terminal({
      allowProposedApi: true,
      allowTransparency: true,
      windowOptions: { pushTitle: true, popTitle: true },
      ...terminalAppearance(),
    });
    this.terminal.onTitleChange((value) => {
      if (this.atPrompt) return;
      let title = value
        .slice(0, 1024)
        .replace(/[\x00-\x1f\x7f-\x9f]/g, "")
        .trim();
      // Normalize leading dot-spinner frames without interpreting the title text.
      const spinner = /^[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏](?:\s+|$)/u.exec(title);
      const titleBusy = spinner !== null;
      if (spinner) title = title.slice(spinner[0].length);
      let clearSignal = false;
      if (
        titleBusy &&
        !this.snapshot.titleBusy &&
        this.activityOverrideSource !== "agent-signal"
      ) {
        this.activityOverride = "working";
        this.activityOverrideSource = "title";
        clearSignal = this.snapshot.agentSignal !== null;
      } else if (!titleBusy && this.activityOverrideSource === "title") {
        this.activityOverride = undefined;
        this.activityOverrideSource = undefined;
      }
      if (
        title !== this.snapshot.title ||
        titleBusy !== this.snapshot.titleBusy ||
        clearSignal
      )
        this.update({
          title,
          titleBusy,
          ...(clearSignal ? { agentSignal: null } : {}),
        });
    });
    window.addEventListener(themeAppliedEvent, this.applyTheme);
    this.host.className = "terminal-host";
    this.host.addEventListener("paste", this.onPaste, true);
    this.terminal.loadAddon(this.fitAddon);
    this.terminal.loadAddon(this.searchAddon);
    this.terminal.loadAddon(
      new WebLinksAddon((event, uri) => {
        if (!(event.ctrlKey || event.metaKey)) return;
        if (/^https?:\/\//i.test(uri))
          void openUrl(uri).catch((error) => reportError(errorMessage(error)));
      }),
    );
    classifyTerminalInput(this.terminal, (data, human) =>
      this.send(data, human),
    );
    this.terminal.attachCustomKeyEventHandler((event) => {
      if (
        event.type !== "keydown" ||
        event.defaultPrevented ||
        event.isComposing ||
        event.keyCode === 229
      )
        return true;
      if (
        !event.altKey &&
        ((macOS &&
          event.metaKey &&
          !event.ctrlKey &&
          !event.shiftKey &&
          event.code === "KeyV") ||
          (windows &&
            !event.metaKey &&
            ((event.ctrlKey && !event.shiftKey && event.code === "KeyV") ||
              (event.shiftKey && !event.ctrlKey && event.code === "Insert"))))
      ) {
        // Read native image formats even when the webview only exposes clipboard text.
        event.preventDefault();
        if (!event.repeat) void this.pasteClipboard();
        return false;
      }
      if (
        event.key !== "Enter" ||
        !event.shiftKey ||
        event.ctrlKey ||
        event.altKey ||
        event.metaKey
      )
        return true;
      // xterm encodes Shift+Enter as CR; CSI u preserves Shift for CLI input.
      event.preventDefault();
      this.terminal.input("\x1b[13;2u");
      return false;
    });
    this.terminal.parser.registerOscHandler(7, (value) => {
      // Shared Host terminals get advisory CWD from the Host stream metadata.
      // Do not let xterm's renderer parser become a second authority source.
      if (this.hostBinding) return true;
      try {
        const url = new URL(value);
        if (url.protocol !== "file:") return false;
        let directory = decodeURIComponent(url.pathname);
        if (!profile.distro && /^\/[a-z]:/i.test(directory))
          directory = directory.slice(1);
        if (
          directory &&
          !/[\x00-\x1f]/.test(directory) &&
          directory !== this.snapshot.cwd
        ) {
          this.update({ cwd: directory });
          directoryListener(this.paneId, directory);
        }
      } catch {
        /* Malformed shell directory reports must not interrupt terminal parsing. */
      }
      return true;
    });
    this.terminal.parser.registerOscHandler(133, (value) => {
      const [event, status] = value.split(";");
      if (event === "A") this.finishBlock();
      if (event === "B") {
        this.promptEnd?.marker.dispose();
        this.promptEnd = {
          marker: this.terminal.registerMarker(0),
          column: this.terminal.buffer.active.cursorX,
        };
      }
      if (event === "C") this.startBlock();
      if (event === "D")
        this.finishBlock(status === undefined ? undefined : Number(status));
      return true;
    });
    const shouldNotify = createAgentNotificationGate();
    this.terminal.parser.registerOscHandler(9, (value) => {
      const progress = parseOsc9Progress(value);
      if (progress) {
        if (!this.disposed)
          this.updateActivitySignal(
            progress.activity,
            progress.activity === "working"
              ? "working"
              : progress.activity === "attention"
                ? "attention"
                : null,
            "progress",
          );
        return true;
      }
      // OSC 9;4 is a progress report, not a notification. Never forward raw
      // terminal text into a system notification.
      if (!value || /^\d;/.test(value)) return false;
      if (!this.disposed && shouldNotify("attention"))
        emitAgentNotification({
          paneId: this.paneId,
          sessionId: this.sessionId,
          kind: "attention",
          source: "terminal",
        });
      return true;
    });
    this.terminal.parser.registerOscHandler(7501, (value) => {
      const status = parseTerminalProgramStatus(value);
      if (!status) return false;
      if (this.disposed) return true;
      if (this.atPrompt && status.state !== "clear") return true;

      const cli = terminalAgents[this.paneId];
      if (cli && cli !== "codex") return true;

      if (status.state === "clear") {
        if (cli === "codex" || this.snapshot.programStatus !== null)
          this.clearProgramStatus(true);
        return true;
      }

      // A status can arrive before native process discovery. Keep the typed,
      // bounded value pending until the process identity is verified.
      this.update({ programStatus: status });
      return true;
    });
    this.terminal.parser.registerOscHandler(777, (value) => {
      const kind = parseAgentSignal(value);
      if (!kind) return false;
      if (!this.disposed)
        this.updateActivitySignal(
          kind === "finished" ? null : kind,
          kind,
          "agent-signal",
        );
      if (!this.disposed && shouldNotify(kind) && kind !== "working")
        emitAgentNotification({
          paneId: this.paneId,
          sessionId: this.sessionId,
          kind,
          source: "claude",
        });
      return true;
    });
    this.searchAddon.onDidChangeResults(({ resultIndex, resultCount }) =>
      this.update({
        searchResult: resultCount
          ? `${resultIndex + 1} / ${resultCount}`
          : "No matches",
      }),
    );
  }

  private readonly applyTheme = () => {
    if (this.disposed) return;
    this.measuredFont = undefined;
    const appearance = terminalAppearance();
    if (this.terminal.options.fontFamily === appearance.fontFamily) {
      // A reloaded font may keep its name; change the option to invalidate xterm's cached metrics.
      this.terminal.options.fontFamily = `${appearance.fontFamily} `;
    }
    this.terminal.options = appearance;
    this.searchAddon.clearDecorations();
    if (this.snapshot.searchOpen && this.lastSearch)
      this.find(...this.lastSearch);
    if (this.opened) this.terminal.refresh(0, this.terminal.rows - 1);
    this.scheduleFit();
  };
  private lastSearch?: [string, boolean, boolean, boolean];

  readonly subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  /** Read the same parsed buffer that the retained visible xterm renders. */
  controlScreen(maxBytes: number) {
    const terminal = this.terminal;
    if (this.disposed || terminal.rows > 1024 || terminal.cols > 1024)
      return null;
    const buffer = terminal.buffer.active;
    const encoder = new TextEncoder();
    let text = "";
    let bytes = 0;
    let truncated = false;
    outer: for (let row = 0; row < terminal.rows; row++) {
      const line =
        (row ? "\n" : "") +
        (buffer.getLine(buffer.viewportY + row)?.translateToString(true) ?? "");
      for (const scalar of line) {
        const size = encoder.encode(scalar).length;
        if (bytes + size > maxBytes) {
          truncated = true;
          break outer;
        }
        bytes += size;
        text += scalar;
      }
    }
    const cursorRow = buffer.baseY + buffer.cursorY - buffer.viewportY;
    return {
      columns: terminal.cols,
      rows: terminal.rows,
      cursorColumn: Math.min(buffer.cursorX, terminal.cols - 1),
      cursorRow: Math.max(0, Math.min(cursorRow, terminal.rows - 1)),
      cursorVisible: cursorRow >= 0 && cursorRow < terminal.rows,
      viewportOffset: buffer.viewportY,
      buffer: buffer.type,
      text,
      truncated,
      parsedSequence: String(this.controlParsedSequence),
      streamSequence: String(this.controlStreamSequence),
      parserPending: this.controlParsedSequence < this.controlStreamSequence,
    };
  }

  readonly getSnapshot = () => this.snapshot;
  async prepareCloseCheck() {
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      await Promise.race([
        (async () => {
          await this.startPromise;
          await this.input;
        })(),
        new Promise<never>((_, reject) => {
          timer = setTimeout(
            () =>
              reject(
                new Error(
                  "Terminal input is still pending. The shell may be busy.",
                ),
              ),
            500,
          );
        }),
      ]);
    } finally {
      clearTimeout(timer);
    }
  }
  async prepareOwnedClose() {
    if (!this.accountId || this.nativeCloseComplete) return;
    await this.startPromise;
    await this.input;
    await api("close_terminal", { id: this.sessionId });
    this.nativeCloseComplete = true;
  }
  observeControl(agentControlled: boolean, notifyLifecycle = true) {
    if (agentControlled !== this.snapshot.agentControlled) {
      this.update({ agentControlled });
      if (notifyLifecycle) terminalLifecycleChanged();
      return true;
    }
    return false;
  }
  async takeControl() {
    try {
      await api("take_terminal_control", { id: this.sessionId });
      this.observeControl(false);
      this.terminal.focus();
    } catch (error) {
      reportError(errorMessage(error));
    }
  }
  observeForegroundProgram(value: string | null) {
    const foregroundProgram = this.atPrompt
      ? ""
      : (value ?? "").replace(/[\x00-\x1f\x7f-\x9f]/g, "").slice(0, 128);
    if (foregroundProgram !== this.snapshot.foregroundProgram)
      this.update({ foregroundProgram });
  }
  resetCliActivity() {
    this.activityOverride = undefined;
    this.activityOverrideSource = undefined;
    if (
      this.snapshot.agentSignal !== null ||
      this.snapshot.titleBusy ||
      this.snapshot.programStatus !== null
    )
      this.update({ agentSignal: null, titleBusy: false, programStatus: null });
    else publishTerminalAgentState(this);
  }
  clearProgramStatus(clearLegacyActivity = false) {
    if (clearLegacyActivity) {
      this.activityOverride = null;
      this.activityOverrideSource = undefined;
    }
    const patch: Partial<Snapshot> = {};
    if (this.snapshot.programStatus !== null) patch.programStatus = null;
    if (clearLegacyActivity) {
      if (this.snapshot.agentSignal !== null) patch.agentSignal = null;
      if (this.snapshot.titleBusy) patch.titleBusy = false;
    }
    if (Object.keys(patch).length) this.update(patch);
    else publishTerminalAgentState(this);
  }
  getCliActivity(): CliActivity | null {
    if (this.snapshot.status === "error") return "error";
    if (
      this.snapshot.status === "exited" &&
      this.exitCode !== undefined &&
      this.exitCode !== null &&
      this.exitCode !== 0
    )
      return "error";
    const programStatus = this.snapshot.programStatus;
    if (
      terminalAgents[this.paneId] === "codex" &&
      programStatus !== null &&
      programStatus.state !== "clear"
    )
      return terminalProgramStatusActivity(programStatus);
    if (this.activityOverride !== undefined) return this.activityOverride;
    if (this.snapshot.agentSignal === "working") return "working";
    if (this.snapshot.agentSignal === "attention") return "attention";
    if (this.snapshot.agentSignal === "finished") return null;
    return this.snapshot.titleBusy ? "working" : null;
  }
  private updateActivitySignal(
    activity: CliActivity | null,
    agentSignal: AgentSignal | null,
    source: "agent-signal" | "progress" | "notification",
  ) {
    this.activityOverride = activity;
    this.activityOverrideSource = source;
    const patch: Partial<Snapshot> = {};
    if (this.snapshot.agentSignal !== agentSignal)
      patch.agentSignal = agentSignal;
    if (Object.keys(patch).length) this.update(patch);
    else publishTerminalAgentState(this);
  }
  setView(view: "searchOpen" | "composerOpen" | "blocksOpen", open: boolean) {
    this.update({ [view]: open });
  }
  toggleView(view: "searchOpen" | "composerOpen" | "blocksOpen") {
    this.setView(view, !this.snapshot[view]);
  }
  private update(patch: Partial<Snapshot>) {
    if (this.disposed) return;
    this.snapshot = { ...this.snapshot, ...patch };
    publishTerminalAgentState(this);
    for (const listener of this.listeners) listener();
  }

  attach(container: HTMLElement) {
    if (this.disposed) return;
    this.attached = true;
    // Opacity keeps input focus and measurements available while the renderer is prepared.
    this.host.style.opacity = "0";
    container.replaceChildren(this.host);
    if (!this.opened) {
      this.terminal.open(this.host);
      this.opened = true;
    }
    const generation = ++this.rendererGeneration;
    this.rendererPromise = this.initializeWebgl(generation);
    this.observer = new ResizeObserver(() => this.scheduleFit());
    this.observer.observe(container);
    void this.start();
  }

  private async initializeWebgl(generation: number) {
    const options = this.terminal.options;
    const font = JSON.stringify([
      options.fontFamily,
      options.fontSize,
      options.fontWeight,
      options.fontWeightBold,
    ]);
    const measureFont = this.measuredFont !== font;
    if (measureFont) {
      await loadTerminalFonts(this.terminal.options);
      if (
        !this.attached ||
        this.disposed ||
        generation !== this.rendererGeneration
      )
        return;
    }
    // Noncomposited WebKit pages copy WebGL canvases through CPU image buffers.
    // Select DOM before importing the addon or creating a WebGL context.
    if (terminalRenderer === "webgl") {
      let webgl: WebglAddon | undefined;
      try {
        const { WebglAddon } = await (webglModule ??=
          import("@xterm/addon-webgl").catch((error) => {
            webglModule = undefined;
            throw error;
          }));
        if (
          !this.attached ||
          this.disposed ||
          generation !== this.rendererGeneration
        )
          return;
        webgl = new WebglAddon();
        const addon = webgl;
        webgl.onContextLoss(() => {
          if (this.webgl !== addon) return;
          addon.dispose();
          this.webgl = undefined;
          this.update({ renderer: "DOM" });
          this.fit();
        });
        this.terminal.loadAddon(webgl);
        // addon-webgl 0.19 multiplies glyph alpha twice with blendFunc, washing out
        // antialiased and dim text over a transparent canvas. Keep alpha coverage
        // separate from RGB blending for the browser's premultiplied compositor.
        for (const canvas of this.host.querySelectorAll<HTMLCanvasElement>(
          ".xterm-screen > canvas",
        )) {
          const gl = canvas.getContext("webgl2");
          if (!gl) continue;
          gl.blendFuncSeparate(
            gl.SRC_ALPHA,
            gl.ONE_MINUS_SRC_ALPHA,
            gl.ONE,
            gl.ONE_MINUS_SRC_ALPHA,
          );
          break;
        }
        this.webgl = webgl;
        this.update({ renderer: "WebGL" });
      } catch {
        webgl?.dispose();
        if (generation === this.rendererGeneration)
          this.update({ renderer: "DOM" });
      }
    }
    if (measureFont) {
      // Replacing xterm's DOM renderer changes styles and can invalidate WebKit's
      // loaded font faces. Its WebGL atlas may already contain fallback glyphs.
      await loadTerminalFonts(this.terminal.options);
      if (
        !this.attached ||
        this.disposed ||
        generation !== this.rendererGeneration
      )
        return;
      await document.fonts.ready;
      if (
        !this.attached ||
        this.disposed ||
        generation !== this.rendererGeneration
      )
        return;
      const fontFamily = this.terminal.options.fontFamily;
      this.terminal.options.fontFamily = `${fontFamily} `;
      this.terminal.options.fontFamily = fontFamily;
      // Metric invalidation can start another font-loading cycle. Discard glyphs
      // only after the final font change settles, before revealing or starting a PTY.
      await loadTerminalFonts(this.terminal.options);
      if (
        !this.attached ||
        this.disposed ||
        generation !== this.rendererGeneration
      )
        return;
      await document.fonts.ready;
      if (
        !this.attached ||
        this.disposed ||
        generation !== this.rendererGeneration
      )
        return;
      this.terminal.clearTextureAtlas();
      // Returning panes reuse their measurements without loading fonts again.
      this.measuredFont = font;
    }
    // Fit returning panes together; new PTYs need fitted dimensions before startup.
    if (this.snapshot.status !== "starting") await Promise.resolve();
    if (
      !this.attached ||
      this.disposed ||
      generation !== this.rendererGeneration
    )
      return;
    // DOM and WebGL round cell widths differently. Fit and reveal only the chosen renderer.
    this.rendererReady = true;
    this.fit();
    this.firstRender = this.terminal.onRender(() => {
      if (!this.receivedOutput && this.snapshot.status !== "exited") return;
      this.firstRender?.dispose();
      this.firstRender = undefined;
      this.host.style.opacity = "";
      if (
        !this.revealed &&
        !matchMedia("(prefers-reduced-motion: reduce)").matches
      )
        this.revealAnimation = this.host.animate(
          [{ opacity: 0 }, { opacity: 1 }],
          { duration: 100, easing: "ease-out" },
        );
      this.revealed = true;
    });
    this.terminal.refresh(0, this.terminal.rows - 1);
  }

  detach() {
    this.attached = false;
    this.rendererReady = false;
    this.rendererGeneration++;
    cancelAnimationFrame(this.frame);
    this.firstRender?.dispose();
    this.firstRender = undefined;
    this.revealAnimation?.cancel();
    this.revealAnimation = undefined;
    this.observer?.disconnect();
    this.host.remove();
    // Terminal.dispose owns addon teardown on close, avoiding an intermediate DOM renderer.
    // Remove departing hosts together before renderer teardown can measure layout.
    const webgl = this.webgl;
    if (!this.disposed && webgl) queueMicrotask(() => webgl.dispose());
    this.webgl = undefined;
  }

  scheduleFit() {
    cancelAnimationFrame(this.frame);
    if (this.attached && this.rendererReady)
      this.frame = requestAnimationFrame(() => this.fit());
  }

  synchronizeLayoutFit() {
    // Fit the final transaction geometry now so xterm can render on its next frame.
    cancelAnimationFrame(this.frame);
    this.frame = 0;
    this.fit();
  }

  private fit() {
    if (
      !this.attached ||
      !this.rendererReady ||
      this.host.clientWidth < 30 ||
      this.host.clientHeight < 20
    )
      return;
    const before = `${this.terminal.cols}:${this.terminal.rows}`;
    this.fitAddon.fit();
    // Resizing the DOM renderer rounds its canvas width again, which can free one more column.
    if (!this.webgl && before !== `${this.terminal.cols}:${this.terminal.rows}`)
      this.fitAddon.fit();
    if (
      this.snapshot.status === "running" &&
      before !== `${this.terminal.cols}:${this.terminal.rows}`
    ) {
      this.pendingResize = {
        cols: this.terminal.cols,
        rows: this.terminal.rows,
      };
      void this.resize();
    }
  }

  private async resize() {
    if (this.resizing) return;
    this.resizing = true;
    try {
      while (
        this.pendingResize &&
        !this.disposed &&
        this.snapshot.status === "running"
      ) {
        const size = this.pendingResize;
        this.pendingResize = undefined;
        try {
          if (this.hostBinding) await this.resizeSharedHost(size);
          else await api("resize_terminal", { id: this.sessionId, ...size });
        } catch (error) {
          if (!this.disposed && this.snapshot.status === "running")
            reportError(errorMessage(error));
        }
      }
    } finally {
      this.resizing = false;
    }
  }

  async startInBackground() {
    // xterm can parse and acknowledge output before attach reuses this startPromise.
    await this.start();
    if (this.snapshot.status === "error")
      throw new Error(this.snapshot.error ?? "Terminal could not start.");
    if (this.hostBinding && this.snapshot.status !== "running")
      throw new Error(
        "The shared host terminal is unavailable. Restart it explicitly to create a new shell.",
      );
  }

  configureSharedHostBinding(binding?: SharedHostTerminalBinding) {
    if (this.agentStarted || this.cliLaunch || this.accountId) return;
    const previous = this.hostBinding;
    this.hostBinding = binding;
    if (
      previous?.workspaceId !== binding?.workspaceId ||
      previous?.rootPath !== binding?.rootPath ||
      previous?.desktopTabId !== binding?.desktopTabId ||
      previous?.tabName !== binding?.tabName
    ) {
      this.clearObservedHostDirectory();
    }
  }

  private start() {
    if (this.startPromise) return this.startPromise;
    this.startPromise = (async () => {
      let renderer: Promise<void> | undefined;
      do {
        renderer = this.rendererPromise;
        await renderer;
        // Docking or a remount can replace initialization while fonts load.
      } while (renderer !== this.rendererPromise);
      if (this.disposed) return;
      if (this.hostBinding) {
        try {
          await this.startSharedHost();
        } catch (error) {
          this.update({ status: "error", error: errorMessage(error) });
        }
        return;
      }
      const output = new Channel<ArrayBuffer>();
      output.onmessage = (data) => {
        if (this.disposed) return;
        const bytes = new Uint8Array(data);
        if (!bytes.length) {
          this.eof = true;
          this.finishExit();
          return;
        }
        const controlSequence = ++this.controlStreamSequence;
        // Output stays outside React; acknowledgements follow xterm's parser callback.
        this.terminal.write(bytes, () => {
          this.controlParsedSequence = controlSequence;
          this.receivedOutput = true;
          this.unacknowledged += bytes.length;
          if (this.unacknowledged >= 32 * 1024) this.acknowledge();
          else if (this.ackTimer === undefined)
            this.ackTimer = setTimeout(() => this.acknowledge(), 16);
        });
      };
      const exited = new Channel<{ code: number | null }>();
      exited.onmessage = ({ code }) => {
        this.exitCode = code;
        this.finishExit();
      };
      try {
        const requestedCwd = this.snapshot.cwd;
        const started = await api<{
          cwd: string;
          accountId?: string | null;
          accountLabel?: string | null;
        }>("start_terminal", {
          request: {
            id: this.sessionId,
            profileId: this.profile.id,
            cwd: this.snapshot.cwd,
            cols: this.terminal.cols,
            rows: this.terminal.rows,
            agentTicket: this.agentStart
              ? {
                  operationId: this.agentStart.operationId,
                  nonce: this.agentStart.nonce,
                }
              : null,
            cliLaunch: this.cliLaunch ?? null,
            accountId: this.accountId ?? null,
          },
          output,
          exited,
        });
        if (this.disposed) {
          await api("close_terminal", { id: this.sessionId });
          return;
        }
        const cwd =
          this.snapshot.cwd === requestedCwd ? started.cwd : this.snapshot.cwd;
        if (!this.eof)
          this.update({
            status: "running",
            cwd,
            accountLabel: started.accountLabel ?? null,
          });
        directoryListener(this.paneId, cwd);
        this.scheduleFit();
      } catch (error) {
        this.update({ status: "error", error: errorMessage(error) });
      }
    })();
    return this.startPromise;
  }

  private async startSharedHost() {
    const binding = this.hostBinding;
    if (!binding)
      throw new Error("The shared terminal workspace is unavailable.");
    await importSharedWorkspace(
      binding.workspaceId,
      binding.workspaceName,
      binding.rootPath,
    );
    if (this.disposed) return;
    let state = await sharedWorkspaceState(binding.workspaceId);
    const aliases = await sharedHostTerminalAliases(binding.workspaceId);
    let alias = aliases.find(
      (candidate) => candidate.desktopPaneId === this.paneId,
    );
    let located = locateHostPane(state, alias?.hostPaneId ?? this.paneId);
    const intent = sharedHostCreateIntent(this.paneId);

    if (!located && !alias && intent) {
      const profileId = await registerSharedShellProfile(
        binding.workspaceId,
        this.profile.id,
        this.snapshot.cwd,
      );
      const operationId = await sharedHostOperationId(
        binding.workspaceId,
        this.paneId,
        intent.kind,
        intent.kind === "split" ? intent.targetPaneId : "",
        intent.kind === "split" ? intent.axis : "",
      );
      const mutation = {
        operationId,
        context: state.context,
        expectedRevision: state.domainRevision,
      };
      let receipt;
      if (intent.kind === "create") {
        receipt = await this.createSharedHostTerminal(
          mutation,
          binding.tabName,
          profileId,
        );
      } else {
        const targetAlias = aliases.find(
          (candidate) => candidate.desktopPaneId === intent.targetPaneId,
        );
        const targetHostPaneId = targetAlias?.hostPaneId ?? intent.targetPaneId;
        const target = locateHostPane(state, targetHostPaneId);
        if (!target?.pane.session)
          throw new Error("The terminal being split is unavailable.");
        receipt = await this.splitSharedHostTerminal(
          {
            ...mutation,
            operationId,
          },
          target.tabId,
          targetHostPaneId,
          intent.axis,
          profileId,
        );
      }
      const newPaneId =
        receipt.result?.kind === "terminal" ? receipt.result.paneId : "";
      if (!newPaneId)
        throw new Error("Local host did not confirm terminal creation.");
      state = await sharedWorkspaceState(binding.workspaceId);
      located = locateHostPane(state, newPaneId);
      if (!located)
        throw new Error(
          "The created terminal is not present in the host workspace.",
        );
      alias = {
        desktopPaneId: this.paneId,
        desktopTabId: binding.desktopTabId,
        hostPaneId: newPaneId,
        hostTabId: located.tabId,
      };
      await saveSharedHostTerminalAlias(binding.workspaceId, alias);
      finishSharedHostCreate(this.paneId);
    }

    if (this.disposed) return;
    if (!located) {
      this.eof = true;
      this.exitCode = null;
      this.update({ status: "exited" });
      return;
    }
    if (!alias) {
      alias = {
        desktopPaneId: this.paneId,
        desktopTabId: binding.desktopTabId,
        hostPaneId: located.pane.paneId,
        hostTabId: located.tabId,
      };
      await saveSharedHostTerminalAlias(binding.workspaceId, alias);
    }
    if (!located.pane.session) {
      this.eof = true;
      this.exitCode = null;
      this.update({ status: "exited" });
      return;
    }
    if (located.pane.state === "exited") {
      this.eof = true;
      this.exitCode = null;
    }
    await this.attachSharedHostSession(located.pane.session);
  }

  private async attachSharedHostSession(
    terminal: SharedHostTerminalContext | null | undefined,
  ) {
    this.clearObservedHostDirectory();
    if (!terminal) {
      this.eof = true;
      this.exitCode = null;
      this.update({ status: "exited" });
      return;
    }
    this.hostEvents && (this.hostEvents.onmessage = () => {});
    this.hostAssembler?.dispose();
    this.hostAssembler = undefined;
    this.hostSnapshotReady = false;
    const channel = new Channel<SharedHostEvent>();
    channel.onmessage = (event) => this.receiveSharedHostEvent(event);
    const metadata = await sharedHostRpc(
      {
        method: "terminal.attach",
        params: { terminal },
      },
      channel,
    );
    this.sessionId = terminal.sessionId;
    this.hostEvents = channel;
    this.hostTerminal = metadata.terminal;
    this.hostSubscriptionId = metadata.subscriptionId;
    this.hostAssembler = TerminalStreamAssembler.fromSnapshotMetadata(
      metadata.terminal,
      metadata.subscriptionId,
      metadata,
    );
    const pending = this.hostPendingFrames;
    this.hostPendingFrames = [];
    this.hostPendingFrameBytes = 0;
    for (const frame of pending) this.processSharedHostFrame(frame);
    this.scheduleFit();
  }

  private async createSharedHostTerminal(
    mutation: {
      operationId: string;
      context: import("./shared-host").SharedHostWorkspaceContext;
      expectedRevision: string;
    },
    tabName: string,
    profileId: string,
  ) {
    const call = {
      method: "terminal.create" as const,
      params: { mutation, tabName, profileId },
    };
    try {
      return await sharedHostRpc(call);
    } catch (error) {
      try {
        return await sharedHostRpc({
          method: "operation.status",
          params: {
            context: mutation.context,
            operationId: mutation.operationId,
          },
        });
      } catch {
        throw error;
      }
    }
  }

  private async splitSharedHostTerminal(
    mutation: {
      operationId: string;
      context: import("./shared-host").SharedHostWorkspaceContext;
      expectedRevision: string;
    },
    tabId: string,
    paneId: string,
    axis: "horizontal" | "vertical",
    profileId: string,
  ) {
    const call = {
      method: "terminal.split" as const,
      params: { mutation, tabId, paneId, axis, profileId },
    };
    try {
      return await sharedHostRpc(call);
    } catch (error) {
      try {
        return await sharedHostRpc({
          method: "operation.status",
          params: {
            context: mutation.context,
            operationId: mutation.operationId,
          },
        });
      } catch {
        throw error;
      }
    }
  }

  private receiveSharedHostEvent(event: SharedHostEvent) {
    if (this.disposed) return;
    if (event.type === "transportLost") {
      this.clearObservedHostDirectory();
      this.loseSharedHostLease();
      this.update({
        status: "error",
        error:
          "The shared terminal connection was lost. Reopen the terminal to reconnect.",
      });
      return;
    }
    if (event.value.stream !== "terminal") return;
    const frame = JSON.stringify(event.value.frame);
    if (!this.hostAssembler) {
      const size = new TextEncoder().encode(frame).length;
      if (this.hostPendingFrameBytes + size > 1024 * 1024) {
        this.hostPendingFrames = [];
        this.hostPendingFrameBytes = 0;
        this.update({
          status: "error",
          error:
            "The shared terminal snapshot arrived too quickly. Reopen the terminal to retry.",
        });
        return;
      }
      this.hostPendingFrames.push(frame);
      this.hostPendingFrameBytes += size;
      return;
    }
    this.processSharedHostFrame(frame);
  }

  private processSharedHostFrame(frame: string) {
    const assembler = this.hostAssembler;
    if (!assembler || this.disposed) return;
    try {
      const assembled = assembler.push(frame);
      if (assembled) this.applySharedHostValue(assembled);
    } catch (error) {
      this.clearObservedHostDirectory();
      this.loseSharedHostLease();
      const reason =
        error instanceof TerminalStreamResyncError
          ? error.message
          : "invalid terminal stream";
      this.update({
        status: "error",
        error: `The shared terminal needs to be reopened (${reason}).`,
      });
    }
  }

  private applySharedHostValue(value: AssembledTerminalValue) {
    const terminal = this.terminal;
    if (value.kind === "snapshot") {
      applySnapshot(terminal, value.value);
      this.applySharedHostDirectory(value);
      this.hostSnapshotReady = true;
      this.receivedOutput = true;
      this.hostRenderedThroughSeq = value.throughSeq;
      this.update({ status: "running" });
      this.terminal.refresh(0, this.terminal.rows - 1);
      void this.acknowledgeSharedHost(value);
      if (this.hostFocused) void this.claimSharedHostControl();
      return;
    }
    if (value.kind === "delta") {
      applyPatch(terminal, value.value);
      this.applySharedHostDirectory(value);
      this.hostRenderedThroughSeq = value.throughSeq;
      this.receivedOutput = true;
      this.update({ status: "running" });
      setInputEnabled(terminal, Boolean(this.hostLease && this.hostFocused));
      void this.acknowledgeSharedHost(value);
      return;
    }
    const event = value.value as {
      change?: {
        kind?: string;
        data?: {
          columns?: number;
          rows?: number;
          exitCode?: number | null;
          leaseVersion?: string;
        };
      };
    };
    const change = event.change;
    if (change?.kind === "grid" && change.data) {
      applyPatch(terminal, change.data);
      this.setSharedHostInputEnabled();
    } else if (change?.kind === "resize" && change.data) {
      const { columns, rows } = change.data;
      if (columns && rows) {
        terminal.resize(columns, rows);
        this.scheduleFit();
      }
    } else if (change?.kind === "exit") {
      this.exitCode = change.data?.exitCode ?? null;
      this.eof = true;
      this.finishExit();
    } else if (change?.kind === "controlChanged") {
      if (this.hostLease?.version !== change.data?.leaseVersion)
        this.loseSharedHostLease();
    }
    this.hostRenderedThroughSeq = value.throughSeq;
    void this.acknowledgeSharedHost(value);
  }

  private applySharedHostDirectory(value: AssembledTerminalValue) {
    const binding = this.hostBinding;
    const expected = this.hostTerminal;
    if (
      !binding ||
      !expected ||
      expected.workspace.workspaceId !== binding.workspaceId ||
      value.terminal.sessionId !== expected.sessionId ||
      value.terminal.sessionEpoch !== expected.sessionEpoch
    ) {
      return;
    }
    const directory =
      typeof value.workingDirectory === "string"
        ? value.workingDirectory
        : null;
    if (directory === this.snapshot.observedWorkingDirectory) return;
    this.update({ observedWorkingDirectory: directory });
  }

  private clearObservedHostDirectory() {
    if (this.snapshot.observedWorkingDirectory !== null) {
      this.update({ observedWorkingDirectory: null });
    }
  }

  private async acknowledgeSharedHost(value: AssembledTerminalValue) {
    const terminal = this.hostTerminal;
    if (!terminal || this.disposed) return;
    this.hostAckChain = this.hostAckChain
      .then(async () => {
        if (this.disposed) return;
        await sharedHostRpc({
          method: "terminal.ack",
          params: {
            terminal,
            subscriptionId: value.subscriptionId,
            throughSeq: value.throughSeq,
          },
        });
      })
      .catch(() => this.loseSharedHostLease());
  }

  private setSharedHostInputEnabled() {
    try {
      setInputEnabled(
        this.terminal,
        Boolean(this.hostLease && this.hostFocused),
      );
    } catch (error) {
      this.loseSharedHostLease();
      reportError(errorMessage(error));
    }
  }

  setSharedHostFocused(focused: boolean) {
    if (!this.hostBinding || this.disposed || this.hostFocused === focused)
      return;
    this.hostFocused = focused;
    if (focused) void this.claimSharedHostControl();
    else void this.releaseSharedHostControl();
  }

  private async claimSharedHostControl() {
    if (
      !this.hostFocused ||
      !this.hostSnapshotReady ||
      !this.hostTerminal ||
      !this.hostSubscriptionId ||
      this.disposed
    )
      return;
    if (this.hostLease) return;
    try {
      const lease = await sharedHostRpc({
        method: "terminal.claim",
        params: {
          terminal: this.hostTerminal,
          subscriptionId: this.hostSubscriptionId,
          renderedThroughSeq: this.hostRenderedThroughSeq,
        },
      });
      if (!this.hostFocused || this.disposed) {
        await sharedHostRpc({
          method: "terminal.release",
          params: { terminal: lease.terminal, lease: lease.lease },
        });
        return;
      }
      this.hostLease = lease.lease;
      this.hostInputId = BigInt(lease.nextInputId);
      this.hostInputSeq = BigInt(lease.nextInputSeq);
      this.hostResizeSeq = BigInt(lease.nextResizeSeq);
      this.setSharedHostInputEnabled();
      this.scheduleSharedHostLeaseRenewal(lease.expiresInMs);
    } catch {
      this.loseSharedHostLease();
    }
  }

  private scheduleSharedHostLeaseRenewal(expiresInMs: number) {
    clearTimeout(this.hostLeaseTimer);
    this.hostLeaseTimer = setTimeout(
      () => void this.renewSharedHostControl(),
      Math.max(250, Math.floor(expiresInMs / 2)),
    );
  }

  private async renewSharedHostControl() {
    const lease = this.hostLease;
    const terminal = this.hostTerminal;
    if (!lease || !terminal || !this.hostFocused || this.disposed) return;
    try {
      const renewed = await sharedHostRpc({
        method: "terminal.renew",
        params: { terminal, lease },
      });
      if (this.hostLease !== lease || this.disposed) return;
      this.hostLease = renewed.lease;
      this.scheduleSharedHostLeaseRenewal(renewed.expiresInMs);
    } catch {
      this.loseSharedHostLease();
    }
  }

  private async releaseSharedHostControl() {
    const lease = this.hostLease;
    const terminal = this.hostTerminal;
    this.loseSharedHostLease();
    if (!lease || !terminal) return;
    try {
      await sharedHostRpc({
        method: "terminal.release",
        params: { terminal, lease },
      });
    } catch {
      // Lease expiry and transport loss both leave the renderer read-only.
    }
  }

  private loseSharedHostLease() {
    this.hostLease = undefined;
    clearTimeout(this.hostLeaseTimer);
    this.hostLeaseTimer = undefined;
    try {
      setInputEnabled(this.terminal, false);
    } catch {
      // A failed structured render already leaves input disabled.
    }
  }

  private async sendSharedHostInput(data: string) {
    const terminal = this.hostTerminal;
    const lease = this.hostLease;
    if (!terminal || !lease || !this.hostFocused)
      throw new Error("Focus this terminal window to send input.");
    const encoder = new TextEncoder();
    let bytes = new Uint8Array(0);
    let text = "";
    const submit = async () => {
      if (!bytes.length) return;
      const inputId = this.hostInputId.toString();
      const inputSeq = this.hostInputSeq.toString();
      const receipt = await sharedHostRpc({
        method: "terminal.input",
        params: {
          terminal,
          lease,
          inputId,
          inputSeq,
          bytes: [...bytes],
        },
      });
      if (receipt.inputId !== inputId || receipt.inputSeq !== inputSeq)
        throw new Error("The host did not confirm terminal input ordering.");
      this.hostInputId++;
      this.hostInputSeq++;
      bytes = new Uint8Array(0);
      text = "";
    };
    try {
      for (const scalar of data) {
        const next = encoder.encode(scalar);
        if (bytes.length + next.length > 16_384) await submit();
        const joined = new Uint8Array(bytes.length + next.length);
        joined.set(bytes);
        joined.set(next, bytes.length);
        bytes = joined;
        text += scalar;
      }
      if (text) await submit();
    } catch {
      // A lost reply makes the input sequence ambiguous. Release control and
      // require a fresh lease instead of sending these bytes again.
      void this.releaseSharedHostControl();
      throw new Error(
        "The shared terminal input could not be confirmed and was not retried. Reopen the terminal before sending more.",
      );
    }
  }

  private async resizeSharedHost(size: { cols: number; rows: number }) {
    this.hostResizePending = size;
    if (this.hostResizing) return;
    this.hostResizing = true;
    try {
      while (this.hostResizePending && !this.disposed) {
        const next = this.hostResizePending;
        this.hostResizePending = undefined;
        const lease = this.hostLease;
        const terminal = this.hostTerminal;
        if (!lease || !terminal || !this.hostFocused) continue;
        const resizeSeq = this.hostResizeSeq.toString();
        await sharedHostRpc({
          method: "terminal.resize",
          params: {
            terminal,
            lease,
            resizeSeq,
            columns: next.cols,
            rows: next.rows,
          },
        });
        this.hostResizeSeq++;
      }
    } catch {
      this.loseSharedHostLease();
    } finally {
      this.hostResizing = false;
    }
  }

  private finishExit() {
    if (!this.eof || this.exitCode === undefined || this.disposed) return;
    if (this.hostBinding) {
      this.loseSharedHostLease();
      this.update({ status: "exited" });
      return;
    }
    this.terminal.write("", () => {
      this.finishBlock(this.exitCode ?? undefined);
      this.update({ status: "exited" });
      this.terminal.writeln(
        `\r\n\x1b[2mSession ended${this.exitCode === null ? "" : ` (exit ${this.exitCode})`}. Use Restart to open a new shell.\x1b[0m`,
      );
    });
  }

  private acknowledge() {
    clearTimeout(this.ackTimer);
    this.ackTimer = undefined;
    const bytes = this.unacknowledged;
    this.unacknowledged = 0;
    if (bytes && !this.disposed)
      void api("acknowledge_terminal", { id: this.sessionId, bytes }).catch(
        () => {},
      );
  }

  send(data: string, human = true) {
    if (human && this.pasteOutput) {
      this.pasteOutput.push(data);
      return;
    }
    void this.queueInput(() => this.writeInput(data, human));
  }

  private queueInput(write: () => Promise<void>) {
    if (
      this.disposed ||
      this.snapshot.status === "exited" ||
      this.snapshot.status === "error"
    )
      return Promise.resolve();
    this.input = this.input
      .then(async () => {
        await this.startPromise;
        if (!this.disposed) await write();
      })
      .catch((error) => {
        if (!this.disposed && this.snapshot.status === "running")
          reportError(errorMessage(error));
      });
    return this.input;
  }

  private async writeInput(data: string, human = true) {
    if (this.hostBinding) {
      await this.sendSharedHostInput(data);
      return;
    }
    if (human) this.observeControl(false);
    // Accept titles after submission even when a shell has no pre-execution hook.
    if (human && (data === "\r" || data === "\n")) this.atPrompt = false;
    if (
      human &&
      data === "\r" &&
      ["cmd", "pwsh", "powershell"].includes(this.profile.kind)
    )
      this.startBlock();
    for (const chunk of inputChunks(data)) {
      if (this.disposed) return;
      await api(human ? "write_terminal" : "write_terminal_response", {
        id: this.sessionId,
        data: chunk,
      });
    }
  }

  execute(command: string) {
    if (!command.trim()) return;
    if (command.length > 1_048_576) {
      reportError("The command input exceeds 1 MiB.");
      return;
    }
    this.nextCommand = command;
    this.terminal.paste(command);
    this.send("\r");
    this.terminal.focus();
  }

  async pastePaths(paths: string[]) {
    const quoted = await api<string>("quote_paths", {
      profileId: this.profile.id,
      paths,
    });
    if (!this.disposed) {
      this.terminal.paste(`${quoted} `);
      this.terminal.focus();
    }
  }

  async copy() {
    const selection = this.terminal.getSelection();
    if (selection)
      await writeText(selection).catch((error) =>
        reportError(errorMessage(error)),
      );
  }

  private readonly onPaste = (event: ClipboardEvent) => {
    if (
      event.defaultPrevented ||
      !(event.target instanceof Element) ||
      !event.target.classList.contains("xterm-helper-textarea")
    )
      return;
    event.preventDefault();
    event.stopPropagation();
    void this.pasteClipboard();
  };

  pasteClipboard() {
    if (
      this.disposed ||
      this.snapshot.status === "exited" ||
      this.snapshot.status === "error"
    )
      return Promise.resolve();
    return this.queueInput(async () => {
      if (this.hostBinding) {
        const text = await readText();
        if (this.disposed || text === null) return;
        this.pasteOutput = [];
        let data: string;
        try {
          this.terminal.paste(text);
          data = this.pasteOutput.join("");
        } finally {
          this.pasteOutput = undefined;
        }
        if (!this.disposed) await this.writeInput(data);
        return;
      }
      // Native image-paste actions share the input queue with text and later keys.
      const text = await api<string | null>("paste_terminal_clipboard", {
        id: this.sessionId,
      });
      if (this.disposed || text === null) return;
      this.pasteOutput = [];
      let data: string;
      try {
        // Let xterm normalize line endings and honor the application's bracketed-paste mode.
        this.terminal.paste(text);
        data = this.pasteOutput.join("");
      } finally {
        this.pasteOutput = undefined;
      }
      if (this.disposed) return;
      await this.writeInput(data);
    });
  }

  find(query: string, previous = false, caseSensitive = false, regex = false) {
    this.lastSearch = [query, previous, caseSensitive, regex];
    if (!query) {
      this.searchAddon.clearDecorations();
      this.update({ searchResult: "" });
      return;
    }
    try {
      const options = {
        caseSensitive,
        regex,
        decorations: terminalSearchColors(),
      };
      if (previous) this.searchAddon.findPrevious(query, options);
      else this.searchAddon.findNext(query, options);
    } catch {
      this.update({ searchResult: "Invalid expression" });
    }
  }

  private startBlock() {
    this.atPrompt = false;
    if (this.activeBlock || this.terminal.buffer.active.type !== "normal")
      return;
    const buffer = this.terminal.buffer.active;
    const prompt = this.promptEnd;
    let command = this.nextCommand;
    this.nextCommand = undefined;
    if (!command && prompt && !prompt.marker.isDisposed) {
      const lines: string[] = [];
      for (
        let row = prompt.marker.line;
        row <= buffer.baseY + buffer.cursorY;
        row++
      ) {
        lines.push(
          buffer
            .getLine(row)
            ?.translateToString(
              true,
              row === prompt.marker.line ? prompt.column : 0,
            ) ?? "",
        );
      }
      command = lines.join("\n").trim();
    }
    if (!command) return;
    const block: CommandBlock = {
      id: newId(),
      command,
      started: Date.now(),
      marker: this.terminal.registerMarker(0),
    };
    this.activeBlock = block;
    const blocks = [...this.snapshot.blocks, block];
    if (blocks.length > 100) blocks.shift()?.marker?.dispose();
    this.update({ blocks });
  }

  private finishBlock(exitCode?: number) {
    this.atPrompt = true;
    this.activityOverride = null;
    this.activityOverrideSource = undefined;
    if (
      this.snapshot.title ||
      this.snapshot.titleBusy ||
      this.snapshot.agentSignal ||
      this.snapshot.programStatus ||
      this.snapshot.foregroundProgram
    )
      this.update({
        title: "",
        titleBusy: false,
        agentSignal: null,
        programStatus: null,
        foregroundProgram: "",
      });
    else publishTerminalAgentState(this);
    if (!this.activeBlock) return;
    const id = this.activeBlock.id;
    this.activeBlock = undefined;
    this.update({
      blocks: this.snapshot.blocks.map((block) =>
        block.id === id
          ? {
              ...block,
              finished: Date.now(),
              exitCode: Number.isFinite(exitCode) ? exitCode : undefined,
            }
          : block,
      ),
    });
  }

  jumpTo(block: CommandBlock) {
    if (block.marker && !block.marker.isDisposed) {
      this.terminal.scrollToLine(block.marker.line);
      this.terminal.focus();
    }
  }

  dispose(closeSharedHost = true) {
    if (this.disposed) return;
    this.clearObservedHostDirectory();
    if (this.hostBinding) {
      if (closeSharedHost)
        void closeHostTerminal(this.hostBinding.workspaceId, this.paneId).catch(
          (error) => reportError(errorMessage(error)),
        );
      else void this.releaseSharedHostControl();
      this.hostEvents && (this.hostEvents.onmessage = () => {});
      this.hostAssembler?.dispose();
      this.hostAssembler = undefined;
    }
    window.removeEventListener(themeAppliedEvent, this.applyTheme);
    this.disposed = true;
    removeTerminalAgentState(this.paneId);
    this.host.removeEventListener("paste", this.onPaste, true);
    this.detach();
    clearTimeout(this.ackTimer);
    this.listeners.clear();
    this.terminal.dispose();
    if (this.hostBinding) return;
    if (!this.nativeCloseComplete)
      void this.startPromise?.finally(() =>
        api("close_terminal", { id: this.sessionId }).catch((error) =>
          reportError(errorMessage(error)),
        ),
      );
  }
}

const lifecycleListeners = new Set<() => void>();
export function subscribeTerminalLifecycle(listener: () => void) {
  lifecycleListeners.add(listener);
  return () => {
    lifecycleListeners.delete(listener);
  };
}
const terminalLifecycleChanged = () => {
  for (const listener of lifecycleListeners) listener();
};
const runtimes = new Map<string, TerminalRuntime>();
let terminalAgents: Readonly<Record<string, CliAgent>> = Object.freeze({});
let terminalAgentProcesses: Readonly<Record<string, TitleProcess>> =
  Object.freeze({});
let terminalAgentStates: Readonly<Record<string, TerminalAgentState>> =
  Object.freeze({});
const terminalAgentStateListeners = new Set<() => void>();
const terminalAgentListeners = new Set<() => void>();
export const getTerminalAgents = () => terminalAgents;
export const getTerminalAgentStates = () => terminalAgentStates;
export function subscribeTerminalAgentStates(listener: () => void) {
  terminalAgentStateListeners.add(listener);
  return () => {
    terminalAgentStateListeners.delete(listener);
  };
}
export function subscribeTerminalAgents(listener: () => void) {
  terminalAgentListeners.add(listener);
  return () => {
    terminalAgentListeners.delete(listener);
  };
}
function sameTitleProcess(
  left: TitleProcess | null | undefined,
  right: TitleProcess | null | undefined,
) {
  return left?.cli === right?.cli && left?.pid === right?.pid;
}
function sameTerminalAgentState(
  left: TerminalAgentState | undefined,
  right: TerminalAgentState | undefined,
) {
  return (
    left === right ||
    Boolean(
      left &&
      right &&
      left.cli === right.cli &&
      left.sessionId === right.sessionId &&
      sameTitleProcess(left.process, right.process) &&
      left.activity === right.activity,
    )
  );
}
function publishTerminalAgentState(runtime: TerminalRuntime) {
  const paneId = runtime.paneId;
  const cli = terminalAgents[paneId];
  const process = terminalAgentProcesses[paneId] ?? null;
  const state: TerminalAgentState | undefined = cli
    ? {
        cli,
        sessionId: runtime.sessionId,
        process,
        activity: runtime.getCliActivity(),
      }
    : undefined;
  const previous = terminalAgentStates[paneId];
  if (sameTerminalAgentState(previous, state)) return false;
  const next = { ...terminalAgentStates };
  if (state) next[paneId] = Object.freeze(state);
  else delete next[paneId];
  terminalAgentStates = Object.freeze(next);
  for (const listener of terminalAgentStateListeners) listener();
  return true;
}
function removeTerminalAgentState(paneId: string) {
  if (!terminalAgentStates[paneId]) return false;
  const next = { ...terminalAgentStates };
  delete next[paneId];
  terminalAgentStates = Object.freeze(next);
  for (const listener of terminalAgentStateListeners) listener();
  return true;
}
function updateTerminalAgentStates() {
  let changed = false;
  for (const runtime of runtimes.values())
    changed = publishTerminalAgentState(runtime) || changed;
  for (const id of Object.keys(terminalAgentStates)) {
    if (!runtimes.has(id) || !terminalAgents[id]) {
      changed = removeTerminalAgentState(id) || changed;
    }
  }
  return changed;
}
function updateTerminalAgents(
  next: Record<string, CliAgent>,
  processes: Record<string, TitleProcess>,
  notifyLifecycle = true,
) {
  const ids = Object.keys(next);
  const agentsChanged = !(
    ids.length === Object.keys(terminalAgents).length &&
    ids.every((id) => next[id] === terminalAgents[id])
  );
  const processIds = Object.keys(processes);
  const processesChanged = !(
    processIds.length === Object.keys(terminalAgentProcesses).length &&
    processIds.every((id) =>
      sameTitleProcess(processes[id], terminalAgentProcesses[id]),
    )
  );
  if (!agentsChanged && !processesChanged) return false;

  const resetActivities = new Set<string>();
  const clearPendingProgramStatuses = new Set<string>();
  for (const id of new Set([
    ...Object.keys(terminalAgents),
    ...ids,
    ...Object.keys(terminalAgentProcesses),
    ...processIds,
  ])) {
    const previousCli = terminalAgents[id];
    const identityChanged =
      previousCli !== next[id] ||
      !sameTitleProcess(terminalAgentProcesses[id], processes[id]);
    if (!identityChanged) continue;

    if (previousCli !== undefined) resetActivities.add(id);
    else if (next[id] !== undefined && next[id] !== "codex")
      clearPendingProgramStatuses.add(id);
  }
  terminalAgents = Object.freeze(next);
  terminalAgentProcesses = Object.freeze(processes);
  for (const id of resetActivities) runtimes.get(id)?.resetCliActivity();
  for (const id of clearPendingProgramStatuses)
    runtimes.get(id)?.clearProgramStatus();
  const statesChanged = updateTerminalAgentStates();
  if (agentsChanged) for (const listener of terminalAgentListeners) listener();
  if (notifyLifecycle && (agentsChanged || processesChanged || statesChanged))
    terminalLifecycleChanged();
  return true;
}
export function terminalFor(
  pane: Pane,
  profile: ShellProfile,
  cliLaunch?: CliAgent,
  accountId?: string,
  hostBinding?: SharedHostTerminalBinding,
): TerminalRuntime {
  let runtime = runtimes.get(pane.id);
  if (!runtime) {
    runtime = new TerminalRuntime(
      pane.id,
      profile,
      pane.cwd,
      cliLaunch,
      accountId,
      hostBinding,
    );
    runtimes.set(pane.id, runtime);
    terminalLifecycleChanged();
  }
  runtime.configureSharedHostBinding(hostBinding);
  return runtime;
}
export const runningTerminal = (id: string) => runtimes.get(id);
export function isAgentTerminal(id: string) {
  const runtime = runtimes.get(id);
  return isAgentTerminalState({
    staged: agentStarts.has(id) || runtime?.agentStarted,
    agentControlled: runtime?.getSnapshot().agentControlled,
    titleCli: Boolean(terminalAgents[id]),
  });
}
export function synchronizeVisibleTerminalFits(root: HTMLElement) {
  for (const pane of root.querySelectorAll<HTMLElement>(
    ".terminal-pane[data-pane-id]",
  )) {
    if (!pane.getClientRects().length) continue;
    const id = pane.dataset.paneId;
    if (!id) continue;
    const runtime = runtimes.get(id);
    if (
      !runtime ||
      !pane.contains(runtime.host) ||
      !root.contains(runtime.host)
    )
      continue;
    runtime.synchronizeLayoutFit();
  }
}
export async function terminalsWithProcesses(ids?: readonly string[]) {
  const selected = [...runtimes.values()].filter(
    (runtime) => !ids || ids.includes(runtime.paneId),
  );
  await Promise.all(selected.map((runtime) => runtime.prepareCloseCheck()));
  const running = selected.filter(
    (runtime) => runtime.getSnapshot().status === "running",
  );
  if (!running.length) return 0;
  const busy = new Set(
    await api<string[]>("busy_terminals", {
      ids: running.map((runtime) => runtime.sessionId),
    }),
  );
  return running.filter(
    (runtime) =>
      busy.has(runtime.sessionId) ||
      runtime
        .getSnapshot()
        .blocks.some((block) => block.finished === undefined),
  ).length;
}
export async function prepareOwnedTerminalClose(ids: readonly string[]) {
  for (const id of ids) await runtimes.get(id)?.prepareOwnedClose();
}
export function closeTerminals(
  ids: string[],
  workspaceIds?: string | Readonly<Record<string, string>>,
  backend: "local" | "shared-host" | "detach" = "shared-host",
) {
  for (const id of ids) {
    const workspaceId =
      typeof workspaceIds === "string" ? workspaceIds : workspaceIds?.[id];
    const closeThroughHost = backend === "shared-host" && !!workspaceId;
    runtimes.get(id)?.dispose(backend !== "detach" && !closeThroughHost);
    if (closeThroughHost)
      void closeHostTerminal(workspaceId, id).catch((error) =>
        reportError(errorMessage(error)),
      );
    runtimes.delete(id);
    terminalLifecycleChanged();
  }
  const next = { ...terminalAgents };
  const processes = { ...terminalAgentProcesses };
  for (const id of ids) {
    delete next[id];
    delete processes[id];
  }
  updateTerminalAgents(next, processes);
}
export function observeTerminalContexts(
  contexts: Record<string, TerminalContext>,
) {
  const result: Record<string, string> = {};
  const agents: Record<string, CliAgent> = {};
  const processes: Record<string, TitleProcess> = {};
  let controlChanged = false;
  for (const [id, runtime] of runtimes) {
    const context = contexts[runtime.sessionId];
    if (context?.titleCli?.cli) {
      agents[id] = context.titleCli.cli;
      processes[id] = context.titleCli;
    }
    runtime.observeForegroundProgram(context?.foregroundProgram ?? null);
    controlChanged =
      runtime.observeControl(context?.agentControlled ?? false, false) ||
      controlChanged;
    if (context?.cwd) result[id] = context.cwd;
  }
  const agentsChanged = updateTerminalAgents(agents, processes, false);
  if (controlChanged || agentsChanged) terminalLifecycleChanged();
  return result;
}
if (import.meta.hot)
  import.meta.hot.dispose(() => {
    closeTerminals([...runtimes.keys()], undefined, "detach");
    void detachSharedHostView().catch(() => {});
  });
