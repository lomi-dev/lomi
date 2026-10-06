// Build only the reviewed public source. This never launches Grok or installs it
// globally. A new build's hash requires a separately reviewed manifest update.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmod,
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const pin = "2bdd1d6a6369de0e8c68132ea4539e9abd9e14a8";
const revision = "559751fdcec02d413e4c57c8832ab275e4f44980";
const version = "1.0.45";
const host = {
  "darwin-arm64": "aarch64-apple-darwin",
  "linux-x64": "x86_64-unknown-linux-gnu",
  "linux-arm64": "aarch64-unknown-linux-gnu",
}[`${process.platform}-${process.arch}`];
if (!host)
  throw new Error("The public Grok build only supports reviewed native hosts");
const project = fileURLToPath(new URL("../", import.meta.url));
const scratch = await mkdtemp(path.join(os.tmpdir(), "lomi-grok-source-"));
const home = path.join(scratch, "home");
const cargo = path.join(scratch, "cargo");
const rustup = path.join(scratch, "rustup");
await Promise.all([home, cargo, rustup].map((p) => mkdir(p, { mode: 0o700 })));
// Do not forward credentials, HOME/configuration, compiler flags or version overrides.
const env = {
  HOME: home,
  PATH:
    process.platform === "darwin"
      ? "/usr/bin:/bin:/usr/sbin:/sbin"
      : "/usr/bin:/bin",
  CARGO_HOME: cargo,
  RUSTUP_HOME: rustup,
  CARGO_BUILD_JOBS: "2",
  CARGO_INCREMENTAL: "0",
};
if (process.platform === "darwin") {
  // Select an SDK explicitly when the host's default SDK and linker differ.
  // This sole compiler setting is intentional; credentials remain excluded.
  if (process.env.SDKROOT) {
    if (!path.isAbsolute(process.env.SDKROOT))
      throw new Error("SDKROOT must be an absolute installed SDK path");
    env.SDKROOT = process.env.SDKROOT;
  }
  env.MACOSX_DEPLOYMENT_TARGET = "13.0";
}
function run(program, args, cwd = scratch, capture = false) {
  const result = spawnSync(program, args, {
    cwd,
    env,
    encoding: "utf8",
    stdio: capture ? "pipe" : "inherit",
  });
  if (result.error || result.status !== 0)
    throw new Error(
      `Build command failed: ${program}: ${result.error || result.stderr || result.status}`,
    );
  return result.stdout?.trim();
}
// Resolve rustup before replacing PATH; it only writes into the private homes above.
const rustupExecutable = spawnSync("/usr/bin/which", ["rustup"], {
  encoding: "utf8",
}).stdout.trim();
if (!path.isAbsolute(rustupExecutable))
  throw new Error("Install rustup before building the pinned Grok source");
run(rustupExecutable, [
  "toolchain",
  "install",
  "1.94.0",
  "--profile",
  "minimal",
  "--no-self-update",
]);
env.PATH = `${path.join(rustup, "toolchains", `1.94.0-${host}`, "bin")}:${env.PATH}`;
const source = path.join(scratch, "source");
run("git", [
  "clone",
  "--filter=blob:none",
  "--no-checkout",
  "https://github.com/xai-org/grok-build.git",
  source,
]);
run("git", ["checkout", "--detach", pin], source);
if (
  run("git", ["rev-parse", "HEAD"], source, true) !== pin ||
  (await readFile(path.join(source, "SOURCE_REV"), "utf8")).trim() !== revision
)
  throw new Error("Grok source provenance mismatch");
const platform = {
  "aarch64-apple-darwin": "macos-aarch64",
  "x86_64-unknown-linux-gnu": "linux-x86_64",
  "aarch64-unknown-linux-gnu": "linux-aarch64",
}[host];
const protocSpec = JSON.parse(
  (await readFile(path.join(source, "bin/protoc"), "utf8"))
    .split("\n")
    .slice(1)
    .join("\n"),
).platforms[platform];
const archive = Buffer.from(
  await (await fetch(protocSpec.providers[0].url)).arrayBuffer(),
);
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
if (archive.length !== protocSpec.size || digest(archive) !== protocSpec.digest)
  throw new Error("Pinned protoc checksum mismatch");
const archivePath = path.join(scratch, "protoc.zip");
await writeFile(archivePath, archive, { mode: 0o600 });
run("unzip", ["-q", archivePath, "-d", path.join(scratch, "protoc")]);
env.PROTOC = path.join(scratch, "protoc", protocSpec.path);
run(
  "cargo",
  ["build", "--locked", "-p", "xai-grok-pager-bin", "--release"],
  source,
);
const binary = path.join(source, "target/release/xai-grok-pager");
const record = {
  target: host,
  filename: `grok-${host}`,
  version,
  publicCommit: pin,
  sourceRevision: revision,
  rustToolchain: "1.94.0",
  sha256: digest(await readFile(binary)),
};
// Stage a review candidate, never mutate the compile-owned trust manifest.
const output = path.join(project, "src-tauri/resources/grok");
await mkdir(output, { recursive: true, mode: 0o755 });
const trusted = JSON.parse(
  await readFile(
    path.join(project, "scripts/agent-grok-artifacts.json"),
    "utf8",
  ),
);
const admitted = trusted.artifacts.find(
  (entry) => entry.target === host && entry.sha256 === record.sha256,
);
const destination = path.join(
  output,
  admitted ? record.filename : `${record.filename}.candidate`,
);
await copyFile(binary, destination);
await chmod(destination, 0o755);
for (const [from, to] of [
  ["LICENSE", "LICENSE"],
  ["THIRD-PARTY-NOTICES", "THIRD-PARTY-NOTICES"],
  ["crates/codegen/xai-grok-tools/THIRD_PARTY_NOTICES.md", "TOOLS-NOTICES"],
  ["third_party/NOTICE", "VENDORED-NOTICE"],
  ["third_party/mermaid-to-svg/THIRD_PARTY_NOTICES", "MERMAID-NOTICES"],
])
  await copyFile(path.join(source, from), path.join(output, to));
await writeFile(
  path.join(scratch, "artifact.json"),
  `${JSON.stringify(record, null, 2)}\n`,
);
console.log(
  `Grok source build staged: ${destination}\nSHA256: ${record.sha256}\nBuild provenance: ${scratch}/artifact.json`,
);
if (!admitted)
  console.log(
    "The candidate is unavailable to Lomi until its exact hash is reviewed and added to the compile-owned manifest.",
  );
