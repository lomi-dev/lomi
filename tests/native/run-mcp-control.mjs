import {
  mkdtemp,
  mkdir,
  readFile,
  writeFile,
  rm,
  copyFile,
  statfs,
} from "node:fs/promises";
import { tmpdir, homedir } from "node:os";
import { join, resolve } from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { createServer } from "node:http";
import { uploadFixture } from "../mcp/browser-upload-server.mjs";
import { disconnectFixture } from "../mcp/browser-disconnect-server.mjs";
import { downloadFixture } from "../mcp/browser-download-server.mjs";
import { browserFramePage } from "../mcp/browser-frame-pages.mjs";
import { prepareRoutingFixture } from "../mcp/routing-fixtures.mjs";
import { binaryFingerprint } from "../mcp/routing-build.mjs";
import { newSession, newProject } from "../../src/model.ts";

if (process.platform !== "darwin" || process.arch !== "arm64")
  throw Error("Native control qualification requires macOS ARM64.");
const root = resolve(import.meta.dirname, "../..");
const directory = await mkdtemp(join(tmpdir(), "lomi-mcp-control-"));
const androidRoot = process.env.LOMI_ANDROID_PRODUCT_DIRECTORY;
if (
  (process.env.LOMI_MCP_ROUTING_ONLY === "apk" ||
    process.env.LOMI_MCP_ANDROID_DISCONNECT_ONLY ||
    process.env.LOMI_MCP_ANDROID_STORAGE_ONLY) &&
  !androidRoot
)
  throw Error("APK routing requires the licensed isolated Android fixture.");
if (process.env.LOMI_MCP_ANDROID_LATENCY && !androidRoot)
  throw Error(
    "Android latency requires the licensed isolated Android fixture.",
  );
if (process.env.LOMI_MCP_ANDROID_STORAGE_ONLY) {
  const disk = await statfs(androidRoot, { bigint: true });
  if (disk.bavail * disk.bsize < 12n * 1024n * 1024n * 1024n)
    throw Error(
      "Guest storage qualification needs at least 12 GiB free on the fixture volume.",
    );
}
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
const batchIdentifier = process.env.LOMI_MCP_BATCH_IDENTIFIER;
if (
  batchIdentifier &&
  !/^dev\.lomi\.mcp-control-batch-[a-f0-9]{32}$/.test(batchIdentifier)
)
  throw Error("Invalid disposable batch identifier");
// Sequential batch runs can reuse the compiled identity. mkdir below remains
// exclusive, so a live or incompletely cleaned fixture cannot be overwritten.
const identifier = batchIdentifier ?? `dev.lomi.mcp-control-${Date.now()}`;
const appData = join(homedir(), "Library/Application Support", identifier);
const restartPhase = process.env.LOMI_MCP_RESTART_PHASE;
if (restartPhase && (restartPhase !== "first" || !batchIdentifier))
  throw Error("Start restart qualification only through run-mcp-restart.mjs");
const folder = join(directory, "project");
await mkdir(folder);
if (process.env.LOMI_MCP_TERMINAL_ONLY) {
  if (!["bash", "zsh"].includes(process.env.LOMI_MCP_TERMINAL_ONLY))
    throw Error("Choose bash or zsh for terminal qualification.");
}
{
  const shellHome = join(directory, "shell-home");
  await mkdir(shellHome);
  await writeFile(
    join(shellHome, ".bashrc"),
    "PS1='fixture> '\nHISTFILE=/dev/null\n",
  );
  await writeFile(
    join(shellHome, ".zshrc"),
    "PROMPT='fixture> '\nHISTFILE=/dev/null\n",
  );
  await writeFile(
    join(directory, "terminal-fixture.json"),
    JSON.stringify({ node: process.execPath }),
  );
}
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

// Existing data is never overwritten, including another still-running fixture.
await mkdir(appData);
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
if (process.env.LOMI_MCP_ROUTING_ONLY) {
  const cases = await prepareRoutingFixture(
    folder,
    `http://127.0.0.1:${serverPort}`,
    process.execPath,
  );
  if (!Object.hasOwn(cases, process.env.LOMI_MCP_ROUTING_ONLY))
    throw Error("Unknown model-routing fixture case");
}
const downloads = downloadFixture(
  `http://127.0.0.1:${blockedServer.address().port}`,
);
const uploads = uploadFixture();
const disconnect = disconnectFixture(directory);
const closeStressServer =
  process.env.LOMI_MCP_BROWSER_PROFILES_ONLY ||
  process.env.LOMI_MCP_BROWSER_DISCONNECT_ONLY ||
  process.env.LOMI_MCP_THROUGHPUT_ONLY ||
  process.env.LOMI_MCP_PERFORMANCE_ONLY ||
  process.env.LOMI_MCP_TERMINAL_ONLY ||
  process.env.LOMI_MCP_BROWSER_UPLOAD_ONLY ||
  process.env.LOMI_MCP_BROWSER_DOWNLOAD_ONLY ||
  process.env.LOMI_MCP_ARTIFACT_FILES_ONLY ||
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
        process.env.LOMI_MCP_BROWSER_DISCONNECT_ONLY ||
        process.env.LOMI_MCP_BROWSER_PROFILES_ONLY
          ? disconnect(_request, response)
          : process.env.LOMI_MCP_BROWSER_UPLOAD_ONLY
            ? uploads.serve(_request, response)
            : process.env.LOMI_MCP_BROWSER_DOWNLOAD_ONLY
              ? downloads.serve(_request, response)
              : response.end(
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
if (process.env.LOMI_MCP_BROWSER_PROFILES_ONLY)
  workspace.tabs.push({
    type: "browser",
    id: "mcp-profile-human",
    title: "Ordinary fixture browser",
    url: `http://127.0.0.1:${serverPort}`,
  });
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
  process.env.LOMI_MCP_ANDROID_LAYOUT_ONLY ||
  process.env.LOMI_MCP_BROWSER_PROFILES_ONLY
) {
  const retainedFolder = join(directory, "retained-project");
  await mkdir(retainedFolder);
  const retained = newProject(retainedFolder, "local:zsh");
  retained.workspaces[0].name = "Retained other project";
  if (process.env.LOMI_MCP_BROWSER_PROFILES_ONLY)
    await writeFile(
      join(directory, "browser-profiles-fixture.json"),
      JSON.stringify({
        projectId: retained.id,
        workspaceId: retained.workspaces[0].id,
      }),
    );
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
const prebuilt = process.env.LOMI_MCP_PREBUILT_SHA256;
if (prebuilt && !batchIdentifier)
  throw Error("Prebuilt reuse requires a disposable batch identity");
const child = spawn(
  prebuilt ? process.execPath : "pnpm",
  prebuilt
    ? ["tests/native/start-prebuilt-mcp.mjs"]
    : [
        "tauri",
        "dev",
        "--no-watch",
        "--features",
        "mcp-probe",
        "--config",
        config,
      ],
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
  const deadline =
    Date.now() + (process.env.LOMI_MCP_PERFORMANCE_ONLY ? 2_700_000 : 600_000);
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
  if (result.stage === "passed" && process.env.LOMI_MCP_ANDROID_LATENCY) {
    const proof = JSON.parse(
      await readFile(join(directory, "android-latency.json"), "utf8"),
    );
    if (
      proof.samplesMs?.length !== 50 ||
      proof.warmupSamplesMs?.length !== 3 ||
      !Number.isFinite(proof.p95Ms) ||
      proof.p95Ms > 150 ||
      result.data?.androidInputQualification !== "RUN"
    )
      throw Error("Android latency returned incomplete measurement evidence");
  }
  if (result.stage === "passed" && process.env.LOMI_MCP_ANDROID_STORAGE_ONLY) {
    const proof = JSON.parse(
      await readFile(join(directory, "android-storage.json"), "utf8"),
    );
    if (
      result.data?.profile !== "android-storage-only" ||
      result.data?.catalogCount !== 74 ||
      !proof.passed ||
      proof.filledFreeBytes >= 40 * 1024 * 1024 ||
      proof.restoredFreeBytes <= proof.beforeFreeBytes - 256 * 1024 * 1024 ||
      proof.beforePackage !== proof.afterPackage ||
      proof.rejected?.structuredContent?.data?.result?.installerFailure !==
        "INSTALL_FAILED_INSUFFICIENT_STORAGE"
    )
      throw Error("Incomplete native guest storage refusal evidence");
  } else if (
    result.stage === "passed" &&
    process.env.LOMI_MCP_BROWSER_PROFILES_ONLY
  ) {
    const proof = JSON.parse(
      await readFile(join(directory, "browser-profiles.json"), "utf8"),
    );
    if (
      result.data?.profile !== "browser-profiles-only" ||
      result.data?.catalogCount !== 74 ||
      !proof.passed ||
      !proof.distinctProjects ||
      proof.after?.length !== 3 ||
      proof.checks?.length !== 6
    )
      throw Error("Incomplete native browser profile isolation evidence");
  } else if (
    result.stage === "passed" &&
    process.env.LOMI_MCP_ANDROID_DISCONNECT_ONLY
  ) {
    const proof = JSON.parse(
      await readFile(join(directory, "android-disconnect.json"), "utf8"),
    );
    if (
      result.data?.profile !== "android-disconnect-only" ||
      result.data?.catalogCount !== 74 ||
      !proof.passed ||
      proof.transfer?.transfers !== 1 ||
      proof.transfer?.sentBytes !== 1024 ||
      proof.disconnected?.[0] !== "outcome_unknown" ||
      proof.beforePackage !== proof.finalPackage
    )
      throw Error("Incomplete native APK disconnect evidence");
  } else if (
    result.stage === "passed" &&
    process.env.LOMI_MCP_BROWSER_DISCONNECT_ONLY
  ) {
    const proof = JSON.parse(
      await readFile(join(directory, "browser-disconnect.json"), "utf8"),
    );
    if (
      result.data?.profile !== "browser-disconnect-only" ||
      result.data?.catalogCount !== 74 ||
      !proof.passed ||
      proof.effects?.effects !== 1 ||
      proof.disconnected?.[0] !== "outcome_unknown" ||
      proof.afterLateAck?.[0] !== "outcome_unknown"
    )
      throw Error("Incomplete native click disconnect evidence");
  } else if (result.stage === "passed" && restartPhase) {
    const proof = JSON.parse(
      await readFile(join(directory, "restart-baseline.json"), "utf8"),
    );
    if (
      result.data?.profile !== "restart-only" ||
      result.data?.catalogCount !== 74 ||
      !proof.target?.terminalSessionId ||
      !proof.native?.promptReady
    )
      throw Error("First restart phase returned incomplete native evidence");
  } else if (
    result.stage === "passed" &&
    process.env.LOMI_MCP_CLIENT_ISOLATION_ONLY
  ) {
    const proof = JSON.parse(
      await readFile(join(directory, "client-isolation.json"), "utf8"),
    );
    if (
      result.data?.profile !== "client-isolation-only" ||
      result.data?.catalogCount !== 74 ||
      !proof.passed ||
      proof.helpers?.length !== 2 ||
      proof.helpers[0] === proof.helpers[1] ||
      proof.effectCount !== 1 ||
      !proof.secondClientContinued ||
      !proof.fixtureCommandsStopped
    )
      throw Error("Native two-client isolation evidence is incomplete");
  } else if (
    result.stage === "passed" &&
    process.env.LOMI_MCP_THROUGHPUT_ONLY
  ) {
    const proof = JSON.parse(
      await readFile(join(directory, "terminal-throughput.json"), "utf8"),
    );
    if (
      proof.measuredPairs !== 20 ||
      proof.samples?.length !== 46 ||
      !Number.isFinite(proof.medianRatio) ||
      proof.medianRatio > 1.1 ||
      result.data?.profile !== "throughput-only"
    )
      throw Error(
        "Terminal throughput returned incomplete measurement evidence",
      );
  } else if (result.stage === "passed" && process.env.LOMI_MCP_ROUTING_ONLY) {
    const proof = JSON.parse(
      await readFile(join(directory, "routing-result.json"), "utf8"),
    );
    if (
      result.data?.profile !== "routing-only" ||
      result.data?.catalogCount !== 74 ||
      !proof.nativePostconditionsVerified ||
      !proof.modelTurnCompleted ||
      !proof.expectedToolsObserved
    )
      throw Error("Model routing returned incomplete native evidence");
  } else if (
    result.stage === "passed" &&
    process.env.LOMI_MCP_PERFORMANCE_ONLY
  ) {
    const proof = JSON.parse(
      await readFile(join(directory, "performance.json"), "utf8"),
    );
    if (
      result.data?.profile !== "performance-only" ||
      result.data?.catalogCount !== 74 ||
      !proof.latencyPassed ||
      proof.processSamples.length < 10
    )
      throw Error("Performance qualification returned incomplete evidence");
  } else if (result.stage === "passed" && process.env.LOMI_MCP_TERMINAL_ONLY) {
    const proof = JSON.parse(
      await readFile(join(directory, "terminals.json"), "utf8"),
    );
    if (
      result.data?.profile !== "terminal-only" ||
      result.data?.catalogCount !== 74 ||
      proof.shell !== process.env.LOMI_MCP_TERMINAL_ONLY ||
      proof.checks?.length !== 13
    )
      throw Error("Terminal qualification returned incomplete evidence");
  } else if (
    result.stage === "passed" &&
    process.env.LOMI_MCP_BROWSER_UPLOAD_ONLY
  ) {
    const proof = JSON.parse(
      await readFile(join(directory, "browser-uploads.json"), "utf8"),
    );
    await writeFile(
      join(directory, "browser-upload-requests.json"),
      JSON.stringify(uploads.received, null, 2),
    );
    if (
      deniedRequests !== 0 ||
      result.data?.profile !== "browser-upload-only" ||
      result.data?.catalogCount !== 74 ||
      proof.checks?.length !== 12 ||
      uploads.received.length !== 4 ||
      proof.closed?.structuredContent?.data?.state !== "succeeded"
    )
      throw Error("Browser upload qualification returned incomplete evidence");
    for (const expected of proof.uploads) {
      if (
        !uploads.received.some(
          (item) =>
            item.sha256 === expected.sha256 &&
            item.length === expected.byteLength,
        )
      )
        throw Error("Native uploaded bytes did not reach fixture server");
    }
  } else if (
    result.stage === "passed" &&
    process.env.LOMI_MCP_BROWSER_DOWNLOAD_ONLY
  ) {
    const proof = JSON.parse(
      await readFile(join(directory, "browser-downloads.json"), "utf8"),
    );
    await writeFile(
      join(directory, "browser-download-requests.json"),
      JSON.stringify(downloads.counts, null, 2),
    );
    if (
      deniedRequests !== 0 ||
      result.data?.profile !== "browser-download-only" ||
      result.data?.catalogCount !== 74 ||
      proof.checks?.length !== 12 ||
      downloads.counts["/download/counted"] !== 1 ||
      proof.closed?.structuredContent?.data?.state !== "succeeded"
    )
      throw Error(
        "Browser download qualification returned incomplete evidence",
      );
  } else if (
    result.stage === "passed" &&
    process.env.LOMI_MCP_ARTIFACT_FILES_ONLY
  ) {
    const proof = JSON.parse(
      await readFile(join(directory, "artifact-files.json"), "utf8"),
    );
    if (
      result.data?.profile !== "artifact-files-only" ||
      result.data?.catalogCount !== 74 ||
      proof.checks?.length !== 12
    )
      throw Error("Artifact qualification returned incomplete evidence");
  } else if (
    result.stage === "passed" &&
    process.env.LOMI_MCP_BROWSER_LOGS_ONLY
  ) {
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
      result.data?.catalogCount !== 74 ||
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
      result.data?.catalogCount !== 74 ||
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
      result.data?.catalogCount !== 74 ||
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
  const cleanupStarted = Date.now();
  await waitForExit(3000);
  if (!childClosed && prebuilt) {
    let trace = { events: [] };
    try {
      trace = JSON.parse(
        await readFile(join(directory, "prebuilt-process.json"), "utf8"),
      );
    } catch (error) {
      await writeFile(
        join(directory, "prebuilt-exit-diagnostic-error.txt"),
        String(error),
      );
    }
    const pid = trace.events.find((e) => e.event === "native-start")?.pid;
    if (
      Number.isSafeInteger(pid) &&
      !trace.events.some((e) => e.event === "native-close")
    ) {
      spawnSync(
        "/usr/bin/sample",
        [
          String(pid),
          "1",
          "1",
          "-file",
          join(directory, "native-exit-sample.txt"),
        ],
        { timeout: 5000, stdio: "ignore" },
      );
    }
    await waitForExit(7000);
  }
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
  const retainedForRestart =
    restartPhase === "first" &&
    result?.stage === "passed" &&
    child.exitCode === 0 &&
    child.signalCode === null;
  if (retainedForRestart) {
    await writeFile(
      join(directory, "restart-launch.json"),
      JSON.stringify(
        {
          directory,
          appData,
          identifier,
          binarySha256: await binaryFingerprint(root),
        },
        null,
        2,
      ),
      { mode: 0o600 },
    );
  }
  if (result && !retainedForRestart)
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
        appDataRemoved: Boolean(result) && !retainedForRestart,
        retainedForRestart,
        exitCode: child.exitCode,
        signal: child.signalCode,
        exitWaitMs: Date.now() - cleanupStarted,
      },
      null,
      2,
    ),
  );
  if (process.env.LOMI_MCP_ROUTING_ONLY === "external-playwright") {
    let external;
    try {
      external = JSON.parse(
        await readFile(join(folder, "routing-external-result.json"), "utf8"),
      );
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    const alive = () =>
      external
        ? [external.pid, external.browserPid, external.parentPid].filter(
            (pid) =>
              Number.isSafeInteger(pid) &&
              spawnSync("/bin/ps", ["-p", String(pid)], { stdio: "ignore" })
                .status === 0,
          )
        : [];
    let remainingPids = alive();
    const deadline = Date.now() + 3000;
    while (remainingPids.length && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 100));
      remainingPids = alive();
    }
    await writeFile(
      join(directory, "external-cleanup.json"),
      JSON.stringify({ metadataPresent: Boolean(external), remainingPids }),
    );
    if (remainingPids.length)
      throw Error(
        "External fixture processes remain after native exit: " +
          remainingPids.join(","),
      );
  }
  if (
    result?.stage === "passed" &&
    (child.exitCode !== 0 || child.signalCode !== null)
  )
    throw Error(
      "Native assertions passed but the fixture did not exit normally; inspect cleanup.json",
    );
}
