import { TerminalRuntime } from "../../src/terminal-runtime";

const invoke = window.__TAURI_INTERNALS__.invoke.bind(
  window.__TAURI_INTERNALS__,
);
const runtimes = new Map();
const attached = TerminalRuntime.prototype.attach;
TerminalRuntime.prototype.attach = function (...args) {
  runtimes.set(this.paneId, this);
  return attached.apply(this, args);
};
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const wait = async (predicate, label, timeout = 30000) => {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const value = await predicate();
    if (value) return value;
    await pause(50);
  }
  throw Error(`Timeout: ${label}`);
};
let token;
const call = (stage, data = {}) =>
  invoke("plugin_smoke_result", { stage, data: { ...data, token } });
const checkpoint = (name, details) =>
  call("instance-checkpoint", { name, details });
const screen = (runtime) =>
  Array.from({ length: runtime.terminal.buffer.active.length }, (_, index) =>
    runtime.terminal.buffer.active.getLine(index)?.translateToString(),
  ).join("\n");
try {
  const bootstrap = await call("instance-bootstrap");
  token = bootstrap.token;
  if (bootstrap.phase === "primary") {
    await call("instance-close");
    await wait(
      async () => !(await call("instance-inspect")).visible,
      "hide before frontend readiness",
    );
  }
  await checkpoint("pre-ready", { bootstrap: true });
  await wait(() => call("instance-gate"), "runner startup release", 60000);
  if (bootstrap.phase === "primary" && (await call("instance-inspect")).visible)
    throw Error("Secondary activated the window before frontend readiness");
  await checkpoint("startup-released");
  await import("../../src/main.tsx");
  await wait(
    () =>
      runtimes.size === 2 &&
      [...runtimes.values()].every(
        (runtime) =>
          runtime.host.isConnected &&
          runtime.getSnapshot().status === "running",
      ),
    "two live terminal panes",
  );
  const initial = new Map(runtimes);
  const inspect = () => call("instance-inspect");
  const initialNative = await inspect();
  const initialPtys = JSON.stringify(initialNative.terminals);
  if (Object.keys(initialNative.terminals).length !== 2)
    throw Error("Expected exactly two native PTYs");
  const samples = [];
  const exercise = async (label) => {
    const state = await inspect();
    if (JSON.stringify(state.terminals) !== initialPtys)
      throw Error("Reopen replaced native PTYs");
    for (const [pane, runtime] of initial) {
      if (
        runtimes.get(pane) !== runtime ||
        runtime.getSnapshot().status !== "running"
      )
        throw Error(`Reopen replaced terminal runtime ${pane}`);
      const marker = `INSTANCE_${bootstrap.phase}_${label}_${pane}`;
      // Split the literal so command echo alone cannot satisfy the output check.
      const data = `printf '%s%s\\n' 'INSTANCE_' '${bootstrap.phase}_${label}_${pane}'\r`;
      await invoke("write_terminal", { id: runtime.sessionId, data });
      await wait(
        () => screen(runtime).replaceAll("\n", "").includes(marker),
        `parsed exact output ${marker}`,
      );
      samples.push({ label, pane, session: runtime.sessionId, marker });
    }
    return state;
  };
  await wait(async () => {
    const state = await inspect();
    return state.visible && state.focused;
  }, "initial native visible and focused");
  await exercise("initial");
  const credential =
    bootstrap.phase === "primary"
      ? await call("instance-credential-probe")
      : null;
  if (
    credential &&
    !["set", "matched", "deleted", "absent"].every(
      (flag) => credential[flag] === true,
    )
  )
    throw Error("Native fixture credential round trip was not verified");
  await checkpoint("running", { samples, credential });
  if (bootstrap.phase === "primary") {
    for (const cycle of [1, 2]) {
      await call("instance-close");
      await wait(
        async () => !(await inspect()).visible,
        "ordinary close hides window",
      );
      await checkpoint(`hidden-${cycle}`, { samples, credential });
      // Only the runner's normal secondary launches can reactivate the window.
      await wait(
        async () => {
          const state = await inspect();
          return state.visible && state.focused;
        },
        `secondary launch reopens cycle ${cycle}`,
        60000,
      );
      await exercise(`reopened-${cycle}`);
      await checkpoint(`reopened-${cycle}`, {
        samples,
        credential,
        retainedRuntimeIdentity: true,
      });
      await wait(
        () => call("instance-gate", { name: `cycle-${cycle}` }),
        `runner completed secondary burst ${cycle}`,
        60000,
      );
    }
  }
  await checkpoint("ready-to-quit", {
    samples,
    credential,
    retainedRuntimeIdentity: true,
  });
  await invoke("request_quit");
} catch (error) {
  await checkpoint("failed", {
    error: String(error),
    stack: error?.stack,
  }).catch(() => {});
  console.error(error);
}
