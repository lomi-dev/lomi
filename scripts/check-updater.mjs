import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

export function checkUpdater(tag, manifest) {
  assert.match(tag, /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/);
  assert.equal(manifest.version.replace(/^v/, ""), tag.slice(1));
  for (const target of [
    "darwin-aarch64",
    "darwin-x86_64",
    "windows-x86_64",
    "linux-x86_64",
  ]) {
    const entry = manifest.platforms[target];
    assert.ok(entry, `Missing updater target: ${target}`);
    assert.ok(
      typeof entry.signature === "string" && entry.signature.trim(),
      `Missing signature: ${target}`,
    );
    const url = new URL(entry.url);
    assert.equal(url.origin, "https://github.com");
    assert.ok(
      ["lomi-dev/simplebench", "MaciejKolerski/simplebench"].some(
        (repository) =>
          url.pathname.startsWith(`/${repository}/releases/download/${tag}/`),
      ),
      `Unexpected release URL: ${target}`,
    );
  }
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  checkUpdater(
    process.argv[2],
    JSON.parse(readFileSync(process.argv[3], "utf8")),
  );
  console.log("Updater metadata includes all four signed release targets.");
}
