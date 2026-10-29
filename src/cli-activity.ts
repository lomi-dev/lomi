import type { AgentUsageEntry } from "./agent-usage";

export type CliActivity = "working" | "attention" | "error";

export type TerminalProgramStatusKind = "permission" | "question" | "auth";

export type TerminalProgramStatus =
  | { state: "clear" }
  | {
      state: "idle" | "working" | "blocked" | "done" | "error";
      app: "codex";
      kind?: TerminalProgramStatusKind;
    };

const maxTerminalProgramStatusLength = 256;

export const cliUsageStaleAfterMs = 45_000;

export function aggregateCliActivity(
  activities: Iterable<CliActivity | null | undefined>,
): CliActivity | null {
  let strongest: CliActivity | null = null;
  for (const activity of activities) {
    if (activity === "error") return "error";
    if (activity === "attention") strongest = "attention";
    else if (activity === "working" && strongest === null)
      strongest = "working";
  }
  return strongest;
}

export function usageCliActivity(
  entry: AgentUsageEntry | null | undefined,
  now = Date.now(),
): CliActivity | null {
  if (
    !entry ||
    entry.status !== "ready" ||
    !Number.isFinite(now) ||
    entry.updatedAt === null ||
    !Number.isFinite(entry.updatedAt)
  )
    return null;

  const age = now - entry.updatedAt;
  if (age < 0 || age > cliUsageStaleAfterMs) return null;

  for (const window of entry.windows) {
    if (
      window.resetsAt !== null &&
      (!Number.isFinite(window.resetsAt) || now >= window.resetsAt)
    )
      continue;

    if (
      window.remainingPercent === 0 ||
      (window.used !== null &&
        Number.isFinite(window.used) &&
        window.limit !== null &&
        Number.isFinite(window.limit) &&
        window.limit > 0 &&
        window.used >= window.limit)
    )
      return "error";
  }

  return null;
}

export interface Osc9Progress {
  activity: CliActivity | null;
}

/** Parse the bounded, machine-readable OSC 7501 status emitted by Codex. */
export function parseTerminalProgramStatus(
  value: string,
): TerminalProgramStatus | null {
  if (!value || value.length > maxTerminalProgramStatusLength) return null;

  const fields = value.split(":");
  const stateField = fields[0];
  if (!stateField?.startsWith("state=")) return null;
  const state = stateField.slice("state=".length);

  if (state === "clear") return fields.length === 1 ? { state: "clear" } : null;

  if (
    state !== "idle" &&
    state !== "working" &&
    state !== "blocked" &&
    state !== "done" &&
    state !== "error"
  )
    return null;
  if (fields[1] !== "app=codex" || fields.length > 3) return null;

  if (fields.length === 2) return { state, app: "codex" };

  const kindField = fields[2];
  if (state !== "blocked" || !kindField?.startsWith("kind=")) return null;
  const kind = kindField.slice("kind=".length);
  if (kind !== "permission" && kind !== "question" && kind !== "auth")
    return null;
  return { state, app: "codex", kind };
}

/** Map a verified CLI program status onto the workspace activity indicator. */
export function terminalProgramStatusActivity(
  status: TerminalProgramStatus,
): CliActivity | null {
  if (status.state === "working") return "working";
  if (status.state === "blocked") return "attention";
  if (status.state === "error") return "error";
  return null;
}

function validProgress(value: string | undefined) {
  if (value === undefined || !/^\d{1,3}$/.test(value)) return false;
  const percentage = Number(value);
  return percentage >= 0 && percentage <= 100;
}

/** Parse the OSC 9;4 progress states used by Windows Terminal. */
export function parseOsc9Progress(value: string): Osc9Progress | null {
  const [command, state, progress, ...extra] = value.split(";");
  if (command !== "4" || extra.length) return null;

  if (state === "0")
    return progress === undefined || validProgress(progress)
      ? { activity: null }
      : null;
  if (state === "1")
    return validProgress(progress) ? { activity: "working" } : null;
  if (state === "2")
    return validProgress(progress) ? { activity: "error" } : null;
  if (state === "3")
    return progress === undefined || validProgress(progress)
      ? { activity: "working" }
      : null;
  if (state === "4")
    return validProgress(progress) ? { activity: "attention" } : null;

  return null;
}
