import { spawn } from "node:child_process";
import {
  mkdir,
  readFile,
  writeFile,
  realpath,
  rm,
  stat,
} from "node:fs/promises";
import { createHash } from "node:crypto";
import { join, resolve, basename } from "node:path";
import { homedir } from "node:os";
import { newProject, newSession, newWorkspace } from "../../src/model.ts";

if (process.platform !== "darwin" || process.arch !== "arm64")
  throw Error("The product fixture has only been qualified on macOS ARM64.");
const managed = await realpath(process.argv[2]);
const trial = resolve(managed, "..");
if (
  !basename(managed).startsWith("native-managed-") ||
  !basename(trial).startsWith("lomi-android-stage0-")
)
  throw Error("Use the isolated SDK prepared by the native installer trial.");
const consent = JSON.parse(
  await readFile(join(trial, "evidence/consent.json"), "utf8"),
);
if (consent.accepted !== true)
  throw Error("The isolated SDK license has not been accepted.");
const root = join(trial, "product");
await mkdir(root, { recursive: true, mode: 0o700 });
await mkdir(join(root, "apk-selection"), { recursive: true, mode: 0o700 });
const inputApk = join(trial, "development-tools/input-build/input-test.apk");
if (
  await stat(inputApk).catch((error) => {
    if (error.code !== "ENOENT") throw error;
    return null;
  })
)
  await writeFile(
    join(root, "apk-selection/input-test.apk"),
    await readFile(inputApk),
  );

const repository = resolve(import.meta.dirname, "../..");
const binary = join(repository, "src-tauri/target/release/lomi");
const reuse = process.argv.includes("--reuse-binary");
const digest = async () =>
  createHash("sha256")
    .update(await readFile(binary))
    .digest("hex");
const previous = reuse
  ? JSON.parse(await readFile(join(root, "application.json"), "utf8"))
  : undefined;
if (
  reuse &&
  (previous.managed !== managed ||
    previous.repository !== repository ||
    previous.binarySha256 !== (await digest()))
)
  throw Error("The previous product fixture executable does not match.");
const identifier =
  previous?.identifier ?? `dev.lomi.android-product-${Date.now()}`;
if (!/^dev\.lomi\.android-product-\d+$/.test(identifier))
  throw Error("Invalid fixture identity");
const appData = join(homedir(), "Library/Application Support", identifier);
await mkdir(appData, { recursive: true, mode: 0o700 });
await mkdir(join(root, "workspace"), { recursive: true });
if (!reuse) {
  const project = newProject(join(root, "workspace"), "local:zsh");
  project.workspaces.push(
    newWorkspace(project.path, "local:zsh", "Other workspace"),
  );
  await writeFile(
    join(appData, "session.json"),
    JSON.stringify({
      ...newSession(),
      projects: [project],
      activeProjectId: project.id,
    }),
  );
}
await rm(join(root, "instruction.json"), { force: true });
const application = JSON.parse(
  await readFile(join(repository, "src-tauri/tauri.conf.json"), "utf8"),
);
const config = join(root, "tauri.json");
await writeFile(
  config,
  JSON.stringify({
    identifier,
    app: {
      windows: [
        {
          ...application.app.windows[0],
          title: "Lomi Android product test",
          width: 1000,
          height: 760,
        },
      ],
    },
  }),
);
const env = { ...process.env, LOMI_ANDROID_PRODUCT_DIRECTORY: managed };
delete env.LOMI_ANDROID_PROBE_DIRECTORY;
if (!reuse) {
  const build = spawn(
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
  const code = await new Promise((resolve) => build.on("exit", resolve));
  if (code !== 0) throw Error(`Product fixture build failed: ${code}`);
}
const child = spawn(binary, [], { cwd: repository, env, stdio: "inherit" });
await writeFile(
  join(root, "application.json"),
  JSON.stringify(
    {
      identifier,
      appData,
      managed,
      repository,
      binarySha256: await digest(),
      pid: child.pid,
      startedAt: new Date().toISOString(),
    },
    null,
    2,
  ),
);
console.log(
  `Product test PID ${child.pid}; instructions and evidence: ${root}`,
);
process.exitCode = await new Promise((resolve) =>
  child.on("exit", (code) => resolve(code ?? 1)),
);
