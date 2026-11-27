import assert from "node:assert/strict";
import test from "node:test";
import {
  NativeCoreClient,
  type NativeCoreDeps,
  type NativeCoreEvent,
  type NativeCoreState,
} from "./remote-core-native.ts";

const host = (online: boolean, epoch = "1") => ({
  id: "h1",
  name: null,
  online,
  accessEpoch: epoch,
  runtimeGeneration: "1",
  workspaces: [],
});
const state = (patch: Partial<NativeCoreState>): NativeCoreState => ({
  status: "session",
  hosts: [],
  fromMemory: false,
  refreshing: false,
  fresh: false,
  degraded: false,
  ...patch,
});

function setup(enabled = true) {
  const calls: string[] = [];
  const marks: string[] = [];
  let emit: (event: NativeCoreEvent) => void = () => {};
  const deps: NativeCoreDeps = {
    start: async (onEvent, account) => {
      calls.push(`start:${account}`);
      emit = onEvent;
      return enabled;
    },
    names: async () => void calls.push("names"),
    renew: async () => void calls.push("renew"),
    wipe: async () => void calls.push("wipe"),
    mark: (name) => void marks.push(name),
  };
  return {
    client: new NativeCoreClient(deps),
    calls,
    marks,
    emit: (e: NativeCoreEvent) => emit(e),
  };
}

test("fails closed until Rust speaks and when the core is not configured", async () => {
  const { client } = setup(false);
  assert.equal(client.getState().degraded, true);
  await client.connect("acc");
  assert.equal(client.getState().degraded, true);
  assert.deepEqual(client.getState().hosts, []);
});

test("mirrors Rust state and signals availability changes except for cache snapshots", async () => {
  const { client, emit } = setup();
  let changes = 0;
  client.onAvailabilityChange(() => changes++);
  await client.connect("acc");
  emit({
    type: "state",
    state: state({ status: "cached", fromMemory: true, hosts: [host(true)] }),
  });
  assert.equal(changes, 0);
  emit({ type: "state", state: state({ hosts: [host(false)] }) });
  assert.equal(changes, 1);
  emit({
    type: "state",
    state: state({ hosts: [{ ...host(false), name: "renamed" }] }),
  });
  assert.equal(changes, 1, "names do not change availability");
  assert.equal(client.getState().hosts[0]?.name, "renamed");
});

test("forwards timing marks and asks Rust to renew instead of restarting", async () => {
  const { client, calls, marks, emit } = setup();
  await client.connect("acc");
  await client.connect("acc");
  emit({ type: "mark", name: "link-ready" });
  assert.deepEqual(calls, ["start:acc", "renew"]);
  assert.deepEqual(marks, ["link-ready"]);
});

test("wipe reaches Rust and allows a later start", async () => {
  const { client, calls } = setup();
  await client.connect("acc");
  await client.wipe();
  await client.connect("acc");
  assert.deepEqual(calls, ["start:acc", "wipe", "start:acc"]);
});
