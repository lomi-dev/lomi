import {
  mkdtemp,
  mkdir,
  readFile,
  writeFile,
  rm,
  copyFile,
} from "node:fs/promises";
import { tmpdir, homedir } from "node:os";
import { join, resolve } from "node:path";
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { browserFramePage } from "../mcp/browser-frame-pages.mjs";
import { newSession, newProject } from "../../src/model.ts";

if (process.platform !== "darwin" || process.arch !== "arm64")
  throw Error("Native control qualification requires macOS ARM64.");
const root = resolve(import.meta.dirname, "../..");
const directory = await mkdtemp(join(tmpdir(), "lomi-mcp-control-"));
const androidRoot = process.env.LOMI_ANDROID_PRODUCT_DIRECTORY;
if (androidRoot) {
  const metadata = JSON.parse(
    await readFile(join(androidRoot, "devices.json"), "utf8"),
  );
  const device = metadata.devices.find((d) => d.name === "MCP qualification");
  if (!device) throw Error("The isolated MCP qualification device is missing.");
  await writeFile(
    join(directory, "android-fixture.json"),
    JSON.stringify({ deviceId: device.id, name: device.name }),
  );
}
const identifier = `dev.lomi.mcp-control-${Date.now()}`;
const appData = join(homedir(), "Library/Application Support", identifier);
const folder = join(directory, "project");
await mkdir(folder);
await writeFile(
  join(folder, "mcp-read-fixture.txt"),
  "Disk Zażółć 🙂\r\nsecond line\r\n",
);
await writeFile(join(folder, ".env.fixture"), "PRIVATE_FIXTURE_VALUE=unshared");
if (process.env.LOMI_ANDROID_PRODUCT_DIRECTORY) {
  const { dirname } = await import("node:path");
  for (const [source, target] of [
    ["tests/native/build-mcp-apk.mjs", "build-mcp-apk.mjs"],
    ["tests/native/android-input/AndroidManifest.xml", "AndroidManifest.xml"],
    [
      "tests/native/android-input/src/org/lomi/inputtest/InputTest.java",
      "InputTest.java",
    ],
  ])
    await copyFile(join(root, source), join(folder, target));
  await writeFile(
    join(folder, "android-build.json"),
    JSON.stringify({
      javaHome:
        process.env.LOMI_MCP_JAVA_HOME ??
        "/Library/Java/JavaVirtualMachines/zulu-17.jdk/Contents/Home",
      androidJar:
        process.env.LOMI_MCP_ANDROID_JAR ??
        join(homedir(), "Library/Android/sdk/platforms/android-36/android.jar"),
      buildTools:
        process.env.LOMI_MCP_BUILD_TOOLS ??
        join(homedir(), "Library/Android/sdk/build-tools/36.0.0"),
      fixtureKey: join(
        dirname(process.env.LOMI_ANDROID_PRODUCT_DIRECTORY),
        "development-tools/input-build/fixture.p12",
      ),
    }),
  );
  const quoteBuild = (value) => "'" + value.replaceAll("'", "'\\''") + "'";
  await writeFile(
    join(directory, "apk-build-command.json"),
    JSON.stringify({
      command: `${quoteBuild(process.execPath)} build-mcp-apk.mjs`,
    }),
  );
}

await mkdir(appData, { recursive: true });
let deniedRequests = 0;
const blockedServer = createServer((_req, res) => {
  deniedRequests++;
  void writeFile(
    join(directory, "browser-denied-requests.json"),
    JSON.stringify(deniedRequests),
  );
  res.end("This origin was not approved.");
});
// Reserve a test-only port for explicit origin approval, then release it. The
// actual server must be started by the native fixture through its owned PTY.
const portReservation = createServer();
await new Promise((resolve) => blockedServer.listen(0, "127.0.0.1", resolve));
await new Promise((resolve) => portReservation.listen(0, "127.0.0.1", resolve));
const serverPort = portReservation.address().port;
await new Promise((resolve) => portReservation.close(resolve));
const closeStressServer =
  process.env.LOMI_MCP_BROWSER_LOGS_ONLY ||
  process.env.LOMI_MCP_BROWSER_FRAMES_ONLY ||
  process.env.LOMI_MCP_ANDROID_LAYOUT_ONLY ||
  process.env.LOMI_MCP_ANDROID_SETUP_ONLY ||
  process.env.LOMI_MCP_CHAT_SEND_ONLY ||
  process.env.LOMI_MCP_CHAT_DRAFT_ONLY ||
  process.env.LOMI_MCP_CHAT_OPEN_ONLY ||
  process.env.LOMI_MCP_CHAT_READ_ONLY ||
  process.env.LOMI_MCP_SETTINGS_THEMES_ONLY ||
  process.env.LOMI_MCP_SETTINGS_KEYBINDS_ONLY ||
  process.env.LOMI_MCP_SETTINGS_TERMINAL_ONLY ||
  process.env.LOMI_MCP_SETTINGS_UPDATE_ONLY ||
  process.env.LOMI_MCP_SETTINGS_READ_ONLY ||
  process.env.LOMI_MCP_SETTINGS_OPEN_ONLY ||
  process.env.LOMI_MCP_PROJECT_OPEN_ONLY ||
  process.env.LOMI_MCP_CLOSE_STRESS_ONLY ||
  process.env.LOMI_MCP_PROJECT_CLOSE_ONLY
    ? createServer((_request, response) =>
        response.end(
          process.env.LOMI_MCP_BROWSER_FRAMES_ONLY ||
            process.env.LOMI_MCP_BROWSER_LOGS_ONLY
            ? browserFramePage(_request.url ?? "/frames")
            : "<!doctype html><title>Close fixture</title><p>Owned page</p>",
        ),
      )
    : null;
if (closeStressServer)
  await new Promise((resolve) =>
    closeStressServer.listen(serverPort, "127.0.0.1", resolve),
  );

const quote = (value) => "'" + value.replaceAll("'", "'\\''") + "'";
await writeFile(join(directory, "browser-denied-requests.json"), "0");
await writeFile(
  join(directory, "browser-fixture.json"),
  JSON.stringify({
    origin: `http://127.0.0.1:${serverPort}`,
    blocked: `http://127.0.0.1:${blockedServer.address().port}`,
    command: [
      process.execPath,
      join(root, "tests/mcp/browser-server.mjs"),
      directory,
      String(serverPort),
      String(blockedServer.address().port),
    ]
      .map(quote)
      .join(" "),
  }),
);
const project = newProject(folder, "local:zsh");
const workspace = project.workspaces[0];
workspace.name = "Visible workspace";
// An editor avoids starting any shell during enrollment qualification.
await writeFile(
  join(folder, "fixture.txt"),
  "MCP native control qualification\n",
);
workspace.tabs = [
  {
    type: "file",
    id: "mcp-control-fixture",
    title: "fixture.txt",
    root: folder,
    relative: "fixture.txt",
  },
];
workspace.activeTabId = "mcp-control-fixture";
project.workspaces.push({
  ...workspace,
  id: "foreign-workspace",
  name: "Private workspace",
  tabs: [],
  activeTabId: "",
});
const projects = [project];
if (
  process.env.LOMI_MCP_PROJECT_CLOSE_ONLY ||
  process.env.LOMI_MCP_ANDROID_LAYOUT_ONLY
) {
  const retainedFolder = join(directory, "retained-project");
  await mkdir(retainedFolder);
  const retained = newProject(retainedFolder, "local:zsh");
  retained.workspaces[0].name = "Retained other project";
  projects.push(retained);
}
await writeFile(
  join(appData, "session.json"),
  JSON.stringify({
    ...newSession(),
    projects,
    activeProjectId: project.id,
  }),
);
const config = join(directory, "config.json");
await writeFile(
  config,
  JSON.stringify({
    identifier,
    build: {
      beforeDevCommand: "pnpm dev --port 1444 --strictPort",
      devUrl: "http://127.0.0.1:1444",
    },
  }),
);
console.log(`Native MCP control artifacts: ${directory}`);
const child = spawn(
  "pnpm",
  ["tauri", "dev", "--no-watch", "--features", "mcp-probe", "--config", config],
  {
    cwd: root,
    env: {
      ...process.env,
      GIT_CONFIG_GLOBAL: "/dev/null",
      GIT_CONFIG_NOSYSTEM: "1",
      LOMI_MCP_CONTROL_PROBE_DIRECTORY: directory,
    },
    detached: true,
    stdio: ["ignore", "pipe", "pipe"],
  },
);
let childClosed = false;
const closed = new Promise((resolve) =>
  child.once("close", () => {
    childClosed = true;
    resolve();
  }),
);
const waitForExit = async (ms) => {
  let timer;
  await Promise.race([
    closed,
    new Promise((resolve) => {
      timer = setTimeout(resolve, ms);
    }),
  ]);
  clearTimeout(timer);
};
let log = "";
for (const stream of [child.stdout, child.stderr])
  stream.on("data", (bytes) => {
    log += bytes;
    process.stdout.write(bytes);
  });
let result;
try {
  // This budget includes a cold native build and the growing full-domain
  // regression. Per-operation/renderer deadlines remain unchanged.
  const deadline = Date.now() + 600_000;
  while (!result && Date.now() < deadline) {
    try {
      result = JSON.parse(
        await readFile(join(directory, "result.json"), "utf8"),
      );
    } catch (error) {
      if (error.code !== "ENOENT" && !(error instanceof SyntaxError))
        throw error;
    }
    if (!result) {
      if (child.exitCode !== null)
        throw Error(`Native probe exited: ${child.exitCode}`);
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
  }
  if (!result) throw Error("Native control probe timed out");
  if (result.stage === "passed" && process.env.LOMI_MCP_BROWSER_LOGS_ONLY) {
    const proof = JSON.parse(
      await readFile(join(directory, "browser-expanded-logs.json"), "utf8"),
    );
    if (
      deniedRequests !== 0 ||
      result.data?.profile !== "browser-logs-only" ||
      proof.checks?.length !== 11 ||
      proof.closed?.structuredContent?.data?.state !== "succeeded"
    )
      throw Error("Browser log qualification returned incomplete evidence");
  } else if (
    result.stage === "passed" &&
    process.env.LOMI_MCP_BROWSER_FRAMES_ONLY
  ) {
    const proof = JSON.parse(
      await readFile(join(directory, "browser-frames.json"), "utf8"),
    );
    if (
      deniedRequests !== 0 ||
      result.data?.profile !== "browser-frames-only" ||
      proof.checks?.length !== 13 ||
      proof.closed?.structuredContent?.data?.state !== "succeeded"
    )
      throw Error("Browser frame qualification returned incomplete evidence");
  } else if (
    result.stage === "passed" &&
    process.env.LOMI_MCP_ANDROID_LAYOUT_ONLY
  ) {
    const proof = JSON.parse(
      await readFile(join(directory, "android-layout.json"), "utf8"),
    );
    if (
      deniedRequests !== 0 ||
      result.data?.profile !== "android-layout-only" ||
      result.data?.catalogCount !== 71 ||
      proof.checks?.length !== 14 ||
      !proof.stopped ||
      !proof.originalDevicePreserved
    )
      throw Error("Android layout qualification returned incomplete evidence");
  } else if (
    result.stage === "passed" &&
    process.env.LOMI_MCP_ANDROID_SETUP_ONLY
  ) {
    const proof = JSON.parse(
      await readFile(join(directory, "android-setup.json"), "utf8"),
    );
    if (
      deniedRequests !== 0 ||
      result.data?.profile !== "android-setup-only" ||
      result.data?.catalogCount !== 71 ||
      proof.checks?.length !== 23 ||
      !proof.originalDevicePreserved ||
      !proof.fixtureDeviceRemoved
    )
      throw Error("Android setup qualification returned incomplete evidence");
  } else if (
    result.stage === "passed" &&
    (process.env.LOMI_MCP_CHAT_READ_ONLY ||
      process.env.LOMI_MCP_CHAT_OPEN_ONLY ||
      process.env.LOMI_MCP_CHAT_DRAFT_ONLY ||
      process.env.LOMI_MCP_CHAT_SEND_ONLY)
  ) {
    const proof = JSON.parse(
      await readFile(join(directory, "chat-read.json"), "utf8"),
    );
    if (
      deniedRequests !== 0 ||
      result.data?.profile !==
        (process.env.LOMI_MCP_CHAT_SEND_ONLY
          ? "chat-send-only"
          : process.env.LOMI_MCP_CHAT_DRAFT_ONLY
            ? "chat-draft-only"
            : process.env.LOMI_MCP_CHAT_OPEN_ONLY
              ? "chat-open-only"
              : "chat-read-only") ||
      result.data?.catalogCount !== 71 ||
      proof.checks?.length !==
        (process.env.LOMI_MCP_CHAT_SEND_ONLY
          ? 54
          : process.env.LOMI_MCP_CHAT_DRAFT_ONLY
            ? 23
            : process.env.LOMI_MCP_CHAT_OPEN_ONLY
              ? 17
              : 11) ||
      !proof.readOnly ||
      !proof.revoked
    )
      throw Error("Chat history fixture returned incomplete evidence");
  } else if (
    result.stage === "passed" &&
    (process.env.LOMI_MCP_SETTINGS_TERMINAL_ONLY ||
      process.env.LOMI_MCP_SETTINGS_KEYBINDS_ONLY ||
      process.env.LOMI_MCP_SETTINGS_THEMES_ONLY)
  ) {
    const proof = JSON.parse(
      await readFile(join(directory, "settings-terminal.json"), "utf8"),
    );
    if (
      deniedRequests !== 0 ||
      result.data?.profile !==
        (process.env.LOMI_MCP_SETTINGS_THEMES_ONLY
          ? "settings-themes-only"
          : process.env.LOMI_MCP_SETTINGS_KEYBINDS_ONLY
            ? "settings-keybinds-only"
            : "settings-terminal-only") ||
      proof.cases?.length !== 9 ||
      proof.retained?.length !== 2 ||
      !proof.nativeContextsUnchanged ||
      proof.retained.some(
        (r) => !r.same || !r.sameSession || !r.output || !r.running,
      )
    )
      throw Error("Terminal preference fixture returned incomplete evidence");
    if (process.env.LOMI_MCP_SETTINGS_KEYBINDS_ONLY) {
      const keys = JSON.parse(
        await readFile(join(directory, "settings-keybindings.json"), "utf8"),
      );
      if (
        keys.cases?.length !== 12 ||
        !keys.pluginNeverEnabled ||
        !keys.exactStoredPatches ||
        !keys.retainedDocument
      )
        throw Error("Shortcut fixture returned incomplete evidence");
    }
    if (process.env.LOMI_MCP_SETTINGS_THEMES_ONLY) {
      const themes = JSON.parse(
        await readFile(join(directory, "settings-themes.json"), "utf8"),
      );
      if (
        themes.cases?.length !== 9 ||
        !themes.retainedDocument ||
        !themes.computedTerminalThemes ||
        !themes.protection?.jsonStyles ||
        !themes.protection.externalStylesheet ||
        !themes.protection.lateReload ||
        !themes.protection.nativeMenuHandler ||
        !themes.protection.modalRestore ||
        !themes.protection.preferencesUnchanged
      )
        throw Error("Theme fixture returned incomplete evidence");
    }
    const editor = JSON.parse(
      await readFile(join(directory, "settings-update.json"), "utf8"),
    );
    if (
      editor.cases?.length !== 6 ||
      !editor.retainedDocument ||
      !editor.unrelatedFilesUnchanged ||
      !editor.mainCannotApprove
    )
      throw Error("Editor preference regression returned incomplete evidence");
  } else if (
    result.stage === "passed" &&
    process.env.LOMI_MCP_SETTINGS_UPDATE_ONLY
  ) {
    const proof = JSON.parse(
      await readFile(join(directory, "settings-update.json"), "utf8"),
    );
    if (
      deniedRequests !== 0 ||
      result.data?.profile !== "settings-update-only" ||
      proof.cases?.length !== 6 ||
      !proof.retainedDocument ||
      !proof.unrelatedFilesUnchanged ||
      !proof.terminalsUnchanged ||
      !proof.mainCannotApprove
    )
      throw Error("Settings update fixture returned incomplete evidence");
  } else if (
    result.stage === "passed" &&
    process.env.LOMI_MCP_SETTINGS_READ_ONLY
  ) {
    const proof = JSON.parse(
      await readFile(join(directory, "settings-read.json"), "utf8"),
    );
    if (
      deniedRequests !== 0 ||
      result.data?.profile !== "settings-read-only" ||
      proof.reads?.length !== 4 ||
      !proof.readOnlyFiles ||
      !proof.invalidFilePreserved ||
      proof.recovery?.structuredContent?.data?.readiness !== "recovery_required"
    )
      throw Error("Settings read fixture returned incomplete evidence");
  } else if (
    result.stage === "passed" &&
    process.env.LOMI_MCP_SETTINGS_OPEN_ONLY
  ) {
    const proof = JSON.parse(
      await readFile(join(directory, "settings-open.json"), "utf8"),
    );
    if (
      deniedRequests !== 0 ||
      result.data?.profile !== "settings-open-only" ||
      proof.pages?.length !== 9 ||
      proof.pages.some((p) => p.result.state !== "succeeded") ||
      !proof.preferencesUnchanged ||
      !proof.layoutUnchanged
    )
      throw Error("Settings open fixture returned incomplete evidence");
  } else if (
    result.stage === "passed" &&
    process.env.LOMI_MCP_PROJECT_OPEN_ONLY
  ) {
    const proof = JSON.parse(
      await readFile(join(directory, "project-open.json"), "utf8"),
    );
    if (
      deniedRequests !== 0 ||
      result.data?.profile !== "project-open-only" ||
      proof.cases?.length !== 5 ||
      proof.approved?.length !== 2 ||
      proof.approved.some((p) => p.state !== "succeeded")
    )
      throw Error("Project open fixture returned incomplete evidence");
  } else if (
    result.stage === "passed" &&
    process.env.LOMI_MCP_PROJECT_CLOSE_ONLY
  ) {
    const proof = JSON.parse(
      await readFile(join(directory, "project-close.json"), "utf8"),
    );
    if (
      deniedRequests !== 0 ||
      result.data?.profile !== "project-close-only" ||
      proof.guarded?.length !== 4 ||
      proof.guarded.at(-1)?.structuredContent?.data?.state !== "succeeded"
    )
      throw Error("Project close fixture returned incomplete evidence");
  } else if (result.stage === "passed" && closeStressServer) {
    if (
      deniedRequests !== 0 ||
      result.data?.profile !== "workspace-close-stress-only" ||
      result.data?.rounds !== 6
    )
      throw Error("Focused close fixture returned an incomplete result");
    for (let round = 0; round < 6; round++) {
      const proof = JSON.parse(
        await readFile(
          join(directory, `workspace-close-runtime-${round}.json`),
          "utf8",
        ),
      );
      if (proof.closed?.structuredContent?.data?.state !== "succeeded")
        throw Error(`Focused close round ${round} did not succeed`);
    }
  } else if (result.stage === "passed") {
    const counters = JSON.parse(
      await readFile(join(directory, "browser-server-counters.json"), "utf8"),
    );
    if (
      deniedRequests !== 0 ||
      counters.navigation !== 1 ||
      counters.redirects !== 1 ||
      counters.started !== 2 ||
      counters.closed !== 2
    ) {
      result.stage = "failed";
      result.error = `Native HTTP counters: denied=${deniedRequests}, server=${JSON.stringify(counters)}`;
      await writeFile(join(directory, "result.json"), JSON.stringify(result));
    }
  }
  console.log(
    JSON.stringify(
      {
        stage: result.stage,
        error: result.error,
        profile: result.data?.profile ?? "full",
        checks: result.data?.checks,
      },
      null,
      2,
    ),
  );
  if (result.stage !== "passed") process.exitCode = 1;
} finally {
  blockedServer.closeAllConnections();
  blockedServer.close();
  closeStressServer?.closeAllConnections();
  closeStressServer?.close();
  // The native fixture publishes its report before app.exit has finished.
  // Await its process tree before removing paths that a terminal may still write.
  await waitForExit(3000);
  if (!childClosed) {
    try {
      process.kill(-child.pid, "SIGTERM");
    } catch {}
    await waitForExit(3000);
  }
  if (!childClosed) {
    try {
      process.kill(-child.pid, "SIGKILL");
    } catch {}
    await waitForExit(3000);
  }
  await writeFile(join(directory, "native.log"), log);
  if (!childClosed)
    throw Error("Native fixture did not exit; app data retained.");
  if (result)
    await rm(appData, {
      recursive: true,
      force: true,
      maxRetries: 5,
      retryDelay: 200,
    });
  await writeFile(
    join(directory, "cleanup.json"),
    JSON.stringify(
      {
        hostExited: true,
        appDataRemoved: Boolean(result),
        exitCode: child.exitCode,
        signal: child.signalCode,
      },
      null,
      2,
    ),
  );
  if (
    result?.stage === "passed" &&
    (child.exitCode !== 0 || child.signalCode !== null)
  )
    throw Error(
      "Native assertions passed but the fixture did not exit normally; inspect cleanup.json",
    );
}
