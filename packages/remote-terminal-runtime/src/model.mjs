import headless from "@xterm/headless";
import {
  captureSnapshot,
  restoreSnapshot,
  validateSnapshot,
  safeBoundary,
  SCROLLBACK,
  MAX_SNAPSHOT_BYTES,
  installTerminalProfile,
  assertModelBounds,
} from "./snapshot.mjs";
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const MAX_SUFFIX = 512 * 1024;
const validSeq = (v) => Number.isSafeInteger(v) && v >= 0;
function dims(c, r) {
  if (
    !Number.isInteger(c) ||
    c < 2 ||
    c > 500 ||
    !Number.isInteger(r) ||
    r < 1 ||
    r > 500
  )
    throw new Error("INVALID_DIMENSIONS");
}
function bytes(data) {
  if (
    typeof data !== "string" ||
    data.length > 21848 ||
    !/^[A-Za-z0-9+/]*={0,2}$/.test(data)
  )
    throw new Error("INVALID_DATA");
  const b = Buffer.from(data, "base64");
  if (b.length > 16384 || b.toString("base64") !== data)
    throw new Error("INVALID_DATA");
  return b;
}
export const writeTerminal = (t, bytes) =>
  new Promise((resolve) => t.write(new Uint8Array(bytes), resolve));
export class TerminalModels {
  constructor() {
    this.sessions = new Map();
  }
  async request(req) {
    if (
      !req ||
      typeof req !== "object" ||
      Array.isArray(req) ||
      !validSeq(req.id) ||
      !UUID.test(req.sessionId) ||
      !UUID.test(req.epoch) ||
      !validSeq(req.seq) ||
      !["create", "restore", "write", "resize", "snapshot", "drop"].includes(
        req.op,
      )
    )
      throw new Error("INVALID_REQUEST");
    const allowed = [
      "id",
      "op",
      "sessionId",
      "epoch",
      "seq",
      ...(req.op === "write" ? ["data"] : []),
      ...(req.op === "restore" ? ["state", "suffix", "cols", "rows"] : []),
      ...(["create", "resize"].includes(req.op) ? ["cols", "rows"] : []),
    ];
    if (Object.keys(req).some((k) => !allowed.includes(k)))
      throw new Error("INVALID_REQUEST");
    const old = this.sessions.get(req.sessionId);
    if (["create", "restore"].includes(req.op)) {
      if (
        old ||
        this.sessions.size >= 32 ||
        (req.op === "create" && req.seq !== 0)
      )
        throw new Error("SESSION_LIMIT");
      dims(req.cols, req.rows);
      const term = new headless.Terminal({
        cols: req.cols,
        rows: req.rows,
        scrollback: SCROLLBACK,
        installTerminalProfile,
        assertModelBounds,
        allowProposedApi: true,
        logLevel: "off",
        windowsPty: undefined,
      });
      // Hyperlinks are text only in this profile; no link IDs or arbitrary hrefs cross the wire.
      installTerminalProfile(term);
      const session = {
        term,
        epoch: req.epoch,
        seq: 0,
        checkpoint: captureSnapshot(term),
        suffix: [],
        suffixBytes: 0,
        checkpointAt: Date.now(),
      };
      try {
        if (req.op === "restore") {
          validateSnapshot(req.state);
          if (
            req.state.cols !== req.cols ||
            req.state.rows !== req.rows ||
            typeof req.suffix !== "string" ||
            req.suffix.length > Math.ceil(MAX_SUFFIX / 3) * 4 ||
            !/^[A-Za-z0-9+/]*={0,2}$/.test(req.suffix)
          )
            throw new Error("INVALID_REQUEST");
          const suffix = Buffer.from(req.suffix, "base64");
          if (
            suffix.length > MAX_SUFFIX ||
            suffix.toString("base64") !== req.suffix
          )
            throw new Error("INVALID_DATA");
          restoreSnapshot(term, req.state);
          await writeTerminal(term, suffix);
          assertModelBounds(term);
          session.seq = req.seq;
          session.checkpoint = req.state;
          session.suffix = suffix.length ? [suffix] : [];
          session.suffixBytes = suffix.length;
        }
        this.sessions.set(req.sessionId, session);
        return { seq: session.seq };
      } catch (error) {
        term.dispose();
        throw error;
      }
    }
    if (!old || old.epoch !== req.epoch) throw new Error("STALE_SESSION");
    if (req.op === "drop") {
      if (req.seq !== old.seq) throw new Error("STALE_SEQUENCE");
      old.term?.dispose();
      this.sessions.delete(req.sessionId);
      return { dropped: true };
    }
    if (old.failed) throw new Error("MODEL_LIMIT");
    if (req.op === "snapshot") {
      if (req.seq !== old.seq) throw new Error("STALE_SEQUENCE");
      if (safeBoundary(old.term)) this.checkpoint(old);
      if (!old.checkpoint || old.suffixBytes > MAX_SUFFIX)
        throw new Error("SNAPSHOT_PENDING");
      const result = {
        state: old.checkpoint,
        suffix: Buffer.concat(old.suffix).toString("base64"),
        throughSeq: old.seq,
        cols: old.term.cols,
        rows: old.term.rows,
      };
      // Reserve application metadata; the total encrypted JSON must fit transport reassembly.
      if (
        Buffer.byteLength(JSON.stringify(result), "utf8") >
        MAX_SNAPSHOT_BYTES - 4096
      )
        throw new Error("SNAPSHOT_LIMIT");
      return result;
    }
    if (req.seq !== old.seq + 1) throw new Error("STALE_SEQUENCE");
    if (req.op === "resize") {
      dims(req.cols, req.rows);
      old.term.resize(req.cols, req.rows);
      old.seq = req.seq;
      if (safeBoundary(old.term)) this.checkpoint(old);
      else {
        old.checkpoint = null;
        old.suffix = [];
        old.suffixBytes = 0;
      }
      return { seq: old.seq };
    }
    const data = bytes(req.data);
    await writeTerminal(old.term, data);
    try {
      assertModelBounds(old.term);
    } catch {
      old.term.dispose();
      old.term = null;
      old.checkpoint = null;
      old.suffix = [];
      old.failed = true;
      old.seq = req.seq;
      throw new Error("MODEL_LIMIT");
    }
    old.seq = req.seq;
    old.suffix.push(data);
    old.suffixBytes += data.length;
    if (
      safeBoundary(old.term) &&
      (!old.checkpoint ||
        old.suffixBytes >= 128 * 1024 ||
        Date.now() - old.checkpointAt >= 1000)
    )
      this.checkpoint(old);
    else if (old.suffixBytes > MAX_SUFFIX) {
      old.checkpoint = null;
      old.suffix = [];
      old.suffixBytes = 0;
    }
    return { seq: old.seq };
  }
  checkpoint(s) {
    s.checkpoint = captureSnapshot(s.term);
    s.suffix = [];
    s.suffixBytes = 0;
    s.checkpointAt = Date.now();
  }
  dispose() {
    for (const s of this.sessions.values()) s.term?.dispose();
    this.sessions.clear();
  }
}
