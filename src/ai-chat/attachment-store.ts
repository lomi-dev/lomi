import { useCallback, useState } from "react";
import { api, errorMessage } from "../api";
import type { Attachment } from "./types";

// Pending attachments per tab, kept like drafts (in memory only).
const pending = new Map<string, Attachment[]>();

// Data URLs by attachment id; a failed lookup is evicted so it can retry.
const previews = new Map<string, Promise<string>>();
export function attachmentPreview(id: string): Promise<string> {
  let preview = previews.get(id);
  if (!preview) {
    preview = api<string>("ai_attachment_preview", { id });
    preview.catch(() => previews.delete(id));
    previews.set(id, preview);
  }
  return preview;
}

function readBase64(file: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).split(",")[1] ?? "");
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

export function useAttachments(tabId: string) {
  const [items, setItems] = useState(() => pending.get(tabId) ?? []);
  const [importing, setImporting] = useState(0);
  const [error, setError] = useState("");
  const store = useCallback(
    (update: (current: Attachment[]) => Attachment[]) => {
      const next = update(pending.get(tabId) ?? []);
      if (next.length) pending.set(tabId, next);
      else pending.delete(tabId);
      setItems(next);
    },
    [tabId],
  );
  const run = useCallback(
    async (task: () => Promise<Attachment[]>) => {
      setError("");
      setImporting((count) => count + 1);
      try {
        const added = await task();
        store((current) => [...current, ...added]);
      } catch (cause) {
        setError(errorMessage(cause));
      } finally {
        setImporting((count) => count - 1);
      }
    },
    [store],
  );
  const pick = useCallback(
    () => run(() => api<Attachment[]>("ai_attachment_pick")),
    [run],
  );
  const importPaths = useCallback(
    (paths: string[]) =>
      paths.length
        ? run(() => api<Attachment[]>("ai_attachment_import_paths", { paths }))
        : Promise.resolve(),
    [run],
  );
  const importImages = useCallback(
    (files: File[]) =>
      run(() =>
        Promise.all(
          files.map(async (file, index) =>
            api<Attachment>("ai_attachment_import_bytes", {
              name: file.name || `pasted-${index + 1}.${file.type.slice(6)}`,
              data: await readBase64(file),
            }),
          ),
        ),
      ),
    [run],
  );
  const remove = useCallback(
    (id: string) => {
      store((current) => current.filter((item) => item.id !== id));
      void api("ai_attachment_discard", { id }).catch(() => {});
    },
    [store],
  );
  /** Drops chips the backend has consumed by an accepted send. */
  const consume = useCallback(
    (ids: string[]) =>
      store((current) => current.filter((item) => !ids.includes(item.id))),
    [store],
  );
  return {
    items,
    importing,
    error,
    dismissError: () => setError(""),
    pick,
    importPaths,
    importImages,
    remove,
    consume,
  };
}
