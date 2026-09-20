import { test } from "node:test";
import assert from "node:assert/strict";
import { checkUpdater } from "../scripts/check-updater.mjs";

test("updater publication requires matching versions, all platforms, signatures, and release URLs", () => {
  const manifest = {
    version: "0.2.0",
    platforms: Object.fromEntries(
      ["darwin-aarch64", "darwin-x86_64", "windows-x86_64", "linux-x86_64"].map(
        (target) => [
          target,
          {
            signature: "signed-artifact",
            url: `https://github.com/lomi-dev/simplebench/releases/download/v0.2.0/${target}.tar.gz`,
          },
        ],
      ),
    ),
  };
  checkUpdater("v0.2.0", manifest);
  const legacy = structuredClone(manifest);
  for (const entry of Object.values(legacy.platforms)) {
    entry.url = entry.url.replace("/lomi-dev/", "/MaciejKolerski/");
  }
  checkUpdater("v0.2.0", legacy);
  assert.throws(() => checkUpdater("v0.1.0", manifest));
  assert.throws(() => checkUpdater("v0.2.0-beta.1", manifest));
  for (const target of Object.keys(manifest.platforms)) {
    const missing = structuredClone(manifest);
    delete missing.platforms[target];
    assert.throws(() => checkUpdater("v0.2.0", missing));
    for (const change of [
      { signature: " " },
      {
        url: "http://github.com/MaciejKolerski/simplebench/releases/download/v0.2.0/file",
      },
      {
        url: "http://github.com/lomi-dev/simplebench/releases/download/v0.2.0/file",
      },
      { url: "https://github.com/other/app/releases/download/v0.2.0/file" },
      {
        url: "https://github.com/lomi-dev/other/releases/download/v0.2.0/file",
      },
      {
        url: "https://github.com/lomi-dev/simplebench/releases/download/v0.1.0/file",
      },
      {
        url: "https://github.com/MaciejKolerski/simplebench/releases/download/v0.1.0/file",
      },
    ]) {
      const invalid = structuredClone(manifest);
      Object.assign(invalid.platforms[target], change);
      assert.throws(() => checkUpdater("v0.2.0", invalid));
    }
  }
});
