// Builds lomi-stt-worker and places it where Tauri's `externalBin` expects it:
// src-tauri/binaries/lomi-stt-worker-<target triple>[.exe].
// Requires CMake and a C++ toolchain (transcribe-cpp compiles ggml).
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const crate = path.join(root, "src-tauri", "crates", "lomi-stt-worker");
const targetDir = path.join(root, "src-tauri", "target", "stt-worker");
const triple =
  process.env.TAURI_ENV_TARGET_TRIPLE ||
  /host: (\S+)/.exec(execFileSync("rustc", ["-vV"], { encoding: "utf8" }))[1];

const env = { ...process.env, CARGO_TARGET_DIR: targetDir };
// transcribe-cpp-sys tunes ggml to the build machine outside macOS; a shipped
// binary must run on any x86-64-v3 CPU (the worker checks for it at load).
if (!triple.includes("apple")) {
  env.TRANSCRIBE_CMAKE_ARGS = [env.TRANSCRIBE_CMAKE_ARGS, "-DGGML_NATIVE=OFF"]
    .filter(Boolean)
    .join(" ");
}

execFileSync("cargo", ["build", "--release", "--locked", "--target", triple], {
  cwd: crate,
  env,
  stdio: "inherit",
});

const exe = triple.includes("windows") ? ".exe" : "";
const binaries = path.join(root, "src-tauri", "binaries");
mkdirSync(binaries, { recursive: true });
const output = path.join(binaries, `lomi-stt-worker-${triple}${exe}`);
copyFileSync(
  path.join(targetDir, triple, "release", `lomi-stt-worker${exe}`),
  output,
);
console.log(`lomi-stt-worker -> ${path.relative(root, output)}`);
