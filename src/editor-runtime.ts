import {
  Compartment,
  EditorSelection,
  EditorState,
  Transaction,
} from "@codemirror/state";
import type { Extension, Text, TransactionSpec } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { isolateHistory, redo, undo } from "@codemirror/commands";
import { indentUnit } from "@codemirror/language";
import { gotoLine, openSearchPanel } from "@codemirror/search";
import { codeEditorExtensions } from "./editor-extensions";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage } from "./api";
import type { EditorPosition, FileTab } from "./model";
import {
  editorFileKey,
  editorFileSaved,
  editorPreferences,
} from "./editor-service";
import {
  editorLanguage,
  editorLanguages,
  loadEditorLanguage,
} from "./editor-languages";
import type { Language } from "./editor-languages";
import {
  lineEndingHistory,
  lineEndingLabel,
  lineEndings,
  readEditorText,
  writeEditorText,
} from "./editor-text";
import type { LineEndings } from "./editor-text";
import { editorAppearance } from "./theme/editor";
import { themeAppliedEvent } from "./theme/runtime";

import {
  bufferChanges,
  readBufferSlice,
  type EditorReadInput,
  type EditorEditsInput,
  type EditorSaveInput,
  type EditorSaved,
} from "./editor-control";

export interface DiskFile {
  path: string;
  relative: string;
  content: string | null;
  revision: string;
  encoding: string;
  readOnly: boolean;
}
export interface EditorSnapshot {
  dirty: boolean;
  saving: boolean;
  error: string;
  conflict: boolean;
  readOnly: boolean;
  language: string;
  languageMode: string;
  tabSize: number;
  indentSize: number;
  insertSpaces: boolean;
  customIndentation: boolean;
  encoding: string;
  lineEnding: string;
  large: boolean;
  wrapped: boolean;
  line: number;
  column: number;
}

const buffers = new Map<string, EditorDocument>();
const aliases = new Map<string, EditorDocument>();
const opening = new Map<string, Promise<EditorDocument>>();
const preparedReads = new Map<string, DiskFile>();
export function stageDocumentRead(tab: FileTab, data: DiskFile) {
  const key = editorFileKey(tab);
  if (preparedReads.has(key) || preparedReads.size >= 16)
    throw new Error("TARGET_BUSY");
  preparedReads.set(key, data);
  return () => {
    if (preparedReads.get(key) === data) preparedReads.delete(key);
  };
}
let retained = new Set<string>();
const attachedViews = new Map<EditorDocument, string>();
let notifyAll = () => {};
let watchQueue = Promise.resolve();
let listening: Promise<unknown> | undefined;

function needsLargeFileMode(content: string) {
  if (content.length > 1_048_576) return true;
  let lineLength = 0;
  for (let index = 0; index < content.length; index++) {
    if (content[index] === "\n" || content[index] === "\r") lineLength = 0;
    else if (++lineLength > 20_000) return true;
  }
  return false;
}

interface Indentation {
  tabSize: number;
  indentSize: number;
  insertSpaces: boolean;
}

function indentation({
  tabSize,
  indentSize,
  insertSpaces,
}: Indentation): Extension {
  return [
    EditorState.tabSize.of(tabSize),
    indentUnit.of(insertSpaces ? " ".repeat(indentSize) : "\t"),
  ];
}

export function configureEditors(notify: () => void) {
  notifyAll = notify;
  listening ??= listen<string[]>("editor-files-changed", ({ payload }) => {
    for (const path of payload) buffers.get(path)?.scheduleRefresh();
  }).catch((error) => {
    for (const document of buffers.values())
      document.reportError(`File monitoring: ${errorMessage(error)}`);
  });
  window.addEventListener("focus", () => {
    for (const document of buffers.values()) document.scheduleRefresh();
  });
}

function updateWatches() {
  watchQueue = watchQueue
    .catch(() => {})
    .then(async () => {
      await listening;
      const files = [...buffers.values()]
        .filter((document) => !document.untitled)
        .map((document) => document.location);
      try {
        await api("watch_editor_files", { files });
      } catch (error) {
        for (const document of buffers.values())
          document.reportError(
            `File monitoring: ${errorMessage(error)}. Changes are also checked when the window gains focus.`,
          );
      }
    });
}

export function retainDocuments(tabs: FileTab[]) {
  retained = new Set(tabs.map(editorFileKey));
  for (const key of aliases.keys()) if (!retained.has(key)) aliases.delete(key);
  const kept = new Set(aliases.values());
  let changed = false;
  for (const [path, document] of buffers) {
    if (kept.has(document)) continue;
    document.dispose();
    buffers.delete(path);
    changed = true;
  }
  if (changed) {
    updateWatches();
    notifyAll();
  }
}

export const documents = () => [...buffers.values()];
export function relocateDocuments(
  previous: FileTab[],
  next: FileTab[],
  canonicalChange?: { oldPath: string | null; newPath: string | null },
) {
  const before = new Map(
    previous.map((tab) => [
      tab.id,
      { tab, document: aliases.get(editorFileKey(tab)) },
    ]),
  );
  const changes = next.flatMap((tab) => {
    const old = before.get(tab.id);
    return old?.document && editorFileKey(old.tab) !== editorFileKey(tab)
      ? [{ tab, old }]
      : [];
  });
  for (const { old } of changes) aliases.delete(editorFileKey(old.tab));
  for (const { tab, old } of changes) {
    const document = old.document!;
    buffers.delete(document.path);
    document.relocate(tab, canonicalChange);
    buffers.set(document.path, document);
    aliases.set(editorFileKey(tab), document);
  }
  if (changes.length) {
    updateWatches();
    notifyAll();
  }
}
export const findDocument = (tab: FileTab) => aliases.get(editorFileKey(tab));
export const editorPositions = () =>
  [...attachedViews].map(([document, id]) => ({
    id,
    position: document.position(),
  }));

export function openDocument(tab: FileTab): Promise<EditorDocument> {
  const key = editorFileKey(tab);
  const existing = aliases.get(key);
  if (existing) {
    existing.scheduleRefresh();
    return Promise.resolve(existing);
  }
  let pending = opening.get(key);
  if (!pending) {
    const prepared = preparedReads.get(key);
    pending = (
      prepared
        ? Promise.resolve(prepared)
        : tab.untitled
          ? Promise.resolve<DiskFile>({
              path: tab.title,
              relative: "",
              content: "",
              revision: "",
              encoding: "utf8",
              readOnly: false,
            })
          : api<DiskFile>("read_editor_file", {
              root: tab.root,
              relative: tab.relative,
            })
    )
      .then((data) => {
        if (!retained.has(key))
          throw new Error("This file tab has been closed.");
        let document = buffers.get(data.path);
        if (!document) {
          document = new EditorDocument(tab, data);
          buffers.set(data.path, document);
        }
        aliases.set(key, document);
        updateWatches();
        // Close the gap between the initial read and registering directory watches.
        void watchQueue.then(() => document.scheduleRefresh());
        notifyAll();
        return document;
      })
      .finally(() => opening.delete(key));
    opening.set(key, pending);
  }
  return pending;
}

export class EditorDocument {
  readonly documentId = crypto.randomUUID();
  private bufferVersion = 0;
  private sourcePath: string;
  untitled?: string;
  location: { root: string; relative: string };
  path: string;
  private fileOperationsPaused = false;
  private pendingMatch?: { line: number; column: number; length: number };
  state: EditorState;
  private view?: EditorView;
  private savedDoc: Text;
  private savedEndings: LineEndings;
  private revision: string;
  private language = new Compartment();
  private editable = new Compartment();
  private wrapping = new Compartment();
  private appearance = new Compartment();
  private indentation = new Compartment();
  private indentationOverride: Partial<Indentation> = {};
  private languageExtension: Extension = [];
  private languageDefinition?: Language;
  private syntaxDefinition?: Language;
  private languageVersion = 0;
  private syntaxError = "";
  private external?: DiskFile;
  private savePromise?: Promise<boolean>;
  private checking = false;
  private recheck = false;
  private disposed = false;
  private diskError = "";
  private reloadVersion = 0;
  private refreshTimer?: ReturnType<typeof setTimeout>;
  private dirtyTimer?: ReturnType<typeof setTimeout>;
  private restoreFrame?: number;
  private listeners = new Set<() => void>();
  private textListeners = new Set<() => void>();
  private snapshot: EditorSnapshot;

  constructor(tab: FileTab, data: DiskFile) {
    this.untitled = tab.untitled ? tab.id : undefined;
    this.location = { root: tab.root, relative: tab.relative };
    this.path = data.path;
    this.sourcePath = data.path;
    this.revision = data.revision;
    const content = data.content ?? "";
    const large = needsLargeFileMode(content);
    const definition = editorLanguage(tab.relative);
    this.languageDefinition = definition;
    this.snapshot = {
      dirty: false,
      saving: false,
      error: "",
      conflict: false,
      readOnly: data.readOnly,
      language: definition?.name ?? "Plain text",
      languageMode: "auto",
      ...this.currentIndentation(),
      customIndentation: false,
      encoding: data.encoding.toUpperCase(),
      lineEnding: "LF",
      large,
      wrapped: false,
      line: 1,
      column: 1,
    };
    this.state = this.createState(content);
    this.savedDoc = this.state.doc;
    this.savedEndings = this.state.field(lineEndings);
    this.snapshot.lineEnding = lineEndingLabel(this.state);
    this.loadSyntax();
    window.addEventListener(themeAppliedEvent, this.applyAppearance);
  }

  private applyAppearance = () => {
    if (this.disposed) return;
    this.dispatch({
      effects: this.appearance.reconfigure(
        editorAppearance(this.snapshot.large),
      ),
    });
    this.view?.requestMeasure();
  };

  private loadSyntax() {
    const definition = this.languageDefinition;
    if (
      !definition ||
      this.snapshot.large ||
      this.syntaxDefinition === definition
    )
      return;
    const version = ++this.languageVersion;
    this.syntaxDefinition = definition;
    void loadEditorLanguage(definition)
      .then((extension) => {
        if (this.disposed || version !== this.languageVersion) return;
        this.languageExtension = extension;
        if (!this.snapshot.large) this.applyLanguage(extension);
        if (this.snapshot.error === this.syntaxError)
          this.publish({ error: "" });
        this.syntaxError = "";
      })
      .catch((error) => {
        if (this.disposed || version !== this.languageVersion) return;
        this.syntaxDefinition = undefined;
        this.syntaxError = `Syntax highlighting: ${errorMessage(error)}`;
        this.reportError(this.syntaxError);
      });
  }

  private applyLanguage(extension: Extension) {
    this.dispatch({
      effects: this.language.reconfigure(extension),
      // Recompute selection-based decorations after changing the parser.
      selection: this.state.selection,
      annotations: Transaction.addToHistory.of(false),
    });
  }

  setLanguage(mode: string) {
    const definition =
      mode === "auto"
        ? editorLanguage(this.location.relative)
        : editorLanguages.find((language) => language.name === mode);
    if (
      this.disposed ||
      (!definition && mode !== "auto" && mode !== "Plain text")
    )
      return;
    if (
      mode !== this.snapshot.languageMode ||
      definition !== this.languageDefinition
    ) {
      ++this.languageVersion;
      this.languageDefinition = definition;
      this.syntaxDefinition = undefined;
      this.languageExtension = [];
      this.applyLanguage([]);
      this.publish(
        { languageMode: mode, language: definition?.name ?? "Plain text" },
        false,
      );
      if (this.snapshot.error === this.syntaxError) this.publish({ error: "" });
      this.syntaxError = "";
    }
    this.loadSyntax();
  }

  private createState(
    content: string,
    large = this.snapshot.large,
    readOnly = this.snapshot.readOnly,
  ): EditorState {
    const parsed = readEditorText(content);
    return EditorState.create({
      doc: parsed.doc,
      extensions: [
        codeEditorExtensions(large),
        lineEndings.init(() => parsed.lineEndings),
        lineEndingHistory,
        this.indentation.of(indentation(this.currentIndentation())),
        this.language.of(large ? [] : this.languageExtension),
        this.wrapping.of([]),
        this.editable.of([
          EditorState.readOnly.of(readOnly),
          EditorView.editable.of(!readOnly),
        ]),
        EditorView.contentAttributes.of({
          "aria-label": "File contents",
          spellcheck: "false",
          autocapitalize: "off",
          autocorrect: "off",
        }),
        this.appearance.of(editorAppearance(this.snapshot.large)),
      ],
    });
  }

  get dirty() {
    // Close guards can run before the scheduled UI snapshot notification.
    return !this.matchesSaved();
  }
  private currentIndentation(): Indentation {
    const defaults = editorPreferences();
    return {
      ...defaults,
      indentSize: defaults.tabSize,
      ...this.indentationOverride,
    };
  }
  setIndentation(value: Partial<Indentation> | null) {
    if (
      value &&
      ((value.tabSize !== undefined &&
        (!Number.isInteger(value.tabSize) ||
          value.tabSize < 1 ||
          value.tabSize > 16)) ||
        (value.indentSize !== undefined &&
          (!Number.isInteger(value.indentSize) ||
            value.indentSize < 1 ||
            value.indentSize > 16)) ||
        (value.insertSpaces !== undefined &&
          typeof value.insertSpaces !== "boolean"))
    )
      return;
    this.indentationOverride = value
      ? { ...this.indentationOverride, ...value }
      : {};
    this.applyIndentation();
  }
  applyIndentation() {
    if (this.disposed) return;
    const value = this.currentIndentation();
    this.dispatch({
      effects: this.indentation.reconfigure(indentation(value)),
    });
    this.publish(
      {
        ...value,
        customIndentation: Object.keys(this.indentationOverride).length > 0,
      },
      false,
    );
  }
  focus() {
    this.view?.focus();
  }
  assertCleanDiskChange(paths: ReadonlySet<string>) {
    if (paths.has(this.path) || paths.has(this.sourcePath)) {
      if (
        this.disposed ||
        !this.matchesSaved() ||
        this.snapshot.conflict ||
        this.snapshot.saving
      )
        throw new Error("TARGET_BUSY");
    }
  }
  getSnapshot = () => this.snapshot;
  trashRevision() {
    if (this.disposed || this.untitled || this.snapshot.saving)
      throw new Error("TARGET_BUSY");
    return {
      documentId: this.documentId,
      path: this.sourcePath,
      bufferRevision: `${this.documentId}:${this.bufferVersion}`,
      diskRevision: this.revision,
    };
  }
  getTextSnapshot = () => this.state.doc;
  applyAgentEdits(input: EditorEditsInput, sourcePath: string) {
    if (this.disposed || this.untitled) throw new Error("TARGET_NOT_FOUND");
    if (this.fileOperationsPaused || this.savePromise)
      throw new Error("TARGET_BUSY");
    if (this.snapshot.readOnly || this.sourcePath !== sourcePath)
      throw new Error("SCOPE_DENIED");
    if (input.documentId !== this.documentId)
      throw new Error("STALE_GENERATION");
    const previousBufferRevision = `${this.documentId}:${this.bufferVersion}`;
    if (
      input.expectedBufferRevision !== previousBufferRevision ||
      input.expectedDiskRevision !== this.revision ||
      this.snapshot.conflict
    )
      throw new Error("REVISION_CONFLICT");
    const changes = bufferChanges(this.state.doc, input.edits);
    try {
      this.dispatch({ changes, annotations: isolateHistory.of("full") });
    } catch {
      throw new Error("OUTCOME_UNKNOWN");
    }
    return {
      workspaceId: input.workspaceId,
      panelId: input.panelId,
      relativePath: input.relativePath,
      documentId: this.documentId,
      previousBufferRevision,
      bufferRevision: `${this.documentId}:${this.bufferVersion}`,
      diskRevision: this.revision,
      dirty: !this.matchesSaved(),
      editCount: input.edits.length,
    };
  }
  saveAgent(
    input: EditorSaveInput,
    sourcePath: string,
    operationId: string,
    nonce: string,
  ): Promise<EditorSaved> {
    if (this.disposed || this.untitled) throw new Error("TARGET_NOT_FOUND");
    if (this.fileOperationsPaused || this.savePromise)
      throw new Error("TARGET_BUSY");
    if (this.snapshot.readOnly || this.sourcePath !== sourcePath)
      throw new Error("SCOPE_DENIED");
    if (input.documentId !== this.documentId)
      throw new Error("STALE_GENERATION");
    const bufferRevision = `${this.documentId}:${this.bufferVersion}`;
    if (
      input.expectedBufferRevision !== bufferRevision ||
      input.expectedDiskRevision !== this.revision ||
      this.snapshot.conflict
    )
      throw new Error("REVISION_CONFLICT");
    const savedState = this.state;
    const content = writeEditorText(savedState);
    if (
      new TextEncoder().encode(content).length > 4 * 1024 * 1024 ||
      content.includes("\0")
    )
      throw new Error("RESOURCE_EXHAUSTED");
    this.publish({ saving: true, error: "" });
    const pending = api<EditorSaved>("agent_control_editor_save_file", {
      operationId,
      nonce,
      body: {
        documentId: this.documentId,
        bufferRevision,
        diskRevision: this.revision,
        sourcePath,
        content,
      },
    })
      .then((saved) => {
        if (
          saved.documentId !== input.documentId ||
          saved.savedBufferRevision !== bufferRevision ||
          saved.previousDiskRevision !== input.expectedDiskRevision ||
          saved.workspaceId !== input.workspaceId ||
          saved.panelId !== input.panelId ||
          saved.relativePath !== input.relativePath ||
          !/^[a-f0-9]{64}$/.test(saved.diskRevision)
        )
          throw new Error("OUTCOME_UNKNOWN");
        this.revision = saved.diskRevision;
        this.savedDoc = savedState.doc;
        this.savedEndings = savedState.field(lineEndings);
        this.external = undefined;
        this.publish({
          dirty: !this.matchesSaved(),
          conflict: false,
          error: "",
        });
        return saved;
      })
      .catch((reason: unknown) => {
        const code =
          typeof reason === "string"
            ? reason
            : reason instanceof Error
              ? reason.message
              : "OUTCOME_UNKNOWN";
        this.publish({
          error:
            code === "REVISION_CONFLICT"
              ? "The file changed on disk. Reload it before saving."
              : "The agent save did not complete. Check the disk version before saving again.",
          ...(code === "REVISION_CONFLICT" ? { conflict: true } : {}),
        });
        this.recheck = true;
        throw new Error(code);
      })
      .finally(() => {
        this.savePromise = undefined;
        this.publish({ saving: false });
        if (this.recheck) this.scheduleRefresh();
      });
    this.savePromise = pending.then(
      () => true,
      () => false,
    );
    return pending;
  }
  readAgentBuffer(input: EditorReadInput) {
    if (this.disposed || this.untitled) throw new Error("TARGET_NOT_FOUND");
    if (this.fileOperationsPaused) throw new Error("TARGET_BUSY");
    if (input.documentId && input.documentId !== this.documentId)
      throw new Error("STALE_GENERATION");
    const bufferRevision = `${this.documentId}:${this.bufferVersion}`;
    if (
      input.expectedBufferRevision &&
      input.expectedBufferRevision !== bufferRevision
    )
      throw new Error("REVISION_CONFLICT");
    if (
      (input.startUtf16 ?? 0) > 0 &&
      (!input.documentId || !input.expectedBufferRevision)
    )
      throw new Error("REVISION_CONFLICT");
    const endings = this.state.field(lineEndings);
    return {
      sourcePath: this.sourcePath,
      text: {
        workspaceId: input.workspaceId,
        panelId: input.panelId,
        relativePath: input.relativePath,
        documentId: this.documentId,
        bufferRevision,
        diskRevision: this.revision,
        source: "buffer",
        dirty: !this.matchesSaved(),
        conflict: this.snapshot.conflict,
        encoding: this.snapshot.encoding.toLowerCase(),
        lineEndings:
          this.state.doc.lines === 1
            ? "none"
            : endings.endings
              ? "mixed"
              : endings.separator === "\r\n"
                ? "cr_lf"
                : endings.separator === "\r"
                  ? "cr"
                  : "lf",
        ...readBufferSlice(this.state.doc, input.startUtf16, input.maxChars),
      },
    };
  }

  subscribeText = (listener: () => void) => {
    this.textListeners.add(listener);
    return () => {
      this.textListeners.delete(listener);
    };
  };
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private publish(changes: Partial<EditorSnapshot>, global = true) {
    if (
      this.disposed ||
      Object.entries(changes).every(
        ([key, value]) => this.snapshot[key as keyof EditorSnapshot] === value,
      )
    )
      return;
    this.snapshot = { ...this.snapshot, ...changes };
    for (const listener of this.listeners) listener();
    if (global) notifyAll();
  }
  reportError(message: string) {
    this.publish({ error: message });
  }

  attach(parent: HTMLElement, tab: FileTab): EditorView {
    this.detach();
    const position = tab.position;
    if (position)
      this.state = this.state.update({
        selection: EditorSelection.single(
          Math.min(position.anchor, this.state.doc.length),
          Math.min(position.head, this.state.doc.length),
        ),
        annotations: Transaction.addToHistory.of(false),
      }).state;
    this.view = new EditorView({
      parent,
      state: this.state,
      scrollTo: position
        ? EditorView.scrollIntoView(this.state.selection.main.head)
        : undefined,
      dispatchTransactions: (transactions, view) => {
        view.update(transactions);
        this.state = view.state;
        this.afterTransactions(transactions);
      },
    });
    attachedViews.set(this, tab.id);
    if (position && !this.pendingMatch) {
      this.view.requestMeasure({
        read: () => null,
        write: () => {
          // Restore pixels after CodeMirror replaces its estimated line heights.
          this.restoreFrame = requestAnimationFrame(() => {
            if (!this.view) return;
            this.view.scrollDOM.scrollTop = position.scrollTop;
            this.view.scrollDOM.scrollLeft = position.scrollLeft;
          });
        },
      });
    }
    this.updateCursor();
    if (this.pendingMatch) {
      this.selectMatch(this.pendingMatch);
      this.pendingMatch = undefined;
    }
    return this.view;
  }

  detach() {
    if (this.restoreFrame !== undefined)
      cancelAnimationFrame(this.restoreFrame);
    attachedViews.delete(this);
    this.view?.destroy();
    this.view = undefined;
  }
  position(): EditorPosition {
    return {
      anchor: this.state.selection.main.anchor,
      head: this.state.selection.main.head,
      scrollTop: this.view?.scrollDOM.scrollTop ?? 0,
      scrollLeft: this.view?.scrollDOM.scrollLeft ?? 0,
    };
  }
  private updateCursor() {
    const head = this.state.selection.main.head;
    const line = this.state.doc.lineAt(head);
    this.publish(
      {
        line: line.number,
        column: head - line.from + 1,
        lineEnding: lineEndingLabel(this.state),
      },
      false,
    );
  }
  private afterTransactions(transactions: readonly Transaction[]) {
    if (
      transactions.some(
        (t) =>
          t.docChanged ||
          t.startState.field(lineEndings) !== t.state.field(lineEndings),
      )
    )
      ++this.bufferVersion;
    if (transactions.some((transaction) => transaction.docChanged)) {
      for (const listener of this.textListeners) listener();
      this.publish({ dirty: true });
      clearTimeout(this.dirtyTimer);
      this.dirtyTimer = setTimeout(
        () => this.publish({ dirty: !this.matchesSaved() }),
        200,
      );
    }
    this.updateCursor();
  }
  private dispatch(spec: TransactionSpec) {
    if (this.view) this.view.dispatch(spec);
    else {
      const transaction = this.state.update(spec);
      this.state = transaction.state;
      this.afterTransactions([transaction]);
    }
  }
  private matchesSaved() {
    const endings = this.state.field(lineEndings);
    return (
      this.state.doc.eq(this.savedDoc) &&
      endings.separator === this.savedEndings.separator &&
      (endings.endings === this.savedEndings.endings ||
        (endings.endings !== null &&
          this.savedEndings.endings !== null &&
          endings.endings.length === this.savedEndings.endings.length &&
          endings.endings.every(
            (ending, index) => ending === this.savedEndings.endings![index],
          )))
    );
  }
  command(
    action: "findFile" | "goToLine" | "toggleWordWrap" | "undo" | "redo",
  ) {
    if (!this.view) return;
    if (action === "findFile") openSearchPanel(this.view);
    else if (action === "goToLine") gotoLine(this.view);
    else if (action === "undo") {
      undo(this.view);
      this.view.focus();
    } else if (action === "redo") {
      redo(this.view);
      this.view.focus();
    } else if (!this.snapshot.large) {
      const wrapped = !this.snapshot.wrapped;
      this.dispatch({
        effects: this.wrapping.reconfigure(
          wrapped ? EditorView.lineWrapping : [],
        ),
      });
      this.publish({ wrapped }, false);
    }
  }

  save(overwrite = false): Promise<boolean> {
    if (this.fileOperationsPaused)
      return Promise.reject(
        new Error("A file operation is in progress. Please try saving again."),
      );
    if (this.savePromise)
      return this.savePromise.then((saved) =>
        saved && this.dirty ? this.save(overwrite) : saved,
      );
    if (this.snapshot.readOnly)
      return Promise.reject(new Error("This file is read-only."));
    if (this.snapshot.conflict && !overwrite)
      return Promise.reject(
        new Error("Resolve the file's disk changes before saving."),
      );
    const savedState = this.state;
    this.publish({ saving: true, error: "" });
    const expected =
      overwrite && this.external ? this.external.revision : this.revision;
    const encoding =
      overwrite && this.external
        ? this.external.encoding.toUpperCase()
        : this.snapshot.encoding;
    this.savePromise = (
      this.untitled
        ? this.saveNewFile(writeEditorText(savedState))
        : api<string>("save_editor_file", {
            request: {
              ...this.location,
              content: writeEditorText(savedState),
              revision: expected,
            },
          }).then((revision) => ({ revision, encoding }))
    )
      .then((saved) => {
        if (saved === null) return false;
        this.revision = saved.revision;
        this.savedDoc = savedState.doc;
        this.savedEndings = savedState.field(lineEndings);
        this.external = undefined;
        this.publish({
          dirty: !this.matchesSaved(),
          conflict: false,
          error: "",
          encoding: saved.encoding.toUpperCase(),
        });
        return true;
      })
      .catch((error: unknown) => {
        const conflict =
          !!error &&
          typeof error === "object" &&
          "kind" in error &&
          error.kind === "conflict";
        this.publish({
          error: errorMessage(error),
          ...(conflict ? { conflict: true } : {}),
        });
        this.recheck = true;
        throw error;
      })
      .finally(() => {
        this.savePromise = undefined;
        this.publish({ saving: false });
        if (this.recheck) this.scheduleRefresh();
      });
    return this.savePromise;
  }

  private async saveNewFile(content: string): Promise<DiskFile | null> {
    const result = await api<{
      location: { root: string; relative: string };
      file: DiskFile;
    } | null>("save_new_editor_file", {
      content,
      suggestedName: this.path,
      openFiles: [...buffers.values()]
        .filter((document) => !document.untitled)
        .map((document) => document.location),
    });
    if (!result) return null;
    if (this.disposed) return result.file;
    const id = this.untitled!;
    aliases.delete(`untitled\0${id}`);
    buffers.delete(this.path);
    this.untitled = undefined;
    this.relocate({ type: "file", id, title: "", ...result.location });
    this.path = result.file.path;
    this.sourcePath = result.file.path;
    buffers.set(this.path, this);
    aliases.set(
      editorFileKey({ type: "file", id, title: "", ...result.location }),
      this,
    );
    editorFileSaved(id, result.location);
    updateWatches();
    this.recheck = true;
    return result.file;
  }

  scheduleRefresh() {
    if (this.disposed || this.fileOperationsPaused || this.untitled) return;
    clearTimeout(this.refreshTimer);
    this.refreshTimer = setTimeout(() => void this.refresh(), 150);
  }
  async pauseFileOperations() {
    this.fileOperationsPaused = true;
    ++this.reloadVersion;
    clearTimeout(this.refreshTimer);
    await this.savePromise?.catch(() => {});
  }
  resumeFileOperations() {
    this.fileOperationsPaused = false;
    this.scheduleRefresh();
  }
  relocate(
    tab: FileTab,
    canonicalChange?: { oldPath: string | null; newPath: string | null },
  ) {
    const previousPath = this.path;
    const previousSource = this.sourcePath;
    ++this.bufferVersion;
    ++this.reloadVersion;
    this.location = { root: tab.root, relative: tab.relative };
    const separator = tab.root.includes("\\") ? "\\" : "/";
    this.path =
      tab.root.replace(/[\\/]$/, "") +
      separator +
      tab.relative.replace(/[\\/]/g, separator);
    if (this.sourcePath === previousPath) this.sourcePath = this.path;
    if (
      canonicalChange?.newPath &&
      canonicalChange.oldPath &&
      previousSource &&
      (previousSource === canonicalChange.oldPath ||
        previousSource.startsWith(`${canonicalChange.oldPath}/`))
    )
      this.sourcePath =
        canonicalChange.newPath +
        previousSource.slice(canonicalChange.oldPath.length);
    if (this.diskError === this.snapshot.error) this.publish({ error: "" });
    this.diskError = "";
    if (this.snapshot.languageMode === "auto") this.setLanguage("auto");
  }
  selectMatch(match: { line: number; column: number; length: number }) {
    if (!this.view) {
      this.pendingMatch = match;
      return;
    }
    const line = this.state.doc.line(
      Math.max(1, Math.min(match.line, this.state.doc.lines)),
    );
    const anchor = Math.min(line.to, line.from + Math.max(0, match.column - 1));
    const head = Math.min(line.to, anchor + match.length);
    this.dispatch({
      selection: EditorSelection.single(anchor, head),
      effects: EditorView.scrollIntoView(anchor, { y: "center" }),
    });
    this.view.focus();
  }
  private async refresh() {
    if (this.disposed || this.fileOperationsPaused || this.untitled) return;
    if (this.checking || this.savePromise) {
      this.recheck = true;
      return;
    }
    this.checking = true;
    this.recheck = false;
    const knownRevision = this.revision;
    const reloadVersion = this.reloadVersion;
    try {
      const data = await api<DiskFile>("read_editor_file", {
        ...this.location,
        knownRevision,
      });
      if (this.disposed) return;
      if (
        knownRevision !== this.revision ||
        reloadVersion !== this.reloadVersion
      ) {
        this.recheck = true;
        return;
      }
      if (this.diskError === this.snapshot.error) this.publish({ error: "" });
      this.diskError = "";
      this.setReadOnly(data.readOnly);
      if (data.revision === this.revision) {
        this.external = undefined;
        this.publish({ conflict: false });
      } else if (this.dirty) {
        this.external = data;
        this.publish({ conflict: true, error: "" });
      } else this.replaceFromDisk(data);
    } catch (error) {
      if (this.disposed || reloadVersion !== this.reloadVersion) return;
      this.diskError = `Cannot check the file on disk: ${errorMessage(error)}`;
      this.reportError(this.diskError);
    } finally {
      this.checking = false;
      if (this.recheck && !this.savePromise) this.scheduleRefresh();
    }
  }
  private setReadOnly(value: boolean) {
    if (this.snapshot.readOnly === value) return;
    this.dispatch({
      effects: this.editable.reconfigure([
        EditorState.readOnly.of(value),
        EditorView.editable.of(!value),
      ]),
    });
    this.publish({ readOnly: value });
  }
  private replaceFromDisk(data: DiskFile) {
    ++this.bufferVersion;
    this.sourcePath = data.path;
    const position = this.position();
    this.revision = data.revision;
    const content = data.content ?? "";
    const large = needsLargeFileMode(content);
    this.state = this.createState(content, large, data.readOnly);
    this.state = this.state.update({
      selection: EditorSelection.single(
        Math.min(position.anchor, this.state.doc.length),
        Math.min(position.head, this.state.doc.length),
      ),
    }).state;
    this.savedDoc = this.state.doc;
    this.savedEndings = this.state.field(lineEndings);
    this.external = undefined;
    this.view?.setState(this.state);
    for (const listener of this.textListeners) listener();
    if (this.view) {
      this.view.scrollDOM.scrollTop = position.scrollTop;
      this.view.scrollDOM.scrollLeft = position.scrollLeft;
    }
    this.publish({
      dirty: false,
      conflict: false,
      error: "",
      readOnly: data.readOnly,
      large,
      encoding: data.encoding.toUpperCase(),
      lineEnding: lineEndingLabel(this.state),
      wrapped: false,
    });
    this.loadSyntax();
    this.updateCursor();
  }
  async reload() {
    if (this.fileOperationsPaused || this.untitled) return;
    if (this.savePromise) await this.savePromise;
    const version = ++this.reloadVersion;
    const data = await api<DiskFile>("read_editor_file", this.location);
    if (!this.disposed && version === this.reloadVersion) {
      ++this.reloadVersion;
      this.replaceFromDisk(data);
      this.scheduleRefresh();
    }
  }
  dispose() {
    this.disposed = true;
    window.removeEventListener(themeAppliedEvent, this.applyAppearance);
    clearTimeout(this.refreshTimer);
    clearTimeout(this.dirtyTimer);
    this.detach();
    this.listeners.clear();
    this.textListeners.clear();
  }
}
