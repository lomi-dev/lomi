// Real headless emulator against an independently owned ADB fixture and version-changing proxy.
import { createServer, connect } from "node:net";
import { connect as grpcConnect } from "node:http2";
import { spawn } from "node:child_process";
import { generateKeyPairSync, sign, createHash } from "node:crypto";
import { readFile, writeFile, mkdir, realpath, chmod } from "node:fs/promises";
import { createWriteStream } from "node:fs";
import { join, dirname, resolve } from "node:path";
import { homedir } from "node:os";
if (process.platform !== "darwin" || process.arch !== "arm64")
  throw Error("This native fixture has only been qualified on macOS ARM64");
const root = await realpath(
  resolve(process.argv[2] ?? "/tmp/lomi-android-stage0-20260918"),
);
await chmod(root, 0o700);
await chmod(join(root, "emulator-home"), 0o700);
await chmod(join(root, "emulator-home/adbkey"), 0o600);
const consent = JSON.parse(await readFile(join(root, "evidence/consent.json")));
if (!consent.accepted) throw Error("Explicit trial acceptance required");
for (const port of [15039, 15041, 5580, 5581, 18557]) {
  const listener = createServer();
  await new Promise((resolve, reject) => {
    listener.once("error", reject);
    listener.listen(port, "127.0.0.1", resolve);
  });
  await new Promise((resolve) => listener.close(resolve));
}
const userKey = join(homedir(), ".android/adbkey");
// ADB ignores ANDROID_USER_HOME for its default authentication key. Never create
// or replace that key in this fixture; only check that an existing identity survives.
await readFile(userKey);
const external = join(root, "user/other-tool");
await mkdir(external, { recursive: true, mode: 0o700 });
const env = Object.fromEntries(
  Object.entries(process.env).filter(
    ([name]) => !/^(ANDROID_|ADB_|JAVA_|JDK_|_JAVA_|QT_|DYLD_)/.test(name),
  ),
);
Object.assign(env, {
  ANDROID_HOME: join(root, "sdk"),
  ANDROID_SDK_ROOT: join(root, "sdk"),
  ANDROID_USER_HOME: join(root, "user"),
  ANDROID_EMULATOR_HOME: join(root, "emulator-home"),
  ANDROID_AVD_HOME: join(root, "avd"),
  TMPDIR: join(root, "tmp"),
  ADB_SERVER_SOCKET: "tcp:127.0.0.1:15041",
  ANDROID_ADB_SERVER_PORT: "15041",
});
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const adbLog = createWriteStream(join(root, "logs/other-tool-adb.log"));
const adb = spawn(
  join(root, "sdk/platform-tools/adb"),
  ["-L", "tcp:15039", "server", "nodaemon"],
  {
    env: {
      ...env,
      ANDROID_USER_HOME: external,
      ADB_SERVER_SOCKET: "tcp:127.0.0.1:15039",
      ANDROID_ADB_SERVER_PORT: "15039",
    },
    stdio: ["ignore", "pipe", "pipe"],
  },
);
adb.stdout.pipe(adbLog);
adb.stderr.pipe(adbLog);
let emulator;
let grpc;
let proxy;
const sockets = new Set();
const events = [];
const failures = [];
let version = "0029";
const { privateKey, publicKey } = generateKeyPairSync("ec", {
  namedCurve: "prime256v1",
});
const jwk = {
  ...publicKey.export({ format: "jwk" }),
  kid: "shared-adb-trial",
  alg: "ES256",
  use: "sig",
};
const methods = ["getStatus", "setVmState"].map(
  (m) => "/android.emulation.control.EmulatorController/" + m,
);
const encoded = (x) => Buffer.from(JSON.stringify(x)).toString("base64url");
const now = Math.floor(Date.now() / 1000);
const body =
  encoded({ alg: "ES256", kid: jwk.kid }) +
  "." +
  encoded({
    iss: "lomi-shared-adb-trial",
    aud: methods,
    iat: now - 30,
    exp: now + 600,
  });
const token =
  body +
  "." +
  sign("sha256", Buffer.from(body), {
    key: privateKey,
    dsaEncoding: "ieee-p1363",
  }).toString("base64url");
async function externalService(service) {
  return new Promise((resolve, reject) => {
    const socket = connect(15039, "127.0.0.1", () =>
      socket.write(service.length.toString(16).padStart(4, "0") + service),
    );
    let bytes = Buffer.alloc(0);
    socket.setTimeout(2000, () =>
      socket.destroy(Error("External ADB service timeout")),
    );
    socket.on("error", reject);
    socket.on("data", (chunk) => {
      bytes = Buffer.concat([bytes, chunk]);
      if (bytes.length > 8192)
        socket.destroy(Error("External ADB reply limit"));
    });
    socket.on("end", () => {
      if (bytes.subarray(0, 4).toString() !== "OKAY")
        return reject(Error("External ADB request failed"));
      const length = Number.parseInt(bytes.subarray(4, 8).toString(), 16);
      if (!Number.isInteger(length) || length !== bytes.length - 8)
        return reject(Error("Invalid external ADB response"));
      resolve(bytes.subarray(8).toString());
    });
  });
}
async function rpc(method, payload = Buffer.alloc(0)) {
  return new Promise((resolve, reject) => {
    const request = grpc.request({
      ":method": "POST",
      ":path": "/android.emulation.control.EmulatorController/" + method,
      "content-type": "application/grpc",
      te: "trailers",
      authorization: "Bearer " + token,
    });
    let status;
    let size = 0;
    const timeout = setTimeout(
      () => request.destroy(Error("gRPC request timeout")),
      5000,
    );
    request.on("response", (headers) => {
      status = headers["grpc-status"];
    });
    request.on("trailers", (headers) => {
      status = headers["grpc-status"];
    });
    request.on("data", (data) => {
      size += data.length;
      if (size > 1024 * 1024) request.destroy(Error("gRPC output limit"));
    });
    request.on("error", (error) => {
      clearTimeout(timeout);
      reject(error);
    });
    request.on("end", () => {
      clearTimeout(timeout);
      status === "0"
        ? resolve({ status, size })
        : reject(Error("gRPC status " + status));
    });
    const header = Buffer.alloc(5);
    header.writeUInt32BE(payload.length, 1);
    request.end(Buffer.concat([header, payload]));
  });
}
try {
  for (let n = 0; n < 50; n++) {
    try {
      // ADB ignores ANDROID_USER_HOME for its default authentication key. Never create
      // or replace that key in this fixture; only check that an existing identity survives.
      await readFile(userKey);
      break;
    } catch {
      await sleep(100);
    }
  }
  const before = createHash("sha256")
    .update(await readFile(userKey))
    .digest("hex");
  proxy = createServer((socket) => {
    sockets.add(socket);
    socket.on("close", () => sockets.delete(socket));
    socket.on("error", () => {});
    let buffered = Buffer.alloc(0);
    const first = (data) => {
      buffered = Buffer.concat([buffered, data]);
      if (buffered.length > 65540) return socket.destroy();
      if (buffered.length < 4) return;
      const length = Number.parseInt(buffered.subarray(0, 4).toString(), 16);
      if (!Number.isInteger(length)) return socket.destroy();
      if (buffered.length < length + 4) return;
      socket.removeListener("data", first);
      const service = buffered.subarray(4, length + 4).toString();
      if (events.length < 1000) events.push(service);
      if (service === "host:version") {
        socket.end("OKAY0004" + version);
        version = "0028";
        return;
      }
      if (service === "host:kill") {
        socket.end("FAIL0012Test refuses kill!");
        return;
      }
      const upstream = connect(15039, "127.0.0.1", () => {
        upstream.write(buffered);
        socket.pipe(upstream);
        upstream.pipe(socket);
      });
      sockets.add(upstream);
      upstream.on("close", () => {
        sockets.delete(upstream);
        socket.destroy();
      });
      upstream.on("error", () => socket.destroy());
      socket.on("close", () => upstream.destroy());
    };
    socket.on("data", first);
  });
  await new Promise((resolve) => proxy.listen(15041, "127.0.0.1", resolve));
  await writeFile(
    join(root, "runtime/shared-adb-allowlist.json"),
    JSON.stringify({
      unprotected: [],
      allowlist: [{ iss: "lomi-shared-adb-trial", protected: methods }],
    }),
  );
  const log = createWriteStream(join(root, "logs/shared-adb-emulator.log"));
  emulator = spawn(
    join(root, "sdk/emulator/emulator"),
    [
      "-avd",
      "sb_stage0",
      "-no-window",
      "-no-audio",
      "-no-boot-anim",
      "-camera-back",
      "none",
      "-camera-front",
      "none",
      "-gpu",
      "host",
      "-cores",
      "2",
      "-memory",
      "2048",
      "-vsync-rate",
      "30",
      "-port",
      "5580",
      "-grpc",
      "18557",
      "-grpc-use-jwt",
      "-grpc-allowlist",
      join(root, "runtime/shared-adb-allowlist.json"),
      "-no-snapshot",
      "-no-metrics",
      "-crash-report-mode",
      "disabled",
      "-adb-path",
      join(root, "runtime/no-external-adb"),
      "-append-userspace-opt",
      "androidboot.lomi.device=00000000-0000-0000-0000-000000000001",
    ],
    { env, stdio: ["ignore", "pipe", "pipe"] },
  );
  emulator.stdout.pipe(log);
  emulator.stderr.pipe(log);
  const discovery = join(
    homedir(),
    "Library/Caches/TemporaryItems/avd/running",
  );
  let registered = false;
  for (let n = 0; n < 200; n++) {
    try {
      const info = await readFile(
        join(discovery, "pid_" + emulator.pid + ".ini"),
        "utf8",
      );
      const jwks = info.match(/^grpc.jwks=(.+)$/m)?.[1];
      if (
        !jwks ||
        dirname(await realpath(jwks)) !==
          (await realpath(join(discovery, String(emulator.pid), "jwks")))
      )
        throw Error("Unexpected own JWK path");
      await writeFile(
        join(jwks, "shared-adb-trial.jwk"),
        JSON.stringify({ keys: [jwk] }),
        { mode: 0o600 },
      );
      registered = true;
      break;
    } catch {
      await sleep(100);
    }
  }
  if (!registered) throw Error("Owned emulator discovery timed out");
  grpc = grpcConnect("http://127.0.0.1:18557");
  grpc.on("error", () => {});
  await sleep(1000);
  const statusBefore = await rpc("getStatus");
  await sleep(20000);
  const statusAfter = await rpc("getStatus");
  const devices = await externalService("host:devices-l");
  if (!/emulator-5580\s+device\b/.test(devices))
    throw Error(
      "Owned guest is not ready through the external ADB server: " + devices,
    );
  await rpc("setVmState", Buffer.from([8, 5]));
  for (
    let n = 0;
    n < 300 && emulator.exitCode === null && emulator.signalCode === null;
    n++
  )
    await sleep(100);
  if (emulator.exitCode === null && emulator.signalCode === null)
    throw Error("Native Stop timeout");
  const after = createHash("sha256")
    .update(await readFile(userKey))
    .digest("hex");
  const result = {
    externalAdbPid: adb.pid,
    emulatorPid: emulator.pid,
    serverAliveAfterEmulatorStop:
      adb.exitCode === null && adb.signalCode === null,
    serverIdentityUnchanged: before === after,
    versionAfterPreflight: version,
    ownedGuestConnected: true,
    externalServerVersionAfterStop: await externalService("host:version"),
    events,
    automaticKillAttempted: events.includes("host:kill"),
    statusBefore,
    statusAfter,
    emulatorExit: emulator.exitCode,
  };
  await writeFile(
    join(root, "evidence/shared-adb-trial.json"),
    JSON.stringify(result, null, 2),
  );
  console.log(JSON.stringify(result, null, 2));
  if (
    !result.serverAliveAfterEmulatorStop ||
    !result.serverIdentityUnchanged ||
    result.automaticKillAttempted
  )
    throw Error("Shared ADB isolation failed");
} catch (error) {
  failures.push(error.message);
  console.error(error);
  throw error;
} finally {
  if (emulator && emulator.exitCode === null && emulator.signalCode === null) {
    try {
      await rpc("setVmState", Buffer.from([8, 5]));
    } catch {}
    for (
      let n = 0;
      n < 100 && emulator.exitCode === null && emulator.signalCode === null;
      n++
    )
      await sleep(100);
    if (emulator.exitCode === null && emulator.signalCode === null)
      emulator.kill("SIGTERM");
  }
  grpc?.close();
  for (const socket of sockets) socket.destroy();
  if (proxy) await new Promise((resolve) => proxy.close(resolve));
  if (adb.exitCode === null && adb.signalCode === null) {
    adb.kill("SIGTERM");
    await new Promise((resolve) => adb.once("exit", resolve));
  }
  if (failures.length)
    await writeFile(
      join(root, "evidence/shared-adb-failure.json"),
      JSON.stringify({ failures, events }, null, 2),
    );
}
