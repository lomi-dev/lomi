import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  mkdtempSync,
  mkdirSync,
  copyFileSync,
  writeFileSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

test("release preparation rejects mismatched versions, unsafe tags, and missing license decisions", () => {
  const root = mkdtempSync(path.join(tmpdir(), "lomi-release-test-"));
  try {
    mkdirSync(path.join(root, "scripts"));
    mkdirSync(path.join(root, "src-tauri"));
    const script = path.join(root, "scripts/check-release.mjs");
    copyFileSync(
      new URL("../scripts/check-release.mjs", import.meta.url),
      script,
    );
    const write = (file: string, content: string) =>
      writeFileSync(path.join(root, file), content);
    const run = (tag: string) =>
      spawnSync(process.execPath, [script, tag], { encoding: "utf8" });
    const manifests = (lineEnding = "\n") => {
      write(
        "package.json",
        JSON.stringify({ version: "0.1.0", license: "MIT" }),
      );
      write("src-tauri/tauri.conf.json", JSON.stringify({ version: "0.1.0" }));
      write(
        "src-tauri/Cargo.toml",
        '[package]\nname = "lomi"\nversion = "0.1.0"\nlicense = "MIT"\n'.replaceAll(
          "\n",
          lineEnding,
        ),
      );
      write(
        "src-tauri/Cargo.lock",
        '[[package]]\nname = "dependency"\nversion = "9.9.9"\n\n[[package]]\nname = "lomi"\nversion = "0.1.0"\n'.replaceAll(
          "\n",
          lineEnding,
        ),
      );
      write("LICENSE", "License fixture for the release-readiness check.\n");
    };
    for (const lineEnding of ["\n", "\r\n"]) {
      manifests(lineEnding);
      const result = run("v0.1.0");
      assert.equal(result.status, 0, result.stderr);
    }
    for (const tag of [
      "v0.1.1",
      "0.1.0",
      "v00.1.0",
      "v0.1.0-beta.1",
      "v0.1.0;echo unsafe",
    ]) {
      assert.notEqual(run(tag).status, 0, tag);
    }
    for (const file of [
      "package.json",
      "src-tauri/tauri.conf.json",
      "src-tauri/Cargo.toml",
      "src-tauri/Cargo.lock",
    ]) {
      manifests();
      write(
        file,
        file.endsWith("json")
          ? JSON.stringify({ version: "0.2.0", license: "MIT" })
          : '[[package]]\nname = "lomi"\nversion = "0.2.0"\nlicense = "MIT"\n',
      );
      assert.notEqual(run("v0.1.0").status, 0, file);
    }
    manifests();
    write("package.json", JSON.stringify({ version: "0.1.0" }));
    assert.match(run("v0.1.0").stderr, /Choose the project license/);
    manifests();
    write(
      "package.json",
      JSON.stringify({ version: "0.1.0", license: "Apache-2.0" }),
    );
    assert.match(run("v0.1.0").stderr, /same license/);
    manifests();
    rmSync(path.join(root, "LICENSE"));
    assert.match(run("v0.1.0").stderr, /Add the chosen LICENSE/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
