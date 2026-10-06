import { api } from "../api";
import { freezeNativeProject } from "../editor-service";
import { newId } from "../model";
import type { PendingPermission } from "./types";
const freezes = new Map<string, { release: () => void; token: string }>();
let polling: ReturnType<typeof setInterval> | undefined;
let checking = false;
async function settle() {
  if (checking) return;
  checking = true;
  try {
    for (const [id, frozen] of freezes) {
      try {
        const result = await api<{ complete: boolean }>(
          "agent_permission_freeze_complete",
          { editorFreezeToken: frozen.token },
        );
        if (result.complete) {
          frozen.release();
          freezes.delete(id);
        }
      } catch {
        /* Retain the editor freeze until native settlement is confirmed. */
      }
    }
    if (!freezes.size && polling) {
      clearInterval(polling);
      polling = undefined;
    }
  } finally {
    checking = false;
  }
}
export async function replyPermission(
  permission: PendingPermission,
  allow: boolean,
  choice: unknown,
) {
  const binding = {
    taskId: permission.taskId,
    attemptId: permission.attemptId,
    generation: permission.generation,
    approvalToken: permission.approvalToken,
  };
  let frozen = freezes.get(permission.approvalToken);
  if (allow && !frozen) {
    const fence = await freezeNativeProject(permission.projectRoot);
    try {
      const registered = await api<{ editorFreezeToken: string }>(
        "agent_permission_freeze",
        { request: { operationId: newId(), ...binding } },
      );
      frozen = { release: fence.release, token: registered.editorFreezeToken };
      freezes.set(permission.approvalToken, frozen);
      if (!polling) polling = setInterval(() => void settle(), 500);
    } catch (cause) {
      fence.release();
      throw cause;
    }
  }
  return api("agent_permission_reply", {
    request: {
      operationId: newId(),
      ...binding,
      allow,
      choice: choice ?? null,
      editorFreezeToken: frozen?.token ?? null,
    },
  });
}
