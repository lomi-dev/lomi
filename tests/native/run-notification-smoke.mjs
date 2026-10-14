import { mkdtemp, mkdir, readFile, writeFile } from "node:fs/promises";
import { tmpdir, homedir } from "node:os";
import { join, resolve } from "node:path";
import { execFileSync, spawn } from "node:child_process";
import { newSession, newProject } from "../../src/model.ts";

if (process.platform !== "darwin")
  throw Error("This native notification runner currently supports macOS.");
const root = resolve(import.meta.dirname, "../..");
const directory = await mkdtemp(join(tmpdir(), "lomi-notification-native-"));
const inboxOnly = process.argv.includes("--inbox-only");
const identifier = `dev.lomi.notification-smoke-${Date.now()}`;
const appData = join(homedir(), "Library/Application Support", identifier);
const folder = join(directory, "project");
const claude = join(directory, "claude");
await mkdir(folder);
await mkdir(claude);
await mkdir(appData, { recursive: true });
const agentSource = join(directory, "notification-agent.c");
await writeFile(
  agentSource,
  '#include <stdio.h>\n#include <unistd.h>\nint main(void) { sleep(1); fputs("\\033]9;PRIVATE_NATIVE_AGENT_MESSAGE\\007", stdout); fflush(stdout); sleep(30); return 0; }\n',
);
const sdkPath = execFileSync(
  "/usr/bin/xcrun",
  ["--sdk", "macosx", "--show-sdk-path"],
  { encoding: "utf8" },
).trim();
execFileSync("/usr/bin/xcrun", [
  "clang",
  "-isysroot",
  sdkPath,
  agentSource,
  "-o",
  join(folder, "codex"),
]);
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
const port = Number(process.env.LOMI_NOTIFICATION_SMOKE_PORT ?? 1437);
await writeFile(
  config,
  JSON.stringify({
    identifier,
    build: {
      beforeDevCommand: `pnpm dev --port ${port}`,
      devUrl: `http://127.0.0.1:${port}${inboxOnly ? "/?notification-smoke=inbox" : ""}`,
    },
  }),
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
      CODEX_HOME: join(directory, "codex-home"),
      LOMI_NOTIFICATION_SMOKE_DIRECTORY: directory,
    },
    detached: true,
    stdio: ["ignore", "pipe", "pipe"],
  },
);
let log = "";
// Cold compilation has its own bound; application checks retain three minutes.
const buildDeadline = Date.now() + 15 * 60_000;
let applicationDeadline;
for (const stream of [child.stdout, child.stderr])
  stream.on("data", (chunk) => {
    log += chunk;
    process.stdout.write(chunk);
    const plainLog = log.replace(/\u001b\[[0-9;]*m/g, "");
    if (
      applicationDeadline === undefined &&
      /Finished `dev` profile/.test(plainLog)
    )
      applicationDeadline = Date.now() + 180000;
  });
try {
  let result;
  while (!result && Date.now() < (applicationDeadline ?? buildDeadline)) {
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
  if (!result)
    throw Error(
      applicationDeadline === undefined
        ? "Native notification smoke build timed out."
        : "Native notification smoke application timed out.",
    );
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
