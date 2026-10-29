import assert from "node:assert/strict";
import test from "node:test";
import {
  aggregateCliActivity,
  cliUsageStaleAfterMs,
  parseOsc9Progress,
  parseTerminalProgramStatus,
  terminalProgramStatusActivity,
  usageCliActivity,
} from "../src/cli-activity.ts";
import type { AgentUsageEntry } from "../src/agent-usage.ts";

function usageEntry(
  windows: AgentUsageEntry["windows"],
  overrides: Partial<AgentUsageEntry> = {},
): AgentUsageEntry {
  return {
    id: "pane-1",
    process: { cli: "codex", pid: 42 },
    status: "ready",
    windows,
    updatedAt: 10_000,
    retryAt: null,
    message: null,
    source: "test",
    ...overrides,
  };
}

test("aggregateCliActivity picks the strongest pane activity", () => {
  assert.equal(aggregateCliActivity([null, "working", undefined]), "working");
  assert.equal(
    aggregateCliActivity(["attention", "working", null]),
    "attention",
  );
  assert.equal(aggregateCliActivity(["attention", "error"]), "error");
  assert.equal(aggregateCliActivity([null, undefined]), null);
});

test("usageCliActivity reports only fresh confirmed quota exhaustion", () => {
  const now = 10_000 + cliUsageStaleAfterMs;
  assert.equal(
    usageCliActivity(
      usageEntry([
        {
          label: "session",
          remainingPercent: 0,
          used: null,
          limit: null,
          unit: null,
          resetsAt: null,
        },
      ]),
      now,
    ),
    "error",
  );
  assert.equal(
    usageCliActivity(
      usageEntry([
        {
          label: "session",
          remainingPercent: null,
          used: 20,
          limit: 20,
          unit: "requests",
          resetsAt: null,
        },
      ]),
      now,
    ),
    "error",
  );
  assert.equal(
    usageCliActivity(
      usageEntry(
        [
          {
            label: "session",
            remainingPercent: 0,
            used: 0,
            limit: 10,
            unit: null,
            resetsAt: null,
          },
        ],
        { status: "rate-limited" },
      ),
      now,
    ),
    null,
  );
  assert.equal(
    usageCliActivity(
      usageEntry(
        [
          {
            label: "session",
            remainingPercent: 0,
            used: 10,
            limit: 10,
            unit: null,
            resetsAt: 20_000,
          },
        ],
        { updatedAt: now - cliUsageStaleAfterMs - 1 },
      ),
      now,
    ),
    null,
  );
  assert.equal(
    usageCliActivity(
      usageEntry([
        {
          label: "session",
          remainingPercent: 0,
          used: 10,
          limit: 10,
          unit: null,
          resetsAt: now,
        },
      ]),
      now,
    ),
    null,
  );
  assert.equal(
    usageCliActivity(
      usageEntry([
        {
          label: "session",
          remainingPercent: 0,
          used: 10,
          limit: 10,
          unit: null,
          resetsAt: null,
        },
      ]),
      now + 1,
    ),
    null,
  );
});

test("parseOsc9Progress maps supported states and percentages", () => {
  assert.deepEqual(parseOsc9Progress("4;0"), { activity: null });
  assert.deepEqual(parseOsc9Progress("4;0;0"), { activity: null });
  assert.deepEqual(parseOsc9Progress("4;1;0"), { activity: "working" });
  assert.deepEqual(parseOsc9Progress("4;1;100"), { activity: "working" });
  assert.deepEqual(parseOsc9Progress("4;2;53"), { activity: "error" });
  assert.deepEqual(parseOsc9Progress("4;3"), { activity: "working" });
  assert.deepEqual(parseOsc9Progress("4;4;22"), { activity: "attention" });
});

test("parseOsc9Progress ignores malformed and unsupported states", () => {
  for (const value of [
    "",
    "9;4;1;20",
    "4",
    "4;1",
    "4;1;-1",
    "4;1;101",
    "4;2",
    "4;2;invalid",
    "4;3;invalid",
    "4;4",
    "4;4;5;extra",
    "4;5;50",
  ])
    assert.equal(parseOsc9Progress(value), null, value);
});

test("parseTerminalProgramStatus accepts Codex states and clear", () => {
  assert.deepEqual(parseTerminalProgramStatus("state=idle:app=codex"), {
    state: "idle",
    app: "codex",
  });
  assert.deepEqual(parseTerminalProgramStatus("state=working:app=codex"), {
    state: "working",
    app: "codex",
  });
  assert.deepEqual(parseTerminalProgramStatus("state=blocked:app=codex"), {
    state: "blocked",
    app: "codex",
  });
  for (const kind of ["permission", "question", "auth"] as const)
    assert.deepEqual(
      parseTerminalProgramStatus(`state=blocked:app=codex:kind=${kind}`),
      { state: "blocked", app: "codex", kind },
    );
  assert.deepEqual(parseTerminalProgramStatus("state=done:app=codex"), {
    state: "done",
    app: "codex",
  });
  assert.deepEqual(parseTerminalProgramStatus("state=error:app=codex"), {
    state: "error",
    app: "codex",
  });
  assert.deepEqual(parseTerminalProgramStatus("state=clear"), {
    state: "clear",
  });
});

test("terminalProgramStatusActivity maps only active and blocked Codex states", () => {
  for (const state of ["working", "blocked", "error"] as const) {
    const status = parseTerminalProgramStatus(`state=${state}:app=codex`);
    assert.ok(status && status.state !== "clear");
    assert.equal(
      terminalProgramStatusActivity(status),
      state === "working"
        ? "working"
        : state === "blocked"
          ? "attention"
          : "error",
    );
  }

  for (const state of ["idle", "done"] as const) {
    const status = parseTerminalProgramStatus(`state=${state}:app=codex`);
    assert.ok(status && status.state !== "clear");
    assert.equal(terminalProgramStatusActivity(status), null);
  }

  for (const kind of ["permission", "question", "auth"] as const) {
    const status = parseTerminalProgramStatus(
      `state=blocked:app=codex:kind=${kind}`,
    );
    assert.ok(status && status.state !== "clear");
    assert.equal(terminalProgramStatusActivity(status), "attention");
  }
});

test("parseTerminalProgramStatus ignores malformed or unsupported payloads", () => {
  for (const value of [
    "",
    "state=clear:app=codex",
    "state=idle",
    "state=working:app=claude",
    "state=working:app=codex:kind=question",
    "state=blocked:app=codex:kind=unknown",
    "state=blocked:app=codex:kind=auth:extra=value",
    "state=blocked:kind=auth:app=codex",
    "state=working:app=codex:app=codex",
    "state=unknown:app=codex",
    `state=working:app=codex:${"x".repeat(256)}`,
  ])
    assert.equal(parseTerminalProgramStatus(value), null, value);
});
