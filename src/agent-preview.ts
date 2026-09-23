import { useSyncExternalStore } from "react";
import { api } from "./api";
import type { FileTab } from "./model";
import type { DiskFile } from "./editor-runtime";

export interface PreviewImage {
  dataBase64: string;
  mimeType: string;
  width: number;
  height: number;
  originalWidth: number;
  originalHeight: number;
}
export type PreparedEditorFile = Omit<DiskFile, "content" | "encoding"> & {
  body:
    | ({ kind: "text" } & Pick<DiskFile, "content" | "encoding">)
    | ({ kind: "image" } & PreviewImage);
  assetPermit: string | null;
};
type PreviewSource = { image?: PreviewImage; assetPermit: string | null };
const sources = new Map<string, PreviewSource>();
const listeners = new Set<() => void>();
const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};
const notify = () => {
  for (const listener of listeners) listener();
};
function release(source: PreviewSource | undefined) {
  if (source?.assetPermit)
    void api("agent_control_preview_release", {
      permitId: source.assetPermit,
    }).catch(() => {});
}
export function stageAgentPreview(id: string, file: PreparedEditorFile) {
  const previous = sources.get(id);
  const source: PreviewSource = {
    assetPermit: file.assetPermit,
    ...(file.body.kind === "image" ? { image: file.body } : {}),
  };
  const bytes = [...sources].reduce(
    (size, [key, value]) =>
      size + (key === id ? 0 : (value.image?.dataBase64.length ?? 0)),
    source.image?.dataBase64.length ?? 0,
  );
  if ((!previous && sources.size >= 64) || bytes > 16 * 1024 * 1024) {
    release(source);
    throw new Error("RESOURCE_EXHAUSTED");
  }
  sources.set(id, source);
  notify();
  return {
    commit: () => release(previous),
    cancel: () => {
      if (sources.get(id) !== source) return;
      if (previous) sources.set(id, previous);
      else sources.delete(id);
      release(source);
      notify();
    },
  };
}
export function retainAgentPreviews(tabs: readonly FileTab[]) {
  const retained = new Set(tabs.map((tab) => tab.id));
  let changed = false;
  for (const [id, source] of sources)
    if (!retained.has(id)) {
      sources.delete(id);
      release(source);
      changed = true;
    }
  if (changed) notify();
}
export const useAgentPreview = (id: string) =>
  useSyncExternalStore(subscribe, () => sources.get(id));

let queue: Promise<unknown> = Promise.resolve();
let pending = 0;
const inFlight = new Map<string, Promise<PreviewImage>>();
export function readAgentPreviewAsset(
  permitId: string | null,
  relative: string,
) {
  if (!permitId)
    return Promise.reject(
      new Error(
        "Reopen this preview through Agent control to load its images.",
      ),
    );
  const key = `${permitId}\0${relative}`;
  const existing = inFlight.get(key);
  if (existing) return existing;
  if (pending >= 64)
    return Promise.reject(new Error("Too many preview images"));
  ++pending;
  const result = queue.then(() =>
    api<PreviewImage>("agent_control_preview_asset", { permitId, relative }),
  );
  inFlight.set(key, result);
  queue = result
    .catch(() => {})
    .finally(() => {
      --pending;
      inFlight.delete(key);
    });
  return result;
}
