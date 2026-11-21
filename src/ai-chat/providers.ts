import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage } from "../api.ts";
import type { CliAgent } from "../cli-agents.ts";
import type {
  ChatSettings,
  CliStatus,
  ModelEntry,
  ProviderEntry,
} from "./types.ts";

export function slugify(label: string) {
  return label
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 48)
    .replace(/-+$/, "");
}

function host(url: string) {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/**
 * Applies stored preferences to a reported model list: stored order first,
 * models no longer reported are dropped (custom ones kept), new ones appended enabled.
 */
export function applyModelPrefs(
  ids: readonly string[],
  prefs: readonly ModelEntry[],
): ModelEntry[] {
  const reported = new Set(ids);
  const out = prefs.filter(
    (model) => model.source === "manual" || reported.has(model.id),
  );
  const seen = new Set(out.map((model) => model.id));
  for (const id of ids)
    if (!seen.has(id)) {
      seen.add(id);
      out.push({ id, source: "fetched", enabled: true, favorite: false });
    }
  return out.slice(0, 500);
}

/** Picker order: enabled models only, favorites first. */
export function pickableModels(models: readonly ModelEntry[]) {
  const on = models.filter((model) => model.enabled);
  return [
    ...on.filter((model) => model.favorite),
    ...on.filter((model) => !model.favorite),
  ].map((model) => model.id);
}

export type CliStatuses = Readonly<Record<string, CliStatus | undefined>>;

export function cliStatusLine(status: CliStatus | undefined) {
  if (!status) return "Checking…";
  if (!status.binaryPath) return "Not installed";
  if (status.signedIn === false) return "Not signed in";
  if (status.signedIn)
    return `Signed in${status.plan ? ` · ${status.plan}` : ""}`;
  return "Sign-in unknown";
}

export function mergeProviders(
  settings: ChatSettings,
  statuses: CliStatuses,
): ProviderEntry[] {
  const entries: ProviderEntry[] = [];
  for (const provider of settings.cliProviders) {
    const status = statuses[provider.cli];
    const found = !!status?.binaryPath;
    const available = provider.enabled && found && status?.signedIn !== false;
    entries.push({
      instanceId: provider.instanceId,
      kind: "cli",
      cli: provider.cli,
      label: provider.label,
      enabled: provider.enabled,
      available,
      unavailableReason: available
        ? null
        : !provider.enabled
          ? "This provider is disabled."
          : !status
            ? "Checking the CLI…"
            : !found
              ? "The CLI is not installed."
              : "Not signed in. Run the CLI login in a terminal.",
      statusLine: cliStatusLine(status),
      versionLabel: status?.version ?? null,
      models: [],
      modelPrefs: provider.models,
      cliStatus: status,
    });
  }
  for (const provider of settings.apiProviders) {
    const count = provider.models.length;
    const models = `${count} ${count === 1 ? "model" : "models"}`;
    const keyless = provider.kind === "openai-compatible";
    entries.push({
      instanceId: provider.instanceId,
      kind: provider.kind,
      cli: null,
      label: provider.label,
      enabled: provider.enabled,
      available: provider.enabled && (provider.apiKeySet || keyless),
      unavailableReason: !provider.enabled
        ? "This provider is disabled."
        : provider.apiKeySet || keyless
          ? null
          : "The API key is missing.",
      statusLine: provider.apiKeySet
        ? `API key stored · ${models}`
        : keyless
          ? `No API key · ${models}`
          : "API key missing",
      versionLabel: host(provider.baseUrl),
      models: pickableModels(provider.models),
      modelPrefs: provider.models,
      api: provider,
    });
  }
  return entries;
}

/** Default provider when usable, else the fallback, else the first available. */
export function providerDefault(
  entries: ProviderEntry[],
  settings: ChatSettings | undefined,
) {
  const usable = entries.filter((entry) => entry.available);
  const find = (id?: string | null) =>
    usable.find((entry) => entry.instanceId === id);
  return (
    find(settings?.defaults.instanceId) ??
    find(settings?.defaults.fallback?.instanceId) ??
    usable[0]
  );
}

export interface DetectResult {
  settings: ChatSettings;
  added: string[];
}

/** Scans for installed CLIs; native side adds new ones as providers. */
export const detectCliProviders = () =>
  api<DetectResult>("ai_providers_detect");

export function useChatProviders() {
  const [settings, setSettings] = useState<ChatSettings>();
  const [statuses, setStatuses] = useState<CliStatuses>({});
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const keepSettings = useCallback((next: ChatSettings) => {
    setSettings((current) =>
      !current || next.revision >= current.revision ? next : current,
    );
  }, []);
  useEffect(() => {
    let live = true;
    const fail = (cause: unknown) => live && setError(errorMessage(cause));
    const stop = listen<{ scope: string }>("ai-chat-changed", (event) => {
      if (event.payload?.scope === "settings")
        api<ChatSettings>("ai_chat_settings")
          .then((next) => live && keepSettings(next))
          .catch(fail);
    });
    api<ChatSettings>("ai_chat_settings")
      .then((next) => live && keepSettings(next))
      .catch(fail);
    return () => {
      live = false;
      void stop.then((fn) => fn()).catch(() => {});
    };
  }, [keepSettings]);
  const clis = (settings?.cliProviders ?? []).map((item) => item.cli).join();
  const refreshStatus = useCallback(async (cli: CliAgent) => {
    try {
      const next = await api<CliStatus>("ai_cli_status", { cli });
      setStatuses((current) => ({ ...current, [cli]: next }));
    } catch (cause) {
      setStatuses((current) => ({
        ...current,
        [cli]: {
          cli,
          binaryPath: null,
          version: null,
          signedIn: null,
          plan: null,
          error: errorMessage(cause),
        },
      }));
    }
  }, []);
  // Statuses are queried on mount, when the CLI set changes and on window focus.
  useEffect(() => {
    const load = () => {
      for (const cli of clis ? (clis.split(",") as CliAgent[]) : [])
        void refreshStatus(cli);
    };
    load();
    window.addEventListener("focus", load);
    return () => window.removeEventListener("focus", load);
  }, [clis, refreshStatus]);
  const run = useCallback(
    async (name: string, request: Record<string, unknown>) => {
      if (busyRef.current) throw new Error("An operation is in progress.");
      busyRef.current = true;
      setBusy(true);
      setError("");
      try {
        const next = await api<ChatSettings>(name, { request });
        keepSettings(next);
        return next;
      } catch (cause) {
        setError(errorMessage(cause));
        throw cause;
      } finally {
        busyRef.current = false;
        setBusy(false);
      }
    },
    [keepSettings],
  );
  const detect = useCallback(async () => {
    try {
      const result = await detectCliProviders();
      keepSettings(result.settings);
      return result.added.length;
    } catch (cause) {
      setError(errorMessage(cause));
      return 0;
    }
  }, [keepSettings]);
  const entries = settings ? mergeProviders(settings, statuses) : [];
  return {
    entries,
    settings,
    error,
    setError,
    busy,
    run,
    refreshStatus,
    detect,
  };
}
