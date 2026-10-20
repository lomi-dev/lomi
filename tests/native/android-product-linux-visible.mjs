// Opt-in Linux product fixture. Run alongside android-product-linux-perf.py.
// Linux Canvas2D uses bitmap preflight, then input/canonical-source provenance.
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  open,
  readFile,
  writeFile,
  realpath,
  readdir,
  readlink,
  stat,
} from "node:fs/promises";
import { createConnection } from "node:net";
import { basename, dirname, join, resolve, sep } from "node:path";

if (process.platform !== "linux" || process.arch !== "x64")
  throw Error("Use the Linux x64 isolated debug product fixture.");
const [
  productArgument,
  deviceId,
  name,
  secondsArgument = "1800",
  tabArgument,
  viewsArgument = "1",
  transportArgument = "any",
] = process.argv.slice(2);
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
if (
  !productArgument ||
  !uuid.test(deviceId ?? "") ||
  !/^[A-Za-z0-9-]{1,32}$/.test(name ?? "") ||
  !/^\d+$/.test(secondsArgument) ||
  !["1", "2"].includes(viewsArgument) ||
  transportArgument !== "any"
)
  throw Error(
    "Use PRODUCT DEVICE_UUID NAME [durationSeconds 1..1800] [activeTab] [views 1|2] [transport any].",
  );
const seconds = Number(secondsArgument),
  views = Number(viewsArgument);
if (seconds < 1 || seconds > 1800) throw Error("Invalid visible duration.");
const root = await realpath(productArgument);
const trial = dirname(root);
const repository = await realpath(resolve(import.meta.dirname, "../.."));
const receiptBytes = await readFile(join(root, "application.json"));
const application = JSON.parse(receiptBytes);
const managed = await realpath(application.managed);
if (
  basename(root) !== "product" ||
  !basename(trial).startsWith("lomi-android-stage0-") ||
  dirname(managed) !== trial ||
  !basename(managed).startsWith("native-managed-") ||
  root === repository ||
  root.startsWith(repository + sep)
)
  throw Error("Use only an isolated trial outside the repository.");
if (
  application.repository !== repository ||
  application.buildProfile !== "debug" ||
  application.platform !== "linux" ||
  application.architecture !== "x64" ||
  !Number.isSafeInteger(application.pid) ||
  application.pid <= 0 ||
  !/^dev\.lomi\.android-product-\d+$/.test(application.identifier)
)
  throw Error("The receipt does not identify this host's debug fixture.");
const binary = join(repository, "src-tauri/target/debug/lomi");
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
if (
  !/^[a-f0-9]{64}$/.test(application.binarySha256) ||
  digest(await readFile(binary)) !== application.binarySha256
)
  throw Error("The fixture executable does not match its receipt.");
// Native Linux identities contain no command line or guest generation key.
const bootId = (
  await readFile("/proc/sys/kernel/random/boot_id", "utf8")
).trim();
function procStat(text) {
  const tail = text
    .slice(text.lastIndexOf(") ") + 2)
    .trim()
    .split(/\s+/);
  if (tail.length < 20 || !/^\d+$/.test(tail[19]))
    throw Error("Invalid native process stat.");
  return { created: tail[19], state: tail[0] };
}
async function nativeIdentity(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 0)
    throw Error("Invalid native PID.");
  const directory = `/proc/${pid}`;
  const before = procStat(await readFile(join(directory, "stat"), "utf8"));
  const executable = await readlink(join(directory, "exe"));
  const executableStat = await stat(join(directory, "exe"), { bigint: true });
  const owner = await stat(directory);
  const executableFile = `${executableStat.dev}:${executableStat.ino}:${executableStat.size}:${executableStat.mtimeNs}`;
  const finalExecutableStat = await stat(join(directory, "exe"), {
    bigint: true,
  });
  const finalExecutableFile = `${finalExecutableStat.dev}:${finalExecutableStat.ino}:${finalExecutableStat.size}:${finalExecutableStat.mtimeNs}`;
  const after = procStat(await readFile(join(directory, "stat"), "utf8"));
  if (
    before.created !== after.created ||
    ["Z", "X"].includes(after.state) ||
    executable !== (await readlink(join(directory, "exe"))) ||
    owner.uid !== process.getuid() ||
    executableFile !== finalExecutableFile ||
    (await readFile("/proc/sys/kernel/random/boot_id", "utf8")).trim() !==
      bootId
  )
    throw Error("Native process identity changed or is inaccessible.");
  return {
    pid,
    created: before.created,
    boot: bootId,
    executable,
    file: executableFile,
  };
}
function sameIdentity(left, right) {
  return ["pid", "created", "boot", "executable", "file"].every(
    (key) => left[key] === right[key],
  );
}
const appIdentity = await nativeIdentity(application.pid);
if (
  appIdentity.executable !== binary ||
  digest(await readFile(binary)) !== application.binarySha256
)
  throw Error("Application PID is not the receipt executable.");
const hzResult = spawnSync("getconf", ["CLK_TCK"], { encoding: "utf8" });
const hz = Number(hzResult.stdout?.trim());
if (hzResult.status !== 0 || !Number.isSafeInteger(hz) || hz <= 0)
  throw Error("Cannot determine Linux process clock ticks.");
const bootTime = /^btime (\d+)$/m.exec(await readFile("/proc/stat", "utf8"));
if (
  !bootTime ||
  Math.abs(
    Number(bootTime[1]) * 1000 +
      (Number(appIdentity.created) / hz) * 1000 -
      Date.parse(application.startedAt),
  ) > 10000 ||
  !Number.isFinite(Date.parse(application.startedAt))
)
  throw Error("Application process creation differs from receipt launch time.");

for (const consent of ["consent.json", "java-consent.json"])
  if (
    JSON.parse(await readFile(join(trial, "evidence", consent))).accepted !==
    true
  )
    throw Error(
      "Record actual SDK and private-Java consent for this isolated trial first.",
    );
const recordPath = join(managed, "runtime", deviceId + ".json");
async function runtimeRecord() {
  try {
    return JSON.parse(await readFile(recordPath));
  } catch {
    throw Error("The private runtime record is unreadable or invalid.");
  }
}
const record = await runtimeRecord();
if (
  record.deviceId !== deviceId ||
  !uuid.test(record.generation) ||
  !uuid.test(record.generationKey) ||
  record.adbPort !== 15047 ||
  !Number.isInteger(record.consolePort) ||
  record.consolePort < 5554 ||
  record.consolePort > 5682 ||
  record.consolePort % 2
)
  throw Error("Invalid private fixture runtime identity or ports.");
const guestIdentity = await nativeIdentity(record.process?.pid);
if (
  record.launching === true ||
  guestIdentity.created !== String(record.process.created) ||
  guestIdentity.boot !== record.process.boot ||
  guestIdentity.executable !== record.process.executable ||
  !guestIdentity.executable.startsWith(join(managed, "sdk/emulator") + sep) ||
  !basename(guestIdentity.executable).startsWith("qemu-system-")
)
  throw Error(
    "The runtime record does not pin the private managed QEMU process.",
  );

const devices = JSON.parse(
  await readFile(join(managed, "devices.json")),
).devices;
const device = devices.find((item) => item.id === deviceId);
if (!device || typeof device.name !== "string")
  throw Error("Missing fixture phone.");
const activeTab = tabArgument ?? device.name;
if (!activeTab || activeTab.length > 200) throw Error("Invalid active tab.");
const config = new Map(
  (
    await readFile(
      join(managed, "avd", `sb_${deviceId}.avd`, "config.ini"),
      "utf8",
    )
  )
    .split(/\r?\n/)
    .filter((line) => line.includes("="))
    .map((line) => {
      const at = line.indexOf("=");
      return [line.slice(0, at).trim(), line.slice(at + 1).trim()];
    }),
);
const width = Number(config.get("hw.lcd.width")),
  height = Number(config.get("hw.lcd.height"));
if (
  ![width, height].every((n) => Number.isInteger(n) && n > 0 && n <= 4096) ||
  width * height > 3840 * 2160
)
  throw Error("Invalid guest dimensions.");
if ((await readdir(root)).some((file) => file.startsWith(name + "-")))
  throw Error("Preserve previous evidence; use a fresh measurement name.");
const driverPath = join(root, name + "-driver.json");
const report = {
  completed: false,
  deviceId,
  generation: record.generation,
  applicationPid: application.pid,
  applicationIdentity: appIdentity,
  guestIdentity,
  qualification: false,
  rendererKind: "linux-canvas-2d",
  softwareBackingInferenceOnly: true,
  formalFingerprintEvidence:
    "successful-input-and-canonical-primary; actual-bitmap-preflight-only",
  binarySha256: application.binarySha256,
  driverSourceSha256: digest(await readFile(new URL(import.meta.url))),
  guest: [width, height],
  activeTab,
  baselineTab: "Terminal",
  views,
  visibleSeconds: seconds,
  warmupSeconds: 30,
  baselineSeconds: 60,
  afterSeconds: 60,
  startedAt: new Date().toISOString(),
  swipeCount: 0,
  phases: [],
};
await writeFile(driverPath, JSON.stringify(report, null, 2), { flag: "wx" });
const journal = await open(join(root, name + "-phases.jsonl"), "wx");
const cancellation = new AbortController();
const sockets = new Set();
const controllers = new Set();
let index = 0,
  animationError,
  animation;
function stop() {
  cancellation.abort();
  for (const socket of sockets) socket.destroy();
  for (const child of controllers) child.kill(); // Only our Node controller helpers.
}
process.on("SIGINT", stop);
process.on("SIGTERM", stop);
function check() {
  if (cancellation.signal.aborted)
    throw Error("Visible qualification cancelled.");
}
async function phase(phase, event, extra = {}) {
  const entry = { phase, event, at: new Date().toISOString(), ...extra };
  report.phases.push(entry);
  await journal.write(JSON.stringify(entry) + "\n");
  console.log(JSON.stringify(entry));
}
async function assertFixture() {
  check();
  if (
    digest(await readFile(join(root, "application.json"))) !==
    digest(receiptBytes)
  )
    throw Error("Application receipt changed.");
  if (
    !sameIdentity(appIdentity, await nativeIdentity(application.pid)) ||
    !sameIdentity(guestIdentity, await nativeIdentity(record.process.pid))
  )
    throw Error("A pinned native process identity changed.");
  const current = await runtimeRecord();
  if (
    current.deviceId !== deviceId ||
    current.generation !== record.generation ||
    current.generationKey !== record.generationKey ||
    current.consolePort !== record.consolePort ||
    current.adbPort !== 15047 ||
    current.process?.pid !== record.process.pid ||
    current.process?.created !== record.process.created ||
    current.process?.boot !== record.process.boot ||
    current.process?.executable !== record.process.executable
  )
    throw Error("The owned runtime generation changed.");
}
async function control(script, timeoutMs = 30000, cleanup = false) {
  if (!cleanup) await assertFixture();
  else if (
    digest(await readFile(join(root, "application.json"))) !==
      digest(receiptBytes) ||
    !sameIdentity(appIdentity, await nativeIdentity(application.pid))
  )
    throw Error(
      "The original app identity is unavailable for benchmark cleanup.",
    );
  return new Promise((resolveResult, reject) => {
    const child = spawn(
      process.execPath,
      [
        join(import.meta.dirname, "android-product-control.mjs"),
        root,
        `${name}-${++index}`,
      ],
      { stdio: ["pipe", "pipe", "pipe"], windowsHide: false },
    );
    if (!cleanup) controllers.add(child);
    let stdout = "",
      stderr = "",
      settled = false;
    const timer = setTimeout(
      () => finish(Error("Product UI controller timed out.")),
      timeoutMs + 5000,
    );
    function finish(error, result) {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      controllers.delete(child);
      if (error) {
        child.kill();
        reject(error);
      } else resolveResult(result);
    }
    child.stdout.on("data", (chunk) => {
      stdout += chunk;
      if (stdout.length > 4 * 1024 * 1024)
        finish(Error("Product report exceeds its limit."));
    });
    child.stderr.on("data", (chunk) => {
      if (stderr.length < 65536) stderr += chunk;
    });
    child.on("error", (error) => finish(error));
    child.stdin.on("error", (error) => finish(error));
    child.on("close", (code) => {
      if (code !== 0)
        return finish(
          Error(
            "Product UI instruction failed: " +
              (stderr || stdout).slice(0, 65536),
          ),
        );
      try {
        const value = JSON.parse(stdout);
        if (!value.ok) throw Error("Product UI instruction rejected.");
        finish(null, value.data);
      } catch (error) {
        finish(error);
      }
    });
    child.stdin.end(
      JSON.stringify({
        window: "main",
        processId: application.pid,
        script,
        timeoutMs,
      }),
    );
  });
}

// A reader owns exactly one loopback socket. Its fixed deadline includes connect,
// transport selection, guest ownership checks and the entire shell-v2 response.
async function adb(operation) {
  check();
  const socket = createConnection({ host: "127.0.0.1", port: 15047 });
  sockets.add(socket);
  let buffer = Buffer.alloc(0),
    failure,
    pending;
  function pump() {
    if (!pending) return;
    if (buffer.length >= pending.length) {
      const waiter = pending;
      pending = undefined;
      const bytes = buffer.subarray(0, waiter.length);
      buffer = buffer.subarray(waiter.length);
      waiter.resolve(bytes);
    } else if (failure) {
      const waiter = pending;
      pending = undefined;
      waiter.reject(failure);
    }
  }
  socket.on("data", (chunk) => {
    if (buffer.length + chunk.length > 1024 * 1024 + 65536)
      socket.destroy(Error("Private ADB response exceeds its bound."));
    else {
      buffer = Buffer.concat([buffer, chunk]);
      pump();
    }
  });
  socket.on("error", (error) => {
    failure = error;
    pump();
  });
  socket.on("close", () => {
    failure ??= Error("Private ADB transport closed.");
    pump();
  });
  const timer = setTimeout(
    () => socket.destroy(Error("Private ADB twenty-second deadline expired.")),
    20000,
  );
  const read = (length) =>
    new Promise((resolveRead, reject) => {
      if (pending)
        return reject(Error("Concurrent private ADB reads are forbidden."));
      pending = { length, resolve: resolveRead, reject };
      pump();
    });
  const request = async (text) => {
    check();
    const payload = Buffer.from(text);
    socket.write(
      Buffer.concat([
        Buffer.from(payload.length.toString(16).padStart(4, "0")),
        payload,
      ]),
    );
    if ((await read(4)).toString() !== "OKAY")
      throw Error("Private ADB rejected the fixed fixture request.");
  };
  try {
    return await operation({ read, request });
  } finally {
    clearTimeout(timer);
    sockets.delete(socket);
    socket.destroy();
  }
}
async function preflightAdb() {
  await adb(async ({ request, read }) => {
    await request("host:version");
    if (
      (await read(4)).toString() !== "0004" ||
      (await read(4)).toString() !== "0029"
    )
      throw Error("The private ADB server version changed.");
  });
}
let settingsHomepage;
async function guestAction(action, iteration = 0) {
  await assertFixture();
  const x = Math.floor((width * 333) / 720),
    top = Math.floor((height * 266) / 1280),
    bottom = Math.floor((height * 1000) / 1280);
  const [from, to] = iteration % 2 ? [top, bottom] : [bottom, top];
  const command =
    action === "swipe"
      ? `input swipe ${x} ${from} ${x} ${to} 1000`
      : action === "resumed-settings"
        ? "dumpsys activity activities"
        : action === "resolve-settings"
          ? "cmd package resolve-activity --brief --user 0 -a android.settings.SETTINGS"
          : action === "reset-settings" && settingsHomepage
            ? `am start -W --user 0 -f 0x10008000 -n '${settingsHomepage}'`
            : null;
  if (!command) throw Error("Unknown fixed guest measurement action.");
  return await adb(async ({ request, read }) => {
    await request("host:transport:emulator-" + record.consolePort);
    // UUID-only values were validated above. Check both identities inside the
    // same transport immediately before each fixed action; never report the key.
    const guard = `[ "$(getprop ro.boot.lomi.device)" = "${deviceId}" ] && [ "$(settings get global lomi_generation)" = "${record.generationKey}" ] || exit 77; `;
    await request("shell,v2,raw:" + guard + command);
    let total = 0;
    const stdout = [],
      stderr = [];
    for (;;) {
      const header = await read(5),
        kind = header[0],
        length = header.readUInt32LE(1);
      total += length;
      if (length > 1024 * 1024 || total > 1024 * 1024)
        throw Error("Private shell-v2 output exceeds 1 MiB.");
      const body = await read(length);
      if (kind === 3) {
        if (length !== 1 || body[0] !== 0)
          throw Error(
            `The owned guest rejected ${action}: shell-v2 exit length ${length}, code ${body[0] ?? "missing"}: ${Buffer.concat(stderr).toString("utf8").replaceAll(record.generationKey, "[redacted]")}.`,
          );
        return Buffer.concat(stdout).toString("utf8");
      }
      if (kind !== 1 && kind !== 2)
        throw Error("Invalid private shell-v2 response.");
      if (kind === 1 && action !== "swipe") stdout.push(body);
      if (kind === 2) stderr.push(body);
    }
  });
}
function normalizedSettingsComponent(component) {
  if (typeof component !== "string" || component.length > 256)
    throw Error("Invalid resolved Settings component.");
  const match =
    /^com\.android\.settings\/((?:\.?[A-Za-z_$][A-Za-z0-9_$]*)(?:\.[A-Za-z_$][A-Za-z0-9_$]*)*)$/.exec(
      component,
    );
  if (!match) throw Error("Unexpected resolved Settings component.");
  const classname = match[1].startsWith(".")
    ? "com.android.settings" + match[1]
    : match[1];
  if (!classname.startsWith("com.android.settings."))
    throw Error("Unexpected resolved Settings class.");
  return "com.android.settings/" + classname;
}
function resolvedSettingsComponent(output) {
  const components = output
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.includes("/"));
  if (components.length !== 1)
    throw Error("The Settings homepage resolution is absent or ambiguous.");
  return normalizedSettingsComponent(components[0]);
}
function resumedSettingsComponent(output) {
  const lines = output.split(/\r?\n/);
  const top = lines.filter((line) => /\btopResumedActivity\s*[:=]/.test(line));
  const resumed = top.length
    ? top
    : lines.filter((line) =>
        /\b(?:mResumedActivity|ResumedActivity)\s*[:=]/.test(line),
      );
  const components = resumed.map((line) => {
    const matches = [
      ...line.matchAll(/\b([A-Za-z][A-Za-z0-9_.]*\/[A-Za-z0-9_.$]+)(?=[\s}])/g),
    ];
    if (matches.length !== 1)
      throw Error("The resumed Settings component is unreadable.");
    return normalizedSettingsComponent(matches[0][1]);
  });
  if (
    !components.length ||
    components.some((component) => component !== components[0])
  )
    throw Error("The resumed Settings component is absent or ambiguous.");
  return components[0];
}
async function settingsResumed(stage) {
  const component = resumedSettingsComponent(
    await guestAction("resumed-settings"),
  );
  if (!settingsHomepage || component !== settingsHomepage)
    throw Error(
      "The resolved Settings homepage is not the resumed guest component.",
    );
  await phase("workload", "settings-resumed", {
    stage,
    component,
    resolvedComponent: settingsHomepage,
  });
}

async function measureHidden(invoke, sleep, seconds) {
  const started = performance.now(),
    startedAt = new Date().toISOString();
  let checks = 0;
  for (;;) {
    const state = await invoke("android_state");
    if (state.streams.some((stream) => stream.phase === "streaming"))
      throw Error("A production stream restarted during a hidden baseline.");
    const visible = [...document.querySelectorAll(".android-screen")].some(
      (canvas) => {
        const bounds = canvas.getBoundingClientRect();
        return (
          canvas.isConnected &&
          bounds.width > 0 &&
          bounds.height > 0 &&
          getComputedStyle(canvas).visibility === "visible"
        );
      },
    );
    if (visible)
      throw Error("A phone canvas became visible during a hidden baseline.");
    checks++;
    const remaining = seconds * 1000 - (performance.now() - started);
    if (remaining <= 0) break;
    await sleep(Math.min(1000, remaining));
  }
  return {
    checks,
    startedAt,
    endedAt: new Date().toISOString(),
    wallElapsed: (performance.now() - started) / 1000,
  };
}

// This function is serialized into the trusted app. It uses the fixture's
// invoke/sleep helpers, and releases only hooks it still owns in its finally block.
async function measureVisible(invoke, sleep, options) {
  const {
    deviceId,
    generation,
    seconds,
    views,
    name,
    requireShared,
    inputEvidence = false,
  } = options;
  const state = await invoke("android_state");
  const live = state.streams.filter((stream) => stream.phase === "streaming");
  if (
    live.length !== 1 ||
    live[0].deviceId !== deviceId ||
    live[0].generation !== generation
  )
    throw Error("Expected exactly the requested phone's production stream.");
  const before = live[0];
  const canvases = [...document.querySelectorAll(".android-screen")];
  if (canvases.length !== views)
    throw Error("Unexpected number of visible phone canvases.");
  const identities = new Map(
    canvases.map((canvas) => [
      canvas,
      {
        paneId: canvas.closest("[data-android-pane-id]")?.dataset.androidPaneId,
        framebuffer: [canvas.width, canvas.height],
        css: [
          canvas.getBoundingClientRect().width,
          canvas.getBoundingClientRect().height,
        ],
        dpr: devicePixelRatio,
        source: null,
        frames: 0,
        draws: [],
        gaps: [],
        last: null,
        fingerprint: null,
        fingerprintedFrames: 0,
        fingerprintTransitions: 0,
        lastChange: null,
        maxUnchangedMs: 0,
        uploadKind: null,
        sourceCanvas: null,
        pendingUpload: null,
        drawnSequence: 0,
        drawnAt: null,
      },
    ]),
  );
  if (
    [...identities.values()].some(
      (view) =>
        !view.paneId ||
        view.framebuffer.some((n) => !Number.isInteger(n) || n <= 0) ||
        view.css.some((n) => n <= 0),
    )
  )
    throw Error(
      "A measured view has no valid pane, framebuffer or CSS identity.",
    );
  let failure = null,
    cancelled = false,
    result;
  const owner = {
    name,
    cancel: () => {
      cancelled = true;
    },
  };
  if (window.__androidProductBenchmark)
    throw Error("Another product benchmark is active.");
  const prototype = CanvasRenderingContext2D.prototype;
  const originalPut = prototype.putImageData,
    originalDraw = prototype.drawImage,
    originalRead = prototype.getImageData;
  const started = performance.now(),
    utcStarted = new Date().toISOString();
  let primaryCanvas = null,
    sourceSequence = 0;
  let sharedBufferEvents = 0;
  const bridge = window.chrome?.webview;
  const sharedFrame = (event) => {
    const data = event.additionalData;
    if (
      data?.protocol === "lomi.android.frame.v1" &&
      data.deviceId === deviceId &&
      data.generation === generation
    )
      sharedBufferEvents++;
  };
  window.__androidProductBenchmark = owner;
  function observe(context, view, start, drawMs, sequence, inputFingerprint) {
    try {
      // The unchanged bitmap preflight reads actual pixels. The formal phase
      // uses successful ImageData input/canonical-primary provenance instead.
      let fingerprint = inputEvidence ? inputFingerprint : 2166136261;
      if (inputEvidence && !Number.isInteger(fingerprint))
        throw Error("Missing input/canonical fingerprint");
      const count = context.canvas.width * context.canvas.height;
      for (let sample = 0; !inputEvidence && sample < 64; sample++) {
        const offset = Math.floor((sample * (count - 1)) / 63);
        const rgba = originalRead.call(
          context,
          offset % context.canvas.width,
          Math.floor(offset / context.canvas.width),
          1,
          1,
        ).data;
        for (let channel = 0; channel < 3; channel++)
          fingerprint = Math.imul(fingerprint ^ rgba[channel], 16777619) >>> 0;
      }
      if (view.fingerprint !== null && view.fingerprint !== fingerprint) {
        view.fingerprintTransitions++;
        view.maxUnchangedMs = Math.max(
          view.maxUnchangedMs,
          start - view.lastChange,
        );
        view.lastChange = start;
      }
      view.lastChange ??= start;
      view.fingerprint = fingerprint;
      view.fingerprintedFrames++;
      view.drawnSequence = sequence;
      view.drawnAt = start;
      view.contextAttributes = context.getContextAttributes?.() ?? null;
      if (
        view.contextAttributes &&
        (("alpha" in view.contextAttributes &&
          view.contextAttributes.alpha !== false) ||
          ("willReadFrequently" in view.contextAttributes &&
            view.contextAttributes.willReadFrequently !== true))
      )
        failure ??=
          "Measured 2D context attributes differ from the software request.";
    } catch {
      failure ??= inputEvidence
        ? "2D input/canonical-source fingerprint is unavailable."
        : "Actual 2D framebuffer fingerprint is unavailable.";
    }
    view.frames++;
    view.draws.push(drawMs);
    if (view.last !== null) view.gaps.push(start - view.last);
    view.last = start;
  }
  function fingerprintInput(pixels) {
    let fingerprint = 2166136261;
    const count = pixels.byteLength / 4;
    for (let sample = 0; sample < 64; sample++) {
      const offset = Math.floor((sample * (count - 1)) / 63) * 4;
      for (let channel = 0; channel < 3; channel++)
        fingerprint =
          Math.imul(fingerprint ^ pixels[offset + channel], 16777619) >>> 0;
    }
    return fingerprint;
  }
  const putHook = function (...args) {
    const start = performance.now(),
      result = originalPut.apply(this, args);
    if (this.canvas.classList.contains("android-screen")) {
      const view = identities.get(this.canvas),
        image = args[0];
      if (!view) failure ??= "A measured canvas was replaced.";
      else if (
        args.length !== 3 ||
        args[1] !== 0 ||
        args[2] !== 0 ||
        !(image instanceof ImageData) ||
        !(image.data instanceof Uint8ClampedArray) ||
        image.width !== 720 ||
        image.height !== 1280 ||
        image.data.byteLength !== 720 * 1280 * 4 ||
        this.canvas.width !== image.width ||
        this.canvas.height !== image.height
      )
        failure ??=
          "The 2D primary did not consume the full 720x1280 ImageData.";
      else {
        primaryCanvas ??= this.canvas;
        if (
          primaryCanvas !== this.canvas ||
          (view.uploadKind && view.uploadKind !== "2d-imagedata")
        )
          failure ??= "The canonical 2D primary was replaced.";
        else {
          view.uploadKind = "2d-imagedata";
          view.source = [image.width, image.height];
          observe(
            this,
            view,
            start,
            performance.now() - start,
            ++sourceSequence,
            inputEvidence ? fingerprintInput(image.data) : undefined,
          );
        }
      }
    }
    return result;
  };
  const drawHook = function (...args) {
    const start = performance.now(),
      result = originalDraw.apply(this, args);
    if (this.canvas.classList.contains("android-screen")) {
      const view = identities.get(this.canvas),
        source = args[0],
        primary = identities.get(source);
      if (!view) failure ??= "A measured canvas was replaced.";
      else if (
        args.length !== 5 ||
        !(source instanceof HTMLCanvasElement) ||
        source === this.canvas ||
        source !== primaryCanvas ||
        !primary ||
        primary.uploadKind !== "2d-imagedata" ||
        primary.source?.[0] !== 720 ||
        primary.source?.[1] !== 1280 ||
        source.width !== 720 ||
        source.height !== 1280 ||
        !source.isConnected ||
        args[1] !== 0 ||
        args[2] !== 0 ||
        args[3] !== this.canvas.width ||
        args[4] !== this.canvas.height ||
        this.canvas.width > source.width ||
        this.canvas.height > source.height ||
        primary.drawnSequence !== sourceSequence ||
        primary.drawnSequence <= view.drawnSequence ||
        primary.drawnAt === null ||
        primary.fingerprint === null ||
        performance.now() - primary.drawnAt > 1000
      )
        failure ??=
          "Secondary 2D drawImage did not copy the fresh canonical full-resolution primary.";
      else if (
        (view.uploadKind && view.uploadKind !== "2d-canvas-copy") ||
        (view.sourceCanvas && view.sourceCanvas !== source)
      )
        failure ??= "The measured 2D secondary source was replaced.";
      else {
        view.uploadKind = "2d-canvas-copy";
        view.sourceCanvas = source;
        view.source = [source.width, source.height];
        observe(
          this,
          view,
          start,
          performance.now() - start,
          primary.drawnSequence,
          inputEvidence ? primary.fingerprint : undefined,
        );
      }
    }
    return result;
  };
  const p95 = (values) =>
    values.sort((a, b) => a - b)[Math.floor(values.length * 0.95)] ?? null;
  let after = before,
    nextState = started;
  try {
    prototype.putImageData = putHook;
    prototype.drawImage = drawHook;
    if (prototype.putImageData !== putHook || prototype.drawImage !== drawHook)
      throw Error("Could not install the 2D measurement hooks.");
    bridge?.addEventListener("sharedbufferreceived", sharedFrame);
    while (!failure && performance.now() - started < seconds * 1000) {
      await sleep(100);
      const now = performance.now();
      if (cancelled) failure = "Product benchmark cancelled.";
      if (document.visibilityState !== "visible")
        failure ??= "The product window became hidden.";
      const current = [...document.querySelectorAll(".android-screen")];
      if (
        current.length !== views ||
        current.some((canvas) => !identities.has(canvas))
      )
        failure ??= "A measured canvas was replaced.";
      for (const canvas of canvases) {
        const view = identities.get(canvas),
          bounds = canvas.getBoundingClientRect();
        if (
          !canvas.isConnected ||
          bounds.width <= 0 ||
          bounds.height <= 0 ||
          getComputedStyle(canvas).visibility !== "visible"
        )
          failure ??= "A measured canvas was hidden or detached.";
        if (
          Math.abs(bounds.width - view.css[0]) > 0.5 ||
          Math.abs(bounds.height - view.css[1]) > 0.5
        )
          failure ??= "A measured view's CSS dimensions changed.";
        if (
          canvas.width !== view.framebuffer[0] ||
          canvas.height !== view.framebuffer[1] ||
          devicePixelRatio !== view.dpr
        )
          failure ??= "A measured framebuffer or DPR changed.";
        if (now - (view.last ?? started) > 5000)
          failure ??= "A measured view received no frames for five seconds.";
        view.maxUnchangedMs = Math.max(
          view.maxUnchangedMs,
          now - (view.lastChange ?? started),
        );
        if (view.maxUnchangedMs > 5000)
          failure ??=
            "A measured view had no changing frame fingerprint for five seconds.";
      }
      if (now >= nextState) {
        const streams = (await invoke("android_state")).streams.filter(
          (stream) => stream.phase === "streaming",
        );
        after = streams.find((stream) => stream.deviceId === deviceId);
        if (
          streams.length !== 1 ||
          !after ||
          after.generation !== before.generation ||
          after.epoch !== before.epoch
        )
          failure ??= "The production stream generation or epoch was replaced.";
        nextState = now + 1000;
      }
    }
    const final = (await invoke("android_state")).streams.filter(
      (stream) => stream.phase === "streaming",
    );
    after = final.find((stream) => stream.deviceId === deviceId);
    if (
      final.length !== 1 ||
      !after ||
      after.epoch !== before.epoch ||
      after.generation !== before.generation
    )
      failure ??= "The measured stream stopped or was replaced.";
    const elapsed = (performance.now() - started) / 1000;
    const primaryFrames = identities.get(primaryCanvas)?.frames;
    if (
      !primaryFrames ||
      canvases.some((canvas) => identities.get(canvas).frames !== primaryFrames)
    )
      failure ??= "The 2D views did not draw the same canonical frame count.";
    const perView = canvases.map((canvas) => {
      const view = identities.get(canvas);
      if (!view.frames || !view.source)
        failure ??= "A measured view produced no uploaded frames.";
      const minimumTransitions = Math.max(3, Math.ceil(seconds));
      if (view.fingerprintTransitions < minimumTransitions)
        failure ??= "A measured view had too few changing frame fingerprints.";
      return {
        paneId: view.paneId,
        source: view.source,
        framebuffer: view.framebuffer,
        css: view.css,
        dpr: view.dpr,
        frames: view.frames,
        fps: view.frames / elapsed,
        drawCallP95Ms: p95(view.draws),
        frameGapP95Ms: p95(view.gaps),
        uploadKind: view.uploadKind,
        contextAttributes: view.contextAttributes,
        sourcePaneId: view.sourceCanvas
          ? identities.get(view.sourceCanvas).paneId
          : view.paneId,
        fingerprintSamplePixels: 64,
        fingerprintProvenance: !inputEvidence
          ? "actual-2d-framebuffer-readback"
          : view.sourceCanvas
            ? "canonical-primary-input-inherited-after-successful-drawImage"
            : "ImageData.data-after-successful-putImageData",
        secondaryBitmapVerified: !inputEvidence && !!view.sourceCanvas,
        fingerprintedFrames: view.fingerprintedFrames,
        fingerprintTransitions: view.fingerprintTransitions,
        minimumFingerprintTransitions: minimumTransitions,
        maxUnchangedFingerprintMs: view.maxUnchangedMs,
        lastDrawnSourceSequence: view.drawnSequence,
      };
    });
    const counters = {};
    for (const key of ["ipcFrames", "ipcBytes", "grpcFrames", "grpcBytes"]) {
      counters[key] =
        after &&
        after.epoch === before.epoch &&
        after.generation === before.generation
          ? after[key] - before[key]
          : null;
      if (!Number.isFinite(counters[key]) || counters[key] <= 0)
        failure ??= "Production stream counters did not advance.";
    }
    if (
      requireShared &&
      (!sharedBufferEvents ||
        Math.abs(
          sharedBufferEvents - (identities.get(primaryCanvas)?.frames ?? 0),
        ) > 2 ||
        Math.abs(sharedBufferEvents - counters.ipcFrames) > 2)
    )
      failure ??=
        "Shared-buffer deliveries did not match production frames (two-frame boundary tolerance).";
    return (result = {
      qualification: false,
      rendererSoftware: true,
      rendererKind: "linux-canvas-2d",
      softwareBackingInferenceOnly: true,
      fingerprintMethod: inputEvidence
        ? "64 RGB ImageData input samples after successful putImageData; secondary inherits canonical primary input after successful drawImage (no secondary bitmap claim)"
        : "64 actual one-pixel 2D getImageData reads per drawn framebuffer",
      measuredPhaseReadback: !inputEvidence,
      completed: !failure,
      failure,
      startedAt: utcStarted,
      endedAt: new Date().toISOString(),
      wallElapsed: elapsed,
      deviceId,
      generation: before.generation,
      epoch: before.epoch,
      counters,
      sharedBufferEvents,
      requiredTransport: requireShared ? "shared-buffer" : "any",
      views: perView,
    });
  } finally {
    try {
      bridge?.removeEventListener("sharedbufferreceived", sharedFrame);
    } finally {
      const ownsDraw = prototype.drawImage === drawHook;
      const ownsPut = prototype.putImageData === putHook;
      const ownsBenchmark = window.__androidProductBenchmark === owner;
      if (ownsDraw) prototype.drawImage = originalDraw;
      if (ownsPut) prototype.putImageData = originalPut;
      if (ownsBenchmark) delete window.__androidProductBenchmark;
      if (!ownsDraw || !ownsPut || !ownsBenchmark) {
        failure ??= "Measurement hook or benchmark ownership changed.";
        if (result) {
          result.completed = false;
          result.failure = failure;
        }
      }
    }
  }
}

const tab = JSON.stringify(activeTab);
const show = `click(${tab});await wait(()=>document.querySelectorAll('.android-screen').length===${views});await sleep(2000);return true;`;
const hide =
  "click('Terminal');await sleep(1000);const state=await invoke('android_state');if(state.streams.some(stream=>stream.phase==='streaming'))throw Error('Hidden panels retained a production stream');return true;";
async function hiddenPhase(label, seconds) {
  await phase(label, "start");
  const result = await control(
    `return await (${measureHidden.toString()})(invoke,sleep,${seconds});`,
    (seconds + 20) * 1000,
  );
  await phase(label, "end", result);
}
let frames;
try {
  await assertFixture();
  await preflightAdb();
  await control(show);
  await control(
    "click('Android actions');await sleep(150);click('Phone settings');await sleep(1000);return true;",
  );
  settingsHomepage = resolvedSettingsComponent(
    await guestAction("resolve-settings"),
  );
  report.settingsHomepage = settingsHomepage;
  const reset = await guestAction("reset-settings");
  if (!/^Status:\s*ok\s*$/m.test(reset))
    throw Error("The owned Settings homepage task did not reset successfully.");
  await settingsResumed("after-settings-task-reset");
  await phase("workload", "start", {
    description:
      "Phone settings opened through actual UI and its resolved homepage task reset; alternating fixed one-second guest swipes",
  });
  animation = (async () => {
    try {
      while (!cancellation.signal.aborted) {
        await guestAction("swipe", report.swipeCount);
        report.swipeCount++;
      }
    } catch (error) {
      if (!cancellation.signal.aborted) {
        animationError = error;
        stop();
      }
    }
  })();
  const preflight = await control(
    `return await (${measureVisible.toString()})(invoke,sleep,${JSON.stringify({ deviceId, generation: record.generation, seconds: 3, views, name, requireShared: transportArgument === "shared" })});`,
    33000,
  );
  report.changingFramePreflight = preflight;
  await phase("workload", "changing-frame-preflight", preflight);
  if (!preflight.completed) throw Error(preflight.failure);
  await settingsResumed("after-changing-frame-preflight");
  await control(hide);
  await hiddenPhase("warmup", 30);
  await hiddenPhase("baseline", 60);
  await assertFixture();
  await control(show);
  await settingsResumed("before-visible-measurement");
  await phase("visible", "start");
  frames = await control(
    `return await (${measureVisible.toString()})(invoke,sleep,${JSON.stringify({ deviceId, generation: record.generation, seconds, views, name, requireShared: transportArgument === "shared", inputEvidence: true })});`,
    (seconds + 30) * 1000,
  );
  await writeFile(
    join(root, name + "-frames.json"),
    JSON.stringify(frames, null, 2),
    { flag: "wx" },
  );
  await phase("visible", "end", {
    completed: frames.completed,
    failure: frames.failure,
  });
  await settingsResumed("after-visible-measurement");
  if (!frames.completed) throw Error(frames.failure);
  await control(hide);
  await hiddenPhase("hidden-after", 60);
  await assertFixture();
  if (digest(await readFile(binary)) !== application.binarySha256)
    throw Error("The fixture executable changed during measurement.");
  if (animationError) throw animationError;
  report.completed = true;
} catch (error) {
  report.error = (animationError ?? error).message.replaceAll(
    record.generationKey,
    "[redacted]",
  );
  await phase("driver", "error", { error: report.error });
} finally {
  stop();
  if (animation) await animation;
  report.animationStopped = true;
  try {
    // The watcher can evaluate this cancellation while the async benchmark is
    // pending. Wait for hook restoration before restoring the requested tab.
    await control(
      `const owner=window.__androidProductBenchmark;if(owner?.name===${JSON.stringify(name)})owner.cancel();await wait(()=>window.__androidProductBenchmark?.name!==${JSON.stringify(name)},10000);${show}`,
      15000,
      true,
    );
    report.activeTabRestored = true;
  } catch (error) {
    report.activeTabRestored = false;
    report.cleanupError = error.message.replaceAll(
      record.generationKey,
      "[redacted]",
    );
    report.completed = false;
  }
  report.endedAt = new Date().toISOString();
  await phase("driver", "end", {
    completed: report.completed,
    swipeCount: report.swipeCount,
  });
  await writeFile(driverPath, JSON.stringify(report, null, 2));
  await journal.close();
  process.removeListener("SIGINT", stop);
  process.removeListener("SIGTERM", stop);
}
if (!report.completed)
  throw Error(
    report.error ?? report.cleanupError ?? "Visible measurement failed.",
  );
console.log(
  JSON.stringify({
    completed: true,
    driver: driverPath,
    frames: join(root, name + "-frames.json"),
    phases: join(root, name + "-phases.jsonl"),
  }),
);
