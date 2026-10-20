import { mkdtemp, mkdir, cp, readFile, writeFile } from "node:fs/promises";
import { tmpdir, homedir } from "node:os";
import { resolve, join } from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { createServer } from "node:http";
import { newSession, newProject, newBrowserTab } from "../../src/model.ts";
import { pluginSDKSpec } from "../../scripts/plugin-sdk-dependency.mjs";
const root = resolve(import.meta.dirname, "../..");
const directory = await mkdtemp(join(tmpdir(), "lomi-native-"));
const windows = process.platform === "win32";
const identifier =
  process.env.LOMI_PLUGIN_SMOKE_IDENTIFIER ??
  `dev.lomi.plugin-smoke-${Date.now()}`;
if (
  !/^dev\.lomi\.(?:plugin-smoke-\d+|desktop-smoke-[a-z0-9-]+)$/.test(identifier)
)
  throw Error("Use a disposable plugin-smoke application identifier.");
if (process.argv[2] && !process.env.LOMI_PLUGIN_SMOKE_IDENTIFIER)
  throw Error(
    "An existing fixture binary requires its LOMI_PLUGIN_SMOKE_IDENTIFIER.",
  );
const run = (args, cwd) => {
  // These fixed fixture commands contain no user-provided shell arguments.
  const result = spawnSync(
    windows ? (process.env.ComSpec ?? "cmd.exe") : "pnpm",
    windows ? ["/d", "/s", "/c", `pnpm ${args.join(" ")}`] : args,
    { cwd, encoding: "utf8" },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(result.stdout + result.stderr);
  return result.stdout;
};
const author = join(directory, "author space żółć");
await cp(join(root, "tests/fixtures/context-plugin"), author, {
  recursive: true,
  filter: (path) =>
    !path
      .split(/[\\/]/)
      .some(
        (part) =>
          [
            "node_modules",
            "dist",
            "package",
            "package.lomi-owned.json",
            "package.lomi-lock",
          ].includes(part) || part.startsWith(".lomi-build-"),
      ),
});
const manifest = JSON.parse(
  await readFile(join(author, "package.json"), "utf8"),
);
manifest.dependencies["@lomi-dev/plugin-sdk"] = await pluginSDKSpec(root);
await writeFile(
  join(author, "package.json"),
  JSON.stringify(manifest, null, 2),
);
await writeFile(
  join(directory, "author-build.log"),
  run(["install", "--ignore-scripts"], author) + run(["build"], author),
);
const data = join(directory, "data"),
  config = join(directory, "config"),
  cache = join(directory, "cache");
if (windows && !process.env.APPDATA)
  throw Error("Windows APPDATA is required.");
const appData = windows
  ? join(process.env.APPDATA, identifier)
  : process.platform === "darwin"
    ? join(homedir(), "Library/Application Support", identifier)
    : join(data, identifier);
await mkdir(appData, { recursive: true });
await writeFile(
  join(appData, "agent-control-preferences.json"),
  JSON.stringify({ version: 1, autoStart: false, yoloMode: false }),
);
const folder = join(directory, "project żółć");
await mkdir(folder);
await writeFile(join(folder, "native.txt"), "Native editor fixture\n");
const server = createServer((_request, response) => {
  response.writeHead(200, { "Content-Type": "text/html" });
  response.end(
    '<!doctype html><title>Native browser smoke</title><label>Native field <input id="input"></label><p>Loopback native page</p>',
  );
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
server.unref();
const project = newProject(
  folder,
  windows
    ? "local:powershell"
    : process.platform === "darwin"
      ? "local:zsh"
      : "local:bash",
);
const workspace = project.workspaces[0];
workspace.name = "Native workspace";
workspace.tabs[0].id = "native-terminal";
workspace.activeTabId = "native-terminal";
const browser = newBrowserTab(`http://127.0.0.1:${server.address().port}`);
browser.id = "native-browser";
workspace.tabs.push(browser);
const session = {
  ...newSession(),
  projects: [project],
  activeProjectId: project.id,
};
await writeFile(join(appData, "session.json"), JSON.stringify(session));
const report = join(directory, "result.json");
console.log(
  `Native proof artifacts: ${directory}\nIsolated app data: ${appData}`,
);
const fixtureConfig = join(directory, "tauri.json");
await writeFile(fixtureConfig, JSON.stringify({ identifier }));
if (!process.argv[2]) {
  const build = spawn(
    process.execPath,
    [
      join(root, "node_modules/@tauri-apps/cli/tauri.js"),
      "build",
      "--debug",
      "--no-bundle",
      "--features",
      "native-smoke",
      "--config",
      fixtureConfig,
    ],
    { cwd: root, stdio: "inherit" },
  );
  const code = await new Promise((resolve, reject) => {
    build.once("error", reject);
    build.once("exit", resolve);
  });
  if (code !== 0) {
    server.close();
    throw Error(`Native fixture build failed: ${code}`);
  }
}
const binary = resolve(
  process.argv[2] ??
    join(
      process.env.CARGO_TARGET_DIR ?? join(root, "src-tauri/target"),
      "debug",
      windows ? "lomi.exe" : "lomi",
    ),
);
const child = spawn(binary, [], {
  cwd: root,
  env: {
    ...process.env,
    XDG_DATA_HOME: data,
    XDG_CONFIG_HOME: config,
    XDG_CACHE_HOME: cache,
    LOMI_PLUGIN_SMOKE_IDENTIFIER: identifier,
    LOMI_PLUGIN_SMOKE_PACKAGE: join(author, "package"),
    LOMI_PLUGIN_SMOKE_REPORT: report,
  },
  stdio: ["ignore", "pipe", "pipe"],
});
let output = "";
child.stdout.on("data", (chunk) => {
  output += chunk;
});
child.stderr.on("data", (chunk) => {
  output += chunk;
});
const stop = (child) => {
  if (windows)
    spawnSync("taskkill", ["/pid", String(child.pid), "/t", "/f"], {
      stdio: "ignore",
    });
  else child.kill("SIGKILL");
};
const timeout = setTimeout(() => stop(child), 120000);
const code = await new Promise((resolve, reject) => {
  child.once("error", reject);
  child.once("exit", resolve);
}).finally(() => clearTimeout(timeout));
server.close();
await writeFile(join(directory, "native.log"), output);
let result;
try {
  result = JSON.parse(await readFile(report, "utf8"));
} catch {
  throw new Error(`No native report (exit ${code}): ${output.slice(-4000)}`);
}
console.log(JSON.stringify(result, null, 2));
if (code !== 0 || result.stage !== "passed") process.exitCode = 1;
else {
  const safeReport = join(directory, "safe-result.json");
  const safe = spawn(binary, ["--safe-mode"], {
    cwd: root,
    env: {
      ...process.env,
      XDG_DATA_HOME: data,
      XDG_CONFIG_HOME: config,
      XDG_CACHE_HOME: cache,
      LOMI_PLUGIN_SMOKE_IDENTIFIER: identifier,
      LOMI_PLUGIN_SMOKE_PACKAGE: join(author, "package"),
      LOMI_PLUGIN_SMOKE_REPORT: safeReport,
    },
    stdio: ["ignore", "ignore", "ignore"],
  });
  const timer = setTimeout(() => stop(safe), 30000);
  const safeCode = await new Promise((resolve, reject) => {
    safe.once("error", reject);
    safe.once("exit", resolve);
  }).finally(() => clearTimeout(timer));
  const safeResult = JSON.parse(await readFile(safeReport, "utf8"));
  console.log(JSON.stringify(safeResult, null, 2));
  if (safeCode !== 0 || safeResult.stage !== "passed") process.exitCode = 1;
}
