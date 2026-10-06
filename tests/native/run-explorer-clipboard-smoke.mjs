import {
  copyFile,
  mkdtemp,
  mkdir,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir, homedir } from "node:os";
import { join, resolve } from "node:path";
import { createServer } from "node:net";
import { spawn, execFileSync } from "node:child_process";
import { newProject, newSession } from "../../src/model.ts";

if (process.platform !== "darwin")
  throw Error("The Explorer clipboard smoke requires macOS.");

const repository = resolve(import.meta.dirname, "../..");
const directory = await mkdtemp(join(tmpdir(), "lomi-explorer-clipboard-"));
const identifier = `dev.lomi.explorer-clipboard-smoke-${Date.now()}`;
const appData = join(homedir(), "Library/Application Support", identifier);
const projectFolder = join(directory, "project with spaces");
const externalFolder = join(directory, "external source with spaces");
const externalFiles = {
  "folder źródło.txt": "External folder drop contents\n",
  "sibling source.txt": "External sibling drop contents\n",
  "root source.txt": "External root drop contents\n",
  "conflict.txt": "External conflict contents must survive\n",
};
const helper = join(directory, "clipboard");
const config = join(directory, "config.json");
let child;
let result;
let clipboardBackedUp = false;
let restoreChangeCount;
let log = "";

async function availablePort() {
  const server = createServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const { port } = server.address();
  await new Promise((resolve, reject) =>
    server.close((error) => (error ? reject(error) : resolve())),
  );
  return port;
}

function isolatedAppPid(parentPid) {
  const processes = execFileSync("ps", ["-axo", "pid=,ppid=,command="], {
    encoding: "utf8",
  })
    .trim()
    .split("\n")
    .map((line) => {
      const match = line.trim().match(/^(\d+)\s+(\d+)\s+(.+)$/);
      return match
        ? { pid: Number(match[1]), ppid: Number(match[2]), command: match[3] }
        : null;
    })
    .filter(Boolean);
  const descendants = new Set([parentPid]);
  let changed = true;
  while (changed) {
    changed = false;
    for (const process of processes) {
      if (!descendants.has(process.pid) && descendants.has(process.ppid)) {
        descendants.add(process.pid);
        changed = true;
      }
    }
  }
  return processes.find(
    (process) =>
      descendants.has(process.pid) &&
      /\/Lomi\.app\/Contents\/MacOS\/lomi(?:\s|$)/.test(process.command),
  )?.pid;
}

try {
  await mkdir(projectFolder, { recursive: true });
  await mkdir(join(projectFolder, "copy-dest"));
  await mkdir(join(projectFolder, "move-dest"));
  await mkdir(join(projectFolder, "drop-dest"));
  await mkdir(externalFolder);
  for (const [filename, contents] of Object.entries(externalFiles))
    await writeFile(join(externalFolder, filename), contents);
  await writeFile(
    join(projectFolder, "drop-dest/conflict.txt"),
    "Keep existing conflict bytes\n",
  );
  await mkdir(appData, { recursive: true });
  await writeFile(
    join(projectFolder, "alpha.txt"),
    "Explorer native clipboard smoke fixture\n",
  );
  await copyFile(
    join(repository, "tests/native/terminal-clipboard.swift"),
    join(directory, "clipboard.swift"),
  );
  execFileSync("swiftc", [join(directory, "clipboard.swift"), "-o", helper]);
  execFileSync(helper, ["backup", directory]);
  clipboardBackedUp = true;
  execFileSync(helper, ["text", directory]);

  const project = newProject(projectFolder, "local:zsh");
  await writeFile(
    join(appData, "session.json"),
    JSON.stringify({
      ...newSession(),
      projects: [project],
      activeProjectId: project.id,
    }),
  );
  const port = await availablePort();
  await writeFile(
    config,
    JSON.stringify({
      identifier,
      build: {
        beforeDevCommand: `pnpm dev --port ${port}`,
        devUrl: `http://127.0.0.1:${port}`,
      },
    }),
  );

  console.log(`Explorer clipboard smoke artifacts: ${directory}`);
  child = spawn(
    "pnpm",
    [
      "tauri",
      "dev",
      "--no-watch",
      "--features",
      "native-smoke",
      "--config",
      config,
    ],
    {
      cwd: repository,
      env: {
        ...process.env,
        LOMI_EXPLORER_CLIPBOARD_SMOKE_DIRECTORY: directory,
      },
      detached: true,
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  for (const stream of [child.stdout, child.stderr])
    stream.on("data", (chunk) => {
      log += chunk;
      process.stdout.write(chunk);
    });

  let appPid;
  for (let i = 0; i < 600 && child.exitCode === null; i++) {
    appPid = isolatedAppPid(child.pid);
    if (appPid) break;
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  if (!appPid) throw Error("Could not find the isolated Lomi app process.");
  let activation = "not-registered";
  for (let i = 0; i < 60; i++) {
    try {
      activation = execFileSync(
        helper,
        ["activate", directory, String(appPid)],
        { encoding: "utf8" },
      ).trim();
    } catch (error) {
      const helperOutput = `${error.stdout ?? ""}${error.stderr ?? ""}`;
      if (!helperOutput.includes("not-registered")) throw error;
      activation = "not-registered";
    }
    const frontmostPid = Number(activation.match(/frontmostPid=(\d+)/)?.[1]);
    if (frontmostPid === appPid) break;
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  console.log(`Native app activation: ${activation} (pid ${appPid})`);

  for (let i = 0; i < 600; i++) {
    try {
      result = JSON.parse(
        await readFile(join(directory, "result.json"), "utf8"),
      );
    } catch {}
    if (result || child.exitCode !== null) break;
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  if (!result) throw Error("Native Explorer clipboard smoke did not complete.");
  console.log(JSON.stringify(result, null, 2));
  restoreChangeCount = result.data?.clipboardChangeCount;
  if (result.stage !== "passed") process.exitCode = 1;
  else {
    for (const [filename, contents] of Object.entries(externalFiles)) {
      if ((await readFile(join(externalFolder, filename), "utf8")) !== contents)
        throw Error(`External drop modified its source: ${filename}`);
    }
    for (const [relative, expected] of [
      ["drop-dest/folder źródło.txt", externalFiles["folder źródło.txt"]],
      ["drop-dest/sibling source.txt", externalFiles["sibling source.txt"]],
      ["root source.txt", externalFiles["root source.txt"]],
      ["drop-dest/conflict.txt", "Keep existing conflict bytes\n"],
    ]) {
      if ((await readFile(join(projectFolder, relative), "utf8")) !== expected)
        throw Error(`External drop destination contents differ: ${relative}`);
    }
    console.log("External drop source and destination bytes verified on disk.");
  }
} finally {
  if (child?.pid) {
    try {
      process.kill(-child.pid, "SIGTERM");
    } catch {}
    await Promise.race([
      new Promise((resolve) => child.once("exit", resolve)),
      new Promise((resolve) => setTimeout(resolve, 5000)),
    ]);
  }
  if (clipboardBackedUp) {
    const args = ["restore", directory];
    if (Number.isInteger(restoreChangeCount))
      args.push(String(restoreChangeCount));
    try {
      const status = execFileSync(helper, args, { encoding: "utf8" }).trim();
      console.log(`System clipboard cleanup: ${status || "no output"}`);
    } catch (error) {
      console.error("Could not restore the system clipboard", error);
      process.exitCode = 1;
    }
  }
  if (child) await writeFile(join(directory, "native.log"), log);
  await rm(appData, { recursive: true, force: true });
}
