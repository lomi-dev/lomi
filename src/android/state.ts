import { useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { api, errorMessage, native } from "../api";
import { parseDevices, parsePreferences } from "./types";
import type {
  Changed,
  Progress,
  Snapshot,
  Status,
  StreamStatus,
} from "./types";

interface State {
  agentInput: Record<string, string | undefined>;
  snapshot: Snapshot | null;
  loading: boolean;
  error: string;
}
let state: State = {
  snapshot: null,
  loading: false,
  error: "",
  agentInput: {},
};
const subscribers = new Set<() => void>();
const eventListeners = new Set<(event: Changed) => void>();
let stop: (() => void) | undefined;
let connection = 0;
let pending: Promise<void> | undefined;
let changes = 0;
let metadataChanges = 0;
const statusEvents = new Map<string, { sequence: number; value: Status }>();
const streamEvents = new Map<
  string,
  { sequence: number; value: StreamStatus }
>();
let progressEvent: { sequence: number; value: Progress } | undefined;
function publish(next: State) {
  state = next;
  for (const listener of subscribers) listener();
}

export function refreshAndroid(): Promise<void> {
  if (pending) return pending;
  publish({ ...state, loading: true, error: "" });
  pending = (async () => {
    try {
      // Events received during the read win. Only metadata changes require
      // another read; frame/progress events never cause repeated SDK inspection.
      for (;;) {
        const before = changes;
        const metadataBefore = metadataChanges;
        const snapshot = await api<Snapshot>("android_state");
        if (snapshot.preferences)
          snapshot.preferences = parsePreferences(snapshot.preferences);
        if (snapshot.devices) snapshot.devices = parseDevices(snapshot.devices);
        if (metadataBefore !== metadataChanges) continue;
        for (const { sequence, value } of statusEvents.values()) {
          if (sequence > before)
            snapshot.statuses = [
              ...snapshot.statuses.filter(
                (status) => status.deviceId !== value.deviceId,
              ),
              value,
            ];
        }
        for (const { sequence, value } of streamEvents.values()) {
          if (sequence > before)
            snapshot.streams = [
              ...snapshot.streams.filter(
                (status) => status.deviceId !== value.deviceId,
              ),
              value,
            ];
        }
        if (progressEvent && progressEvent.sequence > before)
          snapshot.operation = progressEvent.value;
        publish({ ...state, snapshot, loading: false, error: "" });
        break;
      }
    } catch (error) {
      publish({ ...state, loading: false, error: errorMessage(error) });
    } finally {
      pending = undefined;
    }
  })();
  return pending;
}

function receive(event: Changed) {
  if (event.kind === "inputControl") {
    publish({
      ...state,
      agentInput: {
        ...state.agentInput,
        [event.value.deviceId]: event.value.controlled
          ? event.value.generation
          : undefined,
      },
    });
  }
  changes++;
  if (event.kind === "metadata") metadataChanges++;
  if (event.kind === "status")
    statusEvents.set(event.value.deviceId, {
      sequence: changes,
      value: event.value,
    });
  if (event.kind === "operation")
    progressEvent = { sequence: changes, value: event.value };
  if (event.kind === "stream") {
    const previous = streamEvents.get(event.value.deviceId);
    if (previous && previous.value.epoch > event.value.epoch) return;
    streamEvents.set(event.value.deviceId, {
      sequence: changes,
      value: event.value,
    });
  }
  const snapshot = state.snapshot;
  if (snapshot) {
    switch (event.kind) {
      case "operation":
        publish({
          ...state,
          snapshot: { ...snapshot, operation: event.value },
        });
        break;
      case "status":
        publish({
          ...state,
          snapshot: {
            ...snapshot,
            statuses: [
              ...snapshot.statuses.filter(
                (status) => status.deviceId !== event.value.deviceId,
              ),
              event.value,
            ],
          },
        });
        break;
      case "stream": {
        const current = snapshot.streams.find(
          (status) => status.deviceId === event.value.deviceId,
        );
        if (!current || current.epoch <= event.value.epoch)
          publish({
            ...state,
            snapshot: {
              ...snapshot,
              streams: [
                ...snapshot.streams.filter(
                  (status) => status.deviceId !== event.value.deviceId,
                ),
                event.value,
              ],
            },
          });
        break;
      }
      case "inputError":
        publish({ ...state, error: event.value });
        break;
    }
  }
  for (const listener of eventListeners) listener(event);
  if (event.kind === "metadata") void refreshAndroid();
}

function subscribe(listener: () => void) {
  subscribers.add(listener);
  if (subscribers.size === 1) {
    const current = ++connection;
    if (native)
      void listen<Changed>(
        "android-changed",
        ({ payload }) => {
          if (current === connection) receive(payload);
        },
        { target: { kind: "Webview", label: getCurrentWebview().label } },
      )
        .then((unlisten) => {
          if (current !== connection) unlisten();
          else {
            stop = unlisten;
            void refreshAndroid();
          }
        })
        .catch((error) => publish({ ...state, error: errorMessage(error) }));
    else void refreshAndroid();
  }
  return () => {
    subscribers.delete(listener);
    if (!subscribers.size) {
      connection++;
      stop?.();
      stop = undefined;
    }
  };
}

/** This module is imported only by Android's lazy UI/runtime. No startup timer. */
export function useAndroid() {
  return useSyncExternalStore(subscribe, () => state);
}
export function observeAndroid(listener: (event: Changed) => void) {
  eventListeners.add(listener);
  const unsubscribe = subscribe(() => {});
  return () => {
    eventListeners.delete(listener);
    unsubscribe();
  };
}
export function androidSnapshot() {
  return state.snapshot;
}
