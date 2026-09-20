import {
  mkdtemp,
  mkdir,
  readFile,
  writeFile,
  copyFile,
} from "node:fs/promises";
import { tmpdir, homedir } from "node:os";
import { join, resolve } from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { newProject, newSession } from "../../src/model.ts";

if (process.platform !== "darwin")
  throw Error("This clipboard fixture requires macOS.");
const repository = resolve(import.meta.dirname, "../..");
const directory = await mkdtemp(
  join(tmpdir(), "simplebench-clipboard-native-"),
);
const identifier = `dev.simplebench.clipboard-smoke-${Date.now()}`;
const appData = join(homedir(), "Library/Application Support", identifier);
const projectFolder = join(directory, "project with spaces");
await mkdir(projectFolder);
await mkdir(appData, { recursive: true });
await copyFile(
  join(repository, "src-tauri/icons/128x128.png"),
  join(directory, "fixture.png"),
);
const helper = join(directory, "clipboard");
execFileSync("swiftc", [
  join(import.meta.dirname, "terminal-clipboard.swift"),
  "-o",
  helper,
]);
execFileSync(helper, ["backup", directory]);
const project = newProject(projectFolder, "local:zsh");
project.workspaces[0].tabs[0].layout.id = "clipboard-terminal";
project.workspaces[0].tabs[0].activePaneId = "clipboard-terminal";
await writeFile(
  join(appData, "session.json"),
  JSON.stringify({
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  }),
);
await writeFile(
  join(directory, "capture.py"),
  `import sys,os,tty,termios,json
old=termios.tcgetattr(0)
index=sys.argv[1]
try:
 tty.setraw(0)
 os.write(1,('\\x1b[?2004hCLIPBOARD_CAPTURE_READY_'+index+'\\r\\n').encode())
 data=b''
 while not data.endswith(b'\\x1b[201~'): data+=os.read(0,1)
 with open(${JSON.stringify(projectFolder)}+'/captured-'+index+'.json','w') as f: json.dump(data.decode(),f)
 os.write(1,('CLIPBOARD_CAPTURE_DONE_'+index+'\\r\\n').encode())
finally: termios.tcsetattr(0,termios.TCSADRAIN,old)
`,
);
const port = Number(process.env.SIMPLEBENCH_CLIPBOARD_SMOKE_PORT ?? 1434);
const config = join(directory, "config.json");
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
console.log(`Native clipboard artifacts: ${directory}`);
const child = spawn(
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
      SIMPLEBENCH_CLIPBOARD_SMOKE_DIRECTORY: directory,
      ...(process.argv.includes("--agents")
        ? { SIMPLEBENCH_CLIPBOARD_SMOKE_AGENTS: "1" }
        : {}),
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
try {
  let result;
  for (let i = 0; i < 600; i++) {
    try {
      result = JSON.parse(
        await readFile(join(directory, "result.json"), "utf8"),
      );
    } catch {}
    if (result || child.exitCode !== null) break;
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  if (!result) throw Error("Native clipboard smoke did not complete.");
  console.log(JSON.stringify(result, null, 2));
  if (result.stage !== "passed") process.exitCode = 1;
} finally {
  execFileSync(helper, ["restore", directory]);
  await writeFile(join(directory, "native.log"), log);
  try {
    process.kill(-child.pid, "SIGTERM");
  } catch {}
}
