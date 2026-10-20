import { Terminal } from "@xterm/xterm";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { emitTo } from "@tauri-apps/api/event";
import { TerminalRuntime } from "../../src/terminal-runtime";

// Loaded before the actual main module only by the fixture Vite configuration.
const runtimes = new Map();
const tabs = new Map();
const samples = [];
const stages = [];
const workloads = [];
const now = () => performance.now();
const trace = (sample, field, value) => {
  if (!sample) return;
  if (sample[field].length < 200) sample[field].push(value);
  else sample.traceOverflow[field] = (sample.traceOverflow[field] ?? 0) + 1;
};
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const nextFrame = () =>
  new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      cancelAnimationFrame(frame);
      reject(Error(`Timeout waiting for animation frame: ${checkpoint}`));
    }, 5000);
    const frame = requestAnimationFrame(() => {
      clearTimeout(timer);
      resolve();
    });
  });
const invoke = (stage, data = null) =>
  window.__TAURI_INTERNALS__.invoke("plugin_smoke_result", { stage, data });
let checkpoint = "bootstrap";
let platform;
let current;
let animationFrames = 0;
let lastAnimationFrame;
const observeFrame = () => {
  animationFrames++;
  lastAnimationFrame = now();
  requestAnimationFrame(observeFrame);
};
requestAnimationFrame(observeFrame);
const presentation = () => ({
  visibility: document.visibilityState,
  focused: document.hasFocus(),
  animationFrames,
  sinceAnimationFrameMs:
    lastAnimationFrame === undefined ? null : now() - lastAnimationFrame,
  panes: visible().map((runtime) => {
    const service = runtime.terminal._core?._renderService;
    return {
      pane: runtime.paneId,
      opacity: getComputedStyle(runtime.host).opacity,
      renderer: runtime.getSnapshot().renderer,
      paused: service?._isPaused,
      fullRefresh: service?._needsFullRefresh,
      synchronizedOutput:
        runtime.terminal._core?.coreService?.decPrivateModes
          ?.synchronizedOutput,
      rows: runtime.terminal.rows,
      cols: runtime.terminal.cols,
      rect: runtime.host.getBoundingClientRect().toJSON(),
    };
  }),
});
const screen = (terminal) =>
  Array.from(
    { length: terminal.buffer.active.length },
    (_, i) => terminal.buffer.active.getLine(i)?.translateToString(true) ?? "",
  ).join("");
const match = (terminal, phase) => {
  if (
    current?.terminal === terminal &&
    current[phase] === undefined &&
    screen(terminal).includes(current.marker)
  )
    current[phase] = now();
};
const open = Terminal.prototype.open;
Terminal.prototype.open = function (...args) {
  const result = open.apply(this, args);
  this.onData((data) => {
    if (current?.terminal === this) {
      current.coreData += data;
      trace(current, "onDataEvents", { at: now(), unixMs: Date.now(), data });
    }
  });
  this.onRender(({ start, end }) => {
    if (current?.terminal === this)
      trace(current, "renderEvents", { at: now(), start, end });
    match(this, "rendered");
  });
  return result;
};
const write = Terminal.prototype.write;
Terminal.prototype.write = function (data, callback) {
  const sample = current?.terminal === this ? current : undefined;
  const event = {
    begin: now(),
    unixMs: Date.now(),
    bytes: data.byteLength ?? data.length,
  };
  trace(sample, "writeEvents", event);
  return write.call(this, data, () => {
    if (sample) {
      event.parsed = now();
      event.parsedUnixMs = Date.now();
    }
    match(this, "parsed");
    callback?.();
  });
};
const attach = TerminalRuntime.prototype.attach;
TerminalRuntime.prototype.attach = function (...args) {
  runtimes.set(this.paneId, this);
  const result = attach.apply(this, args);
  const id = this.host
    .closest('.terminal-stage[role="tabpanel"]')
    ?.getAttribute("aria-labelledby");
  if (id) tabs.set(this.paneId, id);
  return result;
};
const visible = () =>
  [...runtimes.values()]
    .map((runtime) => {
      // Dockview can attach the host before React places its enclosing tabpanel.
      // Remember the actual tab while the retained host is connected and visible.
      if (runtime.host.isConnected && runtime.host.checkVisibility()) {
        const id = runtime.host
          .closest('.terminal-stage[role="tabpanel"]')
          ?.getAttribute("aria-labelledby");
        if (id) tabs.set(runtime.paneId, id);
      }
      return runtime;
    })
    .filter(
      (r) =>
        r.host.isConnected &&
        r.host.getBoundingClientRect().width > 0 &&
        r.host.checkVisibility(),
    );
const wait = async (predicate, timeout = 30000) => {
  const end = now() + timeout;
  while (now() < end) {
    if (predicate()) return;
    await pause(10);
  }
  throw Error(`Timeout: ${checkpoint}`);
};
const require = (condition, message) => {
  if (!condition) throw Error(message);
};
const shortcut = (code, shiftKey) =>
  window.dispatchEvent(
    new KeyboardEvent("keydown", {
      key: code.slice(3),
      code,
      ctrlKey: platform !== "macos",
      metaKey: platform === "macos",
      shiftKey,
      bubbles: true,
      cancelable: true,
    }),
  );
const native = (text) => invoke("regression-input", text);
const report = () => ({
  checkpoint,
  presentation: presentation(),
  samples: samples.map(({ terminal, ...sample }) => sample),
  stages,
  workloads,
  limits: [
    "Native committed text: AppKit insertText or owned foreground HWND WM_CHAR; Windows Return uses bounded VK_RETURN keydown/up; no physical keyboard injection.",
    "Shell integration OSC133 prompt readiness precedes ordinary input; two exact startup samples are excluded from timing acceptance.",
    "Final-character request to xterm parser and onRender software callbacks includes IPC; no compositor/input-to-photon claim.",
    "Tab and split shortcuts are DOM key events handled by the real application.",
    "Corrupt-session recovery is covered by existing UI recovery tests, not this fixture.",
  ],
});
try {
  const config = await invoke("regression-bootstrap");
  platform = config.platform;
  await import("../../src/main.tsx");
  checkpoint = "initial running terminal";
  await invoke("regression-present");
  stages.push({
    label: "initial-presentation",
    native: await invoke("regression-window-state"),
  });
  await invoke("regression-checkpoint", report());
  await wait(() => visible()[0]?.getSnapshot().status === "running");
  const first = visible()[0];
  const firstTab = tabs.get(first.paneId);
  const initialPids = await invoke("regression-inspect");
  require(initialPids[first.sessionId], "Missing initial PTY process identity");
  const verifyPids = async (label) => {
    const pids = await invoke("regression-inspect");
    for (const runtime of runtimes.values())
      require(pids[runtime.sessionId] >
        0, `Missing native PID for ${runtime.paneId}`);
    require(pids[first.sessionId] ===
      initialPids[first.sessionId], "Retained first PTY PID changed");
    stages.push({
      label,
      pids,
      runtimeCount: runtimes.size,
      visiblePanes: visible().length,
    });
    await invoke("regression-checkpoint", report());
    return pids;
  };
  const input = async (runtime, label, count = 1, workload) => {
    if (!workload)
      await wait(
        () => runtime.promptEnd && !runtime.promptEnd.marker.isDisposed,
      );
    runtime.terminal.focus();
    await pause(30);
    for (let n = 0; n < count; n++) {
      if (workload) {
        for (const active of workloads) {
          const runtime = runtimes.get(active.pane);
          require(!screen(runtime.terminal).includes(
            `REGRESSION_DONE ${active.token} pid=${active.pid}`,
          ) &&
            screen(runtime.terminal).includes(
              `REGRESSION_READY ${active.token} pid=${active.pid}`,
            ), "All eight ANSI programs must remain live during each sampled input");
        }
      }
      const marker = `r${String(samples.length).padStart(5, "0")}z`;
      current = {
        marker,
        label,
        pane: runtime.paneId,
        session: runtime.sessionId,
        terminal: runtime.terminal,
        coreData: "",
        started: now(),
        native: [],
        onDataEvents: [],
        writeEvents: [],
        renderEvents: [],
        nativeRequests: [],
        traceOverflow: {},
      };
      samples.push(current);
      // The ordinary shell executes a portable echo command, leaving no draft
      // or cleanup shortcut that depends on shell line-editing bindings.
      const text = `echo ${marker}`;
      for (const [i, char] of [...text].entries()) {
        if (i === text.length - 1) {
          current.submit = now();
          current.submitUnixMs = Date.now();
        }
        const request = { char, requested: now(), requestedUnixMs: Date.now() };
        current.native.push(await native(char));
        request.returned = now();
        request.returnedUnixMs = Date.now();
        trace(current, "nativeRequests", request);
        if (i < text.length - 1) await pause(8);
      }
      await wait(
        () => current.parsed !== undefined && current.rendered !== undefined,
        10000,
      );
      require(current.coreData ===
        text, `Input loss/replay: expected ${JSON.stringify(text)}, got ${JSON.stringify(current.coreData)}`);
      current.exactInputVerified = true;
      current.submitToParsedMs = current.parsed - current.submit;
      current.submitToRenderMs = current.rendered - current.submit;
      current.focus =
        current.native.at(-1).keyWindow === false ? "inactive" : "foreground";
      // Keep listening through return and a short settle, rejecting delayed replay.
      const previousPrompt = runtime.promptEnd;
      current.nativeReturn = await native("\r");
      if (!workload) {
        await wait(
          () =>
            runtime.promptEnd &&
            runtime.promptEnd !== previousPrompt &&
            !runtime.promptEnd.marker.isDisposed,
        );
        current.freshPromptVerified = true;
      }
      await pause(50);
      require(current.coreData ===
        `${text}\r`, `Unexpected trailing native input: ${JSON.stringify(current.coreData)}`);
      if (workload) workload.expected += `${text}\r`;
      current = undefined;
      await pause(30);
    }
  };
  const activate = async (runtime) => {
    const id = tabs.get(runtime.paneId);
    require(id &&
      document.getElementById(
        id,
      ), `Missing actual UI tab for ${runtime.paneId}`);
    document.getElementById(id).click();
    await wait(
      () =>
        visible().includes(runtime) &&
        runtime.getSnapshot().status === "running",
    );
  };
  const actualTabButtons = () => [
    ...document.querySelectorAll('[role="tab"][id^="tab-"]'),
  ];
  const visitActualTabs = async (expectedCount) => {
    await wait(() => actualTabButtons().length === expectedCount);
    const ids = actualTabButtons().map((button) => button.id);
    for (const id of ids) {
      // Discover through the real tab UI: a never-visited runtime may not yet
      // have an enclosing React tabpanel when Dockview first attaches it.
      const button = document.getElementById(id);
      require(button, `Missing actual tab button ${id}`);
      button.click();
      await pause(100);
      await nextFrame();
      await nextFrame();
      await wait(() => {
        const panes = visible(); // Refresh the runtime-to-tab map after mounting.
        return (
          document.getElementById(id)?.getAttribute("aria-selected") ===
            "true" &&
          panes.length > 0 &&
          panes.every(
            (runtime) =>
              tabs.get(runtime.paneId) === id &&
              runtime.getSnapshot().status === "running",
          )
        );
      });
      stages.push({
        label: "actual-tab-visited",
        tab: id,
        panes: visible().map((runtime) => runtime.paneId),
      });
    }
  };
  checkpoint = "shell integration prompt and unmeasured startup warmup";
  await wait(() => first.promptEnd && !first.promptEnd.marker.isDisposed);
  await pause(250);
  await input(first, "startup-warmup-unmeasured", 2);
  stages.push({
    label: "startup-warmup-completed",
    excludedFromTimingAcceptance: true,
  });
  if (config.phase === "exercise") {
    checkpoint = "sustained ordinary shell typing";
    const started = now();
    while (now() - started < 25000) {
      await input(first, "ordinary-shell-25s");
      await pause(100);
    }
    stages.push({
      label: "ordinary-shell-duration",
      durationMs: now() - started,
    });
    for (const total of [2, 4, 8]) {
      checkpoint = `rapid ${total} tabs`;
      for (let n = actualTabButtons().length; n < total; n++) {
        shortcut("KeyT", true);
        await pause(35);
      }
      await visitActualTabs(total);
      await wait(() => runtimes.size === total);
      // Every actual tab has now mounted and populated its pane mapping.
      for (const runtime of runtimes.values()) await activate(runtime);
      await verifyPids(`tabs-${total}`);
      await input(visible()[0], `tabs-${total}`, 3);
    }
    await activate(first);
    for (const total of [2, 4, 8]) {
      checkpoint = `rapid ${total} visible panes`;
      while (visible().length < total) {
        const largest = visible().sort((a, b) => {
          const x = a.host.getBoundingClientRect(),
            y = b.host.getBoundingClientRect();
          return y.width * y.height - x.width * x.height;
        })[0];
        largest.terminal.focus();
        await pause(30);
        const bounds = largest.host.getBoundingClientRect();
        const before = visible().length;
        shortcut("KeyD", bounds.height > bounds.width);
        await wait(() => visible().length > before);
      }
      await wait(
        () =>
          visible().length === total &&
          visible().every((r) => r.getSnapshot().status === "running"),
      );
      await verifyPids(`panes-${total}`);
      for (const runtime of visible())
        await input(runtime, `panes-${total}`, 2);
    }
    checkpoint = "eight finite ANSI workloads with native input";
    const quote = (text) =>
      "'" +
      text.replaceAll("'", config.platform === "windows" ? "''" : "'\\''") +
      "'";
    const active = visible();
    for (const [i, runtime] of active.entries()) {
      const token = `w${i}`;
      const workload = { token, pane: runtime.paneId, expected: "" };
      workloads.push(workload);
      const command = `${config.platform === "windows" ? "& " : ""}${quote(config.node)} ${quote(`${config.directory}/project/terminal-regression-workload.mjs`)} ${token}`;
      workload.launchCommand = command;
      workload.launchRequested = now();
      workload.launchRequestedUnixMs = Date.now();
      checkpoint = `ANSI workload ${token} launch and READY`;
      await invoke("regression-checkpoint", report());
      runtime.execute(command);
      await wait(() =>
        screen(runtime.terminal).includes(`REGRESSION_READY ${token} pid=`),
      );
      workload.pid = Number(
        screen(runtime.terminal)
          .split(`REGRESSION_READY ${token} pid=`)
          .at(-1)
          .match(/^\d+/)?.[0],
      );
      require(workload.pid > 0, "Missing workload native process identity");
      workload.ready = now();
      workload.readyUnixMs = Date.now();
    }
    checkpoint = "eight finite ANSI workloads exact native input";
    await invoke("regression-checkpoint", report());
    for (const [i, runtime] of active.entries())
      await input(runtime, "ansi-eight", 3, workloads[i]);
    for (const [i, runtime] of active.entries()) {
      const workload = workloads[i];
      await wait(
        () =>
          screen(runtime.terminal).includes(
            `REGRESSION_DONE ${workload.token} pid=${workload.pid}`,
          ),
        30000,
      );
      workload.metrics = await invoke("regression-workload", workload.token);
      require(workload.metrics.completedNormally &&
        workload.metrics.pid === workload.pid &&
        workload.metrics.input === workload.expected &&
        workload.metrics.durationMs >= 19900 &&
        workload.metrics.frames >=
          250, `Workload failed or exact input mismatch: ${JSON.stringify(workload)}`);
    }
    // Switch away and back through UI, retaining runtime objects and PIDs.
    const other = [...runtimes.values()].find(
      (r) => tabs.get(r.paneId) !== firstTab,
    );
    await activate(other);
    await input(other, "switch-away", 2);
    await activate(first);
    require(runtimes.get(first.paneId) ===
      first, "Retained runtime object replaced");
    await input(first, "switch-return", 3);
    if (config.platform === "macos") {
      checkpoint = "native macOS close hides and reopens retained terminals";
      const beforeHide = await verifyPids("macos-before-close-hide");
      const window = getCurrentWindow();
      const waitVisibility = async (expected) => {
        const deadline = now() + 10000;
        while (now() < deadline) {
          if ((await window.isVisible()) === expected) return;
          await pause(25);
        }
        throw Error(`Timeout: native window visibility must be ${expected}`);
      };
      await window.close();
      await waitVisibility(false);
      stages.push({ label: "macos-native-close-hidden", visible: false });
      const hiddenPids = await verifyPids("macos-hidden-retained");
      require(JSON.stringify(hiddenPids) ===
        JSON.stringify(
          beforeHide,
        ), "Native close changed retained PTYs while hidden");
      await invoke("regression-reopen");
      await waitVisibility(true);
      await activate(first);
      first.terminal.focus();
      await pause(100);
      require(runtimes.get(first.paneId) ===
        first, "Native close/reopen replaced the retained runtime");
      const reopenedPids = await verifyPids("macos-reopened-retained");
      require(JSON.stringify(reopenedPids) ===
        JSON.stringify(beforeHide), "Native reopen changed retained PTYs");
      await input(first, "macos-native-close-reopen", 2);
    }
  } else {
    checkpoint = "restore eight tabs and first tab's eight panes";
    await wait(() => visible().length === 8);
    // Recreate every saved tab's PTY through the actual tab buttons first.
    await visitActualTabs(8);
    // All restored pane mappings are now available for runtime activation.
    for (const runtime of runtimes.values()) await activate(runtime);
    await wait(
      () =>
        runtimes.size === 15 &&
        [...runtimes.values()].every(
          (r) => r.getSnapshot().status === "running",
        ),
    );
    await activate(first);
    await input(first, "restored-native-input", 5);
  }
  checkpoint = "product quit and guarded session save";
  const pids = await verifyPids(`${config.phase}-before-quit`);
  const summary = Object.fromEntries(
    [
      ...new Set(
        samples
          .filter((s) => s.label !== "startup-warmup-unmeasured")
          .map((s) => `${s.label}/${s.focus}`),
      ),
    ].map((label) => {
      const values = samples
        .filter((s) => `${s.label}/${s.focus}` === label)
        .map((s) => s.submitToRenderMs)
        .sort((a, b) => a - b);
      return [
        label,
        {
          count: values.length,
          p95Ms: values[Math.ceil(values.length * 0.95) - 1],
          maxMs: values.at(-1),
        },
      ];
    }),
  );
  require(samples.length > 0 &&
    samples.every(
      (s) => s.exactInputVerified,
    ), "Missing verified input samples");
  // The runner evaluates the timing thresholds and complete saved session only
  // after observing normal native process exit through the real quit guard.
  await invoke("ready-to-quit", {
    ...report(),
    pids,
    summary,
    panePids: Object.fromEntries(
      [...runtimes.values()].map((r) => [r.paneId, pids[r.sessionId]]),
    ),
    expectedTabs: 8,
    expectedPanes: 15,
  });
  if (config.platform === "windows" && config.phase === "exercise")
    await getCurrentWindow().close();
  else await emitTo("main", "lomi-quit-requested");
} catch (error) {
  await invoke("failed", {
    ...report(),
    nativePresentation: await invoke("regression-window-state").catch(String),
    error: String(error),
    stack: error?.stack,
    // Only our disposable project's generated echo/workload shell output.
    failureScreens: visible()
      .slice(0, 8)
      .map((runtime) => ({
        pane: runtime.paneId,
        session: runtime.sessionId,
        screenTail: screen(runtime.terminal).slice(-1500),
      })),
  });
}
