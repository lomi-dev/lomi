import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const root = new URL("../", import.meta.url);
const read = (file) =>
  readFileSync(new URL(file, root), "utf8").replaceAll("\r\n", "\n");
const tag = process.argv[2];
assert.match(
  tag ?? "",
  /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/,
  "Expected a stable release tag such as v0.1.0",
);
const version = tag.slice(1);
const packageMetadata = JSON.parse(read("package.json"));
const cargoVersion = read("src-tauri/Cargo.toml").match(
  /^version = "([^"]+)"$/m,
)?.[1];
const lockVersion = read("src-tauri/Cargo.lock").match(
  /\[\[package\]\]\nname = "lomi"\nversion = "([^"]+)"/,
)?.[1];
for (const [file, actual] of [
  ["package.json", packageMetadata.version],
  [
    "src-tauri/tauri.conf.json",
    JSON.parse(read("src-tauri/tauri.conf.json")).version,
  ],
  ["src-tauri/Cargo.toml", cargoVersion],
  ["src-tauri/Cargo.lock", lockVersion],
]) {
  assert.equal(
    actual,
    version,
    `${fileURLToPath(new URL(file, root))} must match ${tag}`,
  );
}
assert.match(
  packageMetadata.license ?? "",
  /^[A-Za-z0-9][A-Za-z0-9.+ ()-]*$/,
  "Choose the project license and set package.json > license before publishing",
);
assert.equal(
  read("src-tauri/Cargo.toml").match(/^license = "([^"]+)"$/m)?.[1],
  packageMetadata.license,
  "Cargo.toml and package.json must declare the same license",
);
assert.ok(
  existsSync(new URL("LICENSE", root)),
  "Add the chosen LICENSE before publishing",
);
console.log(`Release ${tag}: package versions and license are ready.`);
