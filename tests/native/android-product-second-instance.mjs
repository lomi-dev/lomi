import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";

const root = resolve(process.argv[2]);
const name = process.argv[3];
if (!/^[a-zA-Z0-9-]{1,50}$/.test(name ?? ""))
  throw Error("Use a fresh second-instance evidence name");
const primary = JSON.parse(await readFile(join(root, "application.json")));
const binary = join(primary.repository, "src-tauri/target/release/lomi");
if (
  createHash("sha256")
    .update(await readFile(binary))
    .digest("hex") !== primary.binarySha256
)
  throw Error("The running fixture binary changed");
const evidence = join(root, `${name}-comparison.json`);
try {
  await readFile(evidence);
  throw Error("Preserve earlier second-instance evidence");
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
const control = (suffix, processId, script) => {
  const result = spawnSync(
    process.execPath,
    [
      join(import.meta.dirname, "android-product-control.mjs"),
      root,
      `${name}-${suffix}`,
    ],
    {
      input: JSON.stringify({
        processId,
        window: "main",
        script,
        timeoutMs: 40000,
      }),
      encoding: "utf8",
      timeout: 45000,
    },
  );
  if (result.status !== 0) throw Error(result.stderr || result.stdout);
  const response = JSON.parse(result.stdout);
  if (!response.ok) throw Error(JSON.stringify(response));
  return response.data;
};
const before = control(
  "primary-before",
  primary.pid,
  "return (await invoke('android_state')).statuses;",
);
if (!before.some((status) => status.processAlive))
  throw Error("Keep an owned phone running in the first instance");
const child = spawn(binary, [], {
  cwd: primary.repository,
  env: {
    ...process.env,
    LOMI_ANDROID_PRODUCT_DIRECTORY: primary.managed,
  },
  stdio: ["ignore", "ignore", "pipe"],
});
let errors = "";
child.stderr.on("data", (chunk) => {
  errors = (errors + chunk).slice(-16384);
});
const exited = new Promise((resolve) =>
  child.on("exit", (code, signal) => resolve({ code, signal })),
);
let directoryRejected = false;
try {
  const second = control(
    "secondary",
    child.pid,
    `
await wait(()=>document.querySelector('.tab-bar'));
let error=null;
try{await invoke('android_state');}catch(reason){error=String(reason);}
if(!error||!/another|busy|already|lock|occupied/i.test(error))throw Error('The second process acquired the Android directory: '+error);
return {error};
`,
  );
  const after = control(
    "primary-after",
    primary.pid,
    "return (await invoke('android_state')).statuses;",
  );
  directoryRejected = true;
  for (const previous of before.filter((status) => status.processAlive)) {
    const current = after.find(
      (status) => status.deviceId === previous.deviceId,
    );
    if (!current?.processAlive || current.generation !== previous.generation)
      throw Error("The second instance disturbed the first phone");
  }
  control(
    "secondary-close",
    child.pid,
    "await invoke('plugin:window|close',{label:'main'});return true;",
  );
  const result = await Promise.race([
    exited,
    new Promise((_, reject) => {
      const timer = setTimeout(
        () => reject(Error("Secondary close timed out")),
        30000,
      );
      timer.unref();
    }),
  ]);
  if (result.code !== 0)
    throw Error(
      `The secondary instance failed to close: ${JSON.stringify(result)} ${errors}`,
    );
  const report = {
    completed: true,
    primaryPid: primary.pid,
    secondaryPid: child.pid,
    binarySha256: primary.binarySha256,
    before,
    second,
    after,
    exit: result,
  };
  await writeFile(evidence, JSON.stringify(report, null, 2) + "\n", {
    flag: "wx",
  });
  console.log(JSON.stringify(report, null, 2));
} finally {
  if (child.exitCode === null && child.signalCode === null) {
    if (!directoryRejected) {
      // Preserve a process whose directory ownership could not be established.
      // Its normal close path must settle any devices before exit.
      try {
        control(
          "cleanup",
          child.pid,
          "await invoke('plugin:window|close',{label:'main'});return true;",
        );
      } catch (error) {
        console.error(
          `Secondary PID ${child.pid} requires normal close: ${error}`,
        );
      }
    } else {
      child.kill("SIGTERM");
    }
    const timeout = new Promise((resolve) => {
      const timer = setTimeout(() => resolve(null), 10000);
      timer.unref();
    });
    if ((await Promise.race([exited, timeout])) === null) {
      child.stderr.destroy();
      child.unref();
      console.error(
        `Secondary PID ${child.pid} is retained for inspection; exit was not confirmed.`,
      );
    }
  }
}
