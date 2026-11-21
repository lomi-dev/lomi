import { t } from "./i18n.ts";
export interface EditorPreferences {
  tabSize: number;
  insertSpaces: boolean;
}

export const defaultEditorPreferences: EditorPreferences = {
  tabSize: 4,
  insertSpaces: true,
};

export function restoreEditorPreferences(value: unknown): EditorPreferences {
  if (value === null) return { ...defaultEditorPreferences };
  if (value && typeof value === "object" && !Array.isArray(value)) {
    const data = value as Record<string, unknown>;
    if (
      data.version === 1 &&
      typeof data.tabSize === "number" &&
      Number.isInteger(data.tabSize) &&
      data.tabSize >= 1 &&
      data.tabSize <= 16 &&
      typeof data.insertSpaces === "boolean"
    )
      return { tabSize: data.tabSize, insertSpaces: data.insertSpaces };
  }
  throw new Error(
    t(
      "Invalid or unsupported editor settings. The file has been left intact. Retry loading or reset defaults to replace it.",
    ),
  );
}
