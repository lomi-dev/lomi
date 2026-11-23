import assert from "node:assert/strict";
import test from "node:test";
import { sessionAccess } from "./agent-control-access";

test("groups scopes into pairing areas with change level", () => {
  const areas = sessionAccess([
    "workspace.read",
    "files.read",
    "git.read",
    "git.write",
    "browser.navigate",
  ]);
  assert.deepEqual(
    areas.map(({ label, changes }) => [label, changes]),
    [
      ["Files & editor", false],
      ["Git", true],
      ["Browser", false],
    ],
  );
});

test("default workspace metadata is not an extra area", () => {
  assert.deepEqual(sessionAccess(["workspace.read"]), []);
});

test("unknown scopes in a known area count as changes", () => {
  assert.equal(sessionAccess(["terminal.execute"])[0]?.changes, true);
});
