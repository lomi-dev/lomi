import assert from "node:assert/strict";
import test from "node:test";

// The module calls Tauri through window.__TAURI_INTERNALS__ and webview storage.
const store = new Map<string, string>();
let file: { done: boolean; workMode?: string } = { done: false };
const saves: unknown[] = [];
Object.assign(globalThis, {
  localStorage: {
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => void store.set(key, value),
  },
  window: {
    __TAURI_INTERNALS__: {
      invoke: async (cmd: string, args: { state?: typeof file }) => {
        if (cmd === "load_onboarding_state") return file;
        if (cmd === "save_onboarding_state") {
          saves.push(args.state);
          file = args.state!;
          return null;
        }
        throw new Error(cmd);
      },
      transformCallback: () => 0,
    },
  },
});
const state = await import("./onboarding-state.ts");
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

test("lost webview storage is restored from the native copy", async () => {
  store.clear();
  file = { done: true, workMode: "chat" };
  await state.loadOnboardingState();
  assert.equal(state.onboardingDone(), true);
  assert.equal(state.workMode(), "chat");
});

test("an older install's webview state is copied to the native file", async () => {
  store.clear();
  store.set("lomi.onboarding.v1", "done");
  file = { done: false };
  saves.length = 0;
  await state.loadOnboardingState();
  await settle();
  assert.deepEqual(saves, [{ done: true }]);
});

test("a first launch stays undone and completing it saves both copies", async () => {
  store.clear();
  file = { done: false };
  await state.loadOnboardingState();
  assert.equal(state.onboardingDone(), false);
  state.completeOnboarding();
  state.setWorkMode("chat");
  await settle();
  assert.equal(store.get("lomi.onboarding.v1"), "done");
  assert.deepEqual(file, { done: true, workMode: "chat" });
});
