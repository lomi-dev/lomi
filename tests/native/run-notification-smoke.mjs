import { mkdtemp, mkdir, readFile, writeFile } from "node:fs/promises";
import { tmpdir, homedir } from "node:os";
import { join, resolve } from "node:path";
import { spawn } from "node:child_process";
import { newSession, newProject } from "../../src/model.ts";

if (process.platform !== "darwin")
  throw Error("This native notification runner currently supports macOS.");
const root = resolve(import.meta.dirname, "../..");
const directory = await mkdtemp(join(tmpdir(), "lomi-notification-native-"));
const identifier = `dev.lomi.notification-smoke-${Date.now()}`;
const appData = join(homedir(), "Library/Application Support", identifier);
const folder = join(directory, "project");
const claude = join(directory, "claude");
await mkdir(folder);
await mkdir(claude);
await mkdir(appData, { recursive: true });
await writeFile(join(claude, "settings.json"), '{"env":{"PRESERVE":"yes"}}\n');
const project = newProject(folder, "local:zsh");
const tab = project.workspaces[0].tabs[0];
tab.layout.id = "notification-terminal";
tab.activePaneId = tab.layout.id;
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
  JSON.stringify({ identifier, build: { beforeDevCommand: "" } }),
);
console.log(
  `Native notification artifacts: ${directory}\nIsolated application data: ${appData}`,
);
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
    cwd: root,
    env: {
      ...process.env,
      CLAUDE_CONFIG_DIR: claude,
      LOMI_NOTIFICATION_SMOKE_DIRECTORY: directory,
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
  const deadline = Date.now() + 180000;
  let result;
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
        throw Error(`Tauri exited before reporting: ${child.exitCode}`);
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
  }
  if (!result) throw Error("Native notification smoke timed out.");
  console.log(JSON.stringify(result, null, 2));
  if (result.stage !== "passed") process.exitCode = 1;
} finally {
  try {
    process.kill(-child.pid, "SIGTERM");
  } catch (error) {
    if (error.code !== "ESRCH") throw error;
  }
  await writeFile(join(directory, "native.log"), log);
}
