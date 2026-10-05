import assert from "node:assert/strict";
import {
  mkdir,
  mkdtemp,
  readFile,
  writeFile,
  rename,
  rm,
} from "node:fs/promises";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { spawn } from "node:child_process";
import { chromium } from "@playwright/test";
import {
  newProject,
  newSession,
  newTab,
  newWorkspace,
} from "../../src/model.ts";
import { terminateOwnedProcessGroup } from "./remote-demo-process.mjs";
if (process.platform !== "darwin" || process.arch !== "arm64")
  throw Error("Native qualification requires macOS ARM64");
const root = resolve(import.meta.dirname, "../.."),
  fixtureDirectory =
    process.env.LOMI_REMOTE_E2E_DIRECTORY || "/tmp/lomi-remote-live-e2e";
const manual = process.argv.includes("--manual");
let cancelled = false;
let finishCancellation;
const cancellation = new Promise((resolve) => {
  finishCancellation = resolve;
});
const cancel = () => {
  cancelled = true;
  finishCancellation();
};
process.on("SIGINT", cancel);
process.on("SIGTERM", cancel);
process.on("SIGHUP", cancel);
const fixture = JSON.parse(
  await readFile(join(fixtureDirectory, "fixture.json"), "utf8"),
);
if (!(Date.parse(fixture.desktopExpiresAt) > Date.now()))
  throw Error(
    "The local test account has expired. Renew the isolated fixture before testing.",
  );
const directory = await mkdtemp(join(fixtureDirectory, "native-run-"));
await writeFile(join(directory, "fixture.json"), JSON.stringify(fixture), {
  mode: 0o600,
});
const identifier = `dev.lomi.remote-probe-${crypto.randomUUID()}`,
  appData = join(homedir(), "Library/Application Support", identifier),
  folder = join(directory, "project");
await mkdir(appData, { mode: 0o700 });
await mkdir(folder, { recursive: true, mode: 0o700 });
const project = newProject(folder, "local:zsh");
const workspace = project.workspaces[0];
workspace.name = "Remote test workspace";
workspace.tabs.push(newTab(folder, "local:zsh", "Background terminals", 2));
project.workspaces.push(newWorkspace(folder, "local:zsh", "Private workspace"));
await writeFile(
  join(appData, "session.json"),
  JSON.stringify({
    ...newSession(),
    sidebar: "workspaces",
    projects: [project],
    activeProjectId: project.id,
  }),
  { mode: 0o600 },
);
const port = manual ? 1448 : 1449,
  config = join(directory, "native-config.json");
const base = JSON.parse(
  await readFile(join(root, "src-tauri/tauri.conf.json"), "utf8"),
);
await writeFile(
  config,
  JSON.stringify({
    identifier,
    build: {
      beforeDevCommand: `pnpm dev --port ${port} --strictPort`,
      devUrl: `http://127.0.0.1:${port}`,
    },
    app: {
      security: {
        devCsp: base.app.security.devCsp.replaceAll(
          "ws://127.0.0.1:1420",
          `ws://127.0.0.1:${port}`,
        ),
      },
    },
  }),
  { mode: 0o600 },
);
for (const name of [
  "native-state.json",
  "native-result.json",
  "native-reply.json",
  "command.json",
])
  await rm(join(directory, name), { force: true });
const child = spawn(
  "pnpm",
  [
    "tauri",
    "dev",
    "--no-watch",
    "--features",
    "remote-probe",
    "--config",
    config,
  ],
  {
    cwd: root,
    detached: true,
    env: {
      ...process.env,
      LOMI_REMOTE_PROBE_DIRECTORY: directory,
      LOMI_REMOTE_PROBE_MANUAL: manual ? "1" : "0",
      LOMI_AUTH_ORIGIN: "http://127.0.0.1:4324",
      LOMI_REMOTE_ORIGIN: "http://127.0.0.1:4322",
    },
    stdio: ["ignore", "pipe", "pipe"],
  },
);
child.detached = true;
let log = "";
for (const s of [child.stdout, child.stderr])
  s.on("data", (d) => {
    if (log.length < 2 * 1024 * 1024) log += d.toString();
  });
let browser,
  context,
  page,
  demoDeadline,
  seq = 0;
const checks = [];
const hasOutput = (screen, marker) =>
  screen.split("\n").some((line) => line.trim() === marker);
const wait = async (fn, timeout = 120000, allowCancelled = false) => {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (cancelled && !allowCancelled)
      throw Error("Local Remote demo cancelled.");
    const r = await fn();
    if (r) return r;
    if (child.exitCode !== null)
      throw Error("Native application exited before qualification");
    await new Promise((r) => setTimeout(r, 100));
  }
  throw Error("Qualification timeout");
};
const jsonFile = async (name) =>
  readFile(join(directory, name), "utf8")
    .then(JSON.parse)
    .catch(() => null);
const command = async (op, data = {}, cleanup = false) => {
  seq++;
  const next = join(directory, "command.next");
  await writeFile(next, JSON.stringify({ seq, op, ...data }), { mode: 0o600 });
  await rename(next, join(directory, "command.json"));
  const reply = await wait(
    async () => {
      const r = await jsonFile("native-reply.json");
      return r?.seq === seq ? r : null;
    },
    cleanup ? 3000 : 20000,
    cleanup,
  );
  assert.equal(reply.ok, true, reply.error);
  return reply.result;
};
try {
  console.log(
    JSON.stringify({
      kind: "remote_native_e2e",
      status: "building",
      identifier,
      artifactDirectory: directory,
    }),
  );
  const state = await wait(async () => {
    const result = await jsonFile("native-result.json");
    if (result && !result.passed) throw Error(result.error);
    return jsonFile("native-state.json");
  }, 600000);
  assert.equal(state.ready, true);
  checks.push("real retained desktop PTY and native host enabled");
  browser = await chromium.launch({
    headless: !manual,
    handleSIGINT: false,
    handleSIGTERM: false,
    handleSIGHUP: false,
  });
  context = await browser.newContext();
  page = await context.newPage();
  page.on("pageerror", (error) =>
    console.error("Browser error:", error.message),
  );
  const relaySockets = [];
  page.on("response", (response) => {
    if (response.status() >= 400)
      console.error(
        "Browser response:",
        response.status(),
        new URL(response.url()).pathname,
      );
  });
  page.on("requestfailed", (request) =>
    console.error(
      "Browser request failed:",
      new URL(request.url()).pathname,
      request.failure()?.errorText,
    ),
  );
  page.on("websocket", (socket) => {
    if (socket.url().includes("/v1/relay")) {
      const record = { closed: false };
      relaySockets.push(record);
      socket.on("close", () => {
        record.closed = true;
      });
    }
  });
  await page.goto("http://127.0.0.1:4322/?auth=cancelled");
  await page.getByRole("link", { name: "Continue with Lomi" }).waitFor();
  assert.equal(
    await page
      .getByRole("navigation", { name: "Shared workspaces", exact: true })
      .count(),
    0,
  );
  checks.push("unauthenticated browser exposes login only");
  await context.addCookies([
    {
      name: fixture.browserCookieName,
      value: fixture.browserToken,
      url: "http://127.0.0.1:4322",
      httpOnly: true,
      sameSite: "Lax",
    },
  ]);
  await page.goto("http://127.0.0.1:4322/");
  await page
    .getByRole("navigation", { name: "Shared workspaces", exact: true })
    .waitFor();
  if (manual) {
    await writeFile(
      join(directory, "demo-ready.json"),
      JSON.stringify({
        ready: true,
        url: fixture.remoteOrigin,
        workspaceId: workspace.id,
      }),
      { mode: 0o600 },
    );
    console.log(
      JSON.stringify({
        kind: "remote_manual_demo",
        ready: true,
        url: "http://127.0.0.1:4322",
        expiresAt: fixture.desktopExpiresAt,
        instructions: [
          "The visible browser and isolated Lomi desktop use the same local test account.",
          "Desktop: right-click Remote test workspace → Share remotely, then confirm.",
          "Browser: choose Remote test workspace in the sidebar; all three terminal panes follow the workspace layout.",
          "New terminal panes join the shared workspace automatically; Stop sharing removes browser access.",
          "Close the desktop window to test background sessions; use the Dock to reopen it.",
          "Press Ctrl+C in this terminal to stop the local demo.",
        ],
      }),
    );
    await Promise.race([
      cancellation,
      new Promise((resolve) => {
        const finish = () => {
          clearTimeout(timer);
          resolve();
        };
        const timer = (demoDeadline = setTimeout(
          finish,
          Math.max(1, Date.parse(fixture.desktopExpiresAt) - Date.now()),
        ));
        child.once("exit", finish);
      }),
    ]);
    if (child.exitCode === null)
      await command("quit", {}, true).catch(() => {});
  } else {
    await command("ui-workspace-action", {
      workspaceId: workspace.id,
      action: "share",
    });
    await wait(async () => {
      const r = await command("inspect");
      return r.remote.workspaces?.find(
        (w) => w.id === workspace.id && w.shared && w.online,
      );
    }, 30000);
    let publicWorkspaces;
    const published = await wait(async () => {
      const response = await page.request.get(
        "http://127.0.0.1:4322/v1/remote/workspaces",
        { headers: { "X-Lomi-Request": "1" } },
      );
      assert.equal(response.ok(), true);
      publicWorkspaces = (await response.json()).workspaces;
      return publicWorkspaces.find(
        (w) => w.id === workspace.id && w.hostId === state.remote.hostId,
      );
    }, 30000);
    assert.ok(
      published,
      "Native shared workspace is discoverable by the same account",
    );
    assert.equal(
      published.sessionIds.length,
      3,
      "Inactive split panes are shared together",
    );
    assert.equal(
      publicWorkspaces.some(
        (w) =>
          w.id === project.workspaces[1].id && w.hostId === state.remote.hostId,
      ),
      false,
    );
    const publicWire = JSON.stringify(publicWorkspaces);
    assert.equal(publicWire.includes(workspace.name), false);
    assert.equal(publicWire.includes(folder), false);
    checks.push(
      "one native workspace menu action starts and shares all three terminal panes",
    );
    checks.push(
      "unshared workspace excluded and cloud inventory contains no private names or paths",
    );
    const card = page.getByRole("link", { name: workspace.name, exact: true });
    const terminalPanel = page.locator(".terminal-panel").first();
    await card.click();
    await page.getByRole("tabpanel").waitFor();
    assert.equal(await page.locator(".terminal-tabs button").count(), 2);
    assert.equal(
      await page.getByText("Full pairing fingerprint", { exact: true }).count(),
      0,
    );
    await terminalPanel
      .getByRole("status")
      .filter({ hasText: /^In control$/ })
      .waitFor();
    await wait(
      async () =>
        await terminalPanel
          .locator(".xterm-screen")
          .innerText()
          .then((v) => hasOutput(v, "LOMI_NATIVE_REMOTE_READY"))
          .catch(() => false),
      30000,
    );
    checks.push(
      "automatic account enrollment and encrypted native snapshot without token exchange",
    );
    const input = terminalPanel.locator(".xterm-helper-textarea");
    let snapshotMarker = "LOMI_NATIVE_REMOTE_READY";
    const activateTerminal = async (marker) => {
      await wait(
        async () =>
          hasOutput(
            await terminalPanel.locator(".xterm-screen").innerText(),
            snapshotMarker,
          ),
        30000,
      );
      await input.focus();
      await page.keyboard.insertText(`printf '${marker}\\n'`);
      await page.keyboard.press("Enter");
      await wait(
        async () =>
          hasOutput(
            await terminalPanel.locator(".xterm-screen").innerText(),
            marker,
          ),
        30000,
      );
      await terminalPanel
        .getByRole("status")
        .filter({ hasText: /^In control$/ })
        .waitFor();
      snapshotMarker = marker;
    };
    await input.waitFor({ state: "attached" });
    await input.focus();
    await page.keyboard.insertText("printf 'LOMI_BROWSER_INPUT_OK\\n'");
    await page.keyboard.press("Enter");
    await wait(
      async () =>
        await terminalPanel
          .locator(".xterm-screen")
          .innerText()
          .then((v) => hasOutput(v, "LOMI_BROWSER_INPUT_OK"))
          .catch(() => false),
      30000,
    );
    checks.push("leased browser input writes real native PTY");
    const reconnectTerminal = async (previousSockets) => {
      await wait(async () => {
        const reconnect = page.getByRole("button", {
          name: "Reconnect",
          exact: true,
        });
        if (await reconnect.isVisible().catch(() => false))
          await reconnect.click();
        return (
          relaySockets.length > previousSockets &&
          (await terminalPanel
            .getByRole("status")
            .filter({ hasText: /^(Observing|In control)$/ })
            .isVisible()
            .catch(() => false))
        );
      }, 30000);
      await wait(
        async () =>
          hasOutput(
            await terminalPanel.locator(".xterm-screen").innerText(),
            "LOMI_NATIVE_REMOTE_READY",
          ),
        30000,
      );
    };
    const originalSessions = (await command("inspect")).remote.sessions.map(
      (s) => ({ id: s.id, epoch: s.epoch }),
    );
    for (let cycle = 1; cycle <= 2; cycle++) {
      const socketCount = relaySockets.length;
      const paused = await command("idle-hour");
      assert.equal(paused.remote.paused, true);
      assert.equal(paused.remote.enabled, false);
      assert.equal(
        paused.remote.workspaces.find((w) => w.id === workspace.id).shared,
        true,
      );
      assert.deepEqual(
        paused.remote.sessions.map((s) => ({ id: s.id, epoch: s.epoch })),
        originalSessions,
      );
      await wait(
        async () => relaySockets.slice(0, socketCount).every((s) => s.closed),
        15000,
      );
      await command("local-input", {
        data: `printf 'LOMI_LOCAL_PAUSED_${cycle}\\n'\n`,
      });
      assert.equal((await command("inspect")).remote.paused, true);
      const resumed = await command("resume");
      assert.equal(resumed.remote.paused, false);
      assert.equal(resumed.remote.enabled, true);
      await reconnectTerminal(socketCount);
      await input.focus();
      await page.keyboard.insertText(
        `printf 'LOMI_BROWSER_RESUMED_${cycle}\\n'`,
      );
      await page.keyboard.press("Enter");
      await wait(async () => {
        const screen = await terminalPanel.locator(".xterm-screen").innerText();
        return (
          hasOutput(screen, `LOMI_LOCAL_PAUSED_${cycle}`) &&
          hasOutput(screen, `LOMI_BROWSER_RESUMED_${cycle}`)
        );
      }, 30000);
      await terminalPanel
        .getByRole("status")
        .filter({ hasText: /^In control$/ })
        .waitFor();
      assert.deepEqual(
        (await command("inspect")).remote.sessions.map((s) => ({
          id: s.id,
          epoch: s.epoch,
        })),
        originalSessions,
      );
    }
    checks.push(
      "two idle-hour pauses close real relay sockets, retain workspace consent and PTYs, and public Resume restores encrypted input without desktop restart",
    );
    const exactBefore = await command("snapshot");
    const helperSocketCount = relaySockets.length;
    assert.equal((await command("helper-fault")).faultDetected, true);
    await wait(
      async () =>
        relaySockets.slice(0, helperSocketCount).every((s) => s.closed),
      15000,
    );
    const exactAfter = await wait(
      async () => command("snapshot").catch(() => null),
      30000,
    );
    assert.deepEqual(
      exactAfter,
      exactBefore,
      "Helper restoration preserves exact terminal state and watermark without replaying bytes",
    );
    assert.deepEqual(
      (await command("inspect")).remote.sessions.map((s) => ({
        id: s.id,
        epoch: s.epoch,
      })),
      originalSessions,
    );
    await reconnectTerminal(helperSocketCount);
    await input.focus();
    await page.keyboard.insertText("printf 'LOMI_BROWSER_HELPER_RECOVERED\\n'");
    await page.keyboard.press("Enter");
    await wait(
      async () =>
        hasOutput(
          await terminalPanel.locator(".xterm-screen").innerText(),
          "LOMI_BROWSER_HELPER_RECOVERED",
        ),
      30000,
    );
    checks.push(
      "real helper child failure fences old encrypted sockets, restores exact snapshot/watermark in the same epoch, and accepts input over a new encrypted connection",
    );
    await command("ui-terminal-action", { action: "new-tab" });
    await wait(
      async () => (await page.locator(".terminal-tabs button").count()) === 3,
      30000,
    );
    await terminalPanel
      .getByRole("status")
      .filter({ hasText: /^Observing$/ })
      .waitFor();
    await activateTerminal("LOMI_SCOPE_NEW_TAB");
    await command("ui-terminal-action", { action: "split" });
    await wait(
      async () =>
        (await command("inspect")).remote.sessions.filter((s) => s.available)
          .length === 5,
      30000,
    );
    await terminalPanel
      .getByRole("status")
      .filter({ hasText: /^Observing$/ })
      .waitFor();
    await activateTerminal("LOMI_SCOPE_SPLIT");
    await page.locator(".terminal-tabs button").nth(2).click();
    await wait(
      async () => (await page.locator(".terminal-panel").count()) === 2,
      30000,
    );
    assert.equal(await page.locator(".terminal-tabs button").count(), 3);
    await page.locator(".terminal-tabs button").first().click();
    await terminalPanel
      .getByRole("status")
      .filter({ hasText: /^In control$/ })
      .waitFor();
    checks.push(
      "new terminal tab and split update signed scope, reconnect observing, and accept terminal activation control",
    );
    const selectedUrl = page.url();
    await page.reload();
    await terminalPanel
      .getByRole("status")
      .filter({ hasText: /^In control$/ })
      .waitFor();
    assert.equal(page.url(), selectedUrl);
    await wait(
      async () =>
        await terminalPanel
          .locator(".xterm-screen")
          .innerText()
          .then((v) => hasOutput(v, "LOMI_BROWSER_INPUT_OK"))
          .catch(() => false),
      30000,
    );
    checks.push(
      "browser refresh automatically re-enrolls and restores selected terminal snapshot",
    );
    await command("close-window");
    await wait(async () => !(await command("inspect")).windowVisible, 10000);
    await input.focus();
    await page.keyboard.insertText("printf 'LOMI_HIDDEN_NATIVE_OK\\n'");
    await page.keyboard.press("Enter");
    await wait(
      async () =>
        await terminalPanel
          .locator(".xterm-screen")
          .innerText()
          .then((v) => hasOutput(v, "LOMI_HIDDEN_NATIVE_OK"))
          .catch(() => false),
      30000,
    );
    checks.push(
      "desktop window close keeps renderer and native sessions alive",
    );
    await command("local-input", { data: "printf 'LOMI_LOCAL_PRIORITY\\n'\n" });
    await wait(
      async () =>
        await terminalPanel
          .getByRole("status")
          .filter({ hasText: /^Observing$/ })
          .isVisible(),
      15000,
    );
    checks.push("local human input invalidates remote lease");
    await page
      .getByRole("button", { name: "Collapse sidebar", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Expand sidebar", exact: true })
      .click();
    assert.equal(
      await page.getByRole("region", { name: "Shared terminal" }).count(),
      1,
    );
    checks.push("sidebar navigation retains connected terminal");
    await command("reopen-window");
    await activateTerminal("LOMI_LOCAL_RECLAIM");
    await command("ui-workspace-action", {
      workspaceId: workspace.id,
      action: "stop",
    });
    await wait(async () => {
      const r = await command("inspect");
      return r.remote.workspaces?.find(
        (w) => w.id === workspace.id && !w.shared,
      );
    }, 10000);
    await wait(
      async () =>
        !(await terminalPanel
          .getByRole("status")
          .filter({ hasText: /^In control$/ })
          .count()),
      15000,
    );
    const stoppedInventory = await page.request.get(
      "http://127.0.0.1:4322/v1/remote/workspaces",
      {
        headers: { "X-Lomi-Request": "1" },
      },
    );
    assert.equal(
      (await stoppedInventory.json()).workspaces.some(
        (w) => w.id === workspace.id && w.hostId === state.remote.hostId,
      ),
      false,
    );
    await command("local-input", {
      data: "printf 'LOMI_LOCAL_AFTER_UNSHARE\\n'\n",
    });
    checks.push("Stop sharing fences browser access while local PTYs continue");
    assert.equal(
      (await command("inspect")).remote.sessions.filter((s) => s.available)
        .length,
      5,
    );
    await command("ui-workspace-action", {
      workspaceId: workspace.id,
      action: "share",
    });
    await wait(async () => {
      const current = await command("inspect");
      return current.remote.workspaces?.some(
        (w) => w.id === workspace.id && w.shared,
      );
    }, 10000);
    await card.click();
    await terminalPanel
      .getByRole("status")
      .filter({ hasText: /^(Observing|In control)$/ })
      .waitFor();
    await activateTerminal("LOMI_RESHARED_INPUT");
    assert.equal(await page.locator(".terminal-tabs button").count(), 3);
    checks.push(
      "re-sharing enrolls a fresh scope automatically without refreshing the browser",
    );
    const pausedBeforeStop = await command("idle-hour");
    assert.equal(pausedBeforeStop.remote.paused, true);
    const pausedStop = await command("share-workspace", {
      workspaceId: workspace.id,
      shared: false,
    });
    assert.equal(pausedStop.remote.paused, true);
    assert.equal(pausedStop.remote.enabled, false);
    assert.equal(
      pausedStop.remote.workspaces.find((w) => w.id === workspace.id).shared,
      false,
    );
    await command("resume");
    await command("ui-workspace-action", {
      workspaceId: workspace.id,
      action: "share",
    });
    await wait(async () => {
      const current = await command("inspect");
      return current.remote.workspaces?.some(
        (w) => w.id === workspace.id && w.shared,
      );
    }, 10000);
    await card.click();
    await terminalPanel
      .getByRole("status")
      .filter({ hasText: /^(Observing|In control)$/ })
      .waitFor();
    await activateTerminal("LOMI_RESHARED_AFTER_PAUSED_STOP");
    checks.push(
      "paused Stop retains pause, Resume and fresh Share re-enroll the same browser without reload",
    );
    const retainedWorkspaceId = project.workspaces[1].id;
    await command("ui-workspace-action", {
      workspaceId: retainedWorkspaceId,
      action: "share",
    });
    await wait(async () => {
      const current = await command("inspect");
      return current.remote.workspaces?.some(
        (w) => w.id === retainedWorkspaceId && w.shared,
      );
    }, 10000);
    const barrier = async (mode) => {
      const response = await fetch(
        `${fixture.cleanupOrigin}/fixture/empty-heartbeat`,
        {
          method: "POST",
          headers: {
            authorization: `Bearer ${fixture.cleanupToken}`,
            "content-type": "application/json",
          },
          body: JSON.stringify({ mode }),
        },
      );
      assert.equal(response.status, 200, "Fixture heartbeat barrier failed");
      return response.json();
    };
    await command("idle-hour");
    await barrier("hold");
    try {
      await command("share-workspace-background", {
        workspaceId: workspace.id,
        shared: false,
      });
      await wait(async () => (await barrier("status")).holding, 3000);
      await command("resume-background");
      await wait(async () => (await command("inspect")).remote.enabled, 3000);
      await new Promise((resolve) => setTimeout(resolve, 250));
      assert.equal(await jsonFile("native-overlap-resume.json"), null);
      const blocked = await barrier("status");
      assert.equal(blocked.nonemptyAdmittedDuringHold, 0);
      assert.equal(blocked.timedOut, false);
    } finally {
      await barrier("release");
    }
    const overlappingStop = await wait(
      async () => await jsonFile("native-overlap-stop.json"),
      10000,
    );
    const overlappingResume = await wait(
      async () => await jsonFile("native-overlap-resume.json"),
      10000,
    );
    assert.equal(overlappingStop.ok, true, overlappingStop.error);
    assert.equal(overlappingResume.ok, true, overlappingResume.error);
    assert.equal(
      overlappingResume.remote.workspaces.find(
        (w) => w.id === retainedWorkspaceId,
      ).shared,
      true,
    );
    const settledBarrier = await barrier("status");
    assert.equal(settledBarrier.completed, 1);
    assert.equal(settledBarrier.nonemptyAdmittedDuringHold, 0);
    const retainedInventory = await page.request.get(
      "http://127.0.0.1:4322/v1/remote/workspaces",
      { headers: { "X-Lomi-Request": "1" } },
    );
    assert.equal(
      (await retainedInventory.json()).workspaces.some(
        (w) => w.id === retainedWorkspaceId && w.hostId === state.remote.hostId,
      ),
      true,
    );
    await command("share-workspace", {
      workspaceId: workspace.id,
      shared: true,
    });
    await wait(async () => {
      const current = await command("inspect");
      return current.remote.workspaces?.some(
        (w) => w.id === workspace.id && w.shared,
      );
    }, 10000);
    await card.click();
    await terminalPanel
      .getByRole("status")
      .filter({ hasText: /^(Observing|In control)$/ })
      .waitFor();
    await activateTerminal("LOMI_RESHARED_AFTER_OVERLAPPING_RESUME");
    checks.push(
      "held paused Stop heartbeat blocks overlapping Resume publication, preserves another shared workspace and reconnects without reload",
    );
    await rm(join(directory, "native-overlap-stop.json"), { force: true });
    await rm(join(directory, "native-overlap-resume.json"), { force: true });
    await command("idle-hour");
    await barrier("hold");
    try {
      await command("share-workspace-background", {
        workspaceId: workspace.id,
        shared: false,
      });
      await wait(async () => (await barrier("status")).holding, 3000);
      const expiredStop = await wait(
        async () => await jsonFile("native-overlap-stop.json"),
        8000,
      );
      assert.equal(expiredStop.ok, true, expiredStop.error);
      assert.equal(expiredStop.remote.paused, true);
      assert.equal((await barrier("status")).holding, true);
      await command("resume-background");
      const resumedBeforeRelease = await wait(
        async () => await jsonFile("native-overlap-resume.json"),
        5000,
      );
      assert.equal(resumedBeforeRelease.ok, true, resumedBeforeRelease.error);
      assert.equal(resumedBeforeRelease.remote.enabled, true);
      assert.equal(
        (await barrier("status")).nonemptyAdmittedDuringHold > 0,
        true,
      );
    } finally {
      await barrier("release");
    }
    const rejectedLatePublication = await wait(async () => {
      const status = await barrier("status");
      return !status.holding && status.completed > 0 ? status : null;
    }, 3000);
    assert.equal(rejectedLatePublication.completedStatus, 409);
    assert.equal(rejectedLatePublication.timedOut, false);
    const afterLatePublication = await page.request.get(
      "http://127.0.0.1:4322/v1/remote/workspaces",
      { headers: { "X-Lomi-Request": "1" } },
    );
    assert.equal(
      (await afterLatePublication.json()).workspaces.some(
        (w) => w.id === retainedWorkspaceId && w.hostId === state.remote.hostId,
      ),
      true,
    );
    await command("share-workspace", {
      workspaceId: workspace.id,
      shared: true,
    });
    await wait(async () => {
      const current = await command("inspect");
      return current.remote.workspaces?.some(
        (w) => w.id === workspace.id && w.shared,
      );
    }, 10000);
    await card.click();
    await terminalPanel
      .getByRole("status")
      .filter({ hasText: /^(Observing|In control)$/ })
      .waitFor();
    await activateTerminal("LOMI_RESHARED_AFTER_LATE_PAUSED_HEARTBEAT");
    checks.push(
      "paused Stop returns at its deadline and late empty heartbeat is rejected after Resume, retaining another workspace and same-browser access",
    );
    await command("share-workspace", {
      workspaceId: retainedWorkspaceId,
      shared: false,
    });
    const beforeActiveStop = await page.request.get(
      "http://127.0.0.1:4322/v1/remote/workspaces",
      { headers: { "X-Lomi-Request": "1" } },
    );
    const previousActiveScope = (await beforeActiveStop.json()).workspaces.find(
      (w) => w.id === workspace.id && w.hostId === state.remote.hostId,
    );
    assert.ok(previousActiveScope);
    await rm(join(directory, "native-overlap-stop.json"), { force: true });
    await barrier("hold");
    let freshActiveScope;
    try {
      await command("share-workspace-background", {
        workspaceId: workspace.id,
        shared: false,
      });
      await wait(async () => (await barrier("status")).holding, 3000);
      const activeStop = await wait(
        async () => await jsonFile("native-overlap-stop.json"),
        8000,
      );
      assert.equal(activeStop.ok, true, activeStop.error);
      assert.equal(activeStop.remote.enabled, true);
      assert.equal((await barrier("status")).holding, true);
      await command("share-workspace", {
        workspaceId: workspace.id,
        shared: true,
      });
      freshActiveScope = await wait(async () => {
        const response = await page.request.get(
          "http://127.0.0.1:4322/v1/remote/workspaces",
          { headers: { "X-Lomi-Request": "1" } },
        );
        return (await response.json()).workspaces.find(
          (w) =>
            w.id === workspace.id &&
            w.hostId === state.remote.hostId &&
            w.epoch !== previousActiveScope.epoch,
        );
      }, 5000);
      await card.click();
      await terminalPanel
        .getByRole("status")
        .filter({ hasText: /^(Observing|In control)$/ })
        .waitFor();
      await activateTerminal("LOMI_FRESH_SHARE_BEFORE_LATE_ACTIVE_STOP");
    } finally {
      await barrier("release");
    }
    const rejectedActiveStop = await wait(async () => {
      const status = await barrier("status");
      return !status.holding && status.completed > 0 ? status : null;
    }, 3000);
    assert.equal(rejectedActiveStop.completedStatus, 409);
    assert.equal(rejectedActiveStop.timedOut, false);
    const afterActiveStop = await page.request.get(
      "http://127.0.0.1:4322/v1/remote/workspaces",
      { headers: { "X-Lomi-Request": "1" } },
    );
    assert.equal(
      (await afterActiveStop.json()).workspaces.some(
        (w) =>
          w.id === workspace.id &&
          w.hostId === state.remote.hostId &&
          w.epoch === freshActiveScope.epoch,
      ),
      true,
    );
    await activateTerminal("LOMI_FRESH_SHARE_AFTER_LATE_ACTIVE_STOP");
    checks.push(
      "active Stop returns at its deadline and its late heartbeat is rejected after fresh Share, retaining the new scope and encrypted input grant",
    );
    const current = await command("inspect");
    const grant = current.remote.grants.find(
      (g) => !g.revoked && g.sessionIds.includes(state.sessionId),
    );
    assert.ok(grant);
    await command("revoke", { grantId: grant.id });
    await wait(
      async () =>
        (await page.getByText("Access revoked", { exact: true }).count()) > 0,
      15000,
    );
    await new Promise((resolve) => setTimeout(resolve, 3000));
    assert.equal(
      await page
        .getByRole("status")
        .filter({ hasText: /^In control$/ })
        .count(),
      0,
    );
    checks.push(
      "explicit browser revoke stays revoked instead of automatically re-enrolling",
    );

    const unavailable = await command("terminal-unavailable");
    assert.equal(unavailable.remote.enabled, true);
    assert.equal(
      unavailable.remote.sessions.find((s) => s.id === state.sessionId)
        .available,
      false,
    );
    assert.match(
      unavailable.remote.workspaces.find((w) => w.id === workspace.id).message,
      /exact Remote state is unavailable/,
    );
    const unavailableStop = await command("share-workspace", {
      workspaceId: workspace.id,
      shared: false,
    });
    assert.equal(
      unavailableStop.remote.workspaces.find((w) => w.id === workspace.id)
        .shared,
      false,
    );
    assert.equal(unavailableStop.remote.enabled, true);
    assert.equal(unavailableStop.remote.paused, false);
    await command("local-input", {
      data: "printf 'LOMI_LOCAL_AFTER_UNAVAILABLE_UNSHARE\\n'\n",
    });
    const unavailableInventory = await page.request.get(
      "http://127.0.0.1:4322/v1/remote/workspaces",
      { headers: { "X-Lomi-Request": "1" } },
    );
    assert.equal(
      (await unavailableInventory.json()).workspaces.some(
        (w) => w.id === workspace.id && w.hostId === state.remote.hostId,
      ),
      false,
    );
    checks.push(
      "active unshare succeeds with an unavailable terminal model, removes cloud inventory and keeps the local PTY writable",
    );

    await command("quit");
    await wait(async () => await jsonFile("native-result.json"), 10000);
    await new Promise((resolve, reject) => {
      if (child.exitCode !== null) return resolve();
      const timer = setTimeout(
        () => reject(Error("Explicit quit did not exit the desktop")),
        15000,
      );
      child.once("exit", () => {
        clearTimeout(timer);
        resolve();
      });
    });
    checks.push("explicit quit stops native sessions");
    await writeFile(
      join(directory, "receipt.json"),
      JSON.stringify(
        {
          kind: "remote_native_browser_e2e",
          platform: "macOS ARM64",
          engine: "Chromium",
          passed: true,
          checks,
        },
        null,
        2,
      ) + "\n",
      { mode: 0o600 },
    );
    console.log(
      JSON.stringify({
        kind: "remote_native_browser_e2e",
        passed: true,
        checks,
      }),
    );
  }
} catch (error) {
  await page
    ?.screenshot({ path: join(directory, "failure.png"), fullPage: true })
    .catch(() => {});
  await writeFile(
    join(directory, "receipt.json"),
    JSON.stringify(
      {
        kind: "remote_native_browser_e2e",
        passed: false,
        error: error.message,
        checks,
      },
      null,
      2,
    ) + "\n",
    { mode: 0o600 },
  );
  if (!cancelled) throw error;
  console.log(JSON.stringify({ kind: "remote_manual_demo", stopped: true }));
} finally {
  clearTimeout(demoDeadline);
  let stopped = false;
  try {
    await terminateOwnedProcessGroup(child);
    stopped = true;
  } catch {
    process.exitCode = 1;
    console.error(
      "The owned demo process group could not be stopped; its app data was retained.",
    );
  }
  if (browser) {
    let closeDeadline;
    await Promise.race([
      browser.close().catch(() => {}),
      new Promise((resolve) => {
        closeDeadline = setTimeout(resolve, 5000);
      }),
    ]);
    clearTimeout(closeDeadline);
  }
  await writeFile(join(directory, "native.log"), log, { mode: 0o600 }).catch(
    () => {},
  );
  if (stopped) {
    await rm(appData, { recursive: true, force: true }).catch(() => {});
    await rm(join(directory, "fixture.json"), { force: true }).catch(() => {});
  }
  process.removeListener("SIGINT", cancel);
  process.removeListener("SIGTERM", cancel);
  process.removeListener("SIGHUP", cancel);
}
