import assert from "node:assert/strict";
import test from "node:test";
import { cliNames, type CliAgent } from "../src/cli-agents.ts";
import { cliTerminalDisplayTitle } from "../src/cli-terminal-title.ts";

const conversationId = "01a17375-9413-7ea1-9596-c3eca0271a4f";

test("verified CLI identities use their configured names for missing titles", () => {
  for (const cli of Object.keys(cliNames) as CliAgent[]) {
    assert.equal(
      cliTerminalDisplayTitle(null, "opaque-process", cli),
      cliNames[cli],
    );
  }
});

test("verified CLI identities replace bare UUID and version titles", () => {
  assert.equal(
    cliTerminalDisplayTitle(conversationId, "codex", "codex"),
    cliNames.codex,
  );
  assert.equal(
    cliTerminalDisplayTitle("2.1.287", "claude", "claude"),
    cliNames.claude,
  );
  assert.equal(
    cliTerminalDisplayTitle("v2.1.287", "/usr/local/bin/claude", "claude"),
    cliNames.claude,
  );
});

test("verified CLI identities replace command aliases and executable paths", () => {
  assert.equal(cliTerminalDisplayTitle("codex", null, "codex"), cliNames.codex);
  assert.equal(
    cliTerminalDisplayTitle("/opt/homebrew/bin/claude", null, "claude"),
    cliNames.claude,
  );
  assert.equal(
    cliTerminalDisplayTitle(null, "C:\\Users\\me\\bin\\codex.exe", "codex"),
    cliNames.codex,
  );
  assert.equal(
    cliTerminalDisplayTitle("Claude Code 2.1.287", null, "claude"),
    cliNames.claude,
  );
});

test("verified CLI identities preserve meaningful conversation titles", () => {
  assert.equal(
    cliTerminalDisplayTitle(
      `${conversationId} — restore workspace`,
      null,
      "codex",
    ),
    `${conversationId} — restore workspace`,
  );
  assert.equal(
    cliTerminalDisplayTitle("Release 2.1.287 notes", null, "claude"),
    "Release 2.1.287 notes",
  );
  assert.equal(
    cliTerminalDisplayTitle(
      "Claude Code 2.1.287 upgrade notes",
      null,
      "claude",
    ),
    "Claude Code 2.1.287 upgrade notes",
  );
  assert.equal(
    cliTerminalDisplayTitle("/workspace/src/codex", null, "codex"),
    "/workspace/src/codex",
  );
});

test("non-CLI terminal labels keep their title and foreground fallback", () => {
  assert.equal(cliTerminalDisplayTitle("Local shell", "zsh"), "Local shell");
  assert.equal(cliTerminalDisplayTitle(null, "zsh"), "zsh");
  assert.equal(cliTerminalDisplayTitle(null, null), null);
});
