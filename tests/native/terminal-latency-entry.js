import { Terminal } from "@xterm/xterm";
import { Channel } from "@tauri-apps/api/core";
import { TerminalRuntime } from "../../src/terminal-runtime";

// This module is reachable only through the diagnostic Vite index transform.
const now = () => performance.now();
const events = [];
const samples = [];
const runtimes = new Map();
const tabByPane = new Map();
const deliveredInput = [];
const heartbeats = [];
const contexts = [];
const snapshots = [];
const workloads = [];
const slowSessionSaves = [];
const activeWorkloads = new Map();
let rendererMode = "baseline";
let checkpoint = "bootstrap";
let current;
const record = (phase, extra = {}) =>
  events.push({ phase, at: now(), ...extra });
const screen = (terminal) =>
  Array.from({ length: terminal.buffer.active.length }, (_, i) =>
    terminal.buffer.active.getLine(i)?.translateToString(),
  ).join("\n");
const match = (terminal, phase) => {
  if (
    current &&
    current.terminal === terminal &&
    screen(terminal).replaceAll("\n", "").includes(current.marker) &&
    current[phase] === undefined
  )
    current[phase] = now();
};
const internal = window.__TAURI_INTERNALS__;
const originalInvoke = internal.invoke.bind(internal);
const descriptor = Object.getOwnPropertyDescriptor(
  Channel.prototype,
  "onmessage",
);
if (!descriptor?.set) throw Error("Tauri Channel onmessage setter unavailable");
Object.defineProperty(Channel.prototype, "onmessage", {
  ...descriptor,
  set(handler) {
    descriptor.set.call(this, (data) => {
      if (data instanceof ArrayBuffer)
        record("channel-receipt", {
          bytes: data.byteLength,
          marker: current?.marker,
        });
      return handler(data);
    });
  },
});
const originalOpen = Terminal.prototype.open;
Terminal.prototype.open = function (...args) {
  const result = originalOpen.apply(this, args);
  this.onData((data) => {
    deliveredInput.push({ terminal: this, data });
    if (current?.terminal === this) current.coreData += data;
    record("core-onData", { data, marker: current?.marker });
  });
  this.onRender(({ start, end }) => {
    record("onRender", { start, end, marker: current?.marker });
    match(this, "rendered");
  });
  return result;
};
const originalWrite = Terminal.prototype.write;
Terminal.prototype.write = function (data, callback) {
  const started = now();
  record("xterm-write", { bytes: data.length, marker: current?.marker });
  return originalWrite.call(this, data, () => {
    record("parser-callback", {
      writeStarted: started,
      marker: current?.marker,
    });
    match(this, "parsed");
    callback?.();
  });
};
for (const method of [
  "attach",
  "initializeWebgl",
  "start",
  "send",
  "writeInput",
]) {
  const original = TerminalRuntime.prototype[method];
  TerminalRuntime.prototype[method] = function (...args) {
    runtimes.set(this.paneId, this);
    const started = now();
    record(`${method}-begin`, {
      pane: this.paneId,
      session: this.sessionId,
      marker: current?.marker,
    });
    const result = original.apply(this, args);
    if (method === "attach") {
      const tabId = this.host
        .closest('.terminal-stage[role="tabpanel"]')
        ?.getAttribute("aria-labelledby");
      if (tabId) tabByPane.set(this.paneId, tabId);
    }
    Promise.resolve(result).then(
      () =>
        record(`${method}-end`, {
          pane: this.paneId,
          duration: now() - started,
          snapshot: this.getSnapshot(),
        }),
      (error) =>
        record(`${method}-error`, { pane: this.paneId, error: String(error) }),
    );
    return result;
  };
}
const originalContext = HTMLCanvasElement.prototype.getContext;
HTMLCanvasElement.prototype.getContext = function (type, ...args) {
  if (rendererMode === "dom" && type === "webgl2") {
    record("fixture-dom-webgl2-denied");
    return null;
  }
  const context = originalContext.call(this, type, ...args);
  if (
    (type === "webgl" || type === "webgl2") &&
    context &&
    !contexts.some((item) => item.context === context)
  ) {
    const id = contexts.length;
    contexts.push({ context, canvas: this });
    record("webgl-context-created", { id, type });
    this.addEventListener("webglcontextlost", (event) =>
      record("webgl-context-lost", { id, status: event.statusMessage }),
    );
    this.addEventListener("webglcontextrestored", () =>
      record("webgl-context-restored", { id }),
    );
  }
  return context;
};
let heartbeat = now();
setInterval(() => {
  const at = now();
  heartbeats.push({ at, gap: at - heartbeat });
  heartbeat = at;
}, 16);
document.fonts.addEventListener("loading", () => record("fonts-loading"));
document.fonts.addEventListener("loadingdone", () =>
  record("fonts-loadingdone"),
);
const invoke = (stage, data = null) =>
  originalInvoke("plugin_smoke_result", { stage, data });
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const wait = async (predicate, timeout = 30000) => {
  const until = now() + timeout;
  while (now() < until) {
    if (predicate()) return;
    await pause(5);
  }
  throw Error(`Timeout: ${checkpoint}`);
};
const shortcut = (key, code, shiftKey = false) =>
  window.dispatchEvent(
    new KeyboardEvent("keydown", {
      key,
      code,
      ctrlKey: true,
      shiftKey,
      bubbles: true,
      cancelable: true,
    }),
  );
const rememberVisibleTab = (runtime) => {
  if (runtime.host.isConnected) {
    const tabId = runtime.host
      .closest('.terminal-stage[role="tabpanel"]')
      ?.getAttribute("aria-labelledby");
    if (tabId) tabByPane.set(runtime.paneId, tabId);
  }
  return runtime;
};
const visible = () =>
  [...runtimes.values()]
    .map(rememberVisibleTab)
    .find(
      (runtime) =>
        runtime.host.isConnected &&
        runtime.host.getBoundingClientRect().width > 0 &&
        runtime.host.closest('.terminal-stage[role="tabpanel"]')?.hidden !==
          true &&
        runtime.host.checkVisibility(),
    );
const visiblePanes = () =>
  [...runtimes.values()]
    .map(rememberVisibleTab)
    .filter(
      (runtime) =>
        runtime.host.isConnected &&
        runtime.host.getBoundingClientRect().width > 0 &&
        runtime.host.checkVisibility(),
    );
const area = (runtime) => {
  const bounds = runtime.host.getBoundingClientRect();
  return bounds.width * bounds.height;
};
const report = () => ({
  checkpoint,
  rendererMode,
  snapshots,
  workloads,
  slowSessionSaves,
  events,
  samples: samples.map(({ terminal, ...sample }) => sample),
  heartbeats,
  contexts: contexts.map(({ canvas }, id) => ({
    id,
    connected: canvas.isConnected,
    width: canvas.width,
    height: canvas.height,
  })),
  limits: [
    "Primary submitToParsedMs/submitToRenderMs measure the final native key request through the complete unique marker software event. Prefix typing pauses and IPC calls are included only in descriptive wholeMarkerToRenderMs/typingDurationMs.",
    "Owned GTK widget keypress/release dispatch; not physical keyboard/compositor input.",
    "onRender and parser completion are separate software proxies, not physical presentation.",
    "GTK recent presentation times may be unavailable or unrelated to the glyph; native WebKit snapshots are saved for inspection, without automated visible-glyph assertions.",
    "Native WebKit snapshots can flush rendering and add workload; they are fixed checkpoints outside measured input samples, not compositor screenshots.",
    "Channel receipt events include all terminal output channels and are correlated by sample window, not payload identity.",
    "writeInput-begin precedes the real write invoke; no Tauri immutable IPC globals are replaced.",
  ],
});
try {
  const bootstrap = await invoke("latency-bootstrap");
  rendererMode = bootstrap.rendererMode;
  record("fixture-renderer-mode", { rendererMode });
  const snapshot = async (name) => {
    const requested = now();
    record("native-snapshot-request", { checkpoint: name });
    const native = await originalInvoke("terminal_latency_snapshot", {
      token: bootstrap.token,
      checkpoint: name,
    });
    const completed = now();
    snapshots.push({
      ...native,
      jsRequestedAt: requested,
      jsCompletedAt: completed,
      jsRequestDurationMs: completed - requested,
      sinceLastSampleRenderedMs: samples.length
        ? requested - samples.at(-1).rendered
        : null,
    });
    record("native-snapshot-complete", {
      checkpoint: name,
      jsRequestDurationMs: completed - requested,
    });
    await invoke("latency-checkpoint", report());
  };
  await invoke("latency-checkpoint", report());
  await import("../../src/main.tsx");
  checkpoint = "initial terminal startup";
  await wait(() => visible()?.getSnapshot().status === "running");
  const first = visible();
  const originalPids = await invoke("latency-inspect");
  const retainedTabId = first.host
    .closest('.terminal-stage[role="tabpanel"]')
    ?.getAttribute("aria-labelledby");
  record("startup-running", { pane: first.paneId });
  const requireWorkload = (runtime, workload) => {
    if (
      workload &&
      (runtime.terminal.buffer.active.type !== "alternate" ||
        !screen(runtime.terminal)
          .replaceAll("\n", "")
          .includes(`LATENCY_TUI_READY ${workload.token} pid=${workload.pid}`))
    ) {
      throw Error(
        `Expected active ANSI workload ${workload.token}/${workload.pid} before continuing input`,
      );
    }
  };
  const input = async (runtime, count, label, beforeLast) => {
    const workload = activeWorkloads.get(runtime.paneId);
    requireWorkload(runtime, workload);
    runtime.terminal.focus();
    await pause(30);
    for (let i = 0; i < count; i++) {
      requireWorkload(runtime, workload);
      if (workload) {
        workload.expected.ctrlUClears++;
        workload.expected.receivedBytes++;
      }
      const cleanupStart = deliveredInput.length;
      await originalInvoke("terminal_latency_input", {
        token: bootstrap.token,
        text: "\x15",
      });
      await wait(
        () =>
          deliveredInput
            .slice(cleanupStart)
            .filter((item) => item.terminal === runtime.terminal)
            .map((item) => item.data)
            .join("") === "\x15",
      );
      await pause(20);
      const marker = `l${samples.length.toString().padStart(3, "0")}z`;
      current = {
        marker,
        label,
        pane: runtime.paneId,
        session: runtime.sessionId,
        terminal: runtime.terminal,
        coreData: "",
        typingStarted: now(),
        submit: undefined,
      };
      samples.push(current);
      current.nativeKeys = [];
      for (const [index, character] of [...marker].entries()) {
        requireWorkload(runtime, workload);
        if (workload) {
          workload.expected.printableBytes++;
          workload.expected.receivedBytes++;
        }
        if (index === marker.length - 1) {
          if (beforeLast) await beforeLast();
          current.submit = now();
          current.submitUnixMs = Date.now();
        }
        current.nativeKeys.push(
          await originalInvoke("terminal_latency_input", {
            token: bootstrap.token,
            text: character,
          }),
        );
        if (index === marker.length - 1) current.nativeReturned = now();
        else await pause(8);
      }
      current.native = current.nativeKeys.at(-1);
      await wait(
        () => current.parsed !== undefined && current.rendered !== undefined,
        10000,
      );
      await pause(5);
      if (current.coreData !== marker)
        throw Error(
          `Input replay/loss for ${marker}: ${JSON.stringify(current.coreData)}`,
        );
      requireWorkload(runtime, workload);
      current.exactInputVerified = true;
      current.submitToParsedMs = current.parsed - current.submit;
      current.submitToRenderMs = current.rendered - current.submit;
      current.dispatchToRenderBoundsMs = [
        Math.max(0, current.rendered - current.nativeReturned),
        current.submitToRenderMs,
      ];
      current.nativeDispatchDurationMs =
        (current.native.dispatchEndUs - current.native.dispatchStartUs) / 1000;
      current.allKeyDispatchDurationMs = current.nativeKeys.reduce(
        (sum, key) => sum + (key.dispatchEndUs - key.dispatchStartUs) / 1000,
        0,
      );
      current.typingDurationMs = current.nativeReturned - current.typingStarted;
      current.wholeMarkerToRenderMs = current.rendered - current.typingStarted;
      current = undefined;
      if (label === "steady-1" && i === 0) {
        const measured = samples.at(-1);
        await new Promise((resolve) => requestAnimationFrame(resolve));
        await snapshot("first-input-next-frame");
        await new Promise((resolve) => requestAnimationFrame(resolve));
        await snapshot("first-input-two-frames");
        await pause(Math.max(0, measured.rendered + 100 - now()));
        await snapshot("first-input-100ms");
        await pause(Math.max(0, measured.rendered + 500 - now()));
        await snapshot("first-input-500ms");
      }
      await pause(30);
    }
  };
  const startWorkload = async (runtime, label) => {
    runtime.terminal.focus();
    const begin = deliveredInput.length;
    await originalInvoke("terminal_latency_input", {
      token: bootstrap.token,
      text: "\x15",
    });
    await wait(
      () =>
        deliveredInput
          .slice(begin)
          .filter((item) => item.terminal === runtime.terminal)
          .map((item) => item.data)
          .join("") === "\x15",
    );
    const source = `${bootstrap.directory}/project/terminal-latency-workload.py`;
    const quote = (value) => "'" + value.replaceAll("'", "'\\''") + "'";
    const started = now();
    const token = `w${workloads.length.toString(36).padStart(4, "0")}`;
    runtime.execute(`python3 ${quote(source)} --token ${token}`);
    await wait(
      () =>
        runtime.terminal.buffer.active.type === "alternate" &&
        screen(runtime.terminal)
          .replaceAll("\n", "")
          .includes(`LATENCY_TUI_READY ${token} pid=`),
    );
    const pidMatch = screen(runtime.terminal)
      .replaceAll("\n", "")
      .split(`LATENCY_TUI_READY ${token} pid=`)[1]
      ?.match(/^(\d+)/);
    if (!pidMatch)
      throw Error(`Missing ANSI workload process identity ${token}`);
    const workload = {
      token,
      pid: Number(pidMatch[1]),
      expected: { printableBytes: 0, ctrlUClears: 0, receivedBytes: 0 },
      label,
      pane: runtime.paneId,
      session: runtime.sessionId,
      started,
      ready: now(),
    };
    workloads.push(workload);
    activeWorkloads.set(runtime.paneId, workload);
    return { runtime, workload };
  };
  const finishWorkload = async ({ runtime, workload }) => {
    await wait(
      () =>
        runtime.terminal.buffer.active.type === "normal" &&
        screen(runtime.terminal)
          .replaceAll("\n", "")
          .includes(`LATENCY_TUI_DONE ${workload.token} pid=${workload.pid}`),
      Math.max(1000, workload.started + 25000 - now()),
    );
    workload.completed = now();
    const rows = Array.from(
      { length: runtime.terminal.buffer.active.length },
      (_, i) =>
        runtime.terminal.buffer.active.getLine(i)?.translateToString(true) ??
        "",
    );
    // Python writes one short JSON statistics record, which may wrap across rows.
    const output = rows.join("");
    const prefix = `LATENCY_TUI_STATS ${workload.token} `;
    const start = output.indexOf(prefix);
    const raw =
      start >= 0
        ? output.slice(start + prefix.length).match(/^\{[^}]+\}/)?.[0]
        : undefined;
    if (!raw) throw Error(`Missing workload output metrics: ${workload.label}`);
    workload.metrics = JSON.parse(raw);
    const metrics = workload.metrics;
    if (
      metrics.token !== workload.token ||
      metrics.pid !== workload.pid ||
      !metrics.success ||
      !metrics.completedNormally ||
      metrics.error !== null
    )
      throw Error(
        `ANSI workload ${workload.token} did not complete normally: ${JSON.stringify(metrics)}`,
      );
    if (
      metrics.durationSeconds < 19.5 ||
      metrics.durationSeconds > 25 ||
      metrics.actualFramesPerSecond < 27 ||
      metrics.outputBytes < 1
    )
      throw Error(
        `ANSI workload performance failure ${workload.token}: ${JSON.stringify(metrics)}`,
      );
    for (const field of ["printableBytes", "ctrlUClears", "receivedBytes"])
      if (metrics[field] !== workload.expected[field])
        throw Error(
          `ANSI workload ${workload.token} ${field}: expected ${workload.expected[field]}, received ${metrics[field]}`,
        );
    workload.exactWorkloadInputVerified = true;
    activeWorkloads.delete(runtime.paneId);
    await invoke("latency-checkpoint", report());
  };
  await invoke("latency-checkpoint", report());
  await input(first, 20, "steady-1");
  await snapshot("first-steady");
  checkpoint = "sustained single terminal for 30 seconds";
  const longStarted = now();
  record("long-steady-begin");
  while (now() - longStarted < 30000) {
    await input(first, 1, "steady-long-1");
    await pause(100);
    if (samples.length % 20 === 0) await invoke("latency-checkpoint", report());
  }
  record("long-steady-end", { duration: now() - longStarted });
  checkpoint = "real session save waiting on owned three-second mutex";
  const savedSession = await originalInvoke("load_session");
  if (!savedSession || savedSession.version !== 5)
    throw Error("Missing exact owned session before slow-save probe");
  let pendingSave;
  const slowSave = { label: "steady-session-save-lock", requestedHoldMs: 3000 };
  slowSessionSaves.push(slowSave);
  try {
    await input(first, 1, slowSave.label, async () => {
      slowSave.hold = await originalInvoke("terminal_latency_hold_session", {
        token: bootstrap.token,
      });
      slowSave.holdReady = now();
      slowSave.saveRequested = now();
      record("session-save-request-under-lock");
      pendingSave = originalInvoke("save_session", {
        data: savedSession,
        recovery: false,
      }).then(
        () => {
          slowSave.saveCompleted = now();
          slowSave.saveDurationMs =
            slowSave.saveCompleted - slowSave.saveRequested;
          slowSave.saveSucceeded = true;
          record("session-save-completed-under-lock", {
            duration: slowSave.saveDurationMs,
          });
          return { success: true };
        },
        (error) => {
          slowSave.saveCompleted = now();
          slowSave.saveSucceeded = false;
          slowSave.error = String(error);
          return { success: false, error: String(error) };
        },
      );
      // Prefix keys are already dispatched; only the final key is timed while the
      // actual production save command waits for the fixture-owned mutex.
      await pause(100);
    });
  } finally {
    if (pendingSave) {
      const result = await pendingSave;
      if (!result.success)
        throw Error(`Owned session save failed: ${result.error}`);
    }
    const sample = samples.at(-1);
    const offsetUs =
      sample?.submitUnixMs * 1000 - slowSave.hold?.acquiredUnixUs;
    if (
      sample?.label !== slowSave.label ||
      !Number.isFinite(offsetUs) ||
      offsetUs < 0 ||
      offsetUs > 2500000 ||
      !slowSave.saveSucceeded ||
      !(slowSave.saveDurationMs >= 2800)
    ) {
      throw Error(
        `Slow-save overlap inconclusive: final-key offset ${offsetUs / 1000} ms, save duration ${slowSave.saveDurationMs} ms, success ${slowSave.saveSucceeded}`,
      );
    }
    slowSave.submissionVerifiedDuringHold = true;
    slowSave.marker = sample.marker;
    slowSave.remainingAtSubmitMs = (3000000 - offsetUs) / 1000;
    await invoke("latency-checkpoint", report());
  }

  checkpoint = "one terminal ANSI redraw workload";
  const oneWorkload = await startWorkload(first, "one-terminal-30fps");
  await input(first, 20, "steady-tui-one");
  await finishWorkload(oneWorkload);
  for (const total of [2, 4, 8]) {
    checkpoint = `burst-open-${total}`;
    const before = runtimes.size;
    record("burst-begin", { total });
    for (let i = before; i < total; i++) {
      shortcut("T", "KeyT", true);
      await pause(25);
      visible();
    }
    await wait(
      () =>
        runtimes.size === total &&
        visible()?.getSnapshot().status === "running",
    );
    const active = visible();
    for (const runtime of runtimes.values()) {
      const tab = document.getElementById(tabByPane.get(runtime.paneId));
      if (!tab)
        throw Error(`Missing UI tab for expected runtime ${runtime.paneId}`);
      const requested = now();
      tab.click();
      await wait(
        () =>
          runtime.getSnapshot().status === "running" &&
          runtime.host.isConnected &&
          getComputedStyle(runtime.host).opacity === "1",
      );
      record("burst-tab-startup-ready", {
        total,
        pane: runtime.paneId,
        duration: now() - requested,
      });
    }
    const owned = await invoke("latency-inspect");
    for (const runtime of runtimes.values())
      if (!owned[runtime.sessionId])
        throw Error(`Expected PTY missing for ${runtime.paneId}`);
    document.getElementById(tabByPane.get(active.paneId)).click();
    await wait(
      () =>
        visible() === active && getComputedStyle(active.host).opacity === "1",
    );
    record("burst-active-running", { total });
    await input(active, 10, `after-burst-${total}`);
    await invoke("latency-checkpoint", report());
  }
  checkpoint = "return to retained first terminal";
  // Click the actual retained tab; domain/UI remains responsible for activation.
  const firstTab = retainedTabId && document.getElementById(retainedTabId);
  if (!firstTab) throw Error("Cannot locate retained terminal tab");
  firstTab.click();
  await wait(
    () => visible() === first && first.getSnapshot().status === "running",
  );
  await input(first, 20, "steady-retained-8");
  for (const total of [2, 4, 8]) {
    checkpoint = `visible-pane-burst-${total}`;
    record("visible-pane-burst-begin", { total });
    while (visiblePanes().length < total) {
      const largest = visiblePanes().sort((a, b) => area(b) - area(a))[0];
      largest.terminal.focus();
      await pause(25);
      const bounds = largest.host.getBoundingClientRect();
      // Split the largest pane along its longer dimension; no terminal limits or
      // renderer settings are changed by the fixture.
      const expected = visiblePanes().length;
      shortcut("D", "KeyD", bounds.height > bounds.width);
      await wait(() => visiblePanes().length > expected, 10000);
      await pause(25);
    }
    await wait(
      () =>
        visiblePanes().length === total &&
        visiblePanes().every(
          (runtime) =>
            runtime.getSnapshot().status === "running" &&
            getComputedStyle(runtime.host).opacity === "1",
        ),
    );
    record("visible-pane-burst-running-revealed", {
      total,
      panes: visiblePanes().map((runtime) => ({
        pane: runtime.paneId,
        session: runtime.sessionId,
        renderer: runtime.getSnapshot().renderer,
      })),
    });
    for (const runtime of visiblePanes())
      await input(runtime, 3, `visible-${total}`);
    await input(first, 10, `steady-visible-${total}`);
    if (total === 8) await snapshot("after-visible-8");
    if (total === 8) {
      checkpoint = "eight visible ANSI redraw workloads";
      const running = [];
      for (const runtime of visiblePanes())
        running.push(await startWorkload(runtime, "eight-visible-30fps"));
      for (const { runtime } of running)
        await input(runtime, 3, "tui-eight-each");
      await input(first, 10, "steady-tui-eight-retained");
      for (const workload of running) await finishWorkload(workload);
    }
    await invoke("latency-checkpoint", report());
  }
  checkpoint = "retained PID and rapid pane focus";
  for (const runtime of visiblePanes()) {
    runtime.terminal.focus();
    await pause(25);
  }
  await input(first, 10, "steady-after-pane-switches");
  const finalPids = await invoke("latency-inspect");
  if (originalPids[first.sessionId] !== finalPids[first.sessionId])
    throw Error("Retained PTY PID changed");
  record("retained-pid-verified", {
    session: first.sessionId,
    pid: finalPids[first.sessionId],
  });
  checkpoint = "completed";
  const summarize = (group) => {
    const values = group
      .map((sample) => sample.submitToRenderMs)
      .sort((a, b) => a - b);
    const p95Ms = values[Math.ceil(values.length * 0.95) - 1];
    const wholeMarker = group
      .map((sample) => sample.wholeMarkerToRenderMs)
      .sort((a, b) => a - b);
    const maxMs = Math.max(...values);
    return {
      count: values.length,
      wholeMarkerP95Ms: wholeMarker[Math.ceil(wholeMarker.length * 0.95) - 1],
      wholeMarkerMaxMs: Math.max(...wholeMarker),
      p95Ms,
      maxMs,
      acceptance:
        p95Ms < 100 &&
        maxMs <= 250 &&
        group.every((sample) => sample.exactInputVerified),
    };
  };
  const byStage = Object.fromEntries(
    [...new Set(samples.map((sample) => sample.label))].map((label) => [
      label,
      summarize(samples.filter((sample) => sample.label === label)),
    ]),
  );
  const all = summarize(samples);
  const lost = events.filter(
    (event) => event.phase === "webgl-context-lost",
  ).length;
  const metrics = {
    all,
    byStage,
    contextLosses: lost,
    acceptance:
      all.acceptance &&
      Object.values(byStage).every((stage) => stage.acceptance) &&
      lost === 0,
  };
  await invoke(metrics.acceptance ? "passed" : "failed", {
    ...report(),
    metrics,
  });
} catch (error) {
  await invoke("failed", {
    ...report(),
    error: String(error),
    stack: error?.stack,
  });
}
