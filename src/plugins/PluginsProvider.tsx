import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage, native } from "../api";
import type { PluginCatalog } from "./host";
import { initializePluginBridge, pluginHost } from "./runtime";
const empty: PluginCatalog = { directory: "", entries: [], safeMode: false };
const Context = createContext({
  catalog: empty,
  error: "",
  reload: async () => {},
});
let bridgeReady = false;
export let confirmPluginClose: (owner: string) => Promise<boolean> = async () =>
  false;
export function setPluginCloseHandler(handler: typeof confirmPluginClose) {
  confirmPluginClose = handler;
  return () => {
    confirmPluginClose = async () => false;
  };
}
export function PluginsProvider({ children }: { children: ReactNode }) {
  const [catalog, setCatalog] = useState(empty);
  const [error, setError] = useState("");
  const revision = useRef(0);
  const reload = useCallback(async () => {
    if (!native) return;
    const request = ++revision.current;
    try {
      const value = await api<PluginCatalog>("list_plugins");
      if (request !== revision.current) return;
      setCatalog(value);
      setError("");
      await pluginHost.discover(value);
    } catch (error) {
      if (request === revision.current) setError(errorMessage(error));
    }
  }, []);
  useEffect(() => {
    if (!bridgeReady) {
      initializePluginBridge();
      bridgeReady = true;
    }
    if (!native) return;
    let active = true;
    const changes = listen("plugins-changed", () => void reload());
    let timer: ReturnType<typeof setTimeout> | undefined;
    let previous = "";
    const stopStatus = pluginHost.subscribe(() => {
      clearTimeout(timer);
      timer = setTimeout(() => {
        const statuses = Object.fromEntries(
          [...pluginHost.statuses].map(([id, status]) => [
            id,
            { ...status, error: status.error.slice(0, 4096) },
          ]),
        );
        const serialized = JSON.stringify(statuses);
        setCatalog((current) => ({
          ...current,
          entries: current.entries.map((entry) => ({
            ...entry,
            status: statuses[entry.id] ?? null,
          })),
        }));
        if (serialized !== previous) {
          previous = serialized;
          void api("report_plugin_status", { statuses }).catch((error) => {
            if (active) setError(errorMessage(error));
          });
        }
      }, 50);
    });
    const requests = listen<{ token: string; id: string }>(
      "plugin-close-request",
      ({ payload }) => {
        void (async () => {
          let approved = false;
          try {
            approved = await confirmPluginClose(payload.id);
            await api("finish_plugin_removal", {
              token: payload.token,
              approved,
            });
            if (approved) await pluginHost.deactivate(payload.id);
          } catch (error) {
            setError(errorMessage(error));
            await api("finish_plugin_removal", {
              token: payload.token,
              approved: false,
            }).catch(() => {});
          }
        })();
      },
    );
    void Promise.all([changes, requests])
      .then(() => {
        if (active) void reload();
      })
      .catch((error) => setError(errorMessage(error)));
    window.addEventListener("focus", reload);
    return () => {
      active = false;
      clearTimeout(timer);
      stopStatus();
      ++revision.current;
      window.removeEventListener("focus", reload);
      void changes.then((stop) => stop()).catch(() => {});
      void requests.then((stop) => stop()).catch(() => {});
    };
  }, [reload]);
  return (
    <Context.Provider value={{ catalog, error, reload }}>
      {children}
    </Context.Provider>
  );
}
export const usePlugins = () => useContext(Context);
