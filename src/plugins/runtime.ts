import * as React from "react";
import * as ReactDOM from "react-dom";
import * as ReactDOMClient from "react-dom/client";
import * as JSX from "react/jsx-runtime";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, native } from "../api";
import { PluginHost, emptyContext } from "./host";
import type { PluginEntry } from "./host";
import type { PluginManifest, PluginModule } from "@lomi-dev/plugin-sdk";
export const HostContext = React.createContext(emptyContext);
export const useHostContext = () => React.useContext(HostContext);
export const pluginUrl = (entry: PluginEntry, path: string) =>
  `${native ? convertFileSrc("", "plugin") : "/plugin-assets/"}${entry.id}/${entry.revision}/${path.split("/").map(encodeURIComponent).join("/")}`;
export const pluginHost = new PluginHost({
  prepare: (entry) =>
    api<PluginManifest>("prepare_plugin", {
      id: entry.id,
      expected: entry.revision,
    }),
  import: (entry) =>
    import(
      /* @vite-ignore */ pluginUrl(entry, entry.manifest!.entry!)
    ) as Promise<PluginModule>,
  url: pluginUrl,
  css: async (entry, signal) => {
    const links: HTMLLinkElement[] = [];
    const pending = new Set<() => void>();
    const remove = () => {
      signal.removeEventListener("abort", remove);
      for (const cancel of [...pending]) cancel();
      for (const link of links) link.remove();
    };
    signal.addEventListener("abort", remove, { once: true });
    if (signal.aborted) {
      remove();
      throw new Error("Plugin activation was canceled.");
    }
    try {
      await Promise.all(
        (entry.manifest?.stylesheets ?? []).map(
          (path) =>
            new Promise<void>((resolve, reject) => {
              const link = document.createElement("link");
              link.rel = "stylesheet";
              link.media = "not all";
              link.href = pluginUrl(entry, path);
              link.dataset.plugin = entry.id;
              links.push(link);
              let finished = false;
              const finish = (error?: Error) => {
                if (finished) return;
                finished = true;
                clearTimeout(timer);
                link.onload = link.onerror = null;
                pending.delete(cancel);
                if (error) reject(error);
                else resolve();
              };
              const cancel = () =>
                finish(new Error("Plugin activation was canceled."));
              const timer = setTimeout(
                () => finish(new Error(`Plugin stylesheet timed out: ${path}`)),
                10000,
              );
              pending.add(cancel);
              link.onload = () => finish();
              link.onerror = () => finish(new Error(`Cannot load ${path}`));
              document.head.append(link);
            }),
        ),
      );
      if (signal.aborted) throw new Error("Plugin activation was canceled.");
      for (const link of links) link.media = "all";
      return remove;
    } catch (error) {
      remove();
      throw error;
    }
  },
});
export function initializePluginBridge() {
  if (new URLSearchParams(location.search).get("window") === "settings")
    throw new Error("Plugin code runs only in the main window.");
  Object.defineProperty(globalThis, Symbol.for("simplebench.plugin-api.v1"), {
    configurable: false,
    value: Object.freeze({
      react: React,
      reactDOM: ReactDOM,
      reactDOMClient: ReactDOMClient,
      jsx: JSX,
      jsxDev: { Fragment: React.Fragment, jsxDEV: JSX.jsx },
      sdk: { HostContext, useHostContext },
    }),
  });
}
