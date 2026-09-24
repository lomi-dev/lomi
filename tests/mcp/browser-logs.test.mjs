import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import { test } from "node:test";
const script = readFileSync("src-tauri/src/browser/agent-page-logs.js", "utf8");
function fixture() {
  const listeners = {};
  const calls = [];
  const context = vm.createContext({
    TextEncoder,
    TextDecoder,
    location: {
      origin: "https://example.test",
      href: "https://example.test/page",
    },
    console: Object.fromEntries(
      ["log", "info", "warn", "error", "debug"].map((level) => [
        level,
        (...args) => calls.push({ level, args }),
      ]),
    ),
    addEventListener: (kind, listener) => {
      listeners[kind] = listener;
    },
    PromiseRejectionEvent: class {
      constructor(reason, trusted = true) {
        this.reason = reason;
        this.isTrusted = trusted;
      }
    },
  });
  vm.runInContext(script, context);
  const read = (logKind = "console", after = 0, limit = 64, changes = {}) =>
    JSON.parse(
      context.__lomiAgentPageLogsV1(
        JSON.stringify({
          logKind,
          after,
          limit,
          deadlineEpochMs: Date.now() + 3000,
          origin: context.location.origin,
          url: context.location.href,
          ...changes,
        }),
      ),
    );
  return { context, read, calls, listeners };
}
test("console collector preserves calls without reading object properties or coercing", () => {
  const { context, read, calls } = fixture();
  vm.runInContext(
    `globalThis.touched=0; const object={get secret(){touched++;throw Error('getter')},toString(){touched++;throw Error('coercion')}};console.warn('Zażółć 🙂',object,()=>{},null,42,true,undefined);`,
    context,
  );
  assert.equal(context.touched, 0);
  assert.equal(calls.length, 1);
  const batch = read();
  assert.equal(
    batch.entries[0].message,
    "Zażółć 🙂 [object omitted] [object omitted] null 42 true undefined",
  );
  assert.equal(batch.entries[0].level, "warn");
  assert.deepEqual(read("console", batch.through).entries, []);
});
test("collector survives page intrinsic replacement and bounds retention, bytes and UTF-16", () => {
  const { context, read } = fixture();
  vm.runInContext(
    `const message='x'.repeat(10000);Array.prototype.push=Array.prototype.shift=String.prototype.slice=()=>{throw Error('page override')};JSON.stringify=JSON.parse=()=>{throw Error('page JSON')};Date.now=()=>Infinity;for(let i=0;i<90;i++) console.log(message);console.error('edge-'+String.fromCharCode(0xd800));`,
    context,
  );
  const batch = read("console", 0, 10);
  assert.equal(batch.entries.length, 10);
  assert.equal(batch.dropped, 27);
  assert.equal(batch.hasMore, true);
  assert.ok(batch.entries.every((e) => e.message.length === 256));
  const next = read("console", batch.through);
  assert.ok(next.entries[0].sequence > batch.entries.at(-1).sequence);
  assert.equal(next.entries.at(-1).message, "edge-�");
  assert.throws(() =>
    vm.runInContext(
      `Object.defineProperty(globalThis,'__lomiAgentPageLogsV1',{value:()=> 'forged'})`,
      context,
    ),
  );
});
test("rejection reports preserve provenance, primitive-only payloads and deadlines", () => {
  const { context, read, listeners } = fixture();
  const Event = context.PromiseRejectionEvent;
  listeners.unhandledrejection(new Event("real rejection"));
  listeners.unhandledrejection(new Event({ secret: "not collected" }));
  listeners.unhandledrejection(new Event("synthetic rejection", false));
  assert.deepEqual(read().entries, []);
  const batch = read("promise_rejection");
  assert.deepEqual(
    batch.entries.map((e) => e.message),
    ["real rejection", "[object omitted]", "synthetic rejection"],
  );
  assert.deepEqual(
    batch.entries.map((e) => e.eventTrusted),
    [true, true, false],
  );
  assert.equal(read("promise_rejection", 1000).error, "CURSOR_EXPIRED");
  assert.equal(
    read("console", 0, 64, { origin: "https://foreign.test" }).error,
    "STALE_SNAPSHOT",
  );
  assert.equal(
    read("console", 0, 64, { deadlineEpochMs: 0 }).error,
    "DEADLINE_EXCEEDED",
  );
});
