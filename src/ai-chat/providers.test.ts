import assert from "node:assert/strict";
import test from "node:test";
import {
  applyModelPrefs,
  mergeProviders,
  pickableModels,
  providerDefault,
  slugify,
} from "./providers.ts";
import type { ChatSettings, CliStatus } from "./types.ts";

const settings = (patch: Partial<ChatSettings> = {}): ChatSettings => ({
  version: 2,
  revision: 1,
  apiProviders: [],
  cliProviders: [],
  defaults: {
    instanceId: null,
    model: null,
    effort: null,
    fallback: null,
    permissions: "read-only",
    workspace: "project",
  },
  behavior: { sendKey: "enter", systemInstructions: "", maxOutputTokens: null },
  autoDetected: [],
  ...patch,
});
const status = (patch: Partial<CliStatus> = {}): CliStatus => ({
  cli: "codex",
  binaryPath: "/usr/local/bin/codex",
  version: "0.60.0",
  signedIn: true,
  plan: "Max 5x",
  error: null,
  ...patch,
});
const provider = (enabled = true) => ({
  instanceId: "codex",
  cli: "codex" as const,
  label: "Codex",
  enabled,
  models: [],
});
const cliEntry = (
  statuses: Record<string, CliStatus | undefined>,
  enabled = true,
) =>
  mergeProviders(settings({ cliProviders: [provider(enabled)] }), statuses)[0];
const api = {
  instanceId: "openai",
  kind: "openai" as const,
  label: "OpenAI",
  accentColor: null,
  enabled: true,
  baseUrl: "https://api.openai.com/v1",
  headers: [],
  models: [
    {
      id: "gpt-5.5",
      source: "fetched" as const,
      enabled: true,
      favorite: false,
    },
  ],
  apiKeySet: true,
};

test("signed-in CLI is available and shows the plan", () => {
  const cli = cliEntry({ codex: status() });
  assert.equal(cli.instanceId, "codex");
  assert.equal(cli.available, true);
  assert.equal(cli.statusLine, "Signed in · Max 5x");
  assert.equal(cli.versionLabel, "0.60.0");
  assert.equal(
    cliEntry({ codex: status({ plan: null }) }).statusLine,
    "Signed in",
  );
});

test("CLI status line and availability for other states", () => {
  const signedOut = cliEntry({ codex: status({ signedIn: false }) });
  assert.equal(signedOut.statusLine, "Not signed in");
  assert.equal(signedOut.available, false);
  const missing = cliEntry({ codex: status({ binaryPath: null }) });
  assert.equal(missing.statusLine, "Not installed");
  assert.equal(missing.available, false);
  const checking = cliEntry({});
  assert.equal(checking.statusLine, "Checking…");
  assert.equal(checking.available, false);
  assert.equal(cliEntry({ codex: status({ signedIn: null }) }).available, true);
  assert.equal(cliEntry({ codex: status() }, false).available, false);
});

test("API status lines", () => {
  const entry = (patch: object) =>
    mergeProviders(settings({ apiProviders: [{ ...api, ...patch }] }), {}).at(
      -1,
    )!;
  assert.equal(entry({}).statusLine, "API key stored · 1 model");
  assert.equal(entry({}).versionLabel, "api.openai.com");
  assert.equal(entry({ apiKeySet: false }).statusLine, "API key missing");
  assert.equal(entry({ apiKeySet: false }).available, false);
  const compatible = entry({ kind: "openai-compatible", apiKeySet: false });
  assert.equal(compatible.statusLine, "No API key · 1 model");
  assert.equal(compatible.available, true);
});

test("providerDefault prefers default, then fallback, then first usable", () => {
  const off = { ...api, instanceId: "off", enabled: false };
  const second = { ...api, instanceId: "second" };
  const base = settings({ apiProviders: [off, api, second] }).defaults;
  const pick = (instanceId: string | null, fallbackId?: string) => {
    const s = settings({
      apiProviders: [off, api, second],
      defaults: {
        ...base,
        instanceId,
        fallback: fallbackId
          ? { instanceId: fallbackId, model: "m", effort: null }
          : null,
      },
    });
    return providerDefault(mergeProviders(s, {}), s)?.instanceId;
  };
  assert.equal(pick("second"), "second");
  assert.equal(pick("off", "second"), "second");
  assert.equal(pick("off", "off"), "openai");
  assert.equal(pick("x"), "openai");
});

test("slugify matches the native instance id pattern", () => {
  assert.equal(slugify("  OpenAI (Work)! "), "openai-work");
  assert.equal(slugify("!!!"), "");
});

test("model prefs keep order and flags, drop vanished, append new", () => {
  const m = (id: string, extra: object = {}) => ({
    id,
    source: "fetched" as const,
    enabled: true,
    favorite: false,
    ...extra,
  });
  const prefs = [
    m("b", { enabled: false }),
    m("gone"),
    { ...m("mine"), source: "manual" as const },
    m("a", { favorite: true }),
  ];
  const merged = applyModelPrefs(["a", "b", "c"], prefs);
  assert.deepEqual(
    merged.map((model) => model.id),
    ["b", "mine", "a", "c"],
  );
  assert.equal(merged[0].enabled, false);
  assert.deepEqual(pickableModels(merged), ["a", "mine", "c"]);
});
