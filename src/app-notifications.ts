import { api, errorMessage, native } from "./api";
import { reportNotificationError } from "./notifications";
import type {
  ApplicationNotificationAction,
  ApplicationNotificationSource,
  InboxNotification,
  NotificationSnapshot,
} from "./notifications";

interface ApplicationNotification {
  body: string;
  title?: string;
  source?: ApplicationNotificationSource;
  kind?: InboxNotification["kind"];
  action?: ApplicationNotificationAction;
}

const snapshots = new Set<(snapshot: NotificationSnapshot) => void>();
const recent = new Map<string, number>();
const failed = new Map<
  string,
  { request: ApplicationNotification; item: InboxNotification }
>();
const pending = new Map<string, Promise<void>>();
let droppedNotifications = 0;
let pendingSequence = 0;
let unsaved: InboxNotification[] = [];
const unsavedListeners = new Set<() => void>();
export const unsavedApplicationNotifications = () => unsaved;
export function subscribeUnsavedApplicationNotifications(listener: () => void) {
  unsavedListeners.add(listener);
  return () => {
    unsavedListeners.delete(listener);
  };
}
function publishUnsaved() {
  unsaved = [...failed.values()].map((value) => value.item).reverse();
  for (const listener of unsavedListeners) listener();
}
export function dismissUnsavedApplicationNotification(id: string) {
  for (const [key, value] of failed)
    if (value.item.id === id) failed.delete(key);
  publishUnsaved();
}
export function clearUnsavedApplicationNotifications() {
  failed.clear();
  droppedNotifications = 0;
  publishUnsaved();
}

export function subscribeApplicationNotifications(
  listener: (snapshot: NotificationSnapshot) => void,
) {
  snapshots.add(listener);
  return () => {
    snapshots.delete(listener);
  };
}

export function reportAppNotification(
  notification: ApplicationNotification,
  options: { deduplicate?: boolean } = {},
) {
  if (!native || !notification.body) return Promise.resolve();
  const source = notification.source ?? "app";
  const kind = notification.kind ?? "attention";
  const title =
    notification.title ??
    (kind === "info"
      ? "Application update"
      : {
          app: "Application needs attention",
          android: "Android needs attention",
          mcp: "MCP needs attention",
        }[source]);
  const clean = (value: string, limit: number) => {
    const characters = Array.from(value.replace(/[\x00-\x1f\x7f-\x9f]/g, " "));
    return characters.length > limit
      ? `${characters.slice(0, limit - 1).join("")}…`
      : characters.join("");
  };
  const args = {
    source,
    kind,
    title: clean(title, 100),
    body: clean(notification.body, 4000),
    ...(notification.action ? { action: notification.action } : {}),
  };
  const key = JSON.stringify(args);
  const active = pending.get(key);
  if (active) return active;
  const now = Date.now();
  const previous = recent.get(key);
  if (
    options.deduplicate !== false &&
    previous !== undefined &&
    now - previous < 5000
  )
    return Promise.resolve();
  const request = (async () => {
    try {
      const snapshot = await api<NotificationSnapshot>(
        "record_app_notification",
        args,
      );
      if (failed.delete(key)) publishUnsaved();
      recent.set(key, Date.now());
      while (recent.size > 200) recent.delete(recent.keys().next().value!);
      for (const listener of snapshots) listener(snapshot);
    } catch (reason) {
      failed.set(key, {
        request: args,
        item: failed.get(key)?.item ?? {
          ...args,
          id: `unsaved-${++pendingSequence}`,
          createdAt: Date.now(),
          read: false,
        },
      });
      while (failed.size > 200) {
        failed.delete(failed.keys().next().value!);
        droppedNotifications++;
      }
      publishUnsaved();
      reportNotificationError(
        `Could not save application notification: ${errorMessage(reason)}${droppedNotifications ? " Older unsaved notifications could not be retained." : ""}`,
      );
    } finally {
      pending.delete(key);
    }
  })();
  pending.set(key, request);
  return request;
}

export async function retryApplicationNotifications() {
  for (const [key, value] of [...failed])
    if (failed.get(key) === value) await reportAppNotification(value.request);
  if (!failed.size && droppedNotifications) {
    const dropped = droppedNotifications;
    droppedNotifications -= dropped;
    await reportAppNotification(
      {
        title: "Some notifications could not be saved",
        body: `${dropped} older application notifications could not be saved while notification storage was unavailable.`,
      },
      { deduplicate: false },
    );
  }
  if (failed.size)
    throw new Error(
      "Some application notifications could not be saved. Retry to keep them in your inbox.",
    );
}

export function hasPendingApplicationNotifications() {
  return failed.size > 0 || droppedNotifications > 0;
}

type Action = { label: string; run: () => void | Promise<unknown> };
const actions = new Map<ApplicationNotificationAction, Action>([
  [
    "open-android-settings",
    {
      label: "Android settings",
      run: () => api("open_settings", { page: "android" }),
    },
  ],
  [
    "open-agent-control-settings",
    {
      label: "Agent control settings",
      run: () => api("open_settings", { page: "agent-control" }),
    },
  ],
]);
const actionListeners = new Set<() => void>();
let actionRevision = 0;
export const notificationActionsRevision = () => actionRevision;
export function subscribeNotificationActions(listener: () => void) {
  actionListeners.add(listener);
  return () => {
    actionListeners.delete(listener);
  };
}
export function notificationAction(action: ApplicationNotificationAction) {
  return actions.get(action);
}
export function registerNotificationAction(
  id: ApplicationNotificationAction,
  action: Action,
) {
  actions.set(id, action);
  const publish = () => {
    actionRevision++;
    for (const listener of actionListeners) listener();
  };
  publish();
  return () => {
    if (actions.get(id) === action) {
      actions.delete(id);
      publish();
    }
  };
}
