import { spawn } from "node:child_process";
import { access, copyFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { resolve, join } from "node:path";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";

// Build separately: native-smoke,tauri/custom-protocol ONLY, with the unique
// --config identifier and terminal-regression-vite.config.ts frontend entry.
// This runner does not alter HOME, user shell profiles, or existing app data.
assert(
  ["darwin", "win32"].includes(process.platform),
  "Requires native macOS or Windows",
);
const args = process.argv.slice(2);
const option = (name) => {
  const i = args.indexOf(name);
  assert(i >= 0 && args[i + 1], `Required option: ${name}`);
  return resolve(args[i + 1]);
};
const binary = option("--binary");
const config = JSON.parse(await readFile(option("--config"), "utf8"));
const directory = option("--directory");
const identifier = config.identifier;
assert.match(
  identifier ?? "",
  /^dev\.lomi\.terminal-regression-[a-zA-Z0-9.-]+$/,
  "Supply the exact unique BUILD-time fixture identifier",
);
const base =
  process.platform === "darwin"
    ? join(homedir(), "Library/Application Support")
    : process.env.APPDATA;
assert(base, "Platform application data directory is unavailable");
const appData = join(base, identifier);
await access(binary);
const binaryBytes = await readFile(binary);
assert(
  binaryBytes.includes(Buffer.from(identifier)) &&
    binaryBytes.includes(Buffer.from("regression-bootstrap")),
  "Executable does not contain the requested fixture identity and probe entry",
);
const binarySha256 = createHash("sha256").update(binaryBytes).digest("hex");
for (const path of [directory, appData]) {
  let exists = true;
  try {
    await access(path);
  } catch (error) {
    if (error.code === "ENOENT") exists = false;
    else throw error;
  }
  assert(!exists, `Refusing existing fixture/profile: ${path}`);
}
await mkdir(directory, { recursive: true });
await mkdir(appData, { recursive: true });
const root = join(directory, "project");
await mkdir(root);
await copyFile(
  new URL("./terminal-regression-workload.mjs", import.meta.url),
  join(root, "terminal-regression-workload.mjs"),
);
await writeFile(
  join(appData, "agent-control-preferences.json"),
  JSON.stringify({ version: 1, autoStart: false, yoloMode: false }),
);
const pane = { type: "terminal", id: "regression-first", cwd: root };
const profileId =
  process.platform === "win32" ? "local:powershell" : "local:zsh";
const tab = {
  type: "terminal",
  id: "regression-tab",
  title: "Terminal",
  profileId,
  layout: pane,
  activePaneId: pane.id,
};
const workspace = {
  id: "regression-workspace",
  name: "Native regression",
  tabs: [tab],
  activeTabId: tab.id,
};
const project = {
  id: "regression-project",
  path: root,
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
await writeFile(
  join(directory, "run.json"),
  JSON.stringify(
    {
      binary,
      binarySha256,
      identifier,
      appData,
      root,
      node: process.execPath,
      platform: process.platform,
    },
    null,
    2,
  ),
);
console.log(`Terminal native regression artifacts: ${directory}`);
const paneCount = (layout) =>
  layout.type === "split"
    ? paneCount(layout.first) + paneCount(layout.second)
    : (assert.equal(layout.type, "terminal"), 1);
let previous;
for (const phase of ["exercise", "restore"]) {
  let log = "",
    timedOut = false;
  const child = spawn(binary, [], {
    env: {
      ...process.env,
      LOMI_TERMINAL_REGRESSION_DIRECTORY: directory,
      LOMI_TERMINAL_REGRESSION_IDENTIFIER: identifier,
      LOMI_TERMINAL_REGRESSION_APP_DATA: appData,
      LOMI_TERMINAL_REGRESSION_PHASE: phase,
      LOMI_TERMINAL_REGRESSION_NODE: process.execPath,
      HISTFILE: join(directory, "shell-history"),
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  const completion = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code, signal) => resolve({ code, signal }));
  });
  for (const stream of [child.stdout, child.stderr])
    stream.on("data", (data) => {
      log += data;
      process.stdout.write(data);
    });
  // child.kill targets only this ChildProcess identity. Product quit is exercised
  // before this failure-only deadline; never use taskkill/killall or user PIDs.
  const timeout = setTimeout(() => {
    timedOut = true;
    child.kill();
  }, 180000);
  try {
    const exit = await completion;
    await writeFile(join(directory, `${phase}-native.log`), log);
    const failure = await readFile(
      join(directory, `${phase}-failed.json`),
      "utf8",
    ).catch((error) => {
      if (error.code === "ENOENT") return null;
      throw error;
    });
    assert(!failure, failure ?? "");
    assert(
      !timedOut,
      `${phase}: native product quit did not complete before timeout`,
    );
    assert.equal(
      exit.code,
      0,
      `${phase}: abnormal process exit ${JSON.stringify(exit)}`,
    );
    const receipt = JSON.parse(
      await readFile(join(directory, `${phase}-ready-to-quit.json`), "utf8"),
    );
    assert.equal(receipt.stage, "ready-to-quit");
    const data = receipt.data;
    assert.equal(
      Object.keys(data.pids).length,
      15,
      "Every retained terminal must have a native PID",
    );
    const pids = Object.values(data.pids);
    assert.equal(new Set(pids).size, 15, "PTY PIDs must be distinct");
    const live = (pid) => {
      try {
        process.kill(pid, 0);
        return true;
      } catch (error) {
        assert.equal(
          error.code,
          "ESRCH",
          `Cannot verify PTY retirement ${pid}`,
        );
        return false;
      }
    };
    const retirementDeadline = Date.now() + 5000;
    let remaining = pids.filter(live);
    while (remaining.length && Date.now() < retirementDeadline) {
      await new Promise((resolve) => setTimeout(resolve, 50));
      remaining = pids.filter(live);
    }
    assert.deepEqual(remaining, [], "Product quit left owned PTYs running");
    const saved = JSON.parse(
      await readFile(join(appData, "session.json"), "utf8"),
    );
    assert.equal(saved.version, 5);
    assert.equal(saved.projects.length, 1);
    const restored = saved.projects[0].workspaces[0];
    assert.equal(
      restored.tabs.length,
      8,
      "Product quit must save all eight tabs",
    );
    assert.equal(
      restored.tabs.reduce((n, tab) => n + paneCount(tab.layout), 0),
      15,
      "Product quit must save fifteen panes",
    );
    assert.equal(
      paneCount(restored.tabs.find((t) => t.id === "regression-tab").layout),
      8,
    );
    for (const [label, timing] of Object.entries(data.summary)) {
      assert(
        timing.count > 0 && timing.p95Ms < 100 && timing.maxMs <= 250,
        `${phase}/${label}: software callback timing outside p95<100ms/max<=250ms: ${JSON.stringify(timing)}`,
      );
    }
    assert(
      data.samples.every(
        (s) =>
          s.exactInputVerified &&
          (s.label === "ansi-eight" || s.freshPromptVerified) &&
          Number.isFinite(s.submitToParsedMs) &&
          Number.isFinite(s.submitToRenderMs),
      ),
    );
    if (phase === "exercise") {
      assert(
        data.stages.some(
          (s) => s.label === "ordinary-shell-duration" && s.durationMs >= 25000,
        ),
      );
      for (const total of [2, 4, 8])
        for (const label of [`tabs-${total}`, `panes-${total}`])
          assert(
            data.stages.some((s) => s.label === label),
            `Missing stage ${label}`,
          );
      assert.equal(data.workloads.length, 8);
      for (const workload of data.workloads) {
        assert.equal(workload.metrics.input, workload.expected);
        assert(
          workload.metrics.completedNormally &&
            workload.metrics.durationMs >= 19900,
        );
        let alive = false;
        try {
          process.kill(workload.pid, 0);
          alive = true;
        } catch (error) {
          assert.equal(
            error.code,
            "ESRCH",
            `Cannot confirm workload completion ${workload.pid}`,
          );
        }
        assert(!alive, `Finite workload ${workload.pid} did not terminate`);
      }
      previous = data.panePids;
    } else {
      assert.deepEqual(
        Object.keys(data.panePids).sort(),
        Object.keys(previous).sort(),
        "Restored session IDs must match saved panes",
      );
      for (const [session, pid] of Object.entries(data.panePids))
        assert.notEqual(
          pid,
          previous[session],
          `Restored PTY ${session} reused its prior process`,
        );
    }
    await writeFile(
      join(directory, `${phase}-verified.json`),
      JSON.stringify(
        {
          phase,
          exit,
          savedTabs: restored.tabs.length,
          savedPanes: 15,
          retiredPtyPids: pids,
          summary: data.summary,
        },
        null,
        2,
      ),
    );
    console.log(
      `${phase}: guarded native quit, saved layout, exact native input, PTY identities and software timing passed`,
    );
  } finally {
    clearTimeout(timeout);
    if (child.exitCode === null && child.signalCode === null) child.kill();
  }
}
