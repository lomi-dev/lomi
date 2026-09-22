import { build } from "vite";
import { spawn } from "node:child_process";
import { createServer } from "node:net";
import {
  chmod,
  mkdir,
  readFile,
  writeFile,
  rm,
  realpath,
  rename,
} from "node:fs/promises";
import { resolve, join, dirname } from "node:path";
import { homedir } from "node:os";
import { createHash, generateKeyPairSync, randomUUID, sign } from "node:crypto";

if (process.platform !== "darwin" || process.arch !== "arm64")
  throw Error("This probe has only been tested on macOS ARM64.");
const root = await realpath(
  resolve(process.argv[2] ?? "/tmp/lomi-android-stage0-20260918"),
);
await chmod(root, 0o700);
await chmod(join(root, "emulator-home"), 0o700);
try {
  await chmod(join(root, "emulator-home/adbkey"), 0o600);
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
for (const port of [15037, 5580, 5581, 18557]) {
  const probe = createServer();
  await new Promise((resolve, reject) => {
    probe.on("error", () =>
      reject(
        Error(
          `Probe port ${port} is occupied; stop the previous run before reusing its directory.`,
        ),
      ),
    );
    probe.listen(port, "127.0.0.1", resolve);
  });
  await new Promise((resolve) => probe.close(resolve));
}
const consent = JSON.parse(
  await readFile(join(root, "evidence/consent.json"), "utf8"),
);
if (consent.accepted !== true)
  throw Error(
    "Explicit SDK license acceptance is required; this runner does not accept licenses.",
  );
const repository = resolve(import.meta.dirname, "../..");
const binary = join(repository, "src-tauri/target/release/lomi");
const options = process.argv.slice(3);
const reuse = options.includes("--reuse-binary");
const avdOption = options.indexOf("--avd");
const avd = avdOption < 0 ? "sb_stage0" : options[avdOption + 1];
if (!/^sb_stage0(?:_[a-z0-9]+)?$/.test(avd ?? ""))
  throw Error("Only isolated stage0 AVD names are allowed");
const avdDirectory = await realpath(join(root, "avd", `${avd}.avd`));
if (!avdDirectory.startsWith(join(root, "avd") + "/"))
  throw Error("The fixture AVD resolves outside the isolated directory");
const previousBuild = reuse
  ? JSON.parse(await readFile(join(root, "evidence/application.json"), "utf8"))
  : undefined;
const binaryHash = async () =>
  createHash("sha256")
    .update(await readFile(binary))
    .digest("hex");
if (
  reuse &&
  (previousBuild.root !== root ||
    previousBuild.repository !== repository ||
    !/^dev\.lomi\.android-probe-\d+$/.test(previousBuild.identifier) ||
    previousBuild.binarySha256 !== (await binaryHash()))
)
  throw Error(
    "The prior owned probe executable could not be verified for reuse",
  );
const identifier =
  previousBuild?.identifier ?? `dev.lomi.android-probe-${Date.now()}`;
const appData = join(homedir(), "Library/Application Support", identifier);
await mkdir(appData, { recursive: true });
await mkdir(join(root, "runtime"), { recursive: true, mode: 0o700 });
for (const name of ["started", "boot", "runner-stop"])
  await rm(join(root, `evidence/${name}.json`), { force: true });
await rm(join(root, "native-control.json"), { force: true });
await rm(join(root, "evidence/native-control.json"), { force: true });
await rm(join(root, "runtime/previous-token"), { force: true });
await writeFile(
  join(root, "instruction.json"),
  JSON.stringify({ id: "boot", action: "start" }),
);
const methods = [
  "getStatus",
  "getScreenshot",
  "streamScreenshot",
  "sendKey",
  "sendTouch",
  "setClipboard",
  "getClipboard",
  "setVmState",
  "setPhysicalModel",
].map((method) => `/android.emulation.control.EmulatorController/${method}`);
methods.push("/android.emulation.control.Rtc/requestRtcStream");
await writeFile(
  join(root, "runtime/allowlist.json"),
  JSON.stringify({
    unprotected: [],
    allowlist: [{ iss: "lomi", protected: methods }],
  }),
);
function credentials() {
  const { privateKey, publicKey } = generateKeyPairSync("ec", {
    namedCurve: "prime256v1",
  });
  return {
    privateKey,
    publicJwk: {
      ...publicKey.export({ format: "jwk" }),
      kid: randomUUID(),
      alg: "ES256",
      use: "sig",
    },
  };
}
let authority = credentials();
const encode = (value) =>
  Buffer.from(JSON.stringify(value)).toString("base64url");
function token(aud, expired = false) {
  const now = Math.floor(Date.now() / 1000);
  // The selected emulator's Tink validator rejects a typ header.
  const body =
    encode({ alg: "ES256", kid: authority.publicJwk.kid }) +
    "." +
    encode({
      iss: "lomi",
      aud,
      iat: now - 60,
      exp: expired ? now - 30 : now + 120,
    });
  return (
    body +
    "." +
    sign("sha256", Buffer.from(body), {
      key: authority.privateKey,
      dsaEncoding: "ieee-p1363",
    }).toString("base64url")
  );
}
async function writeCredential(name, value) {
  const temporary = join(root, "runtime", `.${name}.tmp`);
  await writeFile(temporary, value, { mode: 0o600 });
  await chmod(temporary, 0o600);
  await rename(temporary, join(root, "runtime", name));
}
async function renewTokens() {
  for (const [name, value] of [
    ["token", token(methods)],
    ["expired", token(methods, true)],
    ["wrong-audience", token(["/other/service"])],
  ])
    await writeCredential(name, value);
}
await renewTokens();
await build({
  configFile: false,
  logLevel: "warn",
  build: {
    outDir: join(root, "bundle"),
    emptyOutDir: true,
    minify: false,
    lib: {
      entry: join(repository, "tests/native/android-smoke.js"),
      name: "AndroidProbe",
      formats: ["iife"],
      fileName: () => "probe.js",
    },
  },
});
await writeFile(
  join(root, "probe.js"),
  await readFile(join(root, "bundle/probe.js")),
);
const config = join(root, "tauri-probe.json");
const applicationConfig = JSON.parse(
  await readFile(join(repository, "src-tauri/tauri.conf.json"), "utf8"),
);
await writeFile(
  config,
  JSON.stringify({
    identifier,
    app: {
      security: {
        csp: applicationConfig.app.security.csp.replace(
          "img-src 'self'",
          "img-src 'self' blob:",
        ),
      },
    },
  }),
);
const env = {
  ...process.env,
  LOMI_ANDROID_PROBE_DIRECTORY: root,
  LOMI_ANDROID_PROBE_AVD: avd,
};
if (!reuse) {
  const builder = spawn(
    "pnpm",
    [
      "tauri",
      "build",
      "--no-bundle",
      "--features",
      "android-probe",
      "--config",
      config,
    ],
    { cwd: repository, env, stdio: "inherit" },
  );
  const buildCode = await new Promise((resolve) => builder.on("exit", resolve));
  if (buildCode !== 0) throw Error(`Native release build failed: ${buildCode}`);
}
await writeFile(
  join(root, "evidence/application.json"),
  JSON.stringify({
    identifier,
    appData,
    root,
    repository,
    binarySha256: await binaryHash(),
    reusedBinary: reuse,
    avd,
  }),
);
const child = spawn(binary, [], { cwd: repository, env, stdio: "inherit" });
let exited = false;
child.on("exit", (code) => {
  exited = true;
  process.exitCode = code ?? 1;
});
let interrupted = false;
for (const signal of ["SIGINT", "SIGTERM"])
  process.on(signal, () => {
    interrupted = true;
  });
let initialized = false;
let initializedPid;
let refreshed = Date.now();
let stopping = false;
let stopAttempt = 0;
let stopId;
const started = Date.now();
while (!exited) {
  {
    try {
      const { pid } = JSON.parse(
        await readFile(join(root, "evidence/started.json"), "utf8"),
      );
      if (!Number.isSafeInteger(pid) || pid <= 0)
        throw Error("Invalid owned emulator PID");
      if (initializedPid !== pid) {
        // Emulator 37.1.11 ignores ANDROID_EMULATOR_DISCOVERY_DIR on macOS.
        const discovery = join(
          homedir(),
          "Library/Caches/TemporaryItems/avd/running",
        );
        const contents = await readFile(
          join(discovery, `pid_${pid}.ini`),
          "utf8",
        );
        const jwks = contents.match(/^grpc.jwks=(.+)$/m)?.[1];
        if (
          !jwks ||
          dirname(await realpath(jwks)) !==
            (await realpath(join(discovery, String(pid), "jwks")))
        )
          throw Error("Unexpected emulator JWK directory");
        if (initializedPid !== undefined) {
          await writeCredential(
            "previous-token",
            await readFile(join(root, "runtime/token"), "utf8"),
          );
          authority = credentials();
        }
        await renewTokens();
        await writeFile(
          join(jwks, "lomi-stage0.jwk"),
          JSON.stringify({ keys: [authority.publicJwk] }),
          { mode: 0o600 },
        );
        initialized = true;
        initializedPid = pid;
        refreshed = Date.now();
        console.log(
          `Probe app PID ${child.pid}; emulator PID ${pid}; reports ${root}/evidence`,
        );
      }
      if (initialized && Date.now() - refreshed >= 60_000) {
        await renewTokens();
        refreshed = Date.now();
      }
    } catch (error) {
      if (!initialized && Date.now() - started > 120000) {
        console.error("Native probe initialization failed:", error.message);
        interrupted = true;
      }
    }
  }
  if (interrupted && !stopping) {
    stopping = true;
    stopId = `runner-stop-${++stopAttempt}`;
    await writeFile(
      join(root, "native-control.json"),
      JSON.stringify({ id: stopId, action: "stop-and-quit" }),
    );
  }
  if (stopping) {
    try {
      const result = JSON.parse(
        await readFile(join(root, "evidence/native-control.json"), "utf8"),
      );
      if (result.id === stopId && !result.ok) {
        console.error(
          "Stop failed; probe remains open and retains its process handle:",
          result.error,
        );
        stopping = false;
        interrupted = false;
      }
    } catch {}
  }
  await new Promise((resolve) => setTimeout(resolve, 500));
}
