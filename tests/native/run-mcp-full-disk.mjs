import { mkdtemp, mkdir, writeFile, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawn } from "node:child_process";

if (process.platform !== "darwin" || process.arch !== "arm64")
  throw Error("This disk-image qualification is currently macOS ARM64 only.");
const root = resolve(import.meta.dirname, "../..");
const filesystem = process.env.LOMI_MCP_TEST_FILESYSTEM ?? "APFS";
if (!["APFS", "HFS+"].includes(filesystem))
  throw Error("Unsupported test filesystem");
const directory = await mkdtemp(join(tmpdir(), "lomi-mcp-full-disk-"));
const volume = join(directory, "volume");
const image = join(directory, "bounded.dmg");
await mkdir(volume);
console.log(`Full-disk qualification artifacts: ${directory}`);
async function run(command, args, env = process.env) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: root,
      env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let out = "",
      err = "";
    child.stdout.on("data", (data) => {
      out += data;
    });
    child.stderr.on("data", (data) => {
      err += data;
    });
    child.on("error", reject);
    child.on("close", (code) => resolve({ code, out, err }));
  });
}
let mounted = false;
let passed = false;
try {
  const created = await run("/usr/bin/hdiutil", [
    "create",
    "-size",
    "32m",
    "-fs",
    filesystem,
    "-layout",
    "NONE",
    "-volname",
    "Lomi MCP fault fixture",
    "-type",
    "UDIF",
    "-nospotlight",
    image,
  ]);
  await writeFile(join(directory, "create.log"), created.out + created.err);
  if (created.code !== 0)
    throw Error(`Cannot create the isolated volume: ${created.err}`);
  const attached = await run("/usr/bin/hdiutil", [
    "attach",
    "-nobrowse",
    "-noautoopen",
    "-mountpoint",
    volume,
    "-plist",
    image,
  ]);
  await writeFile(join(directory, "attach.plist"), attached.out);
  if (attached.code !== 0)
    throw Error(`Cannot attach the isolated volume: ${attached.err}`);
  mounted = true;
  if ((await stat(volume)).dev === (await stat(directory)).dev)
    throw Error(
      "The isolated volume was not mounted; refusing to run the fill test.",
    );
  await writeFile(
    join(volume, ".lomi-mcp-full-disk"),
    "Owned, bounded 32 MiB test image.\n",
    { flag: "wx", mode: 0o600 },
  );
  const result = await run(
    "cargo",
    [
      "test",
      "--manifest-path",
      "src-tauri/Cargo.toml",
      "--locked",
      "-p",
      "lomi-control-core",
      "--test",
      "full_disk",
      "--",
      "--ignored",
      "--nocapture",
    ],
    { ...process.env, LOMI_MCP_FULL_DISK_DIRECTORY: volume },
  );
  await writeFile(join(directory, "test.log"), result.out + result.err);
  passed = result.code === 0;
  console.log(result.out);
  if (!passed)
    throw Error(
      `Full-disk assertions failed; see ${join(directory, "test.log")}`,
    );
} finally {
  let detached = !mounted;
  if (mounted) {
    const result = await run("/usr/bin/hdiutil", ["detach", volume]);
    await writeFile(join(directory, "detach.log"), result.out + result.err);
    detached = result.code === 0;
  }
  if (passed && detached) await rm(image);
  await writeFile(
    join(directory, "result.json"),
    JSON.stringify(
      { filesystem, passed, detached, imageRemoved: passed && detached },
      null,
      2,
    ),
  );
  if (!detached)
    throw Error(
      `Test volume remains mounted at ${volume}; no forced detach was attempted.`,
    );
}
