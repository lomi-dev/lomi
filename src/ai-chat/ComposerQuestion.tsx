import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { errorMessage } from "../api";
import { useT } from "../i18n";
import { Check, Pencil } from "../icons";
import type { Question } from "./types";

type Answer = Record<string, string[]>;

/**
 * Pending agent question, rendered inside the composer box in place of the
 * editor. Secret answers
 * (passwords) are masked and only live in this component's state.
 */
export default function ComposerQuestion({
  question,
  onAnswer,
}: {
  question: Question;
  /** Resolves when the answer was sent; null declines the question. */
  onAnswer: (requestId: string, answers: Answer | null) => Promise<void>;
}) {
  const { t } = useT();
  const root = useRef<HTMLDivElement>(null);
  const [step, setStep] = useState(0);
  const [chosen, setChosen] = useState<Answer>({});
  const [other, setOther] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const item = question.items[step];
  const last = step === question.items.length - 1;
  const answersOf = (id: string) =>
    [...(chosen[id] ?? []), other[id]?.trim() ?? ""].filter(Boolean);
  const ready = answersOf(item.id).length > 0;
  const focusables = () =>
    Array.from(
      root.current?.querySelectorAll<HTMLElement>("[data-qfocus]") ?? [],
    );
  const firstRender = useRef(true);
  useEffect(() => {
    const active = document.activeElement;
    // On appearing, focus moves only from nowhere or from the editor this panel
    // hides; later steps always move it. The panel itself takes focus, so keys
    // work without highlighting an option the user has not picked.
    const idle =
      !active ||
      active === document.body ||
      !!active.closest(".thread-composer-editor");
    if (!firstRender.current || idle) root.current?.focus();
    firstRender.current = false;
  }, [step]);
  function toggle(label: string) {
    setChosen((prev) => {
      const had = prev[item.id] ?? [];
      return {
        ...prev,
        [item.id]: item.multiSelect
          ? had.includes(label)
            ? had.filter((value) => value !== label)
            : [...had, label]
          : [label],
      };
    });
    if (!item.multiSelect) setOther((prev) => ({ ...prev, [item.id]: "" }));
  }
  function submit(value: Answer | null) {
    setBusy(true);
    setError("");
    onAnswer(question.requestId, value).then(
      () => setBusy(false),
      (cause) => {
        setBusy(false);
        setError(errorMessage(cause));
      },
    );
  }
  function advance() {
    if (!ready || busy) return;
    if (!last) return setStep(step + 1);
    submit(
      Object.fromEntries(
        question.items.map((entry) => [entry.id, answersOf(entry.id)]),
      ),
    );
  }
  function onKey(event: KeyboardEvent) {
    if (event.nativeEvent.isComposing) return;
    const typing = (event.target as HTMLElement).tagName === "INPUT";
    const digit = Number(event.key);
    if (!typing && digit >= 1 && digit <= item.options.length) {
      // Number keys pick an option, as the row badges show.
      event.preventDefault();
      toggle(item.options[digit - 1].label);
      focusables()[digit - 1]?.focus();
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const list = focusables();
      const at = list.indexOf(event.target as HTMLElement);
      const dir = event.key === "ArrowDown" ? 1 : -1;
      // From the panel itself (no option focused yet) start at either end.
      const next =
        at < 0 ? (dir > 0 ? 0 : list.length - 1) : at + dir + list.length;
      list[next % list.length]?.focus();
    } else if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      // Enter selects the focused option first; once answered it submits.
      const label = (event.target as HTMLElement).dataset.label;
      if (ready) advance();
      else if (label !== undefined) toggle(label);
    }
  }
  return (
    <>
      <div
        ref={root}
        className="thread-question"
        role="group"
        tabIndex={-1}
        aria-label={t("Agent question")}
        onKeyDown={onKey}
      >
        <p className="thread-question-text">{item.question}</p>
        {item.multiSelect && (
          <p className="thread-question-multi">{t("Select all that apply")}</p>
        )}
        <div
          className="thread-question-options"
          role={item.multiSelect ? "group" : "radiogroup"}
          aria-label={item.question}
        >
          {item.options.map((option, index) => {
            const on = (chosen[item.id] ?? []).includes(option.label);
            return (
              <button
                key={option.label}
                type="button"
                role={item.multiSelect ? "checkbox" : "radio"}
                aria-checked={on}
                className="thread-question-option"
                data-qfocus=""
                data-label={option.label}
                disabled={busy}
                onClick={() => toggle(option.label)}
              >
                <span className="thread-question-key" aria-hidden="true">
                  {on && item.multiSelect ? <Check size={12} /> : index + 1}
                </span>
                <span className="thread-question-label">
                  {option.label}
                  {option.description && <small>{option.description}</small>}
                </span>
                {on && !item.multiSelect && (
                  <Check size={14} className="thread-question-check" />
                )}
              </button>
            );
          })}
          {(item.allowOther || !item.options.length) && (
            <label
              className="thread-question-option thread-question-own"
              data-on={other[item.id]?.trim() ? "" : undefined}
            >
              <span className="thread-question-key" aria-hidden="true">
                <Pencil size={12} />
              </span>
              <input
                type={item.secret ? "password" : "text"}
                autoComplete="off"
                data-qfocus=""
                aria-label={t("Other answer")}
                placeholder={
                  item.options.length
                    ? t("Something else…")
                    : t("Type your answer…")
                }
                disabled={busy}
                value={other[item.id] ?? ""}
                onChange={(event) => {
                  setOther((prev) => ({
                    ...prev,
                    [item.id]: event.target.value,
                  }));
                  if (!item.multiSelect)
                    setChosen((prev) => ({ ...prev, [item.id]: [] }));
                }}
              />
            </label>
          )}
        </div>
        {error && (
          <div className="thread-notice" role="alert">
            {error}
          </div>
        )}
      </div>
      <div className="thread-composer-bar">
        {question.items.length > 1 && (
          <span className="thread-question-step">
            {step + 1} / {question.items.length}
          </span>
        )}
        <span className="thread-composer-spacer" />
        <button
          type="button"
          className="thread-decline"
          disabled={busy}
          onClick={() => submit(null)}
        >
          {t("Decline")}
        </button>
        {step > 0 && (
          <button
            type="button"
            className="thread-decline"
            disabled={busy}
            onClick={() => setStep(step - 1)}
          >
            {t("Back")}
          </button>
        )}
        <button
          type="button"
          className="thread-send thread-submit"
          disabled={busy || !ready}
          onClick={advance}
        >
          {last ? t("Submit") : t("Next")}
        </button>
      </div>
    </>
  );
}
