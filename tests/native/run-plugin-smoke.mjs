import { mkdtemp, mkdir, cp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { createServer } from "node:http";
import { newSession, newProject, newBrowserTab } from "../../src/model.ts";
import { pluginSDKSpec } from "../../scripts/plugin-sdk-dependency.mjs";
const root = resolve(import.meta.dirname, "../..");
const directory = await mkdtemp(join(tmpdir(), "lomi-native-"));
const run = (args, cwd) => {
  const result = spawnSync("pnpm", args, { cwd, encoding: "utf8" });
  if (result.status !== 0) throw new Error(result.stdout + result.stderr);
  return result.stdout;
};
const author = join(directory, "author space żółć");
await cp(join(root, "tests/fixtures/context-plugin"), author, {
  recursive: true,
  filter: (path) =>
    !path
      .split("/")
      .some((part) => ["node_modules", "dist", "package"].includes(part)),
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
const appData = join(data, "dev.lomi.desktop");
await mkdir(appData, { recursive: true });
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
const project = newProject(folder, "local:bash");
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
console.log(`Native proof artifacts: ${directory}`);
const binary = resolve(
  process.argv[2] ?? join(root, "src-tauri/target/release/lomi"),
);
const child = spawn(binary, [], {
  cwd: root,
  env: {
    ...process.env,
    XDG_DATA_HOME: data,
    XDG_CONFIG_HOME: config,
    XDG_CACHE_HOME: cache,
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
const timeout = setTimeout(() => child.kill("SIGTERM"), 90000);
const code = await new Promise((resolve) => child.on("exit", resolve));
clearTimeout(timeout);
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
      LOMI_PLUGIN_SMOKE_PACKAGE: join(author, "package"),
      LOMI_PLUGIN_SMOKE_REPORT: safeReport,
    },
    stdio: ["ignore", "ignore", "ignore"],
  });
  const timer = setTimeout(() => safe.kill("SIGTERM"), 30000);
  const safeCode = await new Promise((resolve) => safe.on("exit", resolve));
  clearTimeout(timer);
  const safeResult = JSON.parse(await readFile(safeReport, "utf8"));
  console.log(JSON.stringify(safeResult, null, 2));
  if (safeCode !== 0 || safeResult.stage !== "passed") process.exitCode = 1;
}
