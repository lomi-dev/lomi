import {
  mkdir,
  readFile,
  writeFile,
  realpath,
  readdir,
  stat,
  copyFile,
} from "node:fs/promises";
import { resolve, join, isAbsolute } from "node:path";
import { spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";

const options = Object.fromEntries(
  process.argv
    .slice(2)
    .reduce(
      (pairs, value, index, args) =>
        value.startsWith("--")
          ? [...pairs, [value.slice(2), args[index + 1]]]
          : pairs,
      [],
    ),
);
if (process.platform !== "linux")
  throw Error("Run this packaged WebKitGTK fixture on Linux.");
for (const name of ["binary", "config", "artifacts"])
  if (!options[name] || !isAbsolute(options[name]))
    throw Error(`Required absolute --${name} PATH`);
if (
  process.env.LOMI_TERMINAL_LATENCY_RENDERER &&
  process.env.LOMI_TERMINAL_LATENCY_RENDERER !== "dom"
)
  throw Error("Only the explicit fixture renderer experiment dom is supported");
const binary = await realpath(options.binary);
const config = JSON.parse(await readFile(options.config, "utf8"));
const identifier = "dev.lomi.terminal-latency-smoke";
if (config.identifier !== identifier)
  throw Error(
    `Config must identify the separately built fixture as ${identifier}`,
  );
const directory = resolve(options.artifacts);
// Exclusive creation prevents accidental use of a user profile or old fixture.
await mkdir(directory);
for (const name of [
  "xdg-data",
  "xdg-config",
  "xdg-cache",
  "xdg-state",
  "project",
])
  await mkdir(join(directory, name));
await copyFile(
  new URL("./terminal-latency-workload.py", import.meta.url),
  join(directory, "project", "terminal-latency-workload.py"),
);
const appData = join(directory, "xdg-data", identifier);
await mkdir(appData);
const pane = {
  type: "terminal",
  id: "latency-first",
  cwd: join(directory, "project"),
};
const tab = {
  type: "terminal",
  id: "latency-tab",
  title: "Terminal",
  profileId: "local:bash",
  layout: pane,
  activePaneId: pane.id,
};
const workspace = {
  id: "latency-workspace",
  name: "Default",
  tabs: [tab],
  activeTabId: tab.id,
};
const project = {
  id: "latency-project",
  path: pane.cwd,
  workspaces: [workspace],
  activeWorkspaceId: workspace.id,
};
await writeFile(
  join(appData, "session.json"),
  JSON.stringify({
    version: 5,
    projects: [project],
    activeProjectId: project.id,
    sidebar: null,
    sidebarWidth: 250,
    rightSidebar: null,
    rightSidebarWidth: 250,
    sidebarSides: { files: "left", git: "right", workspaces: "left" },
    terminalOverviewSide: "left",
  }),
);
const sha = async (path) =>
  createHash("sha256")
    .update(await readFile(path))
    .digest("hex");
const binaryHash = await sha(binary);
const env = {
  ...process.env,
  XDG_DATA_HOME: join(directory, "xdg-data"),
  XDG_CONFIG_HOME: join(directory, "xdg-config"),
  XDG_CACHE_HOME: join(directory, "xdg-cache"),
  XDG_STATE_HOME: join(directory, "xdg-state"),
  HISTFILE: join(directory, "shell-history"),
  LOMI_TERMINAL_LATENCY_DIRECTORY: directory,
  LOMI_TERMINAL_LATENCY_TOKEN: randomUUID(),
};
// Preserve all caller graphics switches, including explicit zero and absence.
const switches = Object.fromEntries(
  Object.entries(env).filter(([name]) =>
    /DMABUF|COMPOSIT|RENDER|LIBGL|GDK_BACKEND|DISPLAY|WAYLAND/.test(name),
  ),
);
await writeFile(
  join(directory, "run.json"),
  JSON.stringify(
    {
      binary,
      binaryHash,
      config,
      configHash: await sha(options.config),
      switches,
      started: new Date().toISOString(),
      uid: process.getuid(),
      timeoutMs: Number(options.timeout ?? 180000),
    },
    null,
    2,
  ),
);
const child = spawn(binary, [], {
  env,
  detached: true,
  stdio: ["ignore", "pipe", "pipe"],
});
let log = "";
for (const stream of [child.stdout, child.stderr])
  stream.on("data", (chunk) => {
    log += chunk;
    process.stdout.write(chunk);
  });
let exited = false;
child.on("exit", () => {
  exited = true;
});
child.on("error", (error) => {
  log += String(error);
  exited = true;
});
// PID identity survives leader exit and covers PTY children in separate sessions.
const owned = new Map();
const cleanupErrors = new Set();
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
        cleanupErrors.add(String(error));
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
      cleanupErrors.add(String(error));
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
      if (error.code !== "ESRCH") cleanupErrors.add(String(error));
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
    `LOMI_TERMINAL_LATENCY_TOKEN=${env.LOMI_TERMINAL_LATENCY_TOKEN}`,
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
        cleanupErrors.add(String(error));
    }
  }
  return unknown;
}
let interrupted = null;
const onSigint = () => {
  interrupted = "SIGINT";
};
const onSigterm = () => {
  interrupted = "SIGTERM";
};
process.on("SIGINT", onSigint);
process.on("SIGTERM", onSigterm);
const until = Date.now() + Number(options.timeout ?? 180000);
let result;
try {
  const leader = child.pid ? await processIdentity(child.pid) : null;
  const leaderEnvironment = leader
    ? await readFile(`/proc/${leader.pid}/environ`, "utf8").catch(() => "")
    : "";
  if (
    !exited &&
    leader?.uid === uid &&
    leaderEnvironment
      .split("\0")
      .includes(
        `LOMI_TERMINAL_LATENCY_TOKEN=${env.LOMI_TERMINAL_LATENCY_TOKEN}`,
      ) &&
    sameProcess(leader, await processIdentity(leader.pid))
  )
    owned.set(leader.pid, leader);
  else cleanupErrors.add("Could not capture the fixture leader identity");
  while (Date.now() < until && !interrupted) {
    await captureDescendants();
    try {
      result = JSON.parse(
        await readFile(join(directory, "result.json"), "utf8"),
      );
    } catch {}
    if (result || exited) break;
    await new Promise((done) => setTimeout(done, 100));
  }
  if (!result || interrupted) {
    result = {
      stage: "failed",
      data: {
        error: interrupted
          ? `Runner interrupted by ${interrupted}`
          : exited
            ? "Fixture exited before result"
            : "Fatal fixture timeout",
        lastCheckpoint: await readFile(
          join(directory, "checkpoint.json"),
          "utf8",
        ).catch(() => null),
      },
    };
    await writeFile(
      join(directory, "runner-failure.json"),
      JSON.stringify(result, null, 2),
    );
  }
  if ((await sha(binary)) !== binaryHash)
    throw Error("Immutable fixture binary changed during measurement");
  console.log(
    JSON.stringify(
      result.data?.metrics ?? {
        stage: result.stage,
        checkpoint: result.data?.checkpoint,
        error: result.data?.error,
        artifacts: directory,
      },
      null,
      2,
    ),
  );
  process.exitCode = result.stage === "passed" ? 0 : 1;
} finally {
  let survivors = [];
  let unproven = [];
  try {
    await captureDescendants();
    await signalOwned("SIGTERM");
    await waitForRetirement(1000);
    await signalOwned("SIGKILL");
    await waitForRetirement(1000);
    survivors = await liveOwned();
    // A short-lived parent may escape observation. Report token-bearing
    // orphans, but never signal a process without a verified ancestry chain.
    unproven = await unknownTokenProcesses();
  } catch (error) {
    cleanupErrors.add(String(error));
  }
  if (interrupted || survivors.length || unproven.length || cleanupErrors.size)
    process.exitCode = 1;
  try {
    await Promise.all([
      writeFile(join(directory, "native.log"), log),
      writeFile(
        join(directory, "cleanup.json"),
        JSON.stringify(
          {
            interrupted,
            owned: [...owned.values()],
            survivors,
            unproven,
            errors: [...cleanupErrors],
            retired:
              !survivors.length && !unproven.length && !cleanupErrors.size,
          },
          null,
          2,
        ),
      ),
    ]);
  } finally {
    child.stdout.destroy();
    child.stderr.destroy();
    child.unref();
    process.off("SIGINT", onSigint);
    process.off("SIGTERM", onSigterm);
  }
}
