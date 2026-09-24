// Reject source or executable changes before reusing a batch's native build.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { join } from "node:path";

export async function sourceFingerprint(root) {
  const inventory = spawnSync(
    "git",
    [
      "ls-files",
      "--cached",
      "--others",
      "--exclude-standard",
      "-z",
      "--",
      "src",
      "src-tauri",
      "tests/native",
      "tests/mcp",
      "packages",
      "scripts",
      "package.json",
      "pnpm-lock.yaml",
      "vite.config.ts",
    ],
    { cwd: root, encoding: "utf8" },
  );
  if (inventory.status !== 0) throw Error("Cannot inventory batch sources");
  const digest = createHash("sha256");
  for (const path of [
    ...new Set(inventory.stdout.split("\0").filter(Boolean)),
  ].sort()) {
    digest.update(path + "\0");
    try {
      digest.update(await readFile(join(root, path)));
    } catch (error) {
      if (error.code === "ENOENT") digest.update("deleted");
      else throw error;
    }
  }
  return digest.digest("hex");
}

export async function binaryFingerprint(root) {
  return createHash("sha256")
    .update(await readFile(join(root, "src-tauri/target/debug/lomi")))
    .digest("hex");
}
