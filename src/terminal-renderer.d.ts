declare module "@lomi/terminal-renderer-experimental" {
  export function applySnapshot(
    terminal: import("@xterm/xterm").Terminal,
    snapshot: unknown,
  ): unknown;
  export function applyPatch(
    terminal: import("@xterm/xterm").Terminal,
    patch: unknown,
  ): unknown;
  export function setInputEnabled(
    terminal: import("@xterm/xterm").Terminal,
    enabled: boolean,
  ): void;
}

