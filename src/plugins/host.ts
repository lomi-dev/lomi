import mitt from "mitt";
import type { ComponentType } from "react";
import type {
  CommandContext,
  DirtyView,
  Dispose,
  HostEvents,
  HostSnapshot,
  Json,
  PluginContext,
  PluginManifest,
  PluginModule,
  ViewProps,
} from "@lomi-dev/plugin-sdk";
import { jsonState, packagePath, parsePlugin } from "./manifest.ts";
export interface PluginEntry {
  id: string;
  revision: string;
  source: string;
  enabled: boolean;
  trustedRevision: string | null;
  manifest: PluginManifest | null;
  error: string | null;
  restartRequired: boolean;
  evaluated: boolean;
  themeIds?: string[];
  status?: PluginStatus | null;
}
export interface PluginCatalog {
  directory: string;
  entries: PluginEntry[];
  safeMode: boolean;
}
export type PluginPhase =
  | "discovered"
  | "incompatible"
  | "disabled"
  | "activating"
  | "active"
  | "failed"
  | "deactivating";
export interface PluginStatus {
  phase: PluginPhase;
  error: string;
  evaluated: boolean;
}
interface Activation {
  abort: AbortController;
  disposers: Set<Dispose>;
  promise: Promise<void>;
  module?: PluginModule;
  deactivated?: boolean;
}
export const emptyContext: HostSnapshot = {
  projectPath: null,
  workspaceName: null,
  activePanelId: null,
  viewType: null,
  appearance: "dark",
  themeRevision: 0,
};
export function commandAvailable(
  context: CommandContext | undefined,
  snapshot: HostSnapshot,
  textInput: boolean,
) {
  return (
    !(textInput && !context?.textInput) &&
    (!context?.workspace || !!snapshot.workspaceName) &&
    (!context?.viewTypes || context.viewTypes.includes(snapshot.viewType ?? ""))
  );
}
export class PluginHost {
  readonly views = new Map<
    string,
    { owner: string; component: ComponentType<ViewProps> }
  >();
  readonly commands = new Map<
    string,
    { owner: string; handler: () => void | Promise<void> }
  >();
  readonly fills = new Map<
    string,
    { owner: string; component: ComponentType }
  >();
  readonly dirtyViews = new Map<string, { owner: string; view: DirtyView }>();
  readonly statuses = new Map<string, PluginStatus>();
  catalog: PluginCatalog = { directory: "", entries: [], safeMode: false };
  context = emptyContext;
  openView: (id: string, state: Json) => Promise<string> = async () => {
    throw new Error("Open a workspace first.");
  };
  private readonly events = mitt<{ [K in keyof HostEvents]: HostEvents[K] }>();
  private readonly listeners = new Set<() => void>();
  private readonly activations = new Map<string, Activation>();
  private version = 0;
  private ready = false;
  private discovery = 0;
  private themeRevision = -1;
  panelOwner: (id: string) => string | undefined = () => undefined;
  isDirty(id: string) {
    try {
      return this.dirtyViews.get(id)?.view.isDirty() ?? false;
    } catch {
      return true;
    }
  }
  retainPanels(ids: ReadonlySet<string>) {
    for (const id of this.dirtyViews.keys())
      if (!ids.has(id)) this.dirtyViews.delete(id);
  }
  start() {
    if (this.ready) return;
    this.ready = true;
    void this.discover(this.catalog);
  }
  private readonly loader: {
    prepare(entry: PluginEntry): Promise<PluginManifest>;
    import(entry: PluginEntry): Promise<PluginModule>;
    css(entry: PluginEntry, signal: AbortSignal): Promise<Dispose>;
    url(entry: PluginEntry, path: string): string;
  };
  constructor(loader: PluginHost["loader"]) {
    this.loader = loader;
  }
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  revision = () => this.version;
  private changed() {
    ++this.version;
    for (const fn of this.listeners) fn();
  }
  report(owner: string, phase: string, error: unknown) {
    const previous = this.statuses.get(owner);
    this.statuses.set(owner, {
      phase: previous?.phase === "active" ? "active" : "failed",
      evaluated: previous?.evaluated ?? false,
      error: `${phase}: ${error instanceof Error ? error.message : String(error)}`,
    });
    this.changed();
  }
  private safely(owner: string, phase: string, fn: () => unknown) {
    try {
      Promise.resolve(fn()).catch((error) => this.report(owner, phase, error));
    } catch (error) {
      this.report(owner, phase, error);
    }
  }
  setContext(context: HostSnapshot) {
    if (
      Object.entries(context).every(
        ([key, value]) => this.context[key as keyof HostSnapshot] === value,
      )
    )
      return;
    this.context = context;
    this.events.emit("context", context);
  }
  theme(revision: number, appearance: "light" | "dark") {
    if (revision === this.themeRevision) return;
    this.themeRevision = revision;
    this.events.emit("theme", { revision, appearance });
  }
  async discover(catalog: PluginCatalog) {
    const generation = ++this.discovery;
    for (const raw of catalog.entries) {
      try {
        if (raw.manifest) {
          raw.manifest = parsePlugin(raw.manifest);
          if (raw.manifest.id !== raw.id)
            throw new Error("Installed identity differs from the manifest.");
        }
      } catch (error) {
        raw.manifest = null;
        raw.error = String(error);
      }
    }
    this.catalog = catalog;
    for (const [owner] of this.activations) {
      const entry = catalog.entries.find((e) => e.id === owner);
      if (!entry?.enabled || !entry.manifest || catalog.safeMode)
        await this.deactivate(owner);
    }
    if (generation !== this.discovery) return;
    for (const id of this.statuses.keys())
      if (!catalog.entries.some((entry) => entry.id === id))
        this.statuses.delete(id);
    for (const entry of catalog.entries)
      if (!this.activations.has(entry.id))
        this.statuses.set(entry.id, {
          phase: entry.error
            ? "incompatible"
            : entry.enabled && !catalog.safeMode
              ? "discovered"
              : "disabled",
          error: entry.error ?? "",
          evaluated: entry.evaluated,
        });
    this.changed();
    for (const entry of catalog.entries)
      if (
        this.ready &&
        entry.enabled &&
        entry.manifest?.activation === "startup" &&
        !catalog.safeMode
      )
        void this.activate(entry.id).catch(() => {});
  }
  activate(owner: string): Promise<void> {
    const existing = this.activations.get(owner);
    if (existing) return existing.promise;
    const entry = this.catalog.entries.find((e) => e.id === owner);
    if (
      !entry?.enabled ||
      !entry.manifest?.entry ||
      entry.trustedRevision !== entry.revision ||
      this.catalog.safeMode
    )
      return Promise.reject(
        new Error("Enable and trust this plugin in Settings → Plugins first."),
      );
    if (entry.restartRequired)
      return Promise.reject(
        new Error("Restart SimpleBench to use the updated plugin."),
      );
    const activation: Activation = {
      abort: new AbortController(),
      disposers: new Set(),
      promise: Promise.resolve(),
    };
    this.activations.set(owner, activation);
    const live = () => {
      if (
        activation.abort.signal.aborted ||
        this.activations.get(owner) !== activation
      )
        throw new Error("Plugin activation was disposed.");
    };
    const add = (dispose: Dispose): Dispose => {
      if (typeof dispose !== "function")
        throw new Error("Expected a disposer function.");
      try {
        live();
      } catch (error) {
        this.safely(owner, "late cleanup", dispose);
        throw error;
      }
      const once = () => {
        if (!activation.disposers.delete(once)) return;
        this.safely(owner, "cleanup", dispose);
      };
      activation.disposers.add(once);
      return once;
    };
    const owned = <T>(
      registry: Map<string, { owner: string } & T>,
      id: string,
      data: T,
      allowed: { id: string }[] | undefined,
    ) => {
      live();
      if (!allowed?.some((item) => item.id === id))
        throw new Error(`Undeclared contribution: ${id}`);
      if (registry.has(id))
        throw new Error(`Contribution already registered: ${id}`);
      registry.set(id, { owner, ...data });
      this.changed();
      return add(() => {
        registry.delete(id);
        this.changed();
      });
    };
    const manifest = entry.manifest;
    const context: PluginContext = {
      id: owner,
      signal: activation.abort.signal,
      snapshot: () => this.context,
      add,
      registerView: (id, component) =>
        owned(this.views, id, { component }, manifest.contributes?.views),
      registerCommand: (id, handler) =>
        owned(this.commands, id, { handler }, manifest.contributes?.commands),
      registerFill: (id, component) =>
        owned(this.fills, id, { component }, manifest.contributes?.fills),
      registerDirtyView: (id, view) => {
        live();
        if (
          !view ||
          typeof view.title !== "string" ||
          view.title.length > 160 ||
          [view.isDirty, view.save, view.discard].some(
            (fn) => typeof fn !== "function",
          )
        )
          throw new Error("Invalid dirty-view contract.");
        if (this.panelOwner(id) !== owner)
          throw new Error(
            "A close guard must belong to this plugin’s retained panel.",
          );
        if (this.dirtyViews.has(id))
          throw new Error("This panel already owns a close guard.");
        this.dirtyViews.set(id, { owner, view });
        return add(() => this.dirtyViews.delete(id));
      },
      openView: async (id, state = null) => {
        live();
        jsonState(state);
        if (!manifest.contributes?.views?.some((v) => v.id === id))
          throw new Error("A plugin can open only its declared views.");
        return this.openView(id, state);
      },
      executeCommand: async (id) => {
        live();
        await this.execute(id);
      },
      subscribe: (event, handler) => {
        live();
        const safe = (payload: any) =>
          this.safely(owner, `event ${event}`, () => handler(payload));
        this.events.on(event, safe);
        return add(() => this.events.off(event, safe));
      },
      interval: (callback, milliseconds) => {
        live();
        if (
          !Number.isFinite(milliseconds) ||
          milliseconds < 50 ||
          milliseconds > 2147483647
        )
          throw new Error("Timer interval must be between 50 ms and 24 days.");
        const timer = setInterval(
          () => this.safely(owner, "timer", callback),
          milliseconds,
        );
        return add(() => clearInterval(timer));
      },
      assetUrl: (path) => {
        live();
        packagePath(path);
        return this.loader.url(entry, path);
      },
    };
    this.statuses.set(owner, {
      phase: "activating",
      evaluated: entry.evaluated,
      error: "",
    });
    this.changed();
    activation.promise = Promise.resolve().then(async () => {
      try {
        const prepared = parsePlugin(await this.loader.prepare(entry));
        live();
        if (JSON.stringify(prepared) !== JSON.stringify(manifest))
          throw new Error("Plugin metadata changed before activation.");
        add(await this.loader.css(entry, activation.abort.signal));
        live();
        const module = await this.loader.import(entry);
        activation.module = module;
        live();
        this.statuses.set(owner, {
          phase: "activating",
          evaluated: true,
          error: "",
        });
        if (typeof module.activate !== "function")
          throw new Error("The entry must export activate(context).");
        const dispose = await module.activate(context);
        if (dispose) add(dispose);
        live();
        this.statuses.set(owner, {
          phase: "active",
          evaluated: true,
          error: "",
        });
        this.changed();
        this.events.emit("contributions", { owner });
      } catch (error) {
        if (this.activations.get(owner) === activation) {
          this.activations.delete(owner);
          activation.abort.abort();
          for (const dispose of [...activation.disposers].reverse()) dispose();
          this.report(owner, "activation", error);
        }
        await this.stopModule(owner, activation);
        throw error;
      }
    });
    return activation.promise;
  }
  private async stopModule(owner: string, activation: Activation) {
    if (!activation.module || activation.deactivated) return;
    activation.deactivated = true;
    try {
      await activation.module.deactivate?.();
    } catch (error) {
      this.report(owner, "deactivation", error);
    }
  }
  async deactivate(owner: string) {
    const activation = this.activations.get(owner);
    if (!activation) return;
    this.activations.delete(owner);
    activation.abort.abort();
    const evaluated = this.statuses.get(owner)?.evaluated ?? false;
    this.statuses.set(owner, { phase: "deactivating", evaluated, error: "" });
    for (const dispose of [...activation.disposers].reverse()) dispose();
    await this.stopModule(owner, activation);
    this.statuses.set(owner, {
      phase: "disabled",
      evaluated,
      error: this.statuses.get(owner)?.error ?? "",
    });
    this.changed();
    this.events.emit("contributions", { owner });
  }
  async execute(id: string, textInput = false) {
    const entry = this.catalog.entries.find((e) =>
      e.manifest?.contributes?.commands?.some((c) => c.id === id),
    );
    const command = entry?.manifest?.contributes?.commands?.find(
      (c) => c.id === id,
    );
    if (!entry || !command) throw new Error("Command is unavailable.");
    if (!commandAvailable(command.context, this.context, textInput))
      throw new Error("Command is unavailable in this context.");
    try {
      await this.activate(entry.id);
      const registered = this.commands.get(id);
      if (!registered)
        throw new Error("The plugin did not register this command.");
      await registered.handler();
    } catch (error) {
      this.report(entry.id, "command", error);
      throw error;
    }
  }
}
