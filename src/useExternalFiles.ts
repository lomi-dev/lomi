import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import { api, errorMessage } from "./api";

export interface ExternalFileRequest {
  id: number;
  path: string;
}

export type ProcessExternalFile = (
  path: string,
  isCurrent: () => boolean,
) => Promise<boolean>;

export interface UseExternalFilesOptions {
  enabled: boolean;
  ready: boolean;
  blocked: () => boolean;
  process: ProcessExternalFile;
  onError: (message: string) => void;
}

const retryDelayMs = 500;

/** Receives retained native file-open requests after subscribing to enqueue signals. */
export function useExternalFiles(options: UseExternalFilesOptions) {
  const latest = useRef(options);
  const drainRef = useRef<(() => void) | undefined>(undefined);
  latest.current = options;

  useEffect(() => {
    if (!options.enabled) return;

    let current = true;
    let subscribed = false;
    let attaching = false;
    let draining = false;
    let needsDrain = false;
    let retryTimer: ReturnType<typeof setTimeout> | undefined;
    let unlisten: (() => void) | undefined;
    const lastQueueErrors = new Map<string, string>();
    const reportedRequestErrors = new Set<number>();

    const reportQueueError = (command: string, error: unknown) => {
      const message = errorMessage(error);
      if (message !== lastQueueErrors.get(command))
        latest.current.onError(message);
      lastQueueErrors.set(command, message);
    };

    const blocked = () => {
      try {
        return latest.current.blocked();
      } catch {
        return true;
      }
    };

    const scheduleRetry = () => {
      if (!current || retryTimer) return;
      retryTimer = setTimeout(() => {
        retryTimer = undefined;
        if (!subscribed) attach();
        else requestDrain();
      }, retryDelayMs);
    };

    const drain = async () => {
      if (!current || !subscribed || draining) return;
      draining = true;
      try {
        while (current && needsDrain) {
          needsDrain = false;
          let requests: ExternalFileRequest[];
          try {
            requests = await api<ExternalFileRequest[]>(
              "external_files_pending",
            );
            lastQueueErrors.delete("external_files_pending");
          } catch (error) {
            reportQueueError("external_files_pending", error);
            scheduleRetry();
            break;
          }
          if (!current) break;

          if (!latest.current.ready) break;
          if (blocked()) {
            scheduleRetry();
            break;
          }

          let deferred = false;
          for (const request of requests) {
            if (!current) break;
            if (!latest.current.ready) {
              deferred = true;
              break;
            }
            if (blocked()) {
              deferred = true;
              break;
            }

            let processed = true;
            try {
              processed = await latest.current.process(
                request.path,
                () => current,
              );
            } catch (error) {
              if (!reportedRequestErrors.has(request.id)) {
                latest.current.onError(errorMessage(error));
                reportedRequestErrors.add(request.id);
              }
            }
            if (!current) break;
            if (!processed) {
              deferred = true;
              break;
            }

            try {
              await api("external_files_ack", { ids: [request.id] });
              lastQueueErrors.delete("external_files_ack");
              reportedRequestErrors.delete(request.id);
            } catch (error) {
              reportQueueError("external_files_ack", error);
              deferred = true;
              break;
            }
          }
          if (!current) break;
          if (deferred) {
            scheduleRetry();
            break;
          }
        }
      } finally {
        draining = false;
        if (current && needsDrain) void drain();
      }
    };

    const requestDrain = () => {
      needsDrain = true;
      if (subscribed) void drain();
    };

    function attach() {
      if (!current || subscribed || attaching) return;
      attaching = true;
      void listen("external-files-pending", requestDrain)
        .then((stop) => {
          attaching = false;
          if (!current) {
            stop();
            return;
          }
          subscribed = true;
          unlisten = stop;
          requestDrain();
        })
        .catch((error) => {
          attaching = false;
          reportQueueError("external-files-pending listener", error);
          scheduleRetry();
        });
    }

    drainRef.current = requestDrain;
    attach();

    return () => {
      current = false;
      if (drainRef.current === requestDrain) drainRef.current = undefined;
      if (retryTimer) clearTimeout(retryTimer);
      unlisten?.();
    };
  }, [options.enabled]);

  useEffect(() => {
    if (options.ready) drainRef.current?.();
  }, [options.ready]);
}
