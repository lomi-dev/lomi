import {
  mkdir,
  readFile,
  writeFile,
  realpath,
  readdir,
  stat,
} from "node:fs/promises";
import { join, resolve, isAbsolute } from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { fileURLToPath } from "node:url";

const options = {};
for (let index = 2; index < process.argv.length; index += 2) {
  if (!process.argv[index].startsWith("--") || !process.argv[index + 1])
    throw Error("Expected --name value arguments");
  options[process.argv[index].slice(2)] = process.argv[index + 1];
}
if (process.platform !== "linux")
  throw Error("Linux packaged native-smoke fixture only");
for (const name of ["binary", "config", "directory"])
  if (!options[name] || !isAbsolute(options[name]))
    throw Error(`Required absolute --${name} PATH`);
const binary = await realpath(options.binary);
const config = JSON.parse(await readFile(options.config, "utf8"));
const identifier = config.identifier;
if (!/^dev\.lomi\.linux-instance-smoke-[a-z0-9-]+$/.test(identifier))
  throw Error(
    "Use a separately built dev.lomi.linux-instance-smoke-* identifier",
  );
const directory = resolve(options.directory);
await mkdir(directory);
for (const name of [
  "xdg-data",
  "xdg-config",
  "xdg-cache",
  "xdg-state",
  "project",
])
  await mkdir(join(directory, name));
const appData = join(directory, "xdg-data", identifier);
await mkdir(appData);
const panes = ["instance-first", "instance-second"];
const session = {
  version: 5,
  projects: [
    {
      id: "instance-project",
      path: join(directory, "project"),
      activeWorkspaceId: "instance-workspace",
      workspaces: [
        {
          id: "instance-workspace",
          name: "Default",
          activeTabId: "instance-tab",
          tabs: [
            {
              type: "terminal",
              id: "instance-tab",
              title: "Terminal",
              profileId: "local:bash",
              activePaneId: panes[0],
              layout: {
                type: "split",
                id: "instance-split",
                axis: "horizontal",
                ratio: 0.5,
                first: {
                  type: "terminal",
                  id: panes[0],
                  cwd: join(directory, "project"),
                },
                second: {
                  type: "terminal",
                  id: panes[1],
                  cwd: join(directory, "project"),
                },
              },
            },
          ],
        },
      ],
    },
  ],
  activeProjectId: "instance-project",
  sidebar: null,
  sidebarWidth: 250,
  rightSidebar: null,
  rightSidebarWidth: 250,
  sidebarSides: { files: "left", git: "right", workspaces: "left" },
  terminalOverviewSide: "left",
};
await writeFile(join(appData, "session.json"), JSON.stringify(session));
const sha = async (path) =>
  createHash("sha256")
    .update(await readFile(path))
    .digest("hex");
const binaryHash = await sha(binary);
const token = randomUUID();
const env = {
  ...process.env,
  XDG_DATA_HOME: join(directory, "xdg-data"),
  XDG_CONFIG_HOME: join(directory, "xdg-config"),
  XDG_CACHE_HOME: join(directory, "xdg-cache"),
  XDG_STATE_HOME: join(directory, "xdg-state"),
  HISTFILE: join(directory, "shell-history"),
  LOMI_LINUX_INSTANCE_DIRECTORY: directory,
  LOMI_LINUX_INSTANCE_TOKEN: token,
  LOMI_LINUX_INSTANCE_IDENTIFIER: identifier,
};
const json = (name, value) =>
  writeFile(join(directory, name), JSON.stringify(value, null, 2));
let runnerRevision = null;
try {
  runnerRevision = execFileSync("git", ["rev-parse", "HEAD"], {
    cwd: fileURLToPath(new URL("../..", import.meta.url)),
    encoding: "utf8",
    stdio: ["ignore", "pipe", "ignore"],
  }).trim();
} catch {}
await json("run.json", {
  binary,
  binaryHash,
  config,
  configHash: await sha(options.config),
  binarySourceRevision: options["source-revision"] ?? null,
  runnerRevision,
  fixtureHashes: Object.fromEntries(
    await Promise.all(
      [
        "linux-instance-run.mjs",
        "linux-instance-owner-lock.py",
        "linux-instance-accessibility.py",
        "linux-instance-entry.js",
        "linux-instance-support.rs",
        "linux-instance-vite.config.ts",
      ].map(async (name) => [name, await sha(new URL(name, import.meta.url))]),
    ),
  ),
  started: new Date().toISOString(),
  uid: process.getuid(),
  graphics: Object.fromEntries(
    Object.entries(env).filter(([name]) =>
      /DMABUF|COMPOSIT|RENDER|LIBGL|GDK_BACKEND|DISPLAY|WAYLAND/.test(name),
    ),
  ),
});
const pause = (ms) => new Promise((done) => setTimeout(done, ms));
const owned = new Map();
const children = [];
const secondaryResults = [];
const cleanupErrors = [];
let interrupted;
for (const signal of ["SIGINT", "SIGTERM"])
  process.on(signal, () => {
    interrupted = signal;
  });
const uid = process.getuid();
async function processIdentity(pid) {
  try {
    const owner = await stat(`/proc/${pid}`);
    const text = await readFile(`/proc/${pid}/stat`, "utf8");
    const fields = text.slice(text.lastIndexOf(")") + 2).split(" ");
    return {
      pid,
      uid: owner.uid,
      state: fields[0],
      parent: Number(fields[1]),
      start: fields[19],
    };
  } catch (error) {
    if (error.code === "ENOENT" || error.code === "ESRCH") return null;
    throw error;
  }
}
function sameProcess(left, right) {
  return (
    !!left &&
    !!right &&
    left.pid === right.pid &&
    left.start === right.start &&
    left.uid === right.uid
  );
}
async function captureDescendants() {
  const queue = [...owned.values()];
  const visited = new Set();
  for (const parent of queue) {
    if (visited.has(parent.pid)) continue;
    visited.add(parent.pid);
    try {
      if (!sameProcess(parent, await processIdentity(parent.pid))) continue;
      // PTYs can be spawned by a Rust worker thread, so inspect every task.
      const tasks = await readdir(`/proc/${parent.pid}/task`);
      const children = (
        await Promise.all(
          tasks.map(async (task) => {
            try {
              return await readFile(
                `/proc/${parent.pid}/task/${task}/children`,
                "utf8",
              );
            } catch (error) {
              if (error.code === "ENOENT" || error.code === "ESRCH") return "";
              throw error;
            }
          }),
        )
      ).join(" ");
      for (const pid of children
        .trim()
        .split(/\s+/)
        .filter(Boolean)
        .map(Number)) {
        const candidate = await processIdentity(pid);
        // Verify both ends of the ancestry edge before retaining ownership.
        if (
          candidate?.uid !== uid ||
          candidate.parent !== parent.pid ||
          BigInt(candidate.start) < BigInt(parent.start)
        )
          continue;
        if (!sameProcess(parent, await processIdentity(parent.pid))) break;
        const previous = owned.get(pid);
        if (previous && !sameProcess(previous, candidate)) continue;
        if (!previous)
          owned.set(pid, {
            ...candidate,
            ancestor: { pid: parent.pid, start: parent.start },
          });
        queue.push(owned.get(pid));
      }
    } catch (error) {
      if (error.code !== "ENOENT" && error.code !== "ESRCH")
        cleanupErrors.push(String(error));
    }
  }
}
async function liveOwned() {
  const live = [];
  for (const identity of owned.values()) {
    try {
      const current = await processIdentity(identity.pid);
      if (sameProcess(identity, current) && !["Z", "X"].includes(current.state))
        live.push(identity);
    } catch (error) {
      cleanupErrors.push(String(error));
    }
  }
  return live;
}
async function signalOwned(signal) {
  // Children first; never signal a whole group whose leader may have exited.
  for (const identity of [...owned.values()].reverse()) {
    try {
      const current = await processIdentity(identity.pid);
      if (!sameProcess(identity, current) || ["Z", "X"].includes(current.state))
        continue;
      process.kill(identity.pid, signal);
    } catch (error) {
      if (error.code !== "ESRCH") cleanupErrors.push(String(error));
    }
  }
}
async function waitForRetirement(milliseconds) {
  const deadline = Date.now() + milliseconds;
  while (Date.now() < deadline) {
    await captureDescendants();
    if (!(await liveOwned()).length) return;
    await new Promise((done) => setTimeout(done, 50));
  }
}
async function unknownTokenProcesses() {
  const unknown = [];
  const marker = Buffer.from(
    `LOMI_LINUX_INSTANCE_TOKEN=${env.LOMI_LINUX_INSTANCE_TOKEN}`,
  );
  for (const name of await readdir("/proc")) {
    if (!/^\d+$/.test(name)) continue;
    const pid = Number(name);
    try {
      const identity = await processIdentity(pid);
      if (
        identity?.uid !== uid ||
        ["Z", "X"].includes(identity.state) ||
        sameProcess(owned.get(pid), identity)
      )
        continue;
      const environment = await readFile(`/proc/${pid}/environ`);
      if (
        environment
          .toString()
          .split("\0")
          .some((entry) => Buffer.from(entry).equals(marker)) &&
        sameProcess(identity, await processIdentity(pid))
      )
        unknown.push(identity);
    } catch (error) {
      if (!["ENOENT", "ESRCH", "EACCES", "EPERM"].includes(error.code))
        cleanupErrors.push(String(error));
    }
  }
  return unknown;
}
async function launch(name, phase, command = binary, args = []) {
  const child = spawn(command, args, {
    env: { ...env, LOMI_LINUX_INSTANCE_PHASE: phase },
    stdio: [name === "owner-lock-holder" ? "pipe" : "ignore", "pipe", "pipe"],
  });
  const record = {
    name,
    child,
    log: "",
    stdout: "",
    result: null,
    started: Date.now(),
    ended: null,
  };
  children.push(record);
  for (const stream of [child.stdout, child.stderr])
    stream.on("data", (bytes) => {
      record.log += bytes;
      if (stream === child.stdout) record.stdout += bytes;
    });
  child.stdin?.on("error", (error) => {
    record.log += String(error);
  });
  child.on("error", (error) => {
    record.result = { error: String(error) };
  });
  child.on("close", (code, signal) => {
    record.ended = Date.now();
    record.result = { code, signal };
  });
  const identity = child.pid ? await processIdentity(child.pid) : null;
  const environment = identity
    ? await readFile(`/proc/${identity.pid}/environ`, "utf8").catch(() => "")
    : "";
  if (
    identity?.uid === uid &&
    environment.split("\0").includes(`LOMI_LINUX_INSTANCE_TOKEN=${token}`) &&
    sameProcess(identity, await processIdentity(child.pid))
  )
    owned.set(child.pid, identity);
  if (
    ["primary", "restore", "owner-lock-holder", "accessibility"].includes(
      name,
    ) &&
    !owned.has(child.pid)
  )
    throw Error("Could not verify the owned primary process identity");
  return record;
}
async function wait(predicate, label, milliseconds = 45000) {
  const until = Date.now() + milliseconds;
  while (Date.now() < until && !interrupted) {
    await captureDescendants();
    const value = await predicate();
    if (value) return value;
    await pause(50);
  }
  throw Error(
    `Timeout/interruption: ${label}${interrupted ? ` (${interrupted})` : ""}`,
  );
}
async function receipt(phase, name, primary) {
  return wait(async () => {
    const failure = await readFile(
      join(directory, `${phase}-failed.json`),
      "utf8",
    ).catch(() => null);
    if (failure) throw Error(failure);
    const value = await readFile(
      join(directory, `${phase}-${name}.json`),
      "utf8",
    ).catch(() => null);
    if (value) return JSON.parse(value);
    if (primary.result)
      throw Error(
        `${phase} exited before ${name}: ${JSON.stringify(primary.result)} ${primary.log}`,
      );
    return null;
  }, `${phase} ${name}`);
}
async function burst(label, phase, count) {
  const started = Date.now();
  const records = await Promise.all(
    Array.from({ length: count }, (_, index) =>
      launch(`${label}-${index}`, phase),
    ),
  );
  await wait(
    () => records.every((record) => record.result),
    `${label} secondary exits`,
    10000,
  );
  for (const record of records) {
    const result = {
      name: record.name,
      pid: record.child.pid,
      ...record.result,
      elapsedMs: record.ended - record.started,
      burstElapsedMs: Date.now() - started,
      log: record.log,
    };
    secondaryResults.push(result);
    if (
      result.code !== 0 ||
      /Another Lomi instance|panicked|setup.*error/i.test(result.log)
    )
      throw Error(`Secondary launch failed: ${JSON.stringify(result)}`);
  }
  await json("secondary-results.json", secondaryResults);
}
function verifyOwnedPtys(receipt) {
  const pids = Object.values(receipt.state.terminals);
  if (pids.length !== 2 || pids.some((pid) => !owned.has(pid)))
    throw Error(
      "Could not prove ownership of both native PTY process identities",
    );
}
function savedPaneIds(layout) {
  return layout.type === "split"
    ? [...savedPaneIds(layout.first), ...savedPaneIds(layout.second)]
    : [layout.id];
}
async function checkSaved(phase) {
  const raw = await readFile(join(appData, "session.json"), "utf8");
  const saved = JSON.parse(raw);
  const tabs = saved.projects.flatMap((project) =>
    project.workspaces.flatMap((workspace) => workspace.tabs),
  );
  if (
    tabs.length !== 1 ||
    tabs[0].id !== "instance-tab" ||
    JSON.stringify(savedPaneIds(tabs[0].layout).sort()) !==
      JSON.stringify([...panes].sort())
  )
    throw Error("Guarded quit did not preserve the exact saved tab/pane IDs");
  await writeFile(join(directory, `${phase}-saved-session.json`), raw);
}
let accessibility;
let lockHolder;
let handoff;
function holderEvent(stage) {
  return lockHolder.stdout
    .split("\n")
    .filter(Boolean)
    .flatMap((line) => {
      try {
        return [JSON.parse(line)];
      } catch {
        return [];
      }
    })
    .find((event) => event.stage === stage);
}
async function releaseHolder() {
  if (!lockHolder || lockHolder.result)
    throw Error("Owner-lock helper exited before explicit release");
  handoff.releaseRequestedUnixMs = Date.now();
  lockHolder.child.stdin.end("release\n");
  await wait(() => lockHolder.result, "owner-lock helper graceful exit", 5000);
  handoff.released = holderEvent("released");
  handoff.helperExit = lockHolder.result;
  await json("handoff.json", handoff);
  if (
    lockHolder.result.code !== 0 ||
    !handoff.released ||
    handoff.released.holdMs < 250
  )
    throw Error(
      `Owner-lock helper failed its controlled hold: ${lockHolder.log}`,
    );
}
let passed = false;
try {
  const primary = await launch("primary", "primary");
  const beforeReady = await receipt("primary", "pre-ready", primary);
  if (beforeReady.state.visible) throw Error("Pre-ready primary is not hidden");
  if (Object.keys(beforeReady.state.terminals).length)
    throw Error("Startup gate already created PTYs");
  await burst("before-ready", "primary", 4);
  await writeFile(join(directory, "primary-continue-startup"), token);
  const released = await receipt("primary", "startup-released", primary);
  if (released.state.visible)
    throw Error("Secondary bypassed frontend startup barrier");
  const initial = await receipt("primary", "running", primary);
  verifyOwnedPtys(initial);
  const credential = initial.data?.credential;
  if (
    !credential ||
    !["set", "matched", "deleted", "absent"].every(
      (flag) => credential[flag] === true,
    )
  )
    throw Error("Primary did not prove its native credential round trip");
  for (const cycle of [1, 2]) {
    const hidden = await receipt("primary", `hidden-${cycle}`, primary);
    if (hidden.state.visible)
      throw Error("Ordinary main.close did not hide the main window");
    await burst(`hidden-${cycle}`, "primary", 8);
    const reopened = await receipt("primary", `reopened-${cycle}`, primary);
    if (
      !reopened.state.visible ||
      !reopened.state.focused ||
      JSON.stringify(reopened.state.terminals) !==
        JSON.stringify(initial.state.terminals)
    )
      throw Error("Reopen lost native focus, visibility or PTYs");
    if (cycle === 1) {
      const probe = await launch("accessibility", "primary", "python3", [
        fileURLToPath(
          new URL("linux-instance-accessibility.py", import.meta.url),
        ),
        String(primary.child.pid),
      ]);
      await wait(
        () => {
          if (primary.result)
            throw Error("Primary exited during real AT-SPI enumeration");
          return probe.result;
        },
        "real AT-SPI application-bus probe",
        25000,
      );
      try {
        accessibility = JSON.parse(probe.stdout);
      } catch {
        throw Error(`Invalid AT-SPI result: ${probe.log}`);
      }
      if (
        probe.result.code !== 0 ||
        accessibility.stage !== "passed" ||
        accessibility.pid !== primary.child.pid ||
        accessibility.visibleFrameCount < 1 ||
        accessibility.visibleFrameChildCount < 1 ||
        accessibility.probedCount < 2
      )
        throw Error(`AT-SPI regression probe failed: ${probe.log}`);
      const identity = await processIdentity(primary.child.pid);
      if (
        primary.result ||
        !sameProcess(owned.get(primary.child.pid), identity) ||
        ["Z", "X"].includes(identity.state)
      )
        throw Error(
          "Primary did not survive the real AT-SPI application-bus probe",
        );
      await json("accessibility.json", accessibility);
    }
    await writeFile(join(directory, `primary-continue-cycle-${cycle}`), token);
  }
  await receipt("primary", "ready-to-quit", primary);
  await wait(() => primary.result, "guarded primary exit");
  if (primary.result.code !== 0)
    throw Error(`Primary exit failed: ${primary.log}`);
  await checkSaved("primary");
  await wait(
    async () => !(await liveOwned()).length,
    "all original owned processes retire",
    10000,
  );
  handoff = { minimumOverlapMs: 250 };
  lockHolder = await launch("owner-lock-holder", "restore", "python3", [
    "-u",
    fileURLToPath(new URL("linux-instance-owner-lock.py", import.meta.url)),
    appData,
  ]);
  const ready = await wait(
    () => {
      if (lockHolder.result || holderEvent("failed"))
        throw Error(`Owner-lock helper failed: ${lockHolder.log}`);
      return holderEvent("ready");
    },
    "both real profile owner locks held",
    5000,
  );
  handoff.ready = ready;
  await json("handoff.json", handoff);
  const restore = await launch("restore", "restore");
  handoff.restoreStartedUnixMs = restore.started;
  handoff.contention = await wait(async () => {
    if (lockHolder.result || holderEvent("released") || holderEvent("failed"))
      throw Error(
        "Owner-lock helper retired before restore proved native contention",
      );
    if (restore.result)
      throw Error(`Restore exited before native contention: ${restore.log}`);
    if (
      await readFile(join(directory, "restore-pre-ready.json"), "utf8").catch(
        () => null,
      )
    )
      throw Error(
        "Restore reached frontend startup without proving native owner-lock contention",
      );
    const raw = await readFile(
      join(directory, "restore-owner-contended.json"),
      "utf8",
    ).catch(() => null);
    if (!raw) return null;
    const event = JSON.parse(raw);
    if (
      event.pid !== restore.child.pid ||
      event.phase !== "restore" ||
      event.name !== "owner-contended" ||
      JSON.stringify(event.paths) !== JSON.stringify(ready.paths) ||
      BigInt(event.unixNs) < BigInt(ready.unixNs)
    )
      throw Error(
        "Native owner-lock contention receipt does not match the held profile and restore process",
      );
    const identity = await processIdentity(restore.child.pid);
    if (
      !sameProcess(owned.get(restore.child.pid), identity) ||
      ["Z", "X"].includes(identity.state)
    )
      throw Error(
        "Restore process is not alive at the native contention receipt",
      );
    return event;
  }, "actual native owner-lock contention while helper holds both locks");
  handoff.contentionObservedUnixMs = Date.now();
  await json("handoff.json", handoff);
  const contentionHoldStarted = performance.now();
  const holdUntil = contentionHoldStarted + 250;
  while (performance.now() < holdUntil) {
    if (interrupted || lockHolder.result || restore.result)
      throw Error(
        "A process exited or was interrupted during the controlled owner-lock hold",
      );
    await captureDescendants();
    if (
      await readFile(join(directory, "restore-pre-ready.json"), "utf8").catch(
        () => null,
      )
    )
      throw Error(
        "Restore reached frontend readiness while real owner locks were still held",
      );
    await pause(10);
  }
  if (
    await readFile(join(directory, "restore-pre-ready.json"), "utf8").catch(
      () => null,
    )
  )
    throw Error("Restore passed setup before controlled owner-lock release");
  const restoreIdentity = await processIdentity(restore.child.pid);
  if (
    !sameProcess(owned.get(restore.child.pid), restoreIdentity) ||
    ["Z", "X"].includes(restoreIdentity.state) ||
    restore.result ||
    lockHolder.result
  )
    throw Error(
      "Restore did not remain alive during the controlled owner-lock hold",
    );
  if (/Another Lomi instance|panicked|setup.*error/i.test(restore.log))
    throw Error(`Restore reported an owner-lock setup failure: ${restore.log}`);
  handoff.blockedObservation = {
    unixMs: Date.now(),
    overlapSinceContentionObservationMs:
      performance.now() - contentionHoldStarted,
    restoreAlive: true,
    preReadyAbsent: true,
    noWorkbenchPtyStartup: true,
  };
  await json("handoff.json", handoff);
  await releaseHolder();
  await receipt("restore", "pre-ready", restore);
  await writeFile(join(directory, "restore-continue-startup"), token);
  const restored = await receipt("restore", "running", restore);
  verifyOwnedPtys(restored);
  const oldPids = Object.values(initial.state.terminals);
  if (
    Object.values(restored.state.terminals).some((pid) => oldPids.includes(pid))
  )
    throw Error("Restore reused an original PTY PID");
  await receipt("restore", "ready-to-quit", restore);
  await wait(() => restore.result, "guarded restore exit");
  if (restore.result.code !== 0)
    throw Error(`Restore exit failed: ${restore.log}`);
  await checkSaved("restore");
  await wait(
    async () => !(await liveOwned()).length,
    "all restored owned processes retire",
    10000,
  );
  if ((await sha(binary)) !== binaryHash)
    throw Error("Immutable binary changed during trial");
  if ((await unknownTokenProcesses()).length)
    throw Error("Unproven token-bearing processes survived");
  passed = true;
  await json("result.json", {
    stage: "passed",
    secondaryLaunches: secondaryResults.length,
    primaryPid: primary.child.pid,
    restorePid: restore.child.pid,
    savedPaneIds: panes,
    originalPtys: initial.state.terminals,
    restoredPtys: restored.state.terminals,
    handoff,
    accessibility,
    credential,
    limits: [
      "Native GTK visibility/focus and parsed terminal output; no physical presentation or latency claim",
      "No StatusNotifierWatcher or shell startup overrides are introduced",
    ],
  });
} catch (error) {
  await json("result.json", {
    stage: "failed",
    error: String(error),
    stack: error.stack,
  });
  process.exitCode = 1;
} finally {
  if (lockHolder && !lockHolder.result && !handoff?.releaseRequestedUnixMs) {
    try {
      await releaseHolder();
    } catch (error) {
      cleanupErrors.push(String(error));
    }
  }
  await captureDescendants();
  // Successful runs must retire through the product guarded quit path alone.
  if (!passed) {
    await signalOwned("SIGTERM");
    await waitForRetirement(1500);
    await signalOwned("SIGKILL");
    await waitForRetirement(1500);
  }
  const survivors = await liveOwned();
  const unproven = await unknownTokenProcesses();
  await json("cleanup.json", {
    owned: [...owned.values()],
    survivors,
    unproven,
    errors: cleanupErrors,
    retired: !survivors.length && !unproven.length && !cleanupErrors.length,
    forcedCleanup: !passed,
  });
  for (const record of children)
    await writeFile(join(directory, `${record.name}.log`), record.log);
  await json("secondary-results.json", secondaryResults);
  if (survivors.length || unproven.length || cleanupErrors.length)
    process.exitCode = 1;
}
