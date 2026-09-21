import assert from "node:assert/strict";
import { readFile, writeFile, lstat } from "node:fs/promises";
import { resolve } from "node:path";
import { compatibility } from "@lomi-dev/plugin-sdk/compatibility";

const root = resolve(import.meta.dirname, "..");
const source = new URL(
  import.meta.resolve("@lomi-dev/plugin-sdk/contract-fixtures.json"),
);
const snapshot = resolve(root, "tests/fixtures/plugin-contract.json");
const bytes = await readFile(source);
if (process.argv.includes("--sync")) await writeFile(snapshot, bytes);
assert.deepEqual(
  await readFile(snapshot),
  bytes,
  "Rust contract fixtures differ from the installed SDK. Review the SDK update, then run pnpm sdk:sync-contract and both TS/Rust tests.",
);
assert.equal(compatibility.schemaVersion, 1);
assert.equal(compatibility.hostApi, 1);
assert.equal(compatibility.runtimeSymbol, "simplebench.plugin-api.v1");
const metadata = JSON.parse(
  await readFile(
    new URL(import.meta.resolve("@lomi-dev/plugin-sdk/package.json")),
    "utf8",
  ),
);
assert.equal(metadata.version, compatibility.sdk);
const application = JSON.parse(
  await readFile(resolve(root, "package.json"), "utf8"),
);
const fixture = JSON.parse(
  await readFile(
    resolve(root, "tests/fixtures/context-plugin/package.json"),
    "utf8",
  ),
);
const spec = application.dependencies[metadata.name];
assert.equal(
  fixture.dependencies[metadata.name],
  spec,
  "The app and its fixture must pin the same SDK.",
);
const archiveURL = `https://github.com/lomi-dev/plugin-sdk/releases/download/v${metadata.version}/lomi-dev-plugin-sdk-${metadata.version}.tgz`;
const candidate =
  process.env.LOMI_SDK_TARBALL &&
  spec === `file:${resolve(process.env.LOMI_SDK_TARBALL)}`;
assert.ok(
  spec === metadata.version || spec === archiveURL || candidate,
  "Pin an exact SDK version or qualified release URL. Local archives are allowed only with an explicit LOMI_SDK_TARBALL override.",
);
await assert.rejects(lstat(resolve(root, "packages/plugin-sdk")), {
  code: "ENOENT",
});
console.log(
  `Installed SDK ${metadata.name}@${metadata.version}: host API, runtime identity and Rust fixture snapshot verified.`,
);
