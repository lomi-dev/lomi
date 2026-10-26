import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import {
  access,
  copyFile,
  cp,
  mkdir,
  readFile,
  realpath,
  rename,
  writeFile,
} from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { newProject, newSession, openFileTab } from "../../src/model.ts";

if (process.platform !== "darwin")
  throw Error(
    "The external-file native smoke runner currently supports macOS.",
  );

const repository = resolve(import.meta.dirname, "../..");
const args = process.argv.slice(2);
const has = (name) => args.includes(name);
const value = (name) => {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
};
const build = has("--build");
const identifier =
  value("--identifier") ??
  `dev.lomi.external-files-smoke-${Date.now()}-${process.pid}`;
assert.match(identifier, /^dev\.lomi\.external-files-smoke-[a-zA-Z0-9.-]+$/);

const appData = join(homedir(), "Library/Application Support", identifier);
const directoryArg = value("--directory");
const directory = directoryArg
  ? resolve(directoryArg)
  : await import("node:fs/promises").then(({ mkdtemp }) =>
      mkdtemp(join(tmpdir(), "lomi-external-files-native-")),
    );
if (directoryArg) {
  try {
    await mkdir(directory);
  } catch (error) {
    if (error.code !== "EEXIST" || !has("--archive-existing")) throw error;
    const marker = JSON.parse(
      await readFile(join(appData, "external-files-native-smoke.json"), "utf8"),
    );
    assert.equal(marker.identifier, identifier);
    assert.equal(resolve(marker.directory), directory);
    const suffix = `previous-${Date.now()}`;
    const archivedDirectory = `${directory}.${suffix}`;
    const archivedAppData = `${appData}.${suffix}`;
    await rename(appData, archivedAppData);
    await rename(directory, archivedDirectory);
    await mkdir(directory);
    console.log(`Preserved prior smoke artifacts: ${archivedDirectory}`);
    console.log(`Preserved prior smoke app data: ${archivedAppData}`);
  }
}

const log = (stream, label) =>
  stream.on("data", (chunk) => {
    process.stdout.write(`[${label}] ${chunk}`);
  });
const run = (command, commandArgs, options = {}) =>
  new Promise((resolveRun, reject) => {
    const child = spawn(command, commandArgs, {
      cwd: repository,
      stdio: ["ignore", "pipe", "pipe"],
      ...options,
    });
    log(child.stdout, command);
    log(child.stderr, command);
    child.once("error", reject);
    child.once("exit", (code, signal) => {
      if (code === 0) resolveRun({ code, signal });
      else reject(Error(`${command} exited with ${code ?? signal}`));
    });
  });

let app = value("--app") ? resolve(value("--app")) : undefined;
if (build) {
  const configPath = join(directory, "tauri.external-files-smoke.json");
  await writeFile(
    configPath,
    JSON.stringify(
      {
        productName: "Lomi External Files Smoke",
        identifier,
        build: { beforeBuildCommand: "pnpm build" },
        bundle: { createUpdaterArtifacts: false },
      },
      null,
      2,
    ),
  );
  await run("pnpm", [
    "tauri",
    "build",
    "--debug",
    "--bundles",
    "app",
    "--features",
    "native-smoke",
    "--config",
    configPath,
  ]);
  const built = join(
    repository,
    "src-tauri/target/debug/bundle/macos/Lomi External Files Smoke.app",
  );
  await access(built);
  app = join(directory, basename(built));
  await cp(built, app, { recursive: true, preserveTimestamps: true });
  if (has("--build-only")) {
    console.log(`External file smoke app: ${app}`);
    console.log(`External file smoke identifier: ${identifier}`);
    console.log(`External file smoke artifacts: ${directory}`);
    process.exit(0);
  }
}
assert(app, "Supply --app or pass --build.");
await access(app);

const bundleId = await new Promise((resolveId, reject) => {
  const child = spawn(
    "plutil",
    [
      "-extract",
      "CFBundleIdentifier",
      "raw",
      "-o",
      "-",
      join(app, "Contents/Info.plist"),
    ],
    { stdio: ["ignore", "pipe", "pipe"] },
  );
  let output = "";
  let error = "";
  child.stdout.on("data", (chunk) => (output += chunk));
  child.stderr.on("data", (chunk) => (error += chunk));
  child.once("error", reject);
  child.once("exit", (code) =>
    code === 0 ? resolveId(output.trim()) : reject(Error(error.trim())),
  );
});
assert.equal(
  bundleId,
  identifier,
  "The app bundle must use this run's identifier.",
);

for (const path of [appData, join(directory, "project with spaces — żółć")]) {
  try {
    await access(path);
    throw Error(`Refusing to replace existing smoke data: ${path}`);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
}
await mkdir(appData, { recursive: true });
const root = join(directory, "project with spaces — żółć");
const nested = join(root, "nested");
await mkdir(nested, { recursive: true });
await mkdir(join(root, "folder"));
await writeFile(join(root, "seed.txt"), "Restored before OS opens\n");
await writeFile(
  join(root, "name with spaces — żółć.txt"),
  "Dirty original — żółć\n",
);
await writeFile(
  join(root, "unknown extension.odd"),
  "Unknown extension opened as text\n",
);
await writeFile(join(root, ".env"), "TOKEN=smoke\n");
await writeFile(join(root, "README"), "Extensionless text opened\n");
await copyFile(
  join(repository, "src-tauri/icons/128x128.png"),
  join(root, "tiny image.png"),
);
await writeFile(join(nested, "second file.txt"), "Second OS-opened file\n");
await writeFile(join(root, "warm open.txt"), "Warm OS-open file\n");
await writeFile(join(root, "binary.dat"), Buffer.from([0, 255, 0, 1]));

const project = newProject(root, "local:zsh");
const session = openFileTab(
  { ...newSession(), projects: [project], activeProjectId: project.id },
  project.workspaces[0].id,
  root,
  "seed.txt",
);
await writeFile(join(appData, "session.json"), JSON.stringify(session));
await writeFile(
  join(appData, "agent-control-preferences.json"),
  JSON.stringify({ version: 1, autoStart: false, yoloMode: false }),
);
await writeFile(
  join(appData, "external-files-native-smoke.json"),
  JSON.stringify({ identifier, directory }),
);
console.log(`External file native smoke artifacts: ${directory}`);
console.log(`Isolated app data: ${appData}`);
console.log(`Launching app bundle: ${app}`);

const paths = {
  dirty: join(root, "name with spaces — żółć.txt"),
  unknown: join(root, "unknown extension.odd"),
  dotenv: join(root, ".env"),
  extensionless: join(root, "README"),
  image: join(root, "tiny image.png"),
  second: join(nested, "second file.txt"),
  warm: join(root, "warm open.txt"),
  binary: join(root, "binary.dat"),
};

const launch = async (stage, files) => {
  console.log(
    `Opening ${stage} files through macOS Launch Services: ${files.map((path) => basename(path)).join(", ")}`,
  );
  await run("open", ["-a", app, ...files]);
};
const readResult = async (name, timeoutMs = 180_000) => {
  const path = join(directory, name);
  const failurePath = join(directory, "result.json");
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      return JSON.parse(await readFile(path, "utf8"));
    } catch (error) {
      if (error.code !== "ENOENT" && error.name !== "SyntaxError") throw error;
    }
    if (name !== "result.json") {
      try {
        const failure = JSON.parse(await readFile(failurePath, "utf8"));
        if (failure.stage === "failed")
          throw Error(
            `Native external-file smoke failed: ${JSON.stringify(failure.data)}`,
          );
      } catch (error) {
        if (error.code !== "ENOENT" && error.name !== "SyntaxError")
          throw error;
      }
    }
    await new Promise((resolveWait) => setTimeout(resolveWait, 250));
  }
  const diagnostics = {};
  for (const stage of ["page-loaded", "script-started"]) {
    try {
      diagnostics[stage] = JSON.parse(
        await readFile(join(directory, `${stage}.json`), "utf8"),
      );
    } catch {}
  }
  throw Error(
    `Timed out waiting for ${path}; diagnostics=${JSON.stringify(diagnostics)}`,
  );
};

const lsregister =
  "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
let launchServicesRegistered = false;
try {
  const launchServicesOutput = execFileSync(
    "swift",
    [
      join(repository, "tests/native/external-files-launch-services.swift"),
      app,
      identifier,
    ],
    { cwd: repository, encoding: "utf8", maxBuffer: 1024 * 1024 },
  );
  launchServicesRegistered = true;
  const launchServices = JSON.parse(launchServicesOutput);
  await writeFile(
    join(directory, "launch-services.json"),
    JSON.stringify(launchServices, null, 2),
  );
  console.log("Launch Services handler metadata:");
  console.log(JSON.stringify(launchServices, null, 2));
  assert.equal(
    launchServices.defaultsUnchanged,
    true,
    "Alternate registration changed a default handler.",
  );
  assert.equal(
    launchServices.appRegisteredForExpectedTypes,
    true,
    "Smoke app is missing a text or data Open With handler.",
  );

  await launch("cold-start", [
    paths.dirty,
    paths.unknown,
    paths.dotenv,
    paths.extensionless,
    paths.image,
    paths.second,
  ]);
  const cold = await readResult("cold-ready.json", 60_000);
  assert.equal(cold.stage, "cold-ready");
  console.log(JSON.stringify(cold, null, 2));

  await launch("warm-instance", [paths.dirty, paths.warm, paths.binary]);
  const warm = await readResult("warm-ready.json");
  assert.equal(warm.stage, "warm-ready");
  assert.equal(
    warm.pid,
    cold.pid,
    "Warm file open launched a second app process.",
  );
  console.log(JSON.stringify(warm, null, 2));

  const result = await readResult("result.json");
  console.log(JSON.stringify(result, null, 2));
  assert.equal(result.stage, "passed", "Native external-file smoke failed.");
  assert.equal(
    result.pid,
    cold.pid,
    "Final report came from a different app process.",
  );
  console.log("Native external-file smoke passed.");
} finally {
  // The normal guarded quit flow can retain a fixture with intentionally dirty
  // editors. Stop only the executable in this isolated test bundle.
  const executable = await realpath(join(app, "Contents/MacOS/lomi"));
  const processes = execFileSync("ps", ["-ww", "-axo", "pid=,comm="], {
    encoding: "utf8",
  });
  for (const row of processes.split("\n")) {
    const match = /^\s*(\d+)\s+(.+)$/.exec(row);
    if (!match || !match[2].endsWith("/Contents/MacOS/lomi")) continue;
    try {
      if ((await realpath(match[2])) === executable)
        process.kill(Number(match[1]), "SIGTERM");
    } catch (error) {
      if (error.code !== "ENOENT" && error.code !== "ESRCH") throw error;
    }
  }
  if (launchServicesRegistered) {
    await run(lsregister, ["-u", app]);
    console.log(`Unregistered smoke app from Launch Services: ${app}`);
  }
}
