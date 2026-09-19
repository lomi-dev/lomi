import type { Status } from "./types";

export const deviceStatusLabels: Record<Status["phase"], string> = {
  stopped: "Stopped",
  starting: "Preparing phone…",
  booting: "Starting Android…",
  running: "Running",
  stopping: "Stopping…",
  failed: "Needs attention",
};
