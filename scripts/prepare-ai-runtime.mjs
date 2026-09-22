import { createHash } from "node:crypto";
import {
  mkdir,
  readFile,
  writeFile,
  copyFile,
  chmod,
  mkdtemp,
  rm,
} from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";
const root = fileURLToPath(new URL("../", import.meta.url));
const manifest = JSON.parse(
  await readFile(
    path.join(root, "packages/ai-runtime/node-artifacts.json"),
    "utf8",
  ),
);
const host = {
  "darwin-arm64": "aarch64-apple-darwin",
  "darwin-x64": "x86_64-apple-darwin",
  "linux-x64": "x86_64-unknown-linux-gnu",
  "win32-x64": "x86_64-pc-windows-msvc",
}[`${process.platform}-${process.arch}`];
const target =
  process.env.TAURI_ENV_TARGET_TRIPLE || process.env.TARGET || host;
const artifact = manifest.targets[target];
if (!artifact) throw new Error(`Unsupported AI runtime target: ${target}`);
const cache = path.join(root, "src-tauri/target/ai-runtime");
const binaries = path.join(root, "src-tauri/binaries");
const resources = path.join(root, "src-tauri/resources/ai-runtime");
await Promise.all(
  [cache, binaries, resources].map((p) => mkdir(p, { recursive: true })),
);
const archivePath = path.join(cache, artifact.archive);
const digest = (data) => createHash("sha256").update(data).digest("hex");
let archive = await readFile(archivePath).catch(() => null);
if (!archive || digest(archive) !== artifact.sha256) {
  const response = await fetch(manifest.origin + artifact.archive, {
    signal: AbortSignal.timeout(120_000),
  });
  if (!response.ok)
    throw new Error("Could not download the pinned Node runtime");
  archive = Buffer.from(await response.arrayBuffer());
  if (digest(archive) !== artifact.sha256)
    throw new Error("Node archive checksum mismatch");
  await writeFile(archivePath, archive);
}
const scratch = await mkdtemp(path.join(cache, "extract-"));
try {
  execFileSync("tar", ["-xf", archivePath, "-C", scratch], { stdio: "pipe" });
  const name = artifact.archive.replace(/\.tar\.gz$|\.zip$/, "");
  const windows = target.includes("windows");
  const source = path.join(scratch, name, windows ? "node.exe" : "bin/node");
  const destination = path.join(
    binaries,
    `lomi-node-${target}${windows ? ".exe" : ""}`,
  );
  await copyFile(source, destination);
  if (!windows) await chmod(destination, 0o755);
  await copyFile(
    path.join(scratch, name, "LICENSE"),
    path.join(resources, "NODE-LICENSE"),
  );
  await writeFile(
    path.join(resources, "node.json"),
    JSON.stringify({
      version: manifest.version,
      target,
      sha256: digest(await readFile(source)),
    }) + "\n",
  );
} finally {
  await rm(scratch, { recursive: true, force: true });
}
execFileSync(
  process.execPath,
  [path.join(root, "packages/ai-runtime/build.mjs"), ...process.argv.slice(2)],
  { cwd: root, stdio: "inherit" },
);
console.log(`AI runtime staged: Node ${manifest.version}, ${target}`);
