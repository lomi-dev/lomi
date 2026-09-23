import { fileTabs, updateFilePosition } from "./model";
import type { FileTab, Session } from "./model";
import type { DiskFile, EditorDocument } from "./editor-runtime";
import { defaultEditorPreferences } from "./editor-preferences";
import type { EditorPreferences } from "./editor-preferences";
import { retainAgentPreviews } from "./agent-preview";

let preferences = defaultEditorPreferences;
export const editorPreferences = () => preferences;
export function applyEditorPreferences(next: EditorPreferences) {
  if (
    next.tabSize === preferences.tabSize &&
    next.insertSpaces === preferences.insertSpaces
  )
    return;
  preferences = next;
  runtime?.documents().forEach((document) => document.applyIndentation());
}

let runtime: typeof import("./editor-runtime") | undefined;
let loading: Promise<typeof import("./editor-runtime")> | undefined;
let tabs: FileTab[] = [];
let revision = 0;
const listeners = new Set<() => void>();
export const editorFileKey = (file: FileTab) =>
  file.untitled ? `untitled\0${file.id}` : `${file.root}\0${file.relative}`;
const saveListeners = new Set<
  (id: string, location: { root: string; relative: string }) => void
>();
export function subscribeEditorSaves(
  listener: (id: string, location: { root: string; relative: string }) => void,
) {
  saveListeners.add(listener);
  return () => {
    saveListeners.delete(listener);
  };
}
export function editorFileSaved(
  id: string,
  location: { root: string; relative: string },
) {
  for (const listener of saveListeners) listener(id, location);
}
export const editorRevision = () => revision;
export const subscribeEditors = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};
function notify() {
  ++revision;
  for (const listener of listeners) listener();
}

export function retainEditorTabs(session: Session | undefined) {
  tabs = session ? fileTabs(session) : [];
  retainAgentPreviews(tabs);
  runtime?.retainDocuments(tabs);
}

async function editorRuntime() {
  if (!loading)
    loading = import("./editor-runtime")
      .then((module) => {
        runtime = module;
        module.configureEditors(notify);
        module.retainDocuments(tabs);
        return module;
      })
      .catch((error) => {
        loading = undefined;
        throw error;
      });
  return await loading;
}

export async function openEditorDocument(
  tab: FileTab,
): Promise<EditorDocument> {
  return (await editorRuntime()).openDocument(tab);
}
export async function stageEditorRead(tab: FileTab, file: DiskFile) {
  return (await editorRuntime()).stageDocumentRead(tab, file);
}

export function loadedEditor(tab: FileTab): EditorDocument | undefined {
  return runtime?.findDocument(tab);
}

export function assertCleanEditorPaths(paths: ReadonlySet<string>) {
  for (const document of runtime?.documents() ?? [])
    document.assertCleanDiskChange(paths);
}

export async function pauseEditorFileOperations() {
  const documents = runtime?.documents() ?? [];
  await Promise.all(
    documents.map((document) => document.pauseFileOperations()),
  );
  return () => documents.forEach((document) => document.resumeFileOperations());
}

export function relocateEditorFiles(
  previous: Session,
  next: Session,
  canonicalChange?: { oldPath: string | null; newPath: string | null },
) {
  runtime?.relocateDocuments(
    fileTabs(previous),
    fileTabs(next),
    canonicalChange,
  );
}

export function closingEditorDocuments(
  ids?: ReadonlySet<string>,
): EditorDocument[] {
  if (!runtime) return [];
  const kept = new Set(
    tabs
      .filter((tab) => ids && !ids.has(tab.id))
      .map((tab) => runtime!.findDocument(tab)),
  );
  return runtime
    .documents()
    .filter((document) => document.dirty && !kept.has(document));
}

export function captureEditorPositions(session: Session): Session {
  return (runtime?.editorPositions() ?? []).reduce(
    (state, current) => updateFilePosition(state, current.id, current.position),
    session,
  );
}
