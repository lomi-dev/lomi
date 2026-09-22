import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { browserProbe } from "./chat-browser-server.mjs";

const root = resolve(import.meta.dirname, "../..");
const directory = await mkdtemp(join(tmpdir(), "lomi-chat-native-"));
execFileSync(
  process.execPath,
  ["scripts/prepare-ai-runtime.mjs", "--fixture"],
  { cwd: root, stdio: "inherit" },
);
const identifier = `dev.lomi.chat-probe-${Date.now()}`;
const config = join(directory, "config.json");
const port = 1436;
await writeFile(
  config,
  JSON.stringify({
    identifier,
    build: {
      beforeDevCommand: `pnpm icon:macos && pnpm dev --port ${port}`,
      devUrl: `http://127.0.0.1:${port}`,
    },
  }),
);
console.log(`Native chat artifacts: ${directory}`);
const browser = await browserProbe(directory);
const child = spawn(
  "pnpm",
  [
    "tauri",
    "dev",
    "--no-watch",
    "--features",
    "chat-probe",
    "--config",
    config,
  ],
  {
    cwd: root,
    detached: true,
    env: {
      ...process.env,
      LOMI_CHAT_PROBE_DIRECTORY: directory,
      LOMI_CHAT_BROWSER_URL: browser.url,
    },
    stdio: ["ignore", "pipe", "pipe"],
  },
);
let log = "";
for (const stream of [child.stdout, child.stderr])
  stream.on("data", (data) => {
    log += data;
    process.stdout.write(data);
  });
try {
  let result;
  for (let i = 0; i < 600; i++) {
    result = await readFile(join(directory, "result.json"), "utf8")
      .then(JSON.parse)
      .catch(() => null);
    if (result || child.exitCode !== null) break;
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  if (!result) throw new Error("Native Chat AI probe did not complete.");
  console.log(JSON.stringify(result, null, 2));
  if (!result.passed) process.exitCode = 1;
} finally {
  browser.close();
  await writeFile(join(directory, "native.log"), log);
  try {
    process.kill(-child.pid, "SIGTERM");
  } catch {}
}
