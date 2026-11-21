import {
  lazy,
  Suspense,
  useCallback,
  useState,
  useSyncExternalStore,
} from "react";
import { WrapText } from "./icons";
import type { EditorDocument } from "./editor-runtime";
import { useKeybindings } from "./KeybindingsProvider";
import { shortcutTitle } from "./keybindings";
import { IconButton } from "./ui";
import { useT } from "./i18n";

const EditorStatusMenu = lazy(() => import("./EditorStatusMenu"));

export default function FileEditorStatus({
  document,
}: {
  document: EditorDocument;
}) {
  const { t } = useT();
  const status = useSyncExternalStore(document.subscribe, document.getSnapshot);
  const { bindings } = useKeybindings();
  const [menu, setMenu] = useState<{
    kind: "indentation" | "language";
    anchor: HTMLButtonElement;
  } | null>(null);
  const closeMenu = useCallback(() => setMenu(null), []);
  return (
    <div
      className="editor-status"
      role="group"
      aria-label={t("File information")}
    >
      <span>
        {t("Ln {line}, Col {column}", {
          line: status.line,
          column: status.column,
        })}
      </span>
      {status.dirty && <span>{t("Modified")}</span>}
      {status.readOnly && <span>{t("Read-only")}</span>}
      {status.large && (
        <span
          title={t(
            "Syntax highlighting, completion and wrapping are disabled for large files or very long lines.",
          )}
        >
          {t("Large file mode")}
        </span>
      )}
      <span>{status.encoding}</span>
      <span>{status.lineEnding}</span>
      <button
        className="text-button"
        title={t("Change indentation settings (tab display size: {size})", {
          size: status.tabSize,
        })}
        aria-label={t("Change indentation settings")}
        aria-haspopup="dialog"
        aria-expanded={menu?.kind === "indentation"}
        onPointerDown={(event) => {
          event.preventDefault();
          event.currentTarget.focus({ preventScroll: true });
        }}
        onClick={(event) =>
          setMenu(
            menu?.kind === "indentation"
              ? null
              : { kind: "indentation", anchor: event.currentTarget },
          )
        }
      >
        {t("{kind}: {size}", {
          kind: status.insertSpaces ? t("Spaces") : t("Tabs"),
          size: status.insertSpaces ? status.indentSize : status.tabSize,
        })}
      </button>
      <button
        className="text-button"
        title={t("Change language mode")}
        aria-label={t("Change language mode")}
        aria-haspopup="dialog"
        aria-expanded={menu?.kind === "language"}
        onPointerDown={(event) => {
          event.preventDefault();
          event.currentTarget.focus({ preventScroll: true });
        }}
        onClick={(event) =>
          setMenu(
            menu?.kind === "language"
              ? null
              : { kind: "language", anchor: event.currentTarget },
          )
        }
      >
        {status.language === "Plain text" ? t("Plain text") : status.language}
      </button>
      <IconButton
        title={shortcutTitle(t("Toggle word wrap"), bindings.toggleWordWrap)}
        aria-pressed={status.wrapped}
        disabled={status.large}
        onClick={() => document.command("toggleWordWrap")}
      >
        <WrapText size={14} />
      </IconButton>
      {menu && (
        <Suspense fallback={null}>
          <EditorStatusMenu
            key={menu.kind}
            editor={document}
            {...menu}
            onClose={closeMenu}
          />
        </Suspense>
      )}
    </div>
  );
}
