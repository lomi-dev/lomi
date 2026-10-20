// Opt-in destructive crash test of the isolated Linux debug fixture only.
// Run AFTER resource/GPU measurements, as its ordinary owner in the GUI session:
// node tests/native/android-product-linux-recovery.mjs PRODUCT DEVICE_UUID UNIQUE
// The original app alone receives SIGKILL. Recovery uses production Stop/Start.
// Success leaves the replacement app/guest and the private ADB server running.
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { homedir, userInfo } from "node:os";
import {
  open,
  readFile,
  writeFile,
  realpath,
  readlink,
  readdir,
  stat,
  lstat,
} from "node:fs/promises";
import { basename, dirname, isAbsolute, join, resolve, sep } from "node:path";

if (
  process.platform !== "linux" ||
  process.arch !== "x64" ||
  process.getuid() === 0 ||
  process.geteuid() === 0
)
  throw Error(
    "Run only as the ordinary Linux x64 fixture owner; never with sudo.",
  );
if (Number(process.versions.node.split(".")[0]) < 24)
  throw Error("Use the fixture owner's existing Node 24+ environment.");
const [argument, deviceId, name] = process.argv.slice(2);
const uuid = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/;
if (
  !argument ||
  !uuid.test(deviceId ?? "") ||
  !/^[A-Za-z0-9-]{1,40}$/.test(name ?? "")
)
  throw Error(
    "Use PRODUCT DEVICE_UUID FRESH_NAME (1..40 letters, digits or hyphens).",
  );
const uid = process.getuid();
const repository = await realpath(resolve(import.meta.dirname, "../.."));
async function privateDirectory(argument) {
  const absolute = resolve(argument);
  for (let path = absolute; ; path = dirname(path)) {
    if ((await lstat(path)).isSymbolicLink())
      throw Error("A fixture path contains a symlink.");
    if (path === dirname(path)) break;
  }
  const info = await stat(absolute);
  if (!info.isDirectory() || info.uid !== uid)
    throw Error("Fixture directory has another owner or is missing.");
  return absolute;
}
const root = await privateDirectory(argument);
const trial = await privateDirectory(dirname(root));
if (
  basename(root) !== "product" ||
  !basename(trial).startsWith("lomi-android-stage0-") ||
  root === repository ||
  root.startsWith(repository + sep)
)
  throw Error("Use only an isolated product trial outside the repository.");
if ((await readdir(root)).some((file) => file.startsWith(name + "-")))
  throw Error("Preserve previous evidence; use a fresh recovery name.");
async function bytes(path, limit = 65536) {
  const file = await open(path, "r");
  try {
    const info = await file.stat();
    if (
      !info.isFile() ||
      info.uid !== uid ||
      info.size > limit ||
      (await lstat(path)).isSymbolicLink()
    )
      throw Error("Fixture record is not a bounded caller-owned regular file.");
    const data = await file.readFile();
    if (data.length > limit) throw Error("Fixture record exceeds its bound.");
    return data;
  } finally {
    await file.close();
  }
}
function fixtureJson(value) {
  try {
    return JSON.parse(value);
  } catch {
    throw Error("Invalid bounded fixture JSON.");
  }
}
const receiptPath = join(root, "application.json");
const receiptBytes = await bytes(receiptPath);
const original = fixtureJson(receiptBytes);
const managed = await privateDirectory(original.managed);
const binary = join(repository, "src-tauri/target/debug/lomi");
const digest = (data) => createHash("sha256").update(data).digest("hex");
function validReceipt(value) {
  return (
    value.repository === repository &&
    value.managed === managed &&
    value.buildProfile === "debug" &&
    value.platform === "linux" &&
    value.architecture === "x64" &&
    /^dev\.lomi\.android-product-\d+$/.test(value.identifier) &&
    /^[a-f0-9]{64}$/.test(value.binarySha256) &&
    Number.isSafeInteger(value.pid) &&
    value.pid > 0
  );
}
if (
  dirname(managed) !== trial ||
  !basename(managed).startsWith("native-managed-") ||
  !validReceipt(original)
)
  throw Error("Receipt does not identify this isolated Linux debug fixture.");
if (digest(await readFile(binary)) !== original.binarySha256)
  throw Error("Receipt executable SHA256 mismatch.");
for (const file of ["consent.json", "java-consent.json"])
  if (fixtureJson(await bytes(join(trial, "evidence", file))).accepted !== true)
    throw Error("Explicit isolated SDK/private Java consent is required.");
const boot = (await readFile("/proc/sys/kernel/random/boot_id", "utf8")).trim();
const hzQuery = spawnSync("getconf", ["CLK_TCK"], {
  encoding: "utf8",
  timeout: 10000,
  maxBuffer: 4096,
});
const hz = Number(hzQuery.stdout?.trim());
if (hzQuery.status !== 0 || !Number.isSafeInteger(hz) || hz <= 0)
  throw Error("Cannot determine native clock ticks.");
const btime = /^btime (\d+)$/m.exec(await readFile("/proc/stat", "utf8"));
if (!btime) throw Error("Cannot determine the native boot time.");
function procStat(text) {
  const split = text.lastIndexOf(") ");
  const fields = text
    .slice(split + 2)
    .trim()
    .split(/\s+/);
  if (
    split < 0 ||
    fields.length < 20 ||
    !/^\d+$/.test(fields[19]) ||
    !/^\d+$/.test(fields[1])
  )
    throw Error("Malformed native process stat.");
  return {
    created: fields[19],
    parentPid: Number(fields[1]),
    state: fields[0],
  };
}
async function identity(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 0)
    throw Error("Invalid native PID.");
  const directory = `/proc/${pid}`;
  const before = procStat(await readFile(join(directory, "stat"), "utf8"));
  const executable = await readlink(join(directory, "exe"));
  const file = await stat(join(directory, "exe"), { bigint: true });
  const owner = await stat(directory);
  const after = procStat(await readFile(join(directory, "stat"), "utf8"));
  const finalFile = await stat(join(directory, "exe"), { bigint: true });
  const key = (value) =>
    `${value.dev}:${value.ino}:${value.size}:${value.mtimeNs}`;
  if (
    before.created !== after.created ||
    ["Z", "X"].includes(after.state) ||
    owner.uid !== uid ||
    executable !== (await readlink(join(directory, "exe"))) ||
    key(file) !== key(finalFile) ||
    (await readFile("/proc/sys/kernel/random/boot_id", "utf8")).trim() !== boot
  )
    throw Error("Native process identity changed or is unavailable.");
  return {
    pid,
    created: before.created,
    parentPid: after.parentPid,
    boot,
    executable,
    file: key(file),
  };
}
function same(left, right) {
  return ["pid", "created", "boot", "executable", "file"].every(
    (field) => left[field] === right[field],
  );
}
async function assertIdentity(pin) {
  if (!same(pin, await identity(pin.pid)))
    throw Error("A pinned native process changed identity.");
}
async function exited(pin) {
  try {
    const info = procStat(await readFile(`/proc/${pin.pid}/stat`, "utf8"));
    return info.created !== pin.created || ["Z", "X"].includes(info.state);
  } catch (error) {
    if (error.code === "ENOENT" || error.code === "ESRCH") return true;
    throw error;
  }
}
// pidfd pins the kernel process object before revalidation; never fall back to PID kill.
const crashSource = String.raw`
import json, os, signal, sys
fd = None
try:
    request = json.loads(sys.stdin.buffer.read(8193))
    pin, uid = request["pin"], request["uid"]
    if not isinstance(uid, int) or uid <= 0 or os.getuid() != uid or os.geteuid() != uid:
        raise ValueError("owner")
    pid = pin["pid"]
    if not isinstance(pid, int) or isinstance(pid, bool) or pid <= 0:
        raise ValueError("pid")
    if not hasattr(os, "pidfd_open") or not hasattr(signal, "pidfd_send_signal"):
        raise ValueError("pidfd unavailable")
    fd = os.pidfd_open(pid, 0)
    directory = "/proc/" + str(pid)
    def identity():
        with open(directory + "/stat") as handle:
            text = handle.read(65536)
        fields = text[text.rindex(") ") + 2:].split()
        if fields[0] in ("Z", "X") or os.stat(directory).st_uid != uid:
            raise ValueError("owner or state")
        executable = os.readlink(directory + "/exe")
        info = os.stat(directory + "/exe")
        with open("/proc/sys/kernel/random/boot_id") as handle:
            boot = handle.read(128).strip()
        return {"pid": pid, "created": fields[19], "boot": boot,
                "executable": executable,
                "file": f"{info.st_dev}:{info.st_ino}:{info.st_size}:{info.st_mtime_ns}"}
    expected = {key: pin[key] for key in ("pid", "created", "boot", "executable", "file")}
    if identity() != expected or identity() != expected:
        raise ValueError("native identity")
    signal.pidfd_send_signal(fd, signal.SIGKILL, None, 0)
    print(json.dumps({"method": "pidfd_send_signal", "signal": "SIGKILL",
                      "identity": expected, "signalSent": True}))
except Exception:
    print(json.dumps({"signalSent": False, "error": "pidfd crash guard refused or failed"}))
    sys.exit(1)
finally:
    if fd is not None:
        os.close(fd)
`;
async function crashPinnedApp(pin) {
  return await new Promise((resolveResult, reject) => {
    const helper = spawn("python3", ["-c", crashSource], {
      stdio: ["pipe", "pipe", "pipe"],
    });
    let output = "",
      settled = false;
    const timer = setTimeout(
      () => finish(Error("Pinned crash helper timed out.")),
      10000,
    );
    function finish(error, result) {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (error) {
        helper.kill();
        reject(error);
      } else resolveResult(result);
    }
    helper.stdout.on("data", (chunk) => {
      output += chunk;
      if (output.length > 8192)
        finish(Error("Pinned crash receipt exceeds its bound."));
    });
    helper.stderr.resume();
    helper.once("error", () =>
      finish(Error("Cannot launch pidfd crash helper.")),
    );
    helper.stdin.once("error", () =>
      finish(Error("Cannot send pinned crash identity.")),
    );
    helper.once("close", (code) => {
      try {
        const result = JSON.parse(output);
        if (
          code !== 0 ||
          result.signalSent !== true ||
          result.method !== "pidfd_send_signal" ||
          result.signal !== "SIGKILL" ||
          !same(pin, result.identity)
        )
          throw Error();
        finish(null, result);
      } catch {
        finish(
          Error("Pinned crash helper refused or returned an invalid receipt."),
        );
      }
    });
    helper.stdin.end(JSON.stringify({ uid, pin }));
  });
}
function launchMatches(pin, receipt) {
  const launched = Date.parse(receipt.startedAt);
  return (
    pin.executable === binary &&
    Number.isFinite(launched) &&
    Math.abs(
      Number(btime[1]) * 1000 + (Number(pin.created) / hz) * 1000 - launched,
    ) <= 10000
  );
}
const app = await identity(original.pid);
if (!launchMatches(app, original))
  throw Error("Application native identity does not match its receipt.");
const runtimePath = join(managed, "runtime", deviceId + ".json");
const runtimeBytes = await bytes(runtimePath);
const runtime = fixtureJson(runtimeBytes);
const guest = await identity(runtime.process?.pid);
function validRuntime(value, pin) {
  return (
    value.deviceId === deviceId &&
    uuid.test(value.generation) &&
    uuid.test(value.generationKey) &&
    value.launching !== true &&
    value.adbPort === 15047 &&
    Number.isInteger(value.consolePort) &&
    value.consolePort >= 5554 &&
    value.consolePort <= 5682 &&
    value.consolePort % 2 === 0 &&
    pin.created === String(value.process?.created) &&
    pin.boot === value.process?.boot &&
    pin.executable === value.process?.executable &&
    pin.executable.startsWith(join(managed, "sdk/emulator") + sep) &&
    basename(pin.executable).startsWith("qemu-system-")
  );
}
if (!validRuntime(runtime, guest))
  throw Error("Runtime record does not pin the private managed QEMU.");
const secretIds = new Set([runtime.generationKey]);
function safeError(error) {
  let message =
    error instanceof SyntaxError
      ? "Invalid fixture JSON"
      : String(error.message ?? error);
  for (const secret of secretIds)
    message = message.replaceAll(secret, "[redacted]");
  return message
    .replace(
      /\b[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{10,}\b/g,
      "[redacted-token]",
    )
    .slice(0, 4096);
}
async function descendant(pin, ancestor) {
  let current = pin;
  const chain = [pin];
  for (let depth = 0; depth < 32; depth++) {
    if (same(current, ancestor)) {
      for (const member of chain) await assertIdentity(member);
      return chain;
    }
    if (
      current.parentPid <= 1 ||
      chain.some((item) => item.pid === current.parentPid)
    )
      break;
    const parent = await identity(current.parentPid);
    if (BigInt(parent.created) > BigInt(current.created))
      throw Error("Descendant creation ordering changed.");
    chain.push(parent);
    current = parent;
  }
  throw Error(
    "Managed process is not a verified descendant of the fixture app.",
  );
}
const discoveryExclusions = new Map();
const discoveryPins = new Set([app.pid, guest.pid]);
async function protectedSystemProcess(pid) {
  if (discoveryPins.has(pid)) return false;
  const proofs = [];
  const systemFiles = new Map();
  async function bounded(path, limit, firstArgument = false) {
    const file = await open(path, "r");
    try {
      const data = Buffer.alloc(limit + 1);
      const { bytesRead } = await file.read(data, 0, data.length, 0);
      if (firstArgument) {
        const end = data.subarray(0, bytesRead).indexOf(0);
        if (end < 0 || end > limit)
          throw Error("Protected process argument zero is unavailable.");
        return data.subarray(0, end);
      }
      if (bytesRead > limit)
        throw Error("Protected process metadata exceeds its bound.");
      return data.subarray(0, bytesRead);
    } finally {
      await file.close();
    }
  }
  async function inspect(currentPid) {
    const path = `/proc/${currentPid}`;
    const native = procStat(
      (await bounded(join(path, "stat"), 4096)).toString(),
    );
    const owner = (await stat(path)).uid;
    const value = {
      pid: currentPid,
      created: native.created,
      parentPid: native.parentPid,
      uid: owner,
      arg0: (await bounded(join(path, "cmdline"), 256, true)).toString("utf8"),
      cgroup: (await bounded(join(path, "cgroup"), 4096)).toString().trim(),
    };
    if (
      ["Z", "X"].includes(native.state) ||
      BigInt(value.created) >= BigInt(app.created)
    )
      throw Error("Protected process must predate the pinned application.");
    return value;
  }
  async function pin(currentPid) {
    const value = await inspect(currentPid);
    proofs.push(value);
    return value;
  }
  async function systemFile(path) {
    const info = await stat(path);
    if (
      (await realpath(path)) !== path ||
      !info.isFile() ||
      info.uid !== 0 ||
      info.mode & 0o022
    )
      throw Error(
        "Protected service binary is not a canonical root-owned system file.",
      );
    systemFiles.set(
      path,
      `${info.dev}:${info.ino}:${info.size}:${info.mtimeMs}`,
    );
  }
  const initScope = `0::/user.slice/user-${uid}.slice/user@${uid}.service/init.scope`;
  async function manager(value) {
    if (
      value.uid !== uid ||
      value.parentPid !== 1 ||
      value.arg0 !== "/usr/lib/systemd/systemd" ||
      value.cgroup !== initScope
    )
      return false;
    const init = await pin(1);
    await systemFile("/usr/lib/systemd/systemd");
    return (
      init.uid === 0 &&
      init.parentPid === 0 &&
      init.arg0 === "/usr/lib/systemd/systemd" &&
      init.cgroup === "0::/init.scope" &&
      BigInt(init.created) < BigInt(value.created)
    );
  }
  try {
    const value = await pin(pid);
    if (value.uid !== uid) return false;
    let role;
    if (await manager(value)) role = "systemd-user-manager";
    else if (value.arg0 === "(sd-pam)" || value.arg0 === "/usr/bin/ssh-agent") {
      const parent = await pin(value.parentPid);
      if (
        !(await manager(parent)) ||
        BigInt(parent.created) > BigInt(value.created)
      )
        return false;
      if (value.arg0 === "(sd-pam)" && value.cgroup === initScope)
        role = "systemd-pam-helper";
      if (
        value.arg0 === "/usr/bin/ssh-agent" &&
        value.cgroup ===
          `0::/user.slice/user-${uid}.slice/user@${uid}.service/app.slice/ssh-agent.service`
      ) {
        await systemFile("/usr/bin/ssh-agent");
        role = "openssh-agent-service";
      }
    } else {
      const login = `sshd-session: ${userInfo().username}`;
      if (
        !value.arg0.startsWith(login + "@") ||
        !/^(notty|pts\/\d+)$/.test(value.arg0.slice(login.length + 1)) ||
        !new RegExp(
          `^0::/user\\.slice/user-${uid}\\.slice/session-\\d+\\.scope$`,
        ).test(value.cgroup)
      )
        return false;
      const parent = await pin(value.parentPid);
      const listener = await pin(parent.parentPid);
      if (
        parent.uid !== 0 ||
        parent.arg0 !== login + " [priv]" ||
        parent.cgroup !== value.cgroup ||
        listener.uid !== 0 ||
        listener.parentPid !== 1 ||
        listener.cgroup !== "0::/system.slice/ssh.service" ||
        !/^sshd: \/usr\/sbin\/sshd -D \[listener\](?: \d+ of \d+-\d+ startups)?$/.test(
          listener.arg0,
        ) ||
        BigInt(listener.created) > BigInt(parent.created) ||
        BigInt(parent.created) > BigInt(value.created)
      )
        return false;
      const init = await pin(1);
      if (
        init.uid !== 0 ||
        init.parentPid !== 0 ||
        init.arg0 !== "/usr/lib/systemd/systemd" ||
        init.cgroup !== "0::/init.scope" ||
        BigInt(init.created) >= BigInt(listener.created)
      )
        return false;
      for (const path of [
        "/usr/lib/systemd/systemd",
        "/usr/lib/openssh/sshd-session",
        "/usr/sbin/sshd",
      ])
        await systemFile(path);
      role = "openssh-user-session";
    }
    if (!role) return false;
    for (const proof of proofs) {
      if (JSON.stringify(proof) !== JSON.stringify(await inspect(proof.pid)))
        return false;
    }
    for (const [path, key] of systemFiles) {
      await systemFile(path);
      if (systemFiles.get(path) !== key) return false;
    }
    if (
      (await readFile("/proc/sys/kernel/random/boot_id", "utf8")).trim() !==
      boot
    )
      return false;
    discoveryExclusions.set(`${pid}:${value.created}`, {
      pid,
      created: value.created,
      parentPid: value.parentPid,
      boot,
      role,
      proof:
        "bounded argv0, service cgroup, root-owned system files and stable native ancestry; exe unreadable",
      ancestors: proofs.slice(1).map(({ pid, created, parentPid, uid }) => ({
        pid,
        created,
        parentPid,
        uid,
      })),
    });
    return true;
  } catch {
    return false;
  }
}
async function processesMatching(predicate) {
  const found = [];
  for (const entry of await readdir("/proc", { withFileTypes: true })) {
    if (!entry.isDirectory() || !/^\d+$/.test(entry.name)) continue;
    const directory = `/proc/${entry.name}`;
    try {
      if ((await stat(directory)).uid !== uid) continue;
      const executable = await readlink(join(directory, "exe"));
      if (predicate(executable)) found.push(await identity(Number(entry.name)));
    } catch (error) {
      // Non-dumpable OS helpers can deny /proc/exe even to their owner.
      // Unknown live denials and every pinned app/QEMU/ADB denial remain fatal.
      if (error.code === "EACCES") {
        const pid = Number(entry.name);
        if (await protectedSystemProcess(pid)) continue;
        if (!discoveryPins.has(pid)) {
          try {
            await lstat(directory);
          } catch (currentError) {
            // Classification can race a short-lived process being reaped.
            // Only confirmed disappearance permits skipping an unknown denial.
            if (currentError.code === "ENOENT") continue;
          }
        }
      }
      if (error.code !== "ENOENT" && error.code !== "ESRCH") {
        const pid = Number(entry.name);
        const safePid =
          Number.isSafeInteger(pid) && pid > 0 && pid <= 2147483647
            ? String(pid)
            : "unknown";
        const code =
          typeof error.code === "string" &&
          /^[A-Z][A-Z0-9_]{0,31}$/.test(error.code)
            ? error.code
            : "UNKNOWN";
        throw Error(
          `Caller-owned process discovery is unavailable (pid=${safePid}, code=${code}).`,
        );
      }
    }
  }
  return found;
}
const engine = join(managed, "sdk/emulator") + sep;
async function onlyGuest(expected) {
  const found = await processesMatching(
    (file) =>
      file.startsWith(engine) &&
      (basename(file).startsWith("qemu-system-") ||
        basename(file) === "emulator"),
  );
  if (found.length !== 1 || !same(found[0], expected))
    throw Error(
      "Expected exactly the pinned managed QEMU; duplicate or unavailable identity.",
    );
}
const adbExecutable = await realpath(join(managed, "sdk/platform-tools/adb"));
if (adbExecutable !== join(managed, "sdk/platform-tools/adb"))
  throw Error("Private ADB executable is linked elsewhere.");
const adbs = await processesMatching((file) => file === adbExecutable);
if (adbs.length !== 1)
  throw Error("Require exactly one caller-owned managed private ADB process.");
const adb = adbs[0];
discoveryPins.add(adb.pid);
const adbChain = await descendant(adb, app);
await descendant(guest, app);
async function privateListener(pin) {
  await assertIdentity(pin);
  const sockets = new Set();
  for (const fd of await readdir(`/proc/${pin.pid}/fd`)) {
    try {
      const match = /^socket:\[(\d+)\]$/.exec(
        await readlink(`/proc/${pin.pid}/fd/${fd}`),
      );
      if (match) sockets.add(match[1]);
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
  }
  const listeners = [];
  for (const table of ["tcp", "tcp6"]) {
    const text = await readFile(`/proc/net/${table}`, "utf8");
    for (const row of text.trim().split("\n").slice(1)) {
      const fields = row.trim().split(/\s+/);
      const [address, port] = fields[1].split(":");
      if (parseInt(port, 16) !== 15047 || fields[3] !== "0A") continue;
      const loopback =
        table === "tcp"
          ? address === "0100007F"
          : address === "00000000000000000000000001000000";
      if (!loopback || Number(fields[7]) !== uid || !sockets.has(fields[9]))
        throw Error(
          "Port 15047 is not the pinned private ADB loopback listener.",
        );
      listeners.push({
        table,
        port: 15047,
        inode: fields[9],
        ownerPid: pin.pid,
      });
    }
  }
  if (!listeners.length)
    throw Error(
      "The pinned private ADB has no verified LISTEN socket on 15047.",
    );
  await assertIdentity(pin);
  return listeners;
}
await onlyGuest(guest);
const listeners = await privateListener(adb);
const originals = await processesMatching((file) => file === binary);
if (originals.length !== 1 || !same(originals[0], app))
  throw Error("Another fixture executable is already running.");
const reportPath = join(root, name + "-recovery.json");
const journal = await open(join(root, name + "-recovery.jsonl"), "wx", 0o600);
const report = {
  completed: false,
  qualification: false,
  deviceId,
  originalGeneration: runtime.generation,
  binarySha256: original.binarySha256,
  driverSourceSha256: digest(await readFile(new URL(import.meta.url))),
  originalApp: app,
  originalGuest: guest,
  privateAdb: adb,
  privateAdbAncestors: adbChain,
  privateListener: listeners,
  discoveryExclusions: [...discoveryExclusions.values()],
  startedAt: new Date().toISOString(),
  phases: [],
};
await writeFile(reportPath, JSON.stringify(report, null, 2), {
  flag: "wx",
  mode: 0o600,
});
await writeFile(join(root, name + "-original-application.json"), receiptBytes, {
  flag: "wx",
  mode: 0o600,
});
let index = 0;
async function phase(event, data = {}) {
  const entry = { event, at: new Date().toISOString(), ...data };
  report.phases.push(entry);
  await journal.write(JSON.stringify(entry) + "\n");
  console.log(JSON.stringify(entry));
}
async function poll(operation, timeoutMs, description) {
  const deadline = performance.now() + timeoutMs;
  do {
    const value = await operation();
    if (value) return value;
    await new Promise((resolve) => setTimeout(resolve, 200));
  } while (performance.now() < deadline);
  throw Error(
    description + " timed out; owned processes are retained for inspection.",
  );
}
async function control(pin, stage, script, timeoutMs = 30000, action) {
  await assertIdentity(pin);
  const id = `${name}-${String(++index).padStart(3, "0")}-${stage}`;
  return await new Promise((resolveResult, reject) => {
    const helper = spawn(
      process.execPath,
      [join(import.meta.dirname, "android-product-control.mjs"), root, id],
      { cwd: repository, stdio: ["pipe", "pipe", "pipe"] },
    );
    let output = "",
      settled = false;
    const timer = setTimeout(
      () => finish(Error("Process-scoped control timed out at " + stage)),
      timeoutMs + 5000,
    );
    function finish(error, data) {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (error) {
        helper.kill();
        reject(error);
      } else resolveResult(data);
    }
    helper.stdout.on("data", (chunk) => {
      output += chunk;
      if (output.length > 4 * 1024 * 1024)
        finish(Error("Control evidence exceeds its bound."));
    });
    helper.stderr.resume(); // Never echo arbitrary native diagnostics/auth material.
    helper.once("error", () =>
      finish(Error("Cannot launch the process-scoped controller.")),
    );
    helper.stdin.once("error", () =>
      finish(Error("Controller instruction input failed.")),
    );
    helper.once("close", (code) => {
      if (code !== 0)
        return finish(
          Error(
            "Native control failed at " +
              stage +
              "; inspect its fresh process-scoped result.",
          ),
        );
      try {
        const value = JSON.parse(output);
        if (!value.ok) throw Error();
        finish(null, value.data);
      } catch {
        finish(Error("Invalid/rejected process-scoped result at " + stage));
      }
    });
    helper.stdin.end(
      JSON.stringify({
        processId: pin.pid,
        window: "main",
        timeoutMs,
        ...(action
          ? {
              action,
              ...(action === "resize" ? { width: 1400, height: 950 } : {}),
            }
          : { script }),
      }),
    );
  });
}
const statusScript = `const state=await invoke('android_state');return {statuses:state.statuses.map(({deviceId,generation,phase,processAlive,serial,display})=>({deviceId,generation,phase,processAlive,serial,display})),streams:state.streams.map(({deviceId,generation,epoch,phase,grpcFrames,grpcBytes,ipcFrames,ipcBytes})=>({deviceId,generation,epoch,phase,grpcFrames,grpcBytes,ipcFrames,ipcBytes})),device:state.devices.devices.find(item=>item.id===${JSON.stringify(deviceId)})};`;
async function guiEnvironment() {
  const result = spawnSync("systemctl", ["--user", "show-environment"], {
    encoding: "utf8",
    timeout: 10000,
    maxBuffer: 1024 * 1024,
  });
  if (result.status !== 0)
    throw Error("Cannot read the actual user-manager GUI environment.");
  const keys = new Set([
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
    "XDG_CURRENT_DESKTOP",
    "GDK_BACKEND",
    "GDK_SCALE",
    "GDK_DPI_SCALE",
    "GTK_USE_PORTAL",
    "DESKTOP_SESSION",
  ]);
  const env = { ...process.env };
  for (const key of keys) delete env[key];
  const present = [];
  for (const row of result.stdout.split("\n")) {
    const split = row.indexOf("=");
    if (split < 1) continue;
    const key = row.slice(0, split),
      value = row.slice(split + 1);
    if (keys.has(key) && value && !value.includes("\0")) {
      env[key] = value;
      present.push(key);
    }
  }
  if (
    (!env.WAYLAND_DISPLAY && !env.DISPLAY) ||
    !env.DBUS_SESSION_BUS_ADDRESS ||
    !isAbsolute(env.XDG_RUNTIME_DIR ?? "") ||
    (await stat(env.XDG_RUNTIME_DIR)).uid !== uid
  )
    throw Error("Actual user-manager GUI environment is incomplete.");
  delete env.LOMI_ANDROID_PROBE_DIRECTORY;
  env.LOMI_ANDROID_PRODUCT_DIRECTORY = managed;
  const dataHome =
    env.XDG_DATA_HOME && isAbsolute(env.XDG_DATA_HOME)
      ? env.XDG_DATA_HOME
      : join(homedir(), ".local/share");
  if (join(dataHome, original.identifier) !== original.appData)
    throw Error(
      "Caller environment would reopen a different fixture app-data directory.",
    );
  return { env, keys: present };
}
// Detached wrapper keeps the long-lived runner supervised after this driver ends.
// Arbitrary child stdout/stderr is suppressed, not dumped to a secret-bearing log.
const wrapperSource = String.raw`
import {spawn} from 'node:child_process';
const [runner,managed]=process.argv.slice(1);
const emit=value=>process.stdout.write(JSON.stringify({at:new Date().toISOString(),...value})+'\n');
const child=spawn(process.execPath,['--experimental-strip-types',runner,managed,'--reuse-binary'],{stdio:['ignore','pipe','pipe']});
emit({event:'runner-spawn',pid:child.pid});
let stdoutBytes=0,stderrBytes=0,tail='';
child.stdout.on('data',chunk=>{stdoutBytes+=chunk.length;tail=(tail+chunk.toString('utf8')).slice(-4096);for(const line of tail.split('\n').slice(0,-1)){const match=/^Product test PID (\d+); instructions and evidence:/.exec(line);if(match)emit({event:'product-started',pid:Number(match[1])});}tail=tail.slice(tail.lastIndexOf('\n')+1);});
child.stderr.on('data',chunk=>{stderrBytes+=chunk.length;});
child.on('error',error=>{emit({event:'runner-error',code:error.code??'unknown'});process.exitCode=1;});
child.on('close',(code,signal)=>{emit({event:'runner-exit',code,signal,stdoutBytes,stderrBytes});process.exitCode=code??1;});
`;
try {
  const before = await control(app, "before", statusScript);
  const running = before.statuses.filter((value) => value.processAlive);
  if (
    running.length !== 1 ||
    running[0].deviceId !== deviceId ||
    running[0].phase !== "running" ||
    running[0].generation !== runtime.generation ||
    before.device?.hardware?.quickBoot !== false
  )
    throw Error(
      "Require one running fixture phone with snapshots/Quick Boot disabled.",
    );
  const marker = await control(
    app,
    "marker-before",
    `return await invoke('android_probe_product_guest',{deviceId:${JSON.stringify(deviceId)},action:'marker-read'});`,
  );
  if (typeof marker !== "string" || !uuid.test(marker.trim()))
    throw Error("Prepare a fixture marker before the crash test.");
  if (secretIds.has(marker.trim()))
    throw Error(
      "Fixture marker unexpectedly aliases a private runtime identity.",
    );
  report.marker = marker.trim();
  const gui = await guiEnvironment();
  report.guiEnvironmentKeys = gui.keys;
  report.before = before;
  await phase("preflight-verified", {
    markerPresent: true,
    appPid: app.pid,
    guestPid: guest.pid,
    adbPid: adb.pid,
  });
  await assertIdentity(adb);
  await privateListener(adb);
  await onlyGuest(guest);
  if (
    !runtimeBytes.equals(await bytes(runtimePath)) ||
    !receiptBytes.equals(await bytes(receiptPath)) ||
    digest(await readFile(binary)) !== original.binarySha256
  )
    throw Error(
      "Fixture receipt/runtime/binary changed before crash injection.",
    );
  await assertIdentity(guest);
  report.crash = await crashPinnedApp(app);
  await phase("original-app-sigkill", report.crash);
  await poll(() => exited(app), 15000, "Original app exit");
  await assertIdentity(guest);
  await onlyGuest(guest);
  await privateListener(adb);
  if ((await processesMatching((file) => file === binary)).length !== 0)
    throw Error(
      "An app instance unexpectedly survived or replaced the killed app.",
    );
  await phase("orphan-preserved", {
    originalAppGone: true,
    originalGuestAlive: true,
    privateAdbAlive: true,
  });
  const logPath = join(root, name + "-runner.jsonl");
  const log = await open(logPath, "wx", 0o600);
  let wrapper;
  try {
    wrapper = spawn(
      process.execPath,
      [
        "--input-type=module",
        "-e",
        wrapperSource,
        join(import.meta.dirname, "run-android-product.mjs"),
        managed,
      ],
      {
        cwd: repository,
        env: gui.env,
        detached: true,
        stdio: ["ignore", log.fd, log.fd],
      },
    );
    await new Promise((resolveSpawn, reject) => {
      wrapper.once("spawn", resolveSpawn);
      wrapper.once("error", () =>
        reject(Error("Detached reuse runner could not start.")),
      );
    });
    wrapper.unref();
  } finally {
    await log.close();
  }
  report.wrapperPid = wrapper.pid;
  report.runnerLog = logPath;
  await phase("reuse-runner-launched", { wrapperPid: wrapper.pid });
  const replacement = await poll(
    async () => {
      const data = await bytes(receiptPath);
      if (data.equals(receiptBytes)) return false;
      let current;
      try {
        current = JSON.parse(data);
      } catch {
        return false;
      }
      if (
        !validReceipt(current) ||
        current.identifier !== original.identifier ||
        current.appData !== original.appData ||
        current.binarySha256 !== original.binarySha256 ||
        current.pid === app.pid
      )
        throw Error(
          "Replacement receipt is not the exact reused fixture identity.",
        );
      const pin = await identity(current.pid);
      if (!launchMatches(pin, current) || same(pin, app))
        throw Error("Replacement app native identity is invalid.");
      return { current, pin };
    },
    90000,
    "Replacement app receipt",
  );
  const nextApp = replacement.pin;
  report.replacementApp = nextApp;
  const apps = await processesMatching((file) => file === binary);
  if (apps.length !== 1 || !same(apps[0], nextApp))
    throw Error("Duplicate application executable after recovery launch.");
  const recovered = await control(nextApp, "recovered", statusScript, 60000);
  const recoveredPhone = recovered.statuses.filter(
    (value) => value.processAlive,
  );
  if (
    recoveredPhone.length !== 1 ||
    recoveredPhone[0].deviceId !== deviceId ||
    recoveredPhone[0].generation !== runtime.generation ||
    recoveredPhone[0].phase !== "failed" ||
    recovered.streams.some((value) => value.phase === "streaming")
  )
    throw Error(
      "Recovery did not retain the single old failed phone without automatic restart.",
    );
  await assertIdentity(guest);
  await onlyGuest(guest);
  await privateListener(adb);
  if (!runtimeBytes.equals(await bytes(runtimePath)))
    throw Error("Recovery changed the original runtime identity record.");
  report.recovered = recovered;
  await phase("safe-recovery-observed", {
    guestPid: guest.pid,
    generation: runtime.generation,
    phase: "failed",
  });
  const stopped = await control(
    nextApp,
    "stop-recovered",
    `return await invoke('android_stop',{deviceId:${JSON.stringify(deviceId)},force:false});`,
    90000,
  );
  if (stopped.processAlive || stopped.phase !== "stopped")
    throw Error("Production Stop did not confirm recovered guest shutdown.");
  report.stopped = stopped;
  await poll(
    async () => {
      try {
        await lstat(runtimePath);
        return false;
      } catch (error) {
        if (error.code !== "ENOENT") throw error;
      }
      return await exited(guest);
    },
    45000,
    "Recovered runtime and original QEMU removal",
  );
  const settleStarted = performance.now();
  const observedSdkProcesses = new Map();
  report.postStopSettle = {
    timeoutMs: 15000,
    elapsedMs: 0,
    timingScope:
      "additional SDK-process settle after runtime removal and original QEMU exit",
    runtimeRecordStatus: "checking",
    observedSdkProcesses: [],
  };
  async function requireStoppedRuntimeAbsent() {
    report.postStopSettle.runtimeRecordStatus = "checking";
    try {
      await lstat(runtimePath);
    } catch (error) {
      if (error.code === "ENOENT") {
        report.postStopSettle.runtimeRecordStatus = "absent";
        return;
      }
      report.postStopSettle.runtimeRecordStatus = "unknown";
      throw error;
    }
    report.postStopSettle.runtimeRecordStatus = "reappeared";
    throw Error(
      "The stopped runtime record reappeared during SDK process settle.",
    );
  }
  try {
    await poll(
      async () => {
        await requireStoppedRuntimeAbsent();
        const remaining = await processesMatching(
          (file) =>
            file.startsWith(engine) &&
            (basename(file).startsWith("qemu-system-") ||
              basename(file) === "emulator"),
        );
        const elapsedMs = Math.round(performance.now() - settleStarted);
        for (const pin of remaining) {
          const key = JSON.stringify([
            pin.pid,
            pin.created,
            pin.boot,
            pin.executable,
            pin.file,
          ]);
          let observed = observedSdkProcesses.get(key);
          if (!observed) {
            if (observedSdkProcesses.size >= 64)
              throw Error(
                "Post-Stop SDK process evidence exceeds 64 identities.",
              );
            observed = {
              identity: pin,
              firstObservedElapsedMs: elapsedMs,
              lastObservedElapsedMs: elapsedMs,
            };
            observedSdkProcesses.set(key, observed);
            report.postStopSettle.observedSdkProcesses.push(observed);
          }
          observed.lastObservedElapsedMs = elapsedMs;
        }
        await requireStoppedRuntimeAbsent();
        if (performance.now() - settleStarted > report.postStopSettle.timeoutMs)
          throw Error(
            "Managed SDK process settle exceeded its 15-second deadline.",
          );
        return remaining.length === 0;
      },
      report.postStopSettle.timeoutMs,
      "Managed SDK processes settling after confirmed Stop",
    );
  } finally {
    report.postStopSettle.elapsedMs = Math.round(
      performance.now() - settleStarted,
    );
  }
  await phase("recovered-stop-confirmed", {
    originalGuestGone: true,
    runtimeRecordGone: true,
    postStopSettle: report.postStopSettle,
  });
  await control(nextApp, "present", undefined, 15000, "present");
  await control(nextApp, "resize", undefined, 15000, "resize");
  const restarted = await control(
    nextApp,
    "cold-start",
    `
const deviceId=${JSON.stringify(deviceId)},phoneName=${JSON.stringify(before.device.name)};
const prototype=CanvasRenderingContext2D.prototype;
const originalPut=prototype.putImageData,originalDraw=prototype.drawImage;
const putDescriptor=Object.getOwnPropertyDescriptor(prototype,'putImageData'),drawDescriptor=Object.getOwnPropertyDescriptor(prototype,'drawImage');
let primaryDraw,secondaryDraw,observerExpired=false,hookOwnershipChanged=false;
function put(...args){
 const result=originalPut.apply(this,args);
 if(this.canvas.classList.contains('android-screen')&&this.canvas.width===720&&this.canvas.height===1280&&args[0]?.width===720&&args[0]?.height===1280&&args.length===3&&args[1]===0&&args[2]===0)primaryDraw=this.canvas;
 return result;
}
function draw(...args){
 const result=originalDraw.apply(this,args);
 if(this.canvas.classList.contains('android-screen')&&this.canvas.width===321&&this.canvas.height===571&&args.length===5&&args[0]===primaryDraw&&args[0]?.width===720&&args[0]?.height===1280&&args[1]===0&&args[2]===0&&args[3]===321&&args[4]===571)secondaryDraw={canvas:this.canvas,source:args[0]};
 return result;
}
function restore(){
 if(prototype.putImageData===put)Object.defineProperty(prototype,'putImageData',putDescriptor);
 else if(prototype.putImageData!==originalPut)hookOwnershipChanged=true;
 if(prototype.drawImage===draw)Object.defineProperty(prototype,'drawImage',drawDescriptor);
 else if(prototype.drawImage!==originalDraw)hookOwnershipChanged=true;
}
const observerTimer=setTimeout(()=>{observerExpired=true;restore();},225000);
try{
 prototype.putImageData=put;prototype.drawImage=draw;
 const started=await invoke('android_start',{deviceId});
 if(!started.processAlive||!started.generation||started.generation===${JSON.stringify(runtime.generation)})throw Error('Cold Start did not create a fresh phone');
 await wait(async()=>{const state=await invoke('android_state');const value=state.statuses.find(item=>item.deviceId===deviceId);if(value?.generation&&value.generation!==started.generation)throw Error('Cold Start generation changed');if(value?.phase==='failed')throw Error('Cold Start failed before readiness');return value?.generation===started.generation&&value.phase==='running'&&value.processAlive;},180000);
 const panes=await wait(()=>{const candidates=[...document.querySelectorAll('.android-pane')].filter(pane=>pane.getAttribute('aria-label')===phoneName&&pane.getClientRects().length);if(candidates.length>2)throw Error('Keep exactly two requested phone panes visible');return candidates.length===2&&candidates.every(pane=>pane.querySelector('.android-screen'))?candidates:null;},30000);
 panes.sort((left,right)=>right.getBoundingClientRect().width-left.getBoundingClientRect().width);
 if(panes.length!==2||panes[0].getBoundingClientRect().width<=panes[1].getBoundingClientRect().width)throw Error('Keep exactly two requested phone panes with an unambiguous widest pane visible');
 const canvases=panes.map(pane=>pane.querySelector('.android-screen'));
 const paneIds=canvases.map(canvas=>canvas?.closest('[data-android-pane-id]')?.dataset.androidPaneId);
 if(canvases.some(canvas=>!canvas)||paneIds.some(id=>!id)||paneIds[0]===paneIds[1])throw Error('Missing or ambiguous recovery canvas/pane identities');
 for(const [index,pane] of panes.entries()){
  const actions=pane.querySelector('button[aria-label="Android actions"]');if(!actions||actions.disabled)throw Error('Missing phone actions');actions.click();
  const size=await wait(()=>[...document.querySelectorAll('[role="menu"][aria-label="Android actions"] button[role="menuitem"]')].find(item=>['Actual size (1:1)','Preview size (100%)','Fit to panel'].includes(item.textContent.trim())&&!item.disabled&&item.getClientRects().length));
  const isActual=size.textContent.trim()==='Fit to panel';
  if(isActual===(index===0))size.closest('[role="menu"]').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));else size.click();
  await sleep(200);
 }
 function assertCanvases(){
  if(observerExpired)throw Error('Recovery 2D source observer exceeded its deadline');
  const current=[...document.querySelectorAll('.android-screen')];
  if(current.length!==2||current.some(canvas=>!canvases.includes(canvas))||canvases.some((canvas,index)=>!canvas.isConnected||canvas.closest('[data-android-pane-id]')?.dataset.androidPaneId!==paneIds[index]||!canvas.getClientRects().length))throw Error('Recovery canvas/pane identities changed or became hidden');
 }
 const state=await wait(async()=>{assertCanvases();const state=await invoke('android_state');const live=state.streams.filter(stream=>stream.phase==='streaming');return canvases[0].width===720&&canvases[0].height===1280&&canvases[1].width===321&&canvases[1].height===571&&primaryDraw===canvases[0]&&secondaryDraw?.canvas===canvases[1]&&secondaryDraw.source===canvases[0]&&live.length===1&&live[0].deviceId===deviceId&&live[0].generation===started.generation&&live[0].ipcFrames>0?state:null;},30000);
 assertCanvases();
 const value=state.statuses.find(item=>item.deviceId===deviceId);return {status:{deviceId:value.deviceId,generation:value.generation,phase:value.phase,processAlive:value.processAlive,serial:value.serial,display:value.display},streams:state.streams.map(({deviceId,generation,epoch,phase,grpcFrames,grpcBytes,ipcFrames,ipcBytes})=>({deviceId,generation,epoch,phase,grpcFrames,grpcBytes,ipcFrames,ipcBytes})),source:[720,1280],views:canvases.length,viewDetails:canvases.map((canvas,index)=>({paneId:paneIds[index],framebuffer:[canvas.width,canvas.height],sizeMode:index===0?'actual':'fit'})),sourceProof:'Successful 2D primary putImageData(720x1280) and secondary drawImage from that primary into 321x571; source/draw provenance only, no bitmap readback or native input assertion'};
}finally{clearTimeout(observerTimer);restore();if(hookOwnershipChanged)throw Error('Recovery 2D hook ownership changed; foreign replacement preserved');}
`,
    240000,
  );
  const freshRuntime = fixtureJson(await bytes(runtimePath));
  secretIds.add(freshRuntime.generationKey);
  const freshGuest = await identity(freshRuntime.process?.pid);
  if (
    !validRuntime(freshRuntime, freshGuest) ||
    freshRuntime.generation === runtime.generation ||
    same(freshGuest, guest) ||
    restarted.status?.generation !== freshRuntime.generation ||
    restarted.status?.phase !== "running" ||
    !restarted.status?.processAlive
  )
    throw Error(
      "Cold Start native runtime/generation/source identities disagree.",
    );
  await descendant(freshGuest, nextApp);
  await onlyGuest(freshGuest);
  const retained = await control(
    nextApp,
    "marker-after",
    `return await invoke('android_probe_product_guest',{deviceId:${JSON.stringify(deviceId)},action:'marker-read'});`,
  );
  if (typeof retained !== "string" || retained.trim() !== report.marker)
    throw Error("Cold boot did not retain the original guest marker.");
  await assertIdentity(nextApp);
  await assertIdentity(freshGuest);
  await privateListener(adb);
  if (digest(await readFile(binary)) !== original.binarySha256)
    throw Error("The reused executable changed during recovery.");
  report.restarted = restarted;
  report.newGeneration = freshRuntime.generation;
  report.replacementGuest = freshGuest;
  report.markerRetained = true;
  report.privateAdbPreserved = true;
  report.completed = true;
  await phase("recovery-complete", {
    appPid: nextApp.pid,
    guestPid: freshGuest.pid,
    generation: freshRuntime.generation,
    markerRetained: true,
    source: [720, 1280],
    privateAdbPid: adb.pid,
  });
} catch (error) {
  report.error = safeError(error);
  await phase("recovery-failed", {
    error: report.error,
    cleanup: "none; owned app/guest/ADB retained for inspection",
  });
} finally {
  report.discoveryExclusions = [...discoveryExclusions.values()];
  report.endedAt = new Date().toISOString();
  await writeFile(reportPath, JSON.stringify(report, null, 2) + "\n", {
    mode: 0o600,
  });
  await journal.close();
}
if (!report.completed)
  throw Error(
    report.error ?? "Native recovery incomplete; inspect the isolated report.",
  );
console.log(
  JSON.stringify({
    completed: true,
    report: reportPath,
    newApplicationPid: report.replacementApp.pid,
    newGuestPid: report.replacementGuest.pid,
    privateAdbPid: adb.pid,
    leftRunning: true,
  }),
);
