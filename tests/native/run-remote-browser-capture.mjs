import { mkdtemp, mkdir, readFile, writeFile, rm } from "node:fs/promises";
import { tmpdir, homedir, release } from "node:os";
import { join, resolve } from "node:path";
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { newSession, newProject } from "../../src/model.ts";

if (process.platform !== "darwin" || process.arch !== "arm64")
  throw Error("This P0 probe requires macOS ARM64.");
const root = resolve(import.meta.dirname, "../..");
const directory = await mkdtemp(join(tmpdir(), "lomi-remote-browser-capture-"));
const identifier = `dev.lomi.remote-capture-probe-${Date.now()}`;
const appData = join(homedir(), "Library/Application Support", identifier);
const folder = join(directory, "project");
await mkdir(folder);
await mkdir(appData, { recursive: true });
const identity = `fixture-${Date.now()}`;
const fixtureServer = createServer((request, response) => {
  if (request.url === "/main") {
    response.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    response.end(
      "<!doctype html><html><head><title>Isolated native Browser capture host</title></head><body>Native fixture host; ordinary Page created by sync_browsers.</body></html>",
    );
    return;
  }
  response.writeHead(200, {
    "Content-Type": "text/html; charset=utf-8",
    "Set-Cookie": `remote_capture_fixture=${identity}; SameSite=Strict`,
  });
  response.end(
    `<!doctype html><html><head><title>Ordinary capture fixture</title><style>html,body{margin:0;width:100%;height:100%;} #clock{position:absolute;top:10px;left:10px} #animated{width:40px;height:40px;position:absolute;top:60px;animation:move 1s infinite alternate}@keyframes move{from{left:0}to{left:90%}}</style></head><body><canvas id="canvas2d" width="160" height="80" style="position:absolute;left:20px;top:150px"></canvas><canvas id="webgl" width="160" height="80" style="position:absolute;left:220px;top:150px"></canvas><div id="raf" style="position:absolute;left:420px;top:150px;width:160px;height:80px"></div><div id="autoclock" style="position:absolute;bottom:0;width:100%;height:50px"></div><input id="draft" value="retained fixture draft" style="position:absolute;top:100px;left:20px"><div id="clock"></div><div id="animated"></div><script>window.fixtureCounter=0;window.fixturePaint=n=>{window.fixtureCounter++;window.fixtureColor=[n,100,220-n];document.body.style.background='rgb('+window.fixtureColor.join(',')+')';};window.fixturePaint(40);window.fixtureTicks=0;setInterval(()=>{window.fixtureTicks++;document.querySelector('#autoclock').style.background='rgb('+(window.fixtureTicks%256)+',0,0)';document.querySelector('#clock').textContent=performance.now().toFixed(0);document.querySelector('#animated').style.background='white';},50);window.fixtureRafTicks=0;const ctx=document.querySelector('#canvas2d').getContext('2d');const gl=document.querySelector('#webgl').getContext('webgl',{preserveDrawingBuffer:true});window.fixtureWebgl=Boolean(gl);function render(){window.fixtureRafTicks++;const r=30+window.fixtureRafTicks%200;ctx.fillStyle='rgb('+r+',90,150)';ctx.fillRect(0,0,160,80);document.querySelector('#raf').style.background='rgb('+r+',60,180)';if(gl){gl.clearColor(r/255,120/255,120/255,1);gl.clear(gl.COLOR_BUFFER_BIT);gl.flush();}requestAnimationFrame(render);}requestAnimationFrame(render);window.fixtureReady=true;</script></body></html>`,
  );
});
await new Promise((accept) => fixtureServer.listen(0, "127.0.0.1", accept));
const project = newProject(folder, "local:zsh");
const workspace = project.workspaces[0];
workspace.tabs = [
  {
    type: "browser",
    id: "remote-capture-fixture",
    title: "Ordinary Browser capture",
    url: `http://127.0.0.1:${fixtureServer.address().port}/`,
  },
];
workspace.activeTabId = "remote-capture-fixture";
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
      beforeDevCommand: "",
      devUrl: `http://127.0.0.1:${fixtureServer.address().port}/main`,
    },
  }),
);
console.log(
  `Native ordinary Browser artifacts: ${directory}\nIsolated app data: ${appData}`,
);
const child = spawn(
  "pnpm",
  [
    "tauri",
    "dev",
    "--no-watch",
    "--features",
    "remote-visual-probe",
    "--config",
    config,
  ],
  {
    cwd: root,
    env: {
      ...process.env,
      LOMI_REMOTE_BROWSER_CAPTURE_DIRECTORY: directory,
      LOMI_REMOTE_BROWSER_FIXTURE_URL: `http://127.0.0.1:${fixtureServer.address().port}/`,
    },
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
  const deadline = Date.now() + 900_000;
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
  if (!result) throw Error("Native ordinary Browser capture probe timed out");
  if (result.data) {
    result.data.closeQualification =
      "inconclusive: no native submission/identity removal barrier";
    if (result.data.closeRace === "completed before close fence")
      result.data.closeRace =
        "frame returned; ordering relative to close fence is unobserved";
  }
  result.host = {
    platform: process.platform,
    arch: process.arch,
    kernel: release(),
    node: process.version,
  };
  await writeFile(
    join(directory, "result.json"),
    JSON.stringify(result, null, 2),
  );
  console.log(JSON.stringify(result, null, 2));
  if (
    result.stage !== "completed" ||
    result.data?.g1BrowserDecision === "NO-GO" ||
    result.data?.backgroundContentCandidate === false
  )
    process.exitCode = 1;
} finally {
  try {
    process.kill(-child.pid, "SIGTERM");
  } catch {}
  fixtureServer.close();
  await writeFile(join(directory, "native.log"), log);
  // Only remove this run's generated app data; retain reports and screenshots.
  if (result) await rm(appData, { recursive: true, force: true });
}
