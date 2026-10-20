import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFile, writeFile, realpath } from "node:fs/promises";
import { join, resolve, basename, dirname, sep } from "node:path";

const windows = process.platform === "win32" && process.arch === "x64";
const mac = process.platform === "darwin" && process.arch === "arm64";
const linux = process.platform === "linux" && process.arch === "x64";
if (!windows && !mac && !linux)
  throw Error("Use a supported host's isolated debug product fixture.");
const root = await realpath(process.argv[2]);
const name = process.argv[3];
if (!/^[a-zA-Z0-9-]{1,50}$/.test(name ?? ""))
  throw Error("Use a fresh second-instance evidence name");
const receipt = await readFile(join(root, "application.json"));
const primary = JSON.parse(receipt);
const repository = await realpath(resolve(import.meta.dirname, "../.."));
const managed = await realpath(primary.managed);
const trial = dirname(root);
const comparable = (path) => (windows ? path.toLowerCase() : path);
if (
  basename(root) !== "product" ||
  !basename(trial).startsWith("lomi-android-stage0-") ||
  dirname(managed) !== trial ||
  !basename(managed).startsWith("native-managed-") ||
  comparable(root) === comparable(repository) ||
  comparable(root).startsWith(comparable(repository) + sep)
)
  throw Error("Use only an isolated product trial outside the repository.");
if (
  primary.repository !== repository ||
  primary.platform !== process.platform ||
  primary.architecture !== process.arch ||
  primary.buildProfile !== "debug" ||
  !/^dev\.lomi\.android-product-\d+$/.test(primary.identifier) ||
  !Number.isSafeInteger(primary.pid) ||
  primary.pid <= 0 ||
  !/^[a-f0-9]{64}$/.test(primary.binarySha256)
)
  throw Error("The receipt does not match this host's debug product fixture.");
if (
  JSON.parse(await readFile(join(trial, "evidence/consent.json"))).accepted !==
  true
)
  throw Error("Record actual SDK consent for this isolated trial first.");
const binary = join(
  repository,
  "src-tauri/target/debug",
  windows ? "lomi.exe" : "lomi",
);
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
async function assertFixture() {
  if (
    digest(await readFile(join(root, "application.json"))) !==
      digest(receipt) ||
    digest(await readFile(binary)) !== primary.binarySha256
  )
    throw Error("The fixture receipt or executable changed");
}
if (digest(await readFile(binary)) !== primary.binarySha256)
  throw Error("The running fixture binary changed");
const evidence = join(root, `${name}-comparison.json`);
try {
  await readFile(evidence);
  throw Error("Preserve earlier second-instance evidence");
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
const control = (suffix, processId, script) => {
  if (!Number.isSafeInteger(processId) || processId <= 0)
    throw Error("A verified fixture process ID is required for control");
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
function assertPhonesUnchanged(after) {
  for (const previous of before.filter((status) => status.processAlive)) {
    const current = after.find(
      (status) => status.deviceId === previous.deviceId,
    );
    if (!current?.processAlive || current.generation !== previous.generation)
      throw Error("The second instance disturbed the first phone");
  }
}
await assertFixture();
const env = { ...process.env, LOMI_ANDROID_PRODUCT_DIRECTORY: managed };
delete env.LOMI_ANDROID_PROBE_DIRECTORY;
const child = spawn(binary, [], {
  cwd: repository,
  env,
  stdio: ["ignore", "ignore", "pipe"],
});
let errors = "";
child.stderr.on("data", (chunk) => {
  errors = (errors + chunk).slice(-16384);
});
let exitResult;
const exited = new Promise((resolve) => {
  const finish = (result) => {
    exitResult = result;
    resolve(result);
  };
  child.once("exit", (code, signal) => finish({ code, signal }));
  child.once("error", (error) => finish({ error: error.message }));
});
function childRunning() {
  return (
    Number.isSafeInteger(child.pid) &&
    child.pid > 0 &&
    exitResult === undefined &&
    child.exitCode === null &&
    child.signalCode === null
  );
}
async function waitForExit(timeoutMs) {
  let timer;
  try {
    return await Promise.race([
      exited,
      new Promise((resolve) => {
        timer = setTimeout(() => resolve(null), timeoutMs);
        timer.unref();
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}
function secondaryControl(suffix, script) {
  if (!childRunning())
    throw Error(
      `The owned secondary process is no longer running: ${JSON.stringify(exitResult)}`,
    );
  return control(suffix, child.pid, script);
}
let directoryRejected = false;
let observedChatAiGuard = false;
try {
  await new Promise((resolve, reject) => {
    child.once("spawn", resolve);
    child.once("error", reject);
  });
  let earlyExit = linux ? await waitForExit(3000) : null;
  if (linux && !earlyExit) {
    await new Promise((resolve) => setImmediate(resolve));
    earlyExit = exitResult ?? null;
  }
  let second, after, result, shutdown, primaryWindow;
  if (earlyExit) {
    await new Promise((resolve) => setImmediate(resolve));
    const chatAiGuard =
      "Another Lomi instance owns Chat AI. Close it and retry.";
    observedChatAiGuard =
      earlyExit.code === 101 &&
      earlyExit.signal === null &&
      errors.includes(chatAiGuard);
    if (earlyExit.code !== 0 && !observedChatAiGuard)
      throw Error(
        `The secondary launch failed: ${JSON.stringify(earlyExit)} ${errors}`,
      );
    await assertFixture();
    after = control(
      "primary-after",
      primary.pid,
      "return (await invoke('android_state')).statuses;",
    );
    assertPhonesUnchanged(after);
    primaryWindow = control(
      "primary-visible",
      primary.pid,
      "return {visible:await invoke('plugin:window|is_visible',{label:'main'}),minimized:await invoke('plugin:window|is_minimized',{label:'main'}),documentVisibility:document.visibilityState};",
    );
    if (
      primaryWindow.visible !== true ||
      primaryWindow.minimized !== false ||
      primaryWindow.documentVisibility !== "visible"
    )
      throw Error(
        "The original application is not visibly responsive after the second launch",
      );
    await assertFixture();
    second = {
      observed: observedChatAiGuard
        ? "Chat AI ownership guard rejected secondary during setup"
        : "clean second-launch serialization/early exit",
      ...(observedChatAiGuard ? { guardMessage: chatAiGuard } : {}),
      limitation:
        "The exit does not establish that the Android directory lock was exercised",
    };
    result = earlyExit;
    shutdown = observedChatAiGuard
      ? "secondary setup failed with the known Chat AI ownership guard"
      : "secondary exited normally before fixture control";
  } else {
    const response = secondaryControl(
      "secondary",
      `
await wait(()=>document.querySelector('.tab-bar'));
let error=null;
try{await invoke('android_state');}catch(reason){error=String(reason);}
if(!error||!/another|busy|already|lock|occupied/i.test(error))throw Error('The second process acquired the Android directory: '+error);
return {error};
`,
    );
    second = response;
    after = control(
      "primary-after",
      primary.pid,
      "return (await invoke('android_state')).statuses;",
    );
    directoryRejected = true;
    assertPhonesUnchanged(after);
    await new Promise((resolve) => setImmediate(resolve));
    secondaryControl(
      "secondary-quit",
      "await invoke('request_quit');return true;",
    );
    result = await Promise.race([
      exited,
      new Promise((_, reject) => {
        const timer = setTimeout(
          () => reject(Error("Secondary guarded quit timed out")),
          30000,
        );
        timer.unref();
      }),
    ]);
    if (result.code !== 0)
      throw Error(
        `The secondary instance failed to quit: ${JSON.stringify(result)} ${errors}`,
      );
    shutdown = "production guarded request_quit";
  }
  const report = {
    completed: true,
    primaryPid: primary.pid,
    secondaryPid: child.pid,
    binarySha256: primary.binarySha256,
    before,
    second,
    after,
    directoryLockExercised: directoryRejected,
    observedChatAiGuard,
    secondaryStartupFailed: observedChatAiGuard,
    primaryWindow,
    shutdown,
    stderr: errors,
    exit: result,
  };
  await writeFile(evidence, JSON.stringify(report, null, 2) + "\n", {
    flag: "wx",
  });
  console.log(JSON.stringify(report, null, 2));
} finally {
  // Synchronous controllers can delay Node's child-exit callbacks. Process
  // pending native events before considering any cleanup instruction or signal.
  await new Promise((resolve) => setImmediate(resolve));
  if (childRunning()) {
    if (!directoryRejected) {
      // Preserve a process whose directory ownership could not be established.
      // Its normal close path must settle any devices before exit.
      try {
        secondaryControl(
          "cleanup",
          "await invoke('request_quit');return true;",
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
