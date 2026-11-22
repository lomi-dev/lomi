import assert from "node:assert/strict";
import test from "node:test";
import { githubOwner } from "./workspace-logo.ts";
import { sanitizeWorkspaceAppearance } from "./workspace-appearance.ts";

test("githubOwner parses github.com remotes only", () => {
  for (const url of [
    "https://github.com/acme/app.git",
    "https://github.com/acme/app",
    "https://user@github.com/acme/app.git",
    "git@github.com:acme/app.git",
    "git@github.com:acme/app",
    "ssh://git@github.com/acme/app.git",
  ])
    assert.equal(githubOwner(url), "acme", url);
  for (const url of [
    "https://gitlab.com/acme/app.git",
    "git@example.com:acme/app.git",
    "https://github.com.evil.test/acme/app",
    "https://github.com/acme",
    "",
  ])
    assert.equal(githubOwner(url), null, url);
});

test("workspace appearance keeps only valid logo ids", () => {
  const id = `${"a1".repeat(16)}.png`;
  assert.deepEqual(sanitizeWorkspaceAppearance({ logo: id }), { logo: id });
  for (const logo of ["../x.png", "abc.png", `${"a1".repeat(16)}.gif`, 5])
    assert.equal(sanitizeWorkspaceAppearance({ logo }), undefined);
});
