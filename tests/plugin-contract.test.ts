import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import Ajv from "ajv/dist/2020.js";
import {
  parsePlugin,
  packagePath,
  jsonState,
} from "@lomi-dev/plugin-sdk/manifest";
import { validShortcut } from "@lomi-dev/plugin-sdk/shortcuts";
const cases = JSON.parse(
  readFileSync(
    new URL("./fixtures/plugin-contract.json", import.meta.url),
    "utf8",
  ),
);
const schema = JSON.parse(
  readFileSync(
    new URL(import.meta.resolve("@lomi-dev/plugin-sdk/plugin.schema.json")),
    "utf8",
  ),
);
const validateSchema = new Ajv({ strict: false }).compile(schema);
for (const item of cases)
  test(`shared contract: ${item.name}`, () => {
    const run = () =>
      item.kind === "manifest"
        ? parsePlugin(item.input)
        : item.kind === "path"
          ? packagePath(item.input)
          : assert.ok(validShortcut(item.input));
    if (item.valid) assert.doesNotThrow(run);
    else assert.throws(run);
    if (item.kind === "manifest")
      assert.equal(
        validateSchema(item.input),
        item.schemaValid,
        JSON.stringify(validateSchema.errors),
      );
  });
test("state boundaries reject cycles, excessive nodes, bytes and reserved fields", () => {
  jsonState({ items: [null, 1, "żółć"], visible: true });
  jsonState("a".repeat(65534));
  assert.throws(() => jsonState("a".repeat(65535)));
  assert.throws(() => jsonState(Array(4096).fill(0)));
  const cyclic: any = {};
  cyclic.self = cyclic;
  assert.throws(() => jsonState(cyclic));
  assert.throws(() => jsonState(JSON.parse('{"__proto__":{}}')));
  assert.throws(() => jsonState(new Date()));
});
