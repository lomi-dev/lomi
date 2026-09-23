import { Channel } from "@tauri-apps/api/core";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Terminal } from "@xterm/xterm";
import type { IDisposable, IMarker } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { WebLinksAddon } from "@xterm/addon-web-links";
import type { WebglAddon } from "@xterm/addon-webgl";
import { api, errorMessage, macOS, windows } from "./api";
import { newId } from "./model";
import type { Pane, ShellProfile } from "./model";
import { inputChunks } from "./terminal-utils";
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
  agentSignal: AgentSignal | null;
  foregroundProgram: string;
  agentControlled: boolean;
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
  cli: "codex" | "agy" | "cursor" | "claude";
  pid: number;
}

export interface TerminalContext {
  cwd: string | null;
  foregroundProgram: string | null;
  titleCli: TitleProcess | null;
  agentControlled?: boolean;
}
type DirectoryListener = (id: string, cwd: string) => void;
let directoryListener: DirectoryListener = () => {};
let reportError: (message: string) => void = () => {};
let webglModule: Promise<typeof import("@xterm/addon-webgl")> | undefined;

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
}
export const clearAgentTerminal = (panelId: string) =>
  agentStarts.delete(panelId);
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
  readonly sessionId: string;
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

  constructor(
    readonly paneId: string,
    readonly profile: ShellProfile,
    cwd: string,
  ) {
    this.agentStart = agentStarts.get(paneId);
    agentStarts.delete(paneId);
    this.sessionId = this.agentStart?.sessionId ?? newId();
    this.snapshot = {
      status: "starting",
      title: "",
      titleBusy: false,
      agentSignal: null,
      foregroundProgram: "",
      agentControlled: false,
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
      if (
        title !== this.snapshot.title ||
        titleBusy !== this.snapshot.titleBusy
      )
        this.update({ title, titleBusy });
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
    this.terminal.onData((data) => this.send(data));
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
    this.terminal.parser.registerOscHandler(777, (value) => {
      const kind = parseAgentSignal(value);
      if (!kind) return false;
      if (!this.disposed && kind !== this.snapshot.agentSignal)
        this.update({ agentSignal: kind });
      if (!this.disposed && shouldNotify(kind) && kind !== "working")
        emitAgentNotification({
          paneId: this.paneId,
          sessionId: this.sessionId,
          kind,
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
  observeControl(agentControlled: boolean) {
    if (agentControlled !== this.snapshot.agentControlled)
      this.update({ agentControlled });
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
  setView(view: "searchOpen" | "composerOpen" | "blocksOpen", open: boolean) {
    this.update({ [view]: open });
  }
  toggleView(view: "searchOpen" | "composerOpen" | "blocksOpen") {
    this.setView(view, !this.snapshot[view]);
  }
  private update(patch: Partial<Snapshot>) {
    if (this.disposed) return;
    this.snapshot = { ...this.snapshot, ...patch };
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
    if (measureFont) await loadTerminalFonts(this.terminal.options);
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
    if (measureFont) {
      // Replacing xterm's DOM renderer changes styles and can invalidate WebKit's
      // loaded font faces. Its WebGL atlas may already contain fallback glyphs.
      await loadTerminalFonts(this.terminal.options);
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
          { duration: 180, easing: "ease-out" },
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
          await api("resize_terminal", { id: this.sessionId, ...size });
        } catch (error) {
          if (!this.disposed && this.snapshot.status === "running")
            reportError(errorMessage(error));
        }
      }
    } finally {
      this.resizing = false;
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
        const started = await api<{ cwd: string }>("start_terminal", {
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
        if (!this.eof) this.update({ status: "running", cwd });
        directoryListener(this.paneId, cwd);
        this.scheduleFit();
      } catch (error) {
        this.update({ status: "error", error: errorMessage(error) });
      }
    })();
    return this.startPromise;
  }

  private finishExit() {
    if (!this.eof || this.exitCode === undefined || this.disposed) return;
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

  send(data: string) {
    if (this.pasteOutput) {
      this.pasteOutput.push(data);
      return;
    }
    void this.queueInput(() => this.writeInput(data));
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

  private async writeInput(data: string) {
    this.observeControl(false);
    // Accept titles after submission even when a shell has no pre-execution hook.
    if (data === "\r" || data === "\n") this.atPrompt = false;
    if (
      data === "\r" &&
      ["cmd", "pwsh", "powershell"].includes(this.profile.kind)
    )
      this.startBlock();
    for (const chunk of inputChunks(data)) {
      if (this.disposed) return;
      await api("write_terminal", { id: this.sessionId, data: chunk });
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
    if (
      this.snapshot.title ||
      this.snapshot.titleBusy ||
      this.snapshot.agentSignal ||
      this.snapshot.foregroundProgram
    )
      this.update({
        title: "",
        titleBusy: false,
        agentSignal: null,
        foregroundProgram: "",
      });
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

  dispose() {
    if (this.disposed) return;
    window.removeEventListener(themeAppliedEvent, this.applyTheme);
    this.disposed = true;
    this.host.removeEventListener("paste", this.onPaste, true);
    this.detach();
    clearTimeout(this.ackTimer);
    this.listeners.clear();
    this.terminal.dispose();
    void this.startPromise?.finally(() =>
      api("close_terminal", { id: this.sessionId }).catch(() => {}),
    );
  }
}

const runtimes = new Map<string, TerminalRuntime>();
export function terminalFor(
  pane: Pane,
  profile: ShellProfile,
): TerminalRuntime {
  let runtime = runtimes.get(pane.id);
  if (!runtime) {
    runtime = new TerminalRuntime(pane.id, profile, pane.cwd);
    runtimes.set(pane.id, runtime);
  }
  return runtime;
}
export const runningTerminal = (id: string) => runtimes.get(id);
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
export function closeTerminals(ids: string[]) {
  for (const id of ids) {
    runtimes.get(id)?.dispose();
    runtimes.delete(id);
  }
}
export function observeTerminalContexts(
  contexts: Record<string, TerminalContext>,
) {
  const result: Record<string, string> = {};
  for (const [id, runtime] of runtimes) {
    const context = contexts[runtime.sessionId];
    runtime.observeForegroundProgram(context?.foregroundProgram ?? null);
    runtime.observeControl(context?.agentControlled ?? false);
    if (context?.cwd) result[id] = context.cwd;
  }
  return result;
}
if (import.meta.hot)
  import.meta.hot.dispose(() => closeTerminals([...runtimes.keys()]));
