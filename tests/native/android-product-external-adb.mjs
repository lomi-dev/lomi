import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFile, writeFile, realpath, stat } from "node:fs/promises";
import { createServer } from "node:net";
import { join, basename, dirname } from "node:path";

const root = await realpath(process.argv[2]);
const name = process.argv[3];
if (!/^[a-zA-Z0-9-]{1,50}$/.test(name ?? ""))
  throw Error("Use a fresh external-server fixture name");
const application = JSON.parse(await readFile(join(root, "application.json")));
const managed = await realpath(application.managed);
if (
  basename(root) !== "product" ||
  dirname(managed) !== dirname(root) ||
  !basename(managed).startsWith("native-managed-") ||
  !basename(join(root, "..")).startsWith("lomi-android-stage0-")
)
  throw Error("Use only the isolated Android fixture");
const output = join(root, `${name}.json`);
const stopFile = join(root, `${name}-stop`);
for (const path of [output, stopFile]) {
  if (
    await stat(path).catch((error) => {
      if (error.code !== "ENOENT") throw error;
      return null;
    })
  )
    throw Error("Preserve earlier server evidence; choose a new name");
}
const reservation = createServer();
await new Promise((resolve, reject) => {
  reservation.once("error", reject);
  reservation.listen(15047, "127.0.0.1", resolve);
});
await new Promise((resolve, reject) =>
  reservation.close((error) => (error ? reject(error) : resolve())),
);
const env = Object.fromEntries(
  Object.entries(process.env).filter(
    ([key]) =>
      !/^(ANDROID_|ADB_|JAVA_|JDK_|_JAVA_|REPO_|QT_|DYLD_|LD_)/.test(key),
  ),
);
Object.assign(env, {
  ANDROID_HOME: join(managed, "sdk"),
  ANDROID_USER_HOME: join(managed, "user"),
  ANDROID_EMULATOR_HOME: join(managed, "emulator-home"),
  ANDROID_AVD_HOME: join(managed, "avd"),
  TMPDIR: join(managed, "tmp"),
});
const executable = join(managed, "sdk/platform-tools/adb");
const child = spawn(executable, ["-L", "tcp:15047", "server", "nodaemon"], {
  cwd: managed,
  env,
  stdio: ["ignore", "pipe", "pipe"],
});
let log = "";
for (const stream of [child.stdout, child.stderr])
  stream.on("data", (chunk) => {
    log = (log + chunk).slice(-16384);
  });
const exited = new Promise((resolve) =>
  child.once("exit", (code, signal) => resolve({ code, signal })),
);
let stop = false;
process.on("SIGINT", () => {
  stop = true;
});
process.on("SIGTERM", () => {
  stop = true;
});
const report = {
  pid: child.pid,
  port: 15047,
  executable,
  executableSha256: createHash("sha256")
    .update(await readFile(executable))
    .digest("hex"),
  startedAt: new Date().toISOString(),
  owner: "Independent native test driver, not the Tauri application",
  ready: false,
};
try {
  const readinessDeadline = Date.now() + 10000;
  while (
    !stop &&
    Date.now() < readinessDeadline &&
    child.exitCode === null &&
    child.signalCode === null
  ) {
    try {
      const listeners = execFileSync(
        "lsof",
        ["-nP", "-iTCP:15047", "-sTCP:LISTEN", "-Fp"],
        { encoding: "utf8" },
      );
      if (listeners.split("\n").includes(`p${child.pid}`)) {
        report.ready = true;
        break;
      }
    } catch {
      /* The private listener is not ready yet. */
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  if (!report.ready)
    throw Error("The independently owned ADB listener did not become ready");
  await writeFile(output, JSON.stringify(report, null, 2));
  console.log(
    `Independent test ADB PID ${child.pid}; create ${stopFile} to stop only this child.`,
  );
  const deadline = Date.now() + 2 * 60 * 60 * 1000;
  while (
    !stop &&
    Date.now() < deadline &&
    child.exitCode === null &&
    child.signalCode === null
  ) {
    if (
      await stat(stopFile).catch((error) => {
        if (error.code !== "ENOENT") throw error;
        return null;
      })
    )
      break;
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
} finally {
  if (child.exitCode === null && child.signalCode === null)
    child.kill("SIGTERM");
  const result = await exited;
  await writeFile(
    output,
    JSON.stringify(
      { ...report, stoppedAt: new Date().toISOString(), result },
      null,
      2,
    ),
  );
  await writeFile(join(root, `${name}.log`), log);
}
