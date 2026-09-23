import { listen } from "@tauri-apps/api/event";
import { Channel } from "@tauri-apps/api/core";
import { api, errorMessage } from "./api";
import { browserTabs } from "./model";
import type { BrowserTab, Session } from "./model";

export interface BrowserPage {
  id: string;
  revision?: string;
  url: string;
  title: string;
  loading: boolean;
  error: string;
  download: string;
  browserGeneration?: string | null;
  profileId?: string | null;
  navigationId?: string | null;
  agentControlled?: boolean;
}
type Signal = "focus" | "address" | "close" | "find";
type Action =
  | { type: "navigate"; url: string }
  | { type: "back" | "forward" | "reload" | "stop" | "focus" | "takeControl" }
  | { type: "find"; text: string; backwards: boolean };

let retained = new Map<string, BrowserTab>();
const mounts = new Map<
  string,
  { element: HTMLElement; signal: (signal: Signal) => void }
>();
const pages = new Map<string, BrowserPage>();
const live = new Set<string>();
const agentStarts = new Map<string, { operationId: string; nonce: string }>();
const hiddenAgentStarts = new Set<string>();
export function stageAgentBrowser(
  id: string,
  ticket: { operationId: string; nonce: string },
  visible = true,
) {
  agentStarts.set(id, ticket);
  if (!visible) hiddenAgentStarts.add(id);
}
export function clearAgentBrowser(id: string) {
  agentStarts.delete(id);
  hiddenAgentStarts.delete(id);
}
export function hasLiveAgentBrowser(id: string, generation: string) {
  return live.has(id) && pages.get(id)?.browserGeneration === generation;
}
export async function waitForAgentBrowser(
  id: string,
  generation: string,
): Promise<BrowserPage> {
  const deadline = performance.now() + 15_000;
  while (performance.now() < deadline) {
    await synchronize();
    const page = pages.get(id);
    if (
      live.has(id) &&
      page?.browserGeneration === generation &&
      page.navigationId
    )
      return page;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error("Browser panel could not render.");
}
const listeners = new Set<() => void>();
let onChange: (
  id: string,
  change: Pick<BrowserTab, "url" | "title">,
) => void = () => {};
let onOpen: (id: string, url: string) => void = () => {};
let onError: (error: string) => void = () => {};
let events: Promise<unknown> | undefined;
let frame = 0;
let dirty = false;
let pending: Promise<void> | undefined;
let mutation: MutationObserver | undefined;

let localServers: { urls: string[] | null; error: string } = {
  urls: null,
  error: "",
};
let serverRequest: Promise<void> | undefined;
const serverListeners = new Set<() => void>();

export const localServersSnapshot = () => localServers;
export function subscribeLocalServers(listener: () => void) {
  serverListeners.add(listener);
  return () => {
    serverListeners.delete(listener);
  };
}

function receiveLocalServers(urls: string[] | null, error = "") {
  if (JSON.stringify(localServers) === JSON.stringify({ urls, error })) return;
  localServers = { urls, error };
  for (const listener of serverListeners) listener();
}

export function refreshLocalServers(): Promise<void> {
  if (serverRequest) return serverRequest;
  // Share discoveries across panels and retain usable addresses while revalidating.
  const urls = new Set(localServers.urls ?? []);
  let complete = false;
  const onFound = new Channel<string>((url) => {
    if (complete || urls.has(url)) return;
    urls.add(url);
    receiveLocalServers(
      [...urls].sort(
        (a, b) =>
          Number(new URL(a).port) - Number(new URL(b).port) ||
          a.localeCompare(b),
      ),
    );
  });
  receiveLocalServers(localServers.urls);
  serverRequest = api<string[]>("local_web_servers", { onFound })
    .then((result) => receiveLocalServers(result))
    .catch((error) =>
      receiveLocalServers(localServers.urls ?? [], errorMessage(error)),
    )
    .finally(() => {
      complete = true;
      serverRequest = undefined;
    });
  return serverRequest;
}

export function configureBrowsers(
  change: typeof onChange,
  open: typeof onOpen,
  error: typeof onError,
) {
  onChange = change;
  onOpen = open;
  onError = error;
}

export function retainBrowsers(session: Session | undefined) {
  const next = new Map(
    session ? browserTabs(session).map((tab) => [tab.id, tab]) : [],
  );
  const changed =
    next.size !== retained.size ||
    [...next].some(([id, tab]) => retained.get(id)?.url !== tab.url);
  retained = next;
  if (changed) schedule();
}

export function browserSnapshot(tab: BrowserTab): BrowserPage {
  let page = pages.get(tab.id);
  if (!page) {
    page = { ...tab, loading: false, error: "", download: "" };
    pages.set(tab.id, page);
  }
  return page;
}
export function subscribeBrowsers(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function receive(page: BrowserPage) {
  const previous = pages.get(page.id);
  // A delayed sync response can arrive after a newer native event.
  if (
    page.revision &&
    previous?.revision &&
    BigInt(page.revision) < BigInt(previous.revision)
  )
    return;
  const tab = retained.get(page.id);
  if (!tab || JSON.stringify(pages.get(page.id)) === JSON.stringify(page))
    return;
  pages.set(page.id, page);
  if (tab.url !== page.url || tab.title !== page.title)
    onChange(page.id, { url: page.url, title: page.title });
  for (const listener of listeners) listener();
}

function setup() {
  return (events ??= Promise.all([
    listen<BrowserPage>("browser-page", (event) => receive(event.payload)),
    listen<{ id: string; signal: Signal }>("browser-signal", ({ payload }) =>
      mounts.get(payload.id)?.signal(payload.signal),
    ),
    listen<{ id: string; url: string }>("browser-open", ({ payload }) => {
      if (retained.has(payload.id)) onOpen(payload.id, payload.url);
    }),
  ]));
}

let hiddenSyncQueued = false;
function schedule() {
  // A main WKWebView may suspend animation frames while native children remain
  // visible. Domain changes must still hide those children and app overlays.
  if (document.hidden) {
    if (!hiddenSyncQueued) {
      hiddenSyncQueued = true;
      queueMicrotask(() => {
        hiddenSyncQueued = false;
        void synchronize().catch((error) =>
          onError(`Browser: ${errorMessage(error)}`),
        );
      });
    }
    return;
  }
  if (!frame)
    frame = requestAnimationFrame(() => {
      frame = 0;
      void synchronize().catch((error) =>
        onError(`Browser: ${errorMessage(error)}`),
      );
    });
}

function synchronize(): Promise<void> {
  dirty = true;
  if (pending) return pending;
  pending = (async () => {
    await setup();
    do {
      dirty = false;
      // Native views sit above HTML. Hide them while an app overlay or drag owns input.
      const covered =
        document.querySelector(
          "dialog[open], .menu, :popover-open, .tab-drag-ghost, .pane-drag-ghost, .pane-limit-notice, [role=separator]:active",
        ) !== null;
      const zoom =
        Number(
          getComputedStyle(document.documentElement).getPropertyValue(
            "--app-zoom",
          ),
        ) || 1;
      const slots = covered
        ? []
        : [...mounts].flatMap(([id, { element }]) => {
            const tab = retained.get(id);
            const rect = element.getBoundingClientRect();
            const x = Math.max(0, rect.left),
              y = Math.max(0, rect.top);
            const width = Math.min(innerWidth, rect.right) - x,
              height = Math.min(innerHeight, rect.bottom) - y;
            return tab &&
              tab.url !== "about:blank" &&
              element.isConnected &&
              element.getClientRects().length &&
              width > 1 &&
              height > 1
              ? [
                  {
                    id,
                    url: tab.url,
                    ...(tab.automation ? { automation: tab.automation } : {}),
                    ...(agentStarts.has(id)
                      ? { agentTicket: agentStarts.get(id) }
                      : {}),
                    bounds: {
                      x: x * zoom,
                      y: y * zoom,
                      width: width * zoom,
                      height: height * zoom,
                    },
                  },
                ]
              : [];
          });
      const hiddenSlots = [...hiddenAgentStarts].flatMap((id) => {
        const tab = retained.get(id);
        const ticket = agentStarts.get(id);
        return tab?.automation && ticket && !live.has(id)
          ? [
              {
                id,
                url: tab.url,
                automation: tab.automation,
                agentTicket: ticket,
                hidden: true,
                bounds: { x: 0, y: 0, width: 800, height: 600 },
              },
            ]
          : [];
      });
      if (retained.size || pages.size) {
        const result = await api<BrowserPage[]>("sync_browsers", {
          retained: [...retained.keys()],
          slots: [
            ...slots.filter((s) => !hiddenAgentStarts.has(s.id)),
            ...hiddenSlots,
          ],
        });
        live.clear();
        for (const page of result) {
          live.add(page.id);
          receive(page);
        }
      }
      for (const id of pages.keys()) if (!retained.has(id)) pages.delete(id);
    } while (dirty);
  })().finally(() => {
    pending = undefined;
  });
  return pending;
}

function resizeStart(event: PointerEvent) {
  if (
    event.target instanceof Element &&
    event.target.closest("[role=separator]")
  )
    schedule();
}

export function mountBrowser(
  tab: BrowserTab,
  element: HTMLElement,
  signal: (signal: Signal) => void,
) {
  mounts.set(tab.id, { element, signal });
  const resize = new ResizeObserver(schedule);
  resize.observe(element);
  if (!mutation) {
    mutation = new MutationObserver((records) => {
      const overlays =
        "dialog, .menu, .select-menu, .tab-drag-ghost, .pane-drag-ghost, .pane-limit-notice";
      if (
        records.some((record) =>
          record.type === "attributes"
            ? record.target instanceof Element &&
              (record.target.matches("body, html, dialog") ||
                [...mounts.values()].some(({ element }) =>
                  (record.target as Element).contains(element),
                ))
            : [...record.addedNodes, ...record.removedNodes].some(
                (node) =>
                  node instanceof Element &&
                  (node.matches(overlays) || node.querySelector(overlays)),
              ),
        )
      )
        schedule();
    });
    mutation.observe(document.body, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["class", "style", "open", "hidden"],
    });
    document.addEventListener("toggle", schedule, true);
    document.addEventListener("pointerdown", resizeStart, true);
    document.addEventListener("pointerup", schedule, true);
    document.addEventListener("pointercancel", schedule, true);
    window.addEventListener("resize", schedule);
    window.addEventListener("scroll", schedule, true);
  }
  schedule();
  return () => {
    mounts.delete(tab.id);
    resize.disconnect();
    if (!mounts.size) {
      mutation?.disconnect();
      mutation = undefined;
      document.removeEventListener("toggle", schedule, true);
      document.removeEventListener("pointerdown", resizeStart, true);
      document.removeEventListener("pointerup", schedule, true);
      document.removeEventListener("pointercancel", schedule, true);
      window.removeEventListener("resize", schedule);
      window.removeEventListener("scroll", schedule, true);
    }
    schedule();
  };
}

export async function browserAction(tab: BrowserTab, action: Action) {
  if (action.type === "navigate" && !live.has(tab.id)) {
    onChange(tab.id, { url: action.url, title: "Browser" });
    await synchronize();
    return;
  }
  await synchronize();
  await api("browser_action", { id: tab.id, action });
}
