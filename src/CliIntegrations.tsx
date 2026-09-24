import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  isPermissionGranted,
  requestPermission,
} from "@tauri-apps/plugin-notification";
import { X } from "lucide-react";
import { api, errorMessage, native } from "./api";
import type { TerminalContext, TitleProcess } from "./terminal-runtime";
import { useTerminalPreferences } from "./TerminalPreferencesProvider";

import { cliNames } from "./cli-agents";
export { cliNames } from "./cli-agents";
export type CliFeature = "notifications" | "mcp" | "titlebar";
interface FeatureStatus {
  feature: CliFeature;
  configured: boolean;
  path: string;
  revision: string | null;
  error: string | null;
}
interface CliStatus {
  cli: TitleProcess["cli"];
  features: FeatureStatus[];
}
interface Offer extends CliStatus {
  id: string;
  process: TitleProcess;
  issue?: string;
}
const featureNames = {
  notifications: "notifications",
  mcp: "Lomi MCP",
  titlebar: "titlebar",
};
const dismissalKey = (cli: TitleProcess["cli"], feature: CliFeature) =>
  `${cli}:${feature}`;
const key = (id: string, process: TitleProcess) =>
  `${id}:${process.cli}:${process.pid}`;

export function useCliIntegrations(
  activePaneId: string | undefined,
  onError: (message: string) => void,
  onConfigured: (message: string) => void,
) {
  const preferences = useTerminalPreferences();
  const [offers, setOffers] = useState<Offer[]>([]);
  const [permission, setPermission] = useState(false);
  const [busy, setBusy] = useState(false);
  const contexts = useRef<Record<string, TerminalContext>>({});
  const cache = useRef(new Map<string, { offer: Offer; time: number }>());
  const checking = useRef(false);
  const saving = useRef(false);
  const alive = useRef(true);
  const generation = useRef(0);
  const dismissed = useRef(new Set<string>());

  const refresh = useCallback(async () => {
    if (checking.current || !native) return;
    checking.current = true;
    const request = generation.current;
    try {
      const next: Offer[] = [];
      for (const [id, context] of Object.entries(contexts.current)) {
        const process = context.titleCli;
        if (!process) continue;
        const identity = key(id, process);
        let cached = cache.current.get(identity);
        if (!cached || Date.now() - cached.time > 10_000) {
          let offer: Offer;
          try {
            const status = await api<CliStatus>("inspect_cli_integrations", {
              id,
              process,
            });
            offer = { ...status, id, process };
          } catch (error) {
            offer = {
              id,
              process,
              cli: process.cli,
              features: [],
              issue: errorMessage(error),
            };
          }
          if (!alive.current || generation.current !== request) return;
          cached = { offer, time: Date.now() };
          cache.current.set(identity, cached);
        }
        next.push({
          ...cached.offer,
          features: cached.offer.features.filter(
            (feature) =>
              !dismissed.current.has(
                dismissalKey(cached.offer.cli, feature.feature),
              ),
          ),
        });
      }
      if (alive.current && request === generation.current) setOffers(next);
      const keys = new Set(
        Object.entries(contexts.current).flatMap(([id, context]) =>
          context.titleCli ? [key(id, context.titleCli)] : [],
        ),
      );
      for (const cachedKey of cache.current.keys())
        if (!keys.has(cachedKey)) cache.current.delete(cachedKey);
    } finally {
      checking.current = false;
    }
  }, []);
  const invalidate = useCallback(() => {
    ++generation.current;
    cache.current.clear();
    void refresh();
  }, [refresh]);
  const observe = useCallback(
    (next: Record<string, TerminalContext>) => {
      const signature = (value: Record<string, TerminalContext>) =>
        Object.entries(value)
          .flatMap(([id, context]) =>
            context.titleCli ? [key(id, context.titleCli)] : [],
          )
          .join("|");
      if (signature(next) !== signature(contexts.current)) {
        ++generation.current;
        setOffers((current) =>
          current.filter(
            (offer) =>
              next[offer.id]?.titleCli?.pid === offer.process.pid &&
              next[offer.id]?.titleCli?.cli === offer.cli,
          ),
        );
      }
      contexts.current = next;
      void refresh();
    },
    [refresh],
  );

  useEffect(() => {
    alive.current = true;
    const focused = () => {
      invalidate();
      if (native)
        void isPermissionGranted()
          .then((granted) => {
            if (alive.current) setPermission(granted);
          })
          .catch(() => {});
    };
    focused();
    window.addEventListener("focus", focused);
    const stop = native
      ? listen("cli-integrations-changed", invalidate)
      : undefined;
    return () => {
      alive.current = false;
      ++generation.current;
      window.removeEventListener("focus", focused);
      void stop?.then((unlisten) => unlisten()).catch(() => {});
    };
  }, [invalidate]);

  const missing = (offer: Offer) =>
    offer.features.filter(
      (feature) =>
        !dismissed.current.has(dismissalKey(offer.cli, feature.feature)) &&
        (!feature.configured ||
          (feature.feature === "notifications" &&
            (!permission || !preferences.value.agentNotifications))),
    );
  const candidates = offers.filter(
    (offer) => offer.issue || missing(offer).length,
  );
  const offer =
    candidates.find((candidate) => candidate.id === activePaneId) ??
    candidates[0];
  const enable = async (selected: Offer, feature: FeatureStatus) => {
    if (saving.current) return;
    saving.current = true;
    setBusy(true);
    onError("");
    try {
      if (feature.error) throw new Error(feature.error);
      if (feature.feature === "notifications") {
        const granted =
          (await isPermissionGranted()) ||
          (await requestPermission()) === "granted";
        setPermission(granted);
        if (!granted)
          throw new Error(
            "Notifications are blocked. Allow notifications for Lomi in your system settings, then try again.",
          );
      }
      const message = await api<string>("enable_cli_integration", {
        id: selected.id,
        process: selected.process,
        feature: feature.feature,
        path: feature.path,
        revision: feature.revision,
      });
      await preferences.reload();
      onConfigured(message);
    } catch (error) {
      onError(errorMessage(error));
    } finally {
      saving.current = false;
      if (alive.current) {
        setBusy(false);
        invalidate();
      }
    }
  };
  const dismiss = async (selected: Offer, feature: CliFeature) => {
    if (saving.current) return;
    try {
      await api("dismiss_cli_integrations", { cli: selected.cli, feature });
      dismissed.current.add(dismissalKey(selected.cli, feature));
      cache.current.clear();
      setOffers((current) =>
        current
          .map((item) =>
            item.cli === selected.cli
              ? {
                  ...item,
                  features: item.features.filter(
                    (itemFeature) => itemFeature.feature !== feature,
                  ),
                }
              : item,
          )
          .filter((item) => item.issue || missing(item).length),
      );
    } catch (error) {
      onError(errorMessage(error));
    }
  };
  return {
    observe,
    bar: offer && (
      <div
        className="cli-integrations"
        role="group"
        aria-label={`${cliNames[offer.cli]} integrations`}
        aria-busy={busy}
      >
        <div className="cli-integration-actions">
          {offer.issue ? (
            <button
              className="cli-integration-button"
              onClick={() => {
                onError(offer.issue!);
                invalidate();
              }}
            >
              Check {cliNames[offer.cli]} integrations
            </button>
          ) : (
            missing(offer).map((feature) => (
              <div key={feature.feature} className="cli-integration-item">
                <button
                  className="cli-integration-button"
                  disabled={busy}
                  title={
                    feature.error ??
                    `Enable ${featureNames[feature.feature]} in ${feature.path}. Existing configuration is backed up; restart or reload the CLI to apply changes.`
                  }
                  onClick={() => void enable(offer, feature)}
                >
                  Enable {cliNames[offer.cli]} {featureNames[feature.feature]}
                </button>
                <button
                  className="cli-integration-dismiss"
                  disabled={busy}
                  aria-label={`Dismiss ${cliNames[offer.cli]} ${featureNames[feature.feature]} suggestion until Lomi restarts`}
                  title="Hide until Lomi restarts"
                  onClick={() => void dismiss(offer, feature.feature)}
                >
                  <X size={12} aria-hidden="true" />
                </button>
              </div>
            ))
          )}
        </div>
      </div>
    ),
  };
}
