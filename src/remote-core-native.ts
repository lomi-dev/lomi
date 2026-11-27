/**
 * Desktop face of the shared Remote core (plan-1.0 P2.19). The core runs in Rust
 * (`src-tauri/src/remote_core`): session exchange, relay link, encrypted directory and
 * token renewal. This module only mirrors its state for the shared Remote Web panel, so
 * the Host list has one source of truth. The shape matches the web `CoreClient`.
 */
export interface NativeHost {
  id: string;
  name: string | null;
  online: boolean;
  accessEpoch: string;
  runtimeGeneration: string;
  workspaces: Array<{ id: string; name: string | null; epoch: string }>;
}
export interface NativeCoreState {
  status:
    | "idle"
    | "cached"
    | "session"
    | "link-ready"
    | "link-down"
    | "update-required"
    | "signed-out";
  hosts: NativeHost[];
  fromMemory: boolean;
  refreshing: boolean;
  fresh: boolean;
  degraded: boolean;
  accountId?: string;
}
export type NativeCoreEvent =
  | { type: "state"; state: NativeCoreState }
  | { type: "mark"; name: string; detail?: Record<string, number> };

export interface NativeCoreDeps {
  /** Resolves false when the build does not configure the core. */
  start(
    onEvent: (event: NativeCoreEvent) => void,
    expectedAccount?: string,
  ): Promise<boolean>;
  names(hosts: unknown[]): Promise<void>;
  renew(): Promise<void>;
  wipe(): Promise<void>;
  mark(name: string, detail?: Record<string, number>): void;
}

// Fail closed until Rust speaks: the old inventory reads stay on.
const INITIAL: NativeCoreState = {
  status: "idle",
  hosts: [],
  fromMemory: false,
  refreshing: false,
  fresh: false,
  degraded: true,
};

/** Same key as the web client: what the connect flow depends on (not names). */
export const availabilityKey = (hosts: NativeHost[]) =>
  JSON.stringify(
    hosts.map((h) => [
      h.id,
      h.online,
      h.accessEpoch,
      h.runtimeGeneration,
      h.workspaces.map((w) => [w.id, w.epoch]),
    ]),
  );

export class NativeCoreClient {
  private state: NativeCoreState = INITIAL;
  private readonly listeners = new Set<() => void>();
  private readonly availability = new Set<() => void>();
  private startedFor: string | undefined | null = null;

  private readonly deps: NativeCoreDeps;

  constructor(deps: NativeCoreDeps) {
    this.deps = deps;
  }

  getState = (): NativeCoreState => this.state;
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
  onAvailabilityChange(listener: () => void): () => void {
    this.availability.add(listener);
    return () => this.availability.delete(listener);
  }

  /** Account known: start (or re-ask for) the session exchange; Rust guards the account. */
  async connect(accountId: string, _linkUrl?: string): Promise<void> {
    if (this.startedFor === accountId)
      return void this.deps.renew().catch(() => {});
    this.startedFor = accountId;
    try {
      const enabled = await this.deps.start((e) => this.onEvent(e), accountId);
      if (!enabled) this.set({ ...INITIAL });
    } catch {
      this.set({ ...INITIAL });
    }
  }

  start(): Promise<void> {
    return Promise.resolve();
  }
  renew(): Promise<void> {
    return this.deps.renew();
  }
  noteNames(hosts: unknown[]): void {
    void this.deps.names(hosts).catch(() => {});
  }
  async wipe(): Promise<void> {
    await this.deps.wipe();
    this.startedFor = null;
  }
  stop(): void {}

  private set(next: NativeCoreState) {
    this.state = next;
    for (const listener of [...this.listeners]) listener();
  }

  private onEvent(event: NativeCoreEvent) {
    if (event.type === "mark") return this.deps.mark(event.name, event.detail);
    const before = this.state;
    this.set(event.state);
    // Cache snapshots never trigger a connect re-read; session and link changes do.
    if (
      !event.state.fromMemory &&
      availabilityKey(before.hosts) !== availabilityKey(event.state.hosts)
    )
      for (const listener of [...this.availability]) listener();
  }
}
