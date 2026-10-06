import assert from "node:assert/strict";
import test from "node:test";
import {
  acceptTask,
  executionRequest,
  finalTaskIds,
  hasHistoryGrant,
  isRunning,
} from "../src/agent-runtime/model.ts";
import { newAgentTaskTab } from "../src/model.ts";
import type { AccountInstance, Task } from "../src/agent-runtime/types.ts";
const account = (accountId: string, authRevision = 1) =>
  ({ accountId, authRevision }) as AccountInstance;
const task = {
  taskId: "task",
  revision: 9,
  activeAccountId: "a",
  nextAccountId: "b",
} as Task;
test("Send and Continue explicitly retain B after A, and support A-B-A / A-B-C", () => {
  for (const id of ["b", "a", "c"])
    for (const continuing of [false, true]) {
      const request = executionRequest(
        task,
        account(id, 4),
        "operation",
        "draft",
        continuing,
      );
      assert.equal(request.accountId, id);
      assert.equal(request.authRevision, 4);
      assert.equal(request.continue, continuing);
      assert.equal(request.taskId, "task");
    }
});
test("grant survives later history and binds exact login revision", () => {
  const grants = [{ accountId: "b", authRevision: 2, revision: 1 }];
  assert.equal(hasHistoryGrant(grants, account("b", 2)), true);
  assert.equal(hasHistoryGrant(grants, account("b", 3)), false);
  assert.equal(hasHistoryGrant(grants, account("a", 2)), false);
});
test("two cards and mixed pane references fence only removal of final domain view", () => {
  const a = newAgentTaskTab("task"),
    b = newAgentTaskTab("task"),
    c = newAgentTaskTab("other"),
    legacy = newAgentTaskTab("legacy:old");
  assert.deepEqual(finalTaskIds([a, b, c, legacy], new Set([a.id])), []);
  assert.deepEqual(finalTaskIds([a, b, c, legacy], new Set([a.id, b.id])), [
    "task",
  ]);
  assert.deepEqual(finalTaskIds([a, b, c, legacy]), [
    "task",
    "other",
    "legacy:old",
  ]);
});
test("stale reconnect snapshots cannot overwrite newer task attempt", () => {
  const newer = { ...task, revision: 10, activeAccountId: "b" };
  assert.equal(acceptTask(newer, task), newer);
  assert.equal(acceptTask(task, newer), newer);
  for (const state of ["starting", "running", "stopping"] as const)
    assert.equal(isRunning(state), true);
  for (const state of [
    "prepared",
    "recovery_required",
    "delivery_uncertain",
    "completed",
  ] as const)
    assert.equal(isRunning(state), false);
});

test("native permissions preserve protocol-specific allow and deny decisions", async () => {
  const { permissionControls } =
    await import("../src/agent-runtime/permission-model.ts");
  const make = (choices: unknown[], raw: unknown) =>
    ({
      permission: { choices, raw },
    }) as import("../src/agent-runtime/types.ts").PendingPermission;
  for (const [choices, expected] of [
    [["accept", "decline", "cancel"], "decline"],
    [["allow", "deny"], "deny"],
    [["approved", "rejected"], "rejected"],
    [["once", "always", "reject"], "reject"],
  ] as const) {
    const controls = permissionControls(make([...choices], {}));
    assert.equal(controls.cancel?.value, expected);
    assert.equal(controls.cancel?.allow, false);
  }
  const grok = permissionControls(
    make(
      [
        { optionId: "yes", kind: "allow_once", name: "Allow" },
        { optionId: "no", kind: "reject_once", name: "Deny" },
      ],
      {},
    ),
  );
  assert.deepEqual(
    grok.choices.map((choice) => [choice.value, choice.allow]),
    [
      ["yes", true],
      ["no", false],
    ],
  );
  const pi = permissionControls(
    make([], { type: "extension_ui_request", method: "confirm" }),
  );
  assert.equal(pi.cancel?.value, false);
  const select = permissionControls(
    make(["cancel", "reject", "decline"], {
      type: "extension_ui_request",
      method: "select",
    }),
  );
  assert.deepEqual(
    select.choices.map((choice) => [choice.value, choice.allow]),
    [
      [null, false],
      ["cancel", true],
      ["reject", true],
      ["decline", true],
    ],
  );
  assert.equal(select.cancel?.value, null);
  for (const method of ["input", "editor"]) {
    const controls = permissionControls(
      make([], { type: "extension_ui_request", method, title: "Response" }),
    );
    assert.equal(controls.cancel?.value, null);
    assert.equal(controls.textInput?.multiline, method === "editor");
  }
});
