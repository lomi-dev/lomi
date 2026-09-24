// Two native launches over one explicitly owned disposable application state.
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import {
  mkdtemp,
  open,
  readFile,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir, homedir } from "node:os";
import { join, resolve } from "node:path";
import { binaryFingerprint, sourceFingerprint } from "../mcp/routing-build.mjs";
const root = resolve(import.meta.dirname, "../..");
const audit = await mkdtemp(join(tmpdir(), "lomi-mcp-restart-"));
const identifier =
  "dev.lomi.mcp-control-batch-" + randomUUID().replaceAll("-", "");
const expectedAppData = join(
  homedir(),
  "Library/Application Support",
  identifier,
);
const sources = await sourceFingerprint(root);
const readJson = async (path) => JSON.parse(await readFile(path, "utf8"));
async function launch(args, env, logPath) {
  const log = await open(logPath, "wx", 0o600);
  const child = spawn(process.execPath, args, {
    cwd: root,
    env: { ...process.env, ...env },
    stdio: ["ignore", log.fd, log.fd],
    detached: true,
  });
  const done = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (code, signal) => resolve({ code, signal }));
  });
  return { child, done, log };
}
console.log("Native restart artifacts: " + audit);
const first = await launch(
  ["--experimental-strip-types", "tests/native/run-mcp-control.mjs"],
  { LOMI_MCP_BATCH_IDENTIFIER: identifier, LOMI_MCP_RESTART_PHASE: "first" },
  join(audit, "first.log"),
);
const firstExit = await first.done;
await first.log.close();
if (firstExit.code !== 0)
  throw Error("First native restart fixture failed; inspect " + audit);
const firstLog = await readFile(join(audit, "first.log"), "utf8");
const directory = firstLog.match(
  /Native MCP control artifacts: ([^\r\n]+)/,
)?.[1];
if (!directory) throw Error("Missing native restart directory");
const launchInfo = await readJson(join(directory, "restart-launch.json"));
if (
  launchInfo.appData !== expectedAppData ||
  launchInfo.identifier !== identifier ||
  launchInfo.directory !== directory ||
  launchInfo.binarySha256 !== (await binaryFingerprint(root)) ||
  sources !== (await sourceFingerprint(root))
)
  throw Error("Owned restart launch identity changed");
const firstCleanup = await readJson(join(directory, "cleanup.json"));
if (
  !firstCleanup.hostExited ||
  firstCleanup.exitCode !== 0 ||
  !firstCleanup.retainedForRestart
)
  throw Error("First native process did not close normally");
const baseline = await readJson(join(directory, "restart-baseline.json"));
const saved = await readJson(join(expectedAppData, "session.json"));
if (!JSON.stringify(saved).includes(baseline.target.panelId))
  throw Error("Ordinary close did not persist the tested panel");
await rename(
  join(directory, "result.json"),
  join(directory, "restart-first-result.json"),
);
await rename(
  join(directory, "cleanup.json"),
  join(directory, "restart-first-cleanup.json"),
);
console.log("Restarting exact native executable over preserved fixture state");
let second, secondExit;
try {
  second = await launch(
    ["tests/native/start-prebuilt-mcp.mjs"],
    {
      LOMI_MCP_BATCH_IDENTIFIER: identifier,
      LOMI_MCP_PREBUILT_SHA256: launchInfo.binarySha256,
      LOMI_MCP_CONTROL_PROBE_DIRECTORY: directory,
      LOMI_MCP_RESTART_PHASE: "second",
      GIT_CONFIG_GLOBAL: "/dev/null",
      GIT_CONFIG_NOSYSTEM: "1",
    },
    join(audit, "second.log"),
  );
  let timer;
  try {
    secondExit = await Promise.race([
      second.done,
      new Promise((_, reject) => {
        timer = setTimeout(
          () => reject(Error("Second native restart timed out")),
          120000,
        );
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
  if (secondExit.code !== 0)
    throw Error("Second native launch did not exit normally");
  const result = await readJson(join(directory, "result.json"));
  const proof = await readJson(join(directory, "restart-proof.json"));
  const processes = await readJson(join(directory, "prebuilt-process.json"));
  if (
    result.stage !== "passed" ||
    result.data?.profile !== "restart-only" ||
    !proof.passed ||
    proof.effectCount !== 1 ||
    !processes.events.some((e) => e.event === "native-close" && e.code === 0) ||
    !processes.events.some((e) => e.event === "vite-closed")
  )
    throw Error("Incomplete actual restart evidence");
  await writeFile(
    join(audit, "result.json"),
    JSON.stringify(
      {
        passed: true,
        directory,
        identifier,
        sourceFingerprint: sources,
        firstExit,
        secondExit,
        proof,
      },
      null,
      2,
    ),
  );
  console.log("Native application restart: PASS " + directory);
} finally {
  if (second && !secondExit) {
    try {
      process.kill(-second.child.pid, "SIGTERM");
    } catch {}
    let timer;
    try {
      secondExit = await Promise.race([
        second.done,
        new Promise((resolve) => {
          timer = setTimeout(() => resolve(null), 3000);
        }),
      ]);
    } finally {
      clearTimeout(timer);
    }
  }
  if (second && !secondExit) {
    try {
      process.kill(-second.child.pid, "SIGKILL");
    } catch {}
    let timer;
    try {
      secondExit = await Promise.race([
        second.done,
        new Promise((resolve) => {
          timer = setTimeout(() => resolve(null), 3000);
        }),
      ]);
    } finally {
      clearTimeout(timer);
    }
  }
  await second?.log.close();
  if (secondExit) {
    await rm(expectedAppData, {
      recursive: true,
      force: true,
      maxRetries: 5,
      retryDelay: 200,
    });
    await writeFile(
      join(audit, "cleanup.json"),
      JSON.stringify(
        { hostExited: true, appDataRemoved: true, secondExit },
        null,
        2,
      ),
    );
  } else
    await writeFile(
      join(audit, "cleanup.json"),
      JSON.stringify(
        { hostExited: false, appDataRemoved: false, appData: expectedAppData },
        null,
        2,
      ),
    );
}
