import type { Chat } from "@ai-sdk/react";
import type { UIMessage } from "ai";

type Source = Pick<
  Chat<UIMessage>,
  | "messages"
  | "status"
  | "~registerMessagesCallback"
  | "~registerStatusCallback"
>;

// A trailing task keeps parser microtasks independent of render duration. A
// leading elapsed-time throttle can render every token when rendering itself
// takes longer than its interval, starving input and native acknowledgements.
export function chatMessageView(chat: Source) {
  let snapshot = chat.messages;
  const listeners = new Set<() => void>();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let stop: (() => void) | undefined;
  const publish = () => {
    clearTimeout(timer);
    timer = undefined;
    if (snapshot === chat.messages) return;
    snapshot = chat.messages;
    for (const listener of [...listeners]) listener();
  };
  return {
    getSnapshot: () => snapshot,
    subscribe(listener: () => void) {
      listeners.add(listener);
      if (listeners.size === 1) {
        const messages = chat["~registerMessagesCallback"](() => {
          timer ??= setTimeout(publish, 50);
        });
        const status = chat["~registerStatusCallback"](() => {
          if (chat.status === "ready" || chat.status === "error") publish();
        });
        stop = () => {
          messages();
          status();
          clearTimeout(timer);
          timer = undefined;
        };
        // useSyncExternalStore rechecks after subscribing, including a stream
        // update between render and mounting this view.
        snapshot = chat.messages;
      }
      return () => {
        listeners.delete(listener);
        if (!listeners.size) {
          stop?.();
          stop = undefined;
        }
      };
    },
  };
}
