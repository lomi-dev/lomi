// Document-start, main-frame-only listener in the retained private content world.
// Neither object coercion nor stacks, source URLs, console or network are collected.
(() => {
  const state = { started: Date.now(), next: 1, dropped: 0, entries: [] };
  Object.defineProperty(globalThis, "__lomiAgentLogsV1", { value: state });
  const append = (kind, message) => {
    if (state.next >= Number.MAX_SAFE_INTEGER) return;
    if (state.entries.length === 64) {
      state.entries.shift();
      state.dropped++;
    }
    state.entries.push({
      sequence: state.next++,
      capturedAtMillis: Date.now(),
      kind,
      message:
        typeof message === "string"
          ? new TextDecoder().decode(
              new TextEncoder().encode(message.slice(0, 256)),
            )
          : "Non-string error message omitted",
    });
  };
  addEventListener("error", (event) => {
    if (event instanceof ErrorEvent && event.isTrusted)
      append("javascript_error", event.message);
  });
})();
