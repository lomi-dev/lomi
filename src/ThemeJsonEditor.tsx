import { useLayoutEffect, useRef } from "react";
import { Compartment, EditorState, Transaction } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { isolateHistory, redo, undo } from "@codemirror/commands";
import { indentUnit } from "@codemirror/language";
import { json } from "@codemirror/lang-json";
import { gotoLine, openSearchPanel } from "@codemirror/search";
import { FileJson, Redo2, Search, Undo2 } from "./icons";
import { editorAppearance } from "./theme/editor";
import { formatThemeText } from "./theme/format";
import { codeEditorExtensions } from "./editor-extensions";
import { useEditorPreferences } from "./EditorPreferencesProvider";
import { useKeybindings } from "./KeybindingsProvider";
import { actionForEvent } from "./keybindings";
import { useT } from "./i18n";
import { errorMessage } from "./api";
import { themeAppliedEvent } from "./theme/runtime";
import { IconButton } from "./ui";

export default function ThemeJsonEditor({
  value,
  disabled,
  onChange,
  onSave,
  onError,
}: {
  value: string | null;
  disabled: boolean;
  onChange: (value: string) => void;
  onSave: () => void;
  onError: (message: string) => void;
}) {
  const { t } = useT();
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const { value: preferences } = useEditorPreferences();
  const { bindings } = useKeybindings();
  const current = useRef({ value, disabled, onChange, onSave, bindings });
  current.current = { value, disabled, onChange, onSave, bindings };
  const settings = useRef(new Compartment());
  const syncing = useRef(false);

  useLayoutEffect(() => {
    const appearance = new Compartment();
    const wrapping = new Compartment();
    const darkTheme = () => editorAppearance();
    const editor = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: current.current.value ?? "",
        extensions: [
          codeEditorExtensions(),
          json(),
          settings.current.of([]),
          appearance.of(darkTheme()),
          wrapping.of([]),
          EditorView.contentAttributes.of({
            "aria-label": t("Theme JSON"),
            spellcheck: "false",
            autocapitalize: "off",
            autocorrect: "off",
          }),
          EditorView.updateListener.of((update) => {
            if (update.docChanged && !syncing.current)
              current.current.onChange(update.state.doc.toString());
          }),
        ],
      }),
    });
    view.current = editor;
    const keyboard = (event: KeyboardEvent) => {
      if (event.isComposing || event.defaultPrevented) return;
      const action = actionForEvent(event, current.current.bindings);
      if (action === "saveFile") {
        if (!current.current.disabled) current.current.onSave();
      } else if (action === "findFile") openSearchPanel(editor);
      else if (action === "goToLine") gotoLine(editor);
      else if (action === "toggleWordWrap")
        editor.dispatch({
          effects: wrapping.reconfigure(
            editor.lineWrapping ? [] : EditorView.lineWrapping,
          ),
        });
      else return;
      event.preventDefault();
      event.stopPropagation();
    };
    const applyAppearance = () => {
      editor.dispatch({ effects: appearance.reconfigure(darkTheme()) });
      editor.requestMeasure();
    };
    editor.dom.addEventListener("keydown", keyboard, true);
    window.addEventListener(themeAppliedEvent, applyAppearance);
    return () => {
      window.removeEventListener(themeAppliedEvent, applyAppearance);
      editor.dom.removeEventListener("keydown", keyboard, true);
      editor.destroy();
      view.current = null;
    };
  }, []);

  useLayoutEffect(() => {
    view.current!.dispatch({
      effects: settings.current.reconfigure([
        EditorState.readOnly.of(disabled),
        EditorView.editable.of(!disabled),
        EditorView.contentAttributes.of({ "aria-readonly": String(disabled) }),
        EditorState.tabSize.of(preferences.tabSize),
        indentUnit.of(
          preferences.insertSpaces ? " ".repeat(preferences.tabSize) : "\t",
        ),
      ]),
    });
  }, [disabled, preferences.tabSize, preferences.insertSpaces]);

  useLayoutEffect(() => {
    if (value === null) return;
    const editor = view.current!;
    if (editor.state.doc.toString() !== value) {
      syncing.current = true;
      try {
        editor.dispatch({
          changes: { from: 0, to: editor.state.doc.length, insert: value },
          selection: {
            anchor: Math.min(editor.state.selection.main.head, value.length),
          },
          annotations: isolateHistory.of("full"),
        });
      } finally {
        syncing.current = false;
      }
    }
  }, [value]);

  const visible = value !== null;
  useLayoutEffect(() => {
    if (visible) {
      view.current!.requestMeasure();
      view.current!.focus();
    }
  }, [visible]);

  const format = () => {
    const editor = view.current!;
    if (editor.state.readOnly) return;
    try {
      const unit = editor.state.facet(indentUnit);
      const formatted = formatThemeText(
        editor.state.doc.toString(),
        unit.length,
        unit !== "\t",
      );
      if (formatted !== editor.state.doc.toString())
        editor.dispatch({
          changes: { from: 0, to: editor.state.doc.length, insert: formatted },
          selection: {
            anchor: Math.min(
              editor.state.selection.main.head,
              formatted.length,
            ),
          },
          annotations: [
            isolateHistory.of("full"),
            Transaction.userEvent.of("input.format"),
          ],
        });
      onError("");
    } catch (error) {
      onError(errorMessage(error));
    }
    editor.focus();
  };

  return (
    <div className="theme-json" hidden={!visible}>
      <div className="editor-heading">
        <FileJson size={15} />
        <span className="editor-path">theme.jsonc</span>
        <div className="editor-actions">
          <IconButton
            title={t("Undo")}
            disabled={disabled}
            onClick={() => {
              undo(view.current!);
              view.current!.focus();
            }}
          >
            <Undo2 size={15} />
          </IconButton>
          <IconButton
            title={t("Redo")}
            disabled={disabled}
            onClick={() => {
              redo(view.current!);
              view.current!.focus();
            }}
          >
            <Redo2 size={15} />
          </IconButton>
          <IconButton
            title={t("Find in JSON")}
            onClick={() => openSearchPanel(view.current!)}
          >
            <Search size={15} />
          </IconButton>
          <button
            type="button"
            className="button"
            disabled={disabled}
            onClick={format}
          >
            {t("Format JSON")}
          </button>
        </div>
      </div>
      <div className="theme-json-content" ref={host} />
    </div>
  );
}
