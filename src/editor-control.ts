import type { Text } from "@codemirror/state";

export interface EditorEdit {
  fromUtf16: number;
  toUtf16: number;
  insert: string;
}
export interface EditorEditsInput {
  workspaceId: string;
  panelId: string;
  relativePath: string;
  documentId: string;
  expectedBufferRevision: string;
  expectedDiskRevision: string;
  edits: EditorEdit[];
}
export type EditorSaveInput = Omit<EditorEditsInput, "edits">;
export interface EditorSaved {
  workspaceId: string;
  panelId: string;
  relativePath: string;
  documentId: string;
  savedBufferRevision: string;
  previousDiskRevision: string;
  diskRevision: string;
  byteLength: number;
}

export function bufferChanges(doc: Text, edits: EditorEdit[]) {
  if (!edits.length || edits.length > 64) throw new Error("RESOURCE_EXHAUSTED");
  let bytes = 0;
  let length = doc.length;
  let previous: EditorEdit | undefined;
  const encoder = new TextEncoder();
  return edits.map((edit) => {
    const { fromUtf16: from, toUtf16: to, insert } = edit;
    if (
      !Number.isSafeInteger(from) ||
      !Number.isSafeInteger(to) ||
      from < 0 ||
      to < from ||
      to > doc.length ||
      splitsSurrogate(doc, from) ||
      splitsSurrogate(doc, to) ||
      (previous && (from < previous.toUtf16 || from <= previous.fromUtf16)) ||
      (from === to && !insert) ||
      /[\r\0]/.test(insert)
    )
      throw new Error("REVISION_CONFLICT");
    for (const char of insert) {
      const code = char.codePointAt(0)!;
      if (code >= 0xd800 && code <= 0xdfff)
        throw new Error("RESOURCE_EXHAUSTED");
    }
    bytes += encoder.encode(insert).length;
    length += insert.length - (to - from);
    if (bytes > 65536 || length > 16 * 1024 * 1024)
      throw new Error("RESOURCE_EXHAUSTED");
    previous = edit;
    return { from, to, insert };
  });
}

export interface EditorReadInput {
  workspaceId: string;
  panelId: string;
  relativePath: string;
  documentId?: string | null;
  expectedBufferRevision?: string | null;
  startUtf16?: number;
  maxChars?: number;
}

function splitsSurrogate(doc: Text, offset: number) {
  if (!offset || offset >= doc.length) return false;
  const pair = doc.sliceString(offset - 1, offset + 1);
  return (
    pair.charCodeAt(0) >= 0xd800 &&
    pair.charCodeAt(0) <= 0xdbff &&
    pair.charCodeAt(1) >= 0xdc00 &&
    pair.charCodeAt(1) <= 0xdfff
  );
}

export function readBufferSlice(doc: Text, start = 0, max = 4096) {
  if (
    !Number.isSafeInteger(start) ||
    !Number.isSafeInteger(max) ||
    start < 0 ||
    start > doc.length ||
    max < 2 ||
    max > 8192 ||
    doc.length > 16 * 1024 * 1024 ||
    splitsSurrogate(doc, start)
  ) {
    throw new Error("RESOURCE_EXHAUSTED");
  }
  let end = Math.min(doc.length, start + max);
  if (splitsSurrogate(doc, end)) --end;
  return {
    content: doc.sliceString(start, end),
    startUtf16: start,
    totalUtf16: doc.length,
    nextUtf16: end < doc.length ? end : null,
    truncated: end < doc.length,
  };
}
