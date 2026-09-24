// Main-frame, document-start page-world collection. No IPC or native authority.
// Capture intrinsics before page code; retain only bounded primitive messages.
(() => {
  const parse = JSON.parse;
  const quote = JSON.stringify;
  const now = Date.now.bind(Date);
  const apply = Reflect.apply;
  const slice = Function.prototype.call.bind(String.prototype.slice);
  const push = Function.prototype.call.bind(Array.prototype.push);
  const shift = Function.prototype.call.bind(Array.prototype.shift);
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  const encode = encoder.encode.bind(encoder);
  const decode = decoder.decode.bind(decoder);
  const stringifyPrimitive = String;
  const integer = Number.isSafeInteger;
  const rejectionType = globalThis.PromiseRejectionEvent;
  const started = now();
  const consoleRing = { next: 1, dropped: 0, entries: [] };
  const rejectionRing = { next: 1, dropped: 0, entries: [] };
  const primitive = (value) => {
    if (typeof value === "string") return slice(value, 0, 256);
    if (value === null) return "null";
    if (
      typeof value === "number" ||
      typeof value === "boolean" ||
      typeof value === "undefined"
    )
      return stringifyPrimitive(value);
    return "[object omitted]";
  };
  const append = (ring, kind, message, level, eventTrusted = null) => {
    if (ring.next >= 9007199254740991) return;
    const normalized = decode(encode(slice(message, 0, 256)));
    const sequence = ring.next++;
    const entry =
      '{"sequence":' +
      sequence +
      ',"capturedAtMillis":' +
      now() +
      ',"kind":' +
      quote(kind) +
      ',"message":' +
      quote(normalized) +
      ',"level":' +
      quote(level) +
      ',"eventTrusted":' +
      quote(eventTrusted) +
      "}";
    if (ring.entries.length === 64) {
      shift(ring.entries);
      ring.dropped++;
    }
    push(ring.entries, {
      sequence,
      encoded: entry,
      bytes: encode(entry).byteLength,
    });
  };
  for (const level of ["log", "info", "warn", "error", "debug"]) {
    const original = console[level];
    if (typeof original !== "function") continue;
    console[level] = function (...args) {
      let message = "";
      for (let i = 0; i < args.length && i < 16 && message.length < 256; i++) {
        message += (i ? " " : "") + primitive(args[i]);
      }
      append(consoleRing, "console", message, level);
      return apply(original, this, args);
    };
  }
  addEventListener("unhandledrejection", (event) => {
    // WKWebView also marks real unhandled rejections untrusted. Keep the
    // engine's flag; neither synthetic nor genuine page reports are authority.
    if (rejectionType && event instanceof rejectionType)
      append(
        rejectionRing,
        "promise_rejection",
        primitive(event.reason),
        null,
        event.isTrusted,
      );
  });
  const read = (payload) => {
    const q = parse(payload);
    if (now() > q.deadlineEpochMs) return '{"error":"DEADLINE_EXCEEDED"}';
    if (location.origin !== q.origin || location.href !== q.url)
      return '{"error":"STALE_SNAPSHOT"}';
    const ring =
      q.logKind === "console"
        ? consoleRing
        : q.logKind === "promise_rejection"
          ? rejectionRing
          : null;
    if (
      !ring ||
      !integer(q.after) ||
      q.after < 0 ||
      q.after >= ring.next ||
      !integer(q.limit) ||
      q.limit < 1 ||
      q.limit > 64
    )
      return '{"error":"CURSOR_EXPIRED"}';
    let entries = "",
      count = 0,
      bytes = 0,
      through = q.after;
    for (let i = 0; i < ring.entries.length; i++) {
      const entry = ring.entries[i];
      if (entry.sequence <= q.after) continue;
      if (count >= q.limit || bytes + entry.bytes > 49152) break;
      entries += (count++ ? "," : "") + entry.encoded;
      bytes += entry.bytes + 1;
      through = entry.sequence;
    }
    return (
      '{"captureStartedAtMillis":' +
      started +
      ',"entries":[' +
      entries +
      '],"dropped":' +
      ring.dropped +
      ',"hasMore":' +
      (through < ring.next - 1) +
      ',"through":' +
      through +
      "}"
    );
  };
  Object.defineProperty(globalThis, "__lomiAgentPageLogsV1", { value: read });
})();
