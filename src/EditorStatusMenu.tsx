import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ArrowLeft, Check } from "./icons";
import type { EditorDocument } from "./editor-runtime";
import { editorLanguage, editorLanguages } from "./editor-languages";

type Step = "indentation" | "spaces" | "tabs" | "display" | "language";
interface Item {
  label: string;
  detail?: string;
  selected?: boolean;
  divider?: boolean;
  run: () => void;
}

export default function EditorStatusMenu({
  editor,
  kind,
  anchor,
  onClose,
}: {
  editor: EditorDocument;
  kind: "indentation" | "language";
  anchor: HTMLButtonElement;
  onClose: () => void;
}) {
  const [step, setStep] = useState<Step>(kind);
  const [query, setQuery] = useState("");
  const ref = useRef<HTMLDivElement>(null);
  const search = useRef<HTMLInputElement>(null);
  const titleId = useId();
  const status = editor.getSnapshot();
  const finish = (action: () => void) => {
    action();
    onClose();
    editor.focus();
  };
  let title: string;
  let items: Item[];
  if (step === "language") {
    title = "Select Language Mode";
    items = [
      {
        label: "Auto Detect",
        value: "auto",
        detail: editorLanguage(editor.location.relative)?.name ?? "Plain text",
      },
      { label: "Plain text", value: "Plain text" },
      ...editorLanguages.map((language) => ({
        label: language.name,
        value: language.name,
      })),
    ]
      .filter((item) =>
        item.label.toLowerCase().includes(query.trim().toLowerCase()),
      )
      .map((item) => ({
        label: item.label,
        detail: "detail" in item ? item.detail : undefined,
        selected: status.languageMode === item.value,
        run: () => finish(() => editor.setLanguage(item.value)),
      }));
  } else if (step === "indentation") {
    title = "Indentation";
    items = [
      {
        label: "Indent Using Spaces",
        selected: status.insertSpaces,
        run: () => setStep("spaces"),
      },
      {
        label: "Indent Using Tabs",
        selected: !status.insertSpaces,
        run: () => setStep("tabs"),
      },
      {
        label: "Change Tab Display Size",
        detail: String(status.tabSize),
        run: () => setStep("display"),
      },
      {
        label: "Use Default Indentation",
        detail: status.customIndentation ? undefined : "Current",
        divider: true,
        run: () => finish(() => editor.setIndentation(null)),
      },
    ];
  } else {
    title =
      step === "display"
        ? "Select Tab Display Size"
        : step === "spaces"
          ? "Indent Using Spaces"
          : "Indent Using Tabs";
    const size = step === "spaces" ? status.indentSize : status.tabSize;
    items = Array.from({ length: 16 }, (_, index) => {
      const value = index + 1;
      return {
        label: String(value),
        detail: value === size ? "Current" : undefined,
        selected: value === size,
        run: () =>
          finish(() =>
            editor.setIndentation(
              step === "display"
                ? { tabSize: value }
                : {
                    tabSize: value,
                    indentSize: value,
                    insertSpaces: step === "spaces",
                  },
            ),
          ),
      };
    });
  }

  useLayoutEffect(() => {
    const element = ref.current!;
    const bounds = element.getBoundingClientRect();
    const button = anchor.getBoundingClientRect();
    element.style.left = `${Math.max(8, Math.min(button.right - bounds.width, innerWidth - bounds.width - 8))}px`;
    element.style.top = `${Math.max(8, button.top - bounds.height - 6)}px`;
  }, [anchor, step, items.length]);
  useLayoutEffect(() => {
    if (step === "language") search.current?.focus();
    else {
      const selected = ref.current?.querySelector<HTMLButtonElement>(
        '[aria-checked="true"], [role="menuitem"]',
      );
      selected?.focus({ preventScroll: true });
      selected?.scrollIntoView({ block: "nearest" });
    }
  }, [step]);
  useEffect(() => {
    const outside = (event: Event) => {
      if (
        !ref.current?.contains(event.target as Node) &&
        !anchor.contains(event.target as Node)
      )
        onClose();
    };
    document.addEventListener("pointerdown", outside);
    document.addEventListener("scroll", outside, true);
    window.addEventListener("resize", onClose);
    window.addEventListener("blur", onClose);
    return () => {
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("scroll", outside, true);
      window.removeEventListener("resize", onClose);
      window.removeEventListener("blur", onClose);
    };
  }, [anchor, onClose]);

  return createPortal(
    <div
      ref={ref}
      className="menu editor-status-menu"
      role="dialog"
      aria-labelledby={titleId}
      onBlur={(event) => {
        if (
          !event.currentTarget.contains(event.relatedTarget) &&
          !anchor.contains(event.relatedTarget)
        )
          onClose();
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          if (step !== kind) setStep(kind);
          else {
            onClose();
            anchor.focus({ preventScroll: true });
          }
          return;
        }
        if (event.key === "Tab") {
          onClose();
          anchor.focus({ preventScroll: true });
          return;
        }
        if (event.ctrlKey || event.metaKey || event.altKey) return;
        const buttons = Array.from(
          event.currentTarget.querySelectorAll<HTMLButtonElement>(
            '[role="menuitem"], [role="menuitemradio"]',
          ),
        );
        const index = buttons.indexOf(
          document.activeElement as HTMLButtonElement,
        );
        let next: number;
        if (event.key === "ArrowDown") next = (index + 1) % buttons.length;
        else if (event.key === "ArrowUp")
          next =
            (index < 0 ? buttons.length - 1 : index + buttons.length - 1) %
            buttons.length;
        else if (event.key === "Enter" && event.target === search.current) {
          event.preventDefault();
          buttons[0]?.click();
          return;
        } else if (event.target === search.current) return;
        else if (event.key === "Home") next = 0;
        else if (event.key === "End") next = buttons.length - 1;
        else if (
          search.current &&
          event.key.length === 1 &&
          event.key !== " "
        ) {
          event.preventDefault();
          search.current.focus();
          setQuery((query) => query + event.key);
          return;
        } else return;
        event.preventDefault();
        buttons[next]?.focus({ preventScroll: true });
        buttons[next]?.scrollIntoView({ block: "nearest" });
      }}
    >
      <header className="editor-menu-heading">
        {step !== kind && (
          <button
            className="icon-button"
            aria-label="Back to indentation options"
            onClick={() => setStep(kind)}
          >
            <ArrowLeft size={14} />
          </button>
        )}
        <span id={titleId}>{title}</span>
        <small>Current file</small>
      </header>
      {step === "language" && (
        <input
          ref={search}
          type="search"
          aria-label="Filter languages"
          placeholder="Search languages…"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
      )}
      <div className="editor-menu-items" role="menu" aria-label={title}>
        {items.map((item) => (
          <button
            key={item.label}
            className={`menu-item${item.divider ? " editor-menu-divider" : ""}`}
            role={item.selected === undefined ? "menuitem" : "menuitemradio"}
            aria-checked={item.selected}
            tabIndex={-1}
            onClick={item.run}
          >
            <span className="editor-menu-check">
              {item.selected && <Check size={13} />}
            </span>
            <span>{item.label}</span>
            {item.detail && <small>{item.detail}</small>}
          </button>
        ))}
        {items.length === 0 && (
          <p className="editor-menu-empty" role="status">
            No matching languages.
          </p>
        )}
      </div>
      {step === "display" && (
        <p className="editor-menu-help">
          Changes how tab characters are displayed.
        </p>
      )}
      {step === "language" && status.large && (
        <p className="editor-menu-help">
          Syntax highlighting is disabled in large file mode.
        </p>
      )}
    </div>,
    document.body,
  );
}
