import { lazy } from "react";
import TerminalPane from "../TerminalPane";
import BrowserPane from "../BrowserPane";
import FileDiff from "../FileDiff";
import CommitDetails from "../CommitDetails";
// Typed adapters keep each built-in descriptor and its resource ownership intact.
export const builtinViews = {
  terminal: TerminalPane,
  file: lazy(() => import("../FileEditor")),
  browser: BrowserPane,
  android: lazy(() => import("../android/AndroidPane")),
  agentTask: lazy(() => import("../agent-runtime/TaskPane")),
  chat: lazy(() => import("../chat/ChatPane")),
  diff: FileDiff,
  commit: CommitDetails,
};
