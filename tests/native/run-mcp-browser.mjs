import { mkdtemp, mkdir, readFile, writeFile, rm } from "node:fs/promises";
import { tmpdir, homedir } from "node:os";
import { join, resolve } from "node:path";
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { newSession, newProject } from "../../src/model.ts";

if (process.platform !== "darwin" || process.arch !== "arm64")
  throw Error("This P0 probe requires macOS ARM64.");
const root = resolve(import.meta.dirname, "../..");
const directory = await mkdtemp(join(tmpdir(), "lomi-mcp-browser-"));
const identifier = `dev.lomi.mcp-probe-${Date.now()}`;
const appData = join(homedir(), "Library/Application Support", identifier);
const folder = join(directory, "project");
await mkdir(folder);
await mkdir(appData, { recursive: true });
const port = Number(process.env.LOMI_MCP_PROBE_PORT ?? 1443);
let deniedHits = 0;
const deniedServer = createServer((_request, response) => {
  deniedHits++;
  response.end("Navigation should have been denied");
});
await new Promise((accept) => deniedServer.listen(0, "127.0.0.1", accept));
const fixtureServer = createServer((request, response) => {
  if (request.url === "/redirect") {
    response.writeHead(302, {
      Location: `http://127.0.0.1:${deniedServer.address().port}/blocked`,
    });
    response.end();
  } else if (request.url === "/worker.js") {
    response.writeHead(200, { "Content-Type": "text/javascript" });
    response.end("self.addEventListener('install',()=>self.skipWaiting());");
  } else {
    response.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    response.end(
      `<!doctype html><html><head><title>Lomi MCP native fixture</title></head><body><div id="root"></div><script type="module">import RefreshRuntime from 'http://127.0.0.1:${port}/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window); window.$RefreshReg$=()=>{}; window.$RefreshSig$=()=>type=>type; window.__vite_plugin_react_preamble_installed__=true; await import('http://127.0.0.1:${port}/tests/mcp/browser.jsx');</script></body></html>`,
    );
  }
});
await new Promise((accept) => fixtureServer.listen(0, "127.0.0.1", accept));
const project = newProject(folder, "local:zsh");
const workspace = project.workspaces[0];
workspace.tabs = [
  {
    type: "browser",
    id: "mcp-fixture",
    title: "MCP qualification",
    url: `http://127.0.0.1:${fixtureServer.address().port}/`,
  },
];
workspace.activeTabId = "mcp-fixture";
await writeFile(
  join(appData, "session.json"),
  JSON.stringify({
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  }),
);
const config = join(directory, "config.json");
await writeFile(
  config,
  JSON.stringify({
    identifier,
    build: {
      beforeDevCommand: `pnpm dev --port ${port} --strictPort`,
      devUrl: `http://127.0.0.1:${port}`,
    },
  }),
);
console.log(
  `Native MCP artifacts: ${directory}\nIsolated app data: ${appData}`,
);
const child = spawn(
  "pnpm",
  ["tauri", "dev", "--no-watch", "--features", "mcp-probe", "--config", config],
  {
    cwd: root,
    env: { ...process.env, LOMI_MCP_PROBE_DIRECTORY: directory },
    detached: true,
    stdio: ["ignore", "pipe", "pipe"],
  },
);
let log = "";
for (const stream of [child.stdout, child.stderr])
  stream.on("data", (chunk) => {
    log += chunk;
    process.stdout.write(chunk);
  });
let result;
try {
  const deadline = Date.now() + 240_000;
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
        throw Error(`Native probe exited before reporting: ${child.exitCode}`);
      await new Promise((accept) => setTimeout(accept, 200));
    }
  }
  if (!result) throw Error("Native MCP browser probe timed out");
  result.deniedOriginRequests = deniedHits;
  if (deniedHits !== 0) {
    result.stage = "failed";
    result.error = "Denied redirect reached the network";
  }
  await writeFile(
    join(directory, "result.json"),
    JSON.stringify(result, null, 2),
  );
  console.log(JSON.stringify(result, null, 2));
  if (result.stage !== "passed") process.exitCode = 1;
} finally {
  try {
    process.kill(-child.pid, "SIGTERM");
  } catch {}
  fixtureServer.close();
  deniedServer.close();
  await writeFile(join(directory, "native.log"), log);
  // Only remove this run's generated app data; retain reports and screenshots.
  if (result) await rm(appData, { recursive: true, force: true });
}
