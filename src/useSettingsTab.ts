import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage, native } from "./api";
import { reportAppNotification } from "./app-notifications";
import { parseSettingsPage } from "./settings-pages";
import type { SettingsPageId } from "./settings-pages";

export const SETTINGS_TAB_ID = "settings";

interface SettingsRequest {
  revision: number;
  page: string | null;
}
interface SettingsTabState {
  opened: boolean;
  active: boolean;
  mounted: boolean;
  page: SettingsPageId | null;
  requestRevision: number;
}

export function useSettingsTab() {
  const [state, setState] = useState<SettingsTabState>(() => {
    const query = new URLSearchParams(location.search);
    const requested = query.get("window") === "settings";
    return {
      opened: requested,
      active: requested,
      mounted: requested,
      page: requested ? parseSettingsPage(query.get("page")) : null,
      requestRevision: 0,
    };
  });
  const active = useRef(state.active);
  const lastRevision = useRef(0);
  const request = useCallback((page: SettingsPageId | null) => {
    active.current = true;
    setState((previous) => ({
      opened: true,
      active: true,
      mounted: true,
      // A no-page open reuses the panel's current selection. Keep the last
      // navigation token unchanged, including when React batches requests.
      page: page ?? previous.page,
      requestRevision: previous.requestRevision + (page === null ? 0 : 1),
    }));
  }, []);
  const select = useCallback(() => {
    active.current = true;
    setState((previous) => ({
      ...previous,
      opened: true,
      active: true,
      mounted: true,
    }));
  }, []);
  const leave = useCallback(() => {
    active.current = false;
    setState((previous) =>
      previous.active ? { ...previous, active: false } : previous,
    );
  }, []);
  const close = useCallback(() => {
    active.current = false;
    setState((previous) => ({ ...previous, opened: false, active: false }));
  }, []);
  useEffect(() => {
    if (!native) return;
    let current = true;
    let initializing = true;
    const backlog: SettingsRequest[] = [];
    const stops: (() => void)[] = [];
    const accept = (incoming: SettingsRequest | null) => {
      if (
        !current ||
        !incoming ||
        !Number.isSafeInteger(incoming.revision) ||
        incoming.revision <= lastRevision.current
      )
        return;
      const page = parseSettingsPage(incoming.page);
      if (incoming.page !== null && !page) return;
      lastRevision.current = incoming.revision;
      request(page);
    };
    void (async () => {
      try {
        const stop = await listen<SettingsRequest>(
          "settings-tab-requested",
          ({ payload }) => {
            if (initializing) backlog.push(payload);
            else accept(payload);
          },
        );
        if (!current) {
          stop();
          return;
        }
        stops.push(stop);
        // Older in-process page links are navigation requests in the same tab.
        const stopLegacy = await listen<string>(
          "settings-page-changed",
          ({ payload }) => {
            const page = parseSettingsPage(payload);
            if (current && page) request(page);
          },
        );
        if (!current) {
          stopLegacy();
          return;
        }
        stops.push(stopLegacy);
        const pending = await api<SettingsRequest | null>("settings_tab_ready");
        if (pending) backlog.push(pending);
        initializing = false;
        // Events may overtake the ready response. Preserve queued explicit
        // destinations before applying newer no-page opens.
        for (const incoming of backlog.sort((a, b) => a.revision - b.revision))
          accept(incoming);
      } catch (reason) {
        initializing = false;
        for (const incoming of backlog.sort((a, b) => a.revision - b.revision))
          accept(incoming);
        if (current)
          void reportAppNotification({
            body: `Could not initialize Settings navigation: ${errorMessage(reason)}`,
          });
      }
    })();
    return () => {
      current = false;
      for (const stop of stops) stop();
    };
  }, [request]);
  return { ...state, activeRef: active, select, leave, close };
}
