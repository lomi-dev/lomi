import { msg } from "../i18n";
import type { Status } from "./types";

export const deviceStatusLabels: Record<Status["phase"], string> = {
  stopped: msg("Stopped"),
  starting: msg("Preparing phone…"),
  booting: msg("Starting Android…"),
  running: msg("Running"),
  stopping: msg("Stopping…"),
  failed: msg("Needs attention"),
};
