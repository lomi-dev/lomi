import { useEffect, useRef, useState, type CSSProperties } from "react";
import { getInfo } from "./api";
import { CliAgentIcon } from "./CliAgentIcon";
import {
  ArrowLeft,
  ArrowRight,
  Check,
  MessageSquare,
  Monitor,
  Moon,
  ShieldCheck,
  Sun,
  Terminal,
} from "./icons";
import {
  loadInstalledAgentClis,
  type InstalledCli,
} from "./installed-agent-clis";
import Keycaps from "./Keycaps";
import { useKeybindings } from "./KeybindingsProvider";
import { shortcutFromEvent, type ActionId } from "./keybindings";
import LomiSymbol from "./LomiSymbol";
import { useThemes } from "./ThemeProvider";
import type { AppearancePreference } from "./theme/format";
import {
  completeOnboarding,
  setWorkMode,
  workMode,
  type WorkMode,
} from "./onboarding-state";
import "./onboarding.css";

const at = (index: number) => ({ "--i": index }) as CSSProperties;

const STEPS = [
  { id: "intro", label: "Welcome" },
  { id: "mode", label: "Workflow" },
  { id: "appearance", label: "Appearance" },
  { id: "agents", label: "Agents" },
  { id: "keys", label: "Shortcuts" },
  { id: "trust", label: "Ready" },
] as const;

const KEYS: Record<WorkMode, ActionId[]> = {
  cli: ["commandPicker", "newTab", "newTerminal", "toggleExplorer"],
  chat: ["commandPicker", "newChat", "newTab", "toggleExplorer"],
};

const MODES: {
  value: WorkMode;
  label: string;
  text: string;
  Icon: typeof Sun;
}[] = [
  {
    value: "cli",
    label: "CLI first",
    text: "Terminals up front. Run Claude Code, Codex or any agent in real shells.",
    Icon: Terminal,
  },
  {
    value: "chat",
    label: "Chat first",
    text: "A calm chat up front. Talk to your agents; terminals stay one click away.",
    Icon: MessageSquare,
  },
];

const APPEARANCES: {
  value: AppearancePreference;
  label: string;
  Icon: typeof Sun;
}[] = [
  { value: "dark", label: "Dark", Icon: Moon },
  { value: "light", label: "Light", Icon: Sun },
  { value: "system", label: "System", Icon: Monitor },
];

const TRUST = [
  {
    Icon: Terminal,
    title: "Real terminals, real tools",
    text: "Agents run in your own shells, next to your editor and Git.",
  },
  {
    Icon: ShieldCheck,
    title: "You approve every action",
    text: "Agents act on your workspace only after you allow it.",
  },
  {
    Icon: Check,
    title: "Your work stays here",
    text: "Keys live in the system keychain. History stays on this device.",
  },
];

const EXIT_MS = 160;

const ARROWS: Record<string, number> = {
  ArrowRight: 1,
  ArrowDown: 1,
  ArrowLeft: -1,
  ArrowUp: -1,
};

const reducedMotion = () =>
  matchMedia("(prefers-reduced-motion: reduce)").matches;

const nextLabel = (step: number, last: boolean, mode: WorkMode) =>
  step === 0
    ? "Get started"
    : last
      ? mode === "chat"
        ? "Start chatting"
        : "Start building"
      : "Continue";

function moveRadio(radio: HTMLElement, delta: number) {
  const group = radio.closest('[role="radiogroup"]');
  if (!group) return;
  const radios = [...group.querySelectorAll<HTMLElement>('[role="radio"]')];
  const next =
    radios[(radios.indexOf(radio) + delta + radios.length) % radios.length];
  next.focus();
  next.click();
}

export default function Onboarding({ onDone }: { onDone: () => void }) {
  // `step` is on screen; `target` leads it while the old step exits.
  const [step, setStep] = useState(0);
  const [target, setTarget] = useState(0);
  const [direction, setDirection] = useState(1);
  const [mode, setMode] = useState(workMode);
  const [leaving, setLeaving] = useState(false);
  const dialog = useRef<HTMLDivElement>(null);
  const timer = useRef(0);
  const last = target === STEPS.length - 1;

  useEffect(() => () => clearTimeout(timer.current), []);

  const go = (to: number) => {
    if (to < 0 || to >= STEPS.length || to === target) return;
    setDirection(to > target ? 1 : -1);
    setTarget(to);
    clearTimeout(timer.current);
    timer.current = window.setTimeout(
      () => setStep(to),
      reducedMotion() ? 0 : EXIT_MS,
    );
  };
  const finish = () => {
    if (leaving) return;
    completeOnboarding();
    setLeaving(true);
    setTimeout(onDone, reducedMotion() ? 0 : 520);
  };
  const next = () => (last ? finish() : go(target + 1));
  const back = () => go(target - 1);
  const chooseMode = (value: WorkMode) => {
    setWorkMode(value);
    setMode(value);
  };

  useEffect(() => {
    // Keeps keystrokes away from the terminals behind the dialog.
    dialog.current?.focus({ preventScroll: true });
  }, [step]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      // Workspace shortcuts must not act on the workbench behind the dialog.
      event.stopPropagation();
      if (event.key === "Escape") {
        event.preventDefault();
        finish();
      } else if (
        event.key === "Enter" &&
        !(event.target instanceof HTMLButtonElement)
      ) {
        event.preventDefault();
        next();
      } else if (ARROWS[event.key]) {
        event.preventDefault();
        const radio =
          event.target instanceof HTMLElement &&
          event.target.getAttribute("role") === "radio"
            ? event.target
            : null;
        // Radio groups keep arrow keys for moving the selection.
        if (radio) moveRadio(radio, ARROWS[event.key]);
        else if (event.key === "ArrowRight") next();
        else if (event.key === "ArrowLeft") back();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  });

  return (
    <div
      ref={dialog}
      tabIndex={-1}
      className="onboarding"
      role="dialog"
      aria-modal="true"
      aria-label="Welcome to Lomi"
      data-leaving={leaving || undefined}
      style={{ "--step": target } as CSSProperties}
    >
      <div className="onboarding-aurora" aria-hidden="true">
        <i />
        <i />
        <i />
      </div>
      <div className="onboarding-drag" data-tauri-drag-region />
      <button
        type="button"
        className="onboarding-skip"
        onClick={finish}
        hidden={last}
      >
        Skip
      </button>

      <section
        className="onboarding-step"
        key={step}
        data-step={STEPS[step].id}
        data-direction={direction > 0 ? "forward" : "back"}
        data-exiting={target !== step || undefined}
      >
        {STEPS[step].id === "intro" && <Intro />}
        {STEPS[step].id === "mode" && (
          <Mode mode={mode} onChange={chooseMode} />
        )}
        {STEPS[step].id === "appearance" && <Appearance />}
        {STEPS[step].id === "agents" && <Agents mode={mode} />}
        {STEPS[step].id === "keys" && <Keys mode={mode} />}
        {STEPS[step].id === "trust" && <Trust mode={mode} />}
      </section>

      <footer className="onboarding-footer">
        <button
          type="button"
          className="onboarding-back"
          onClick={back}
          aria-label="Back"
          disabled={target === 0}
        >
          <ArrowLeft size={16} />
        </button>
        <div className="onboarding-progress-wrap">
          <span
            className="onboarding-progress-label"
            aria-hidden="true"
            key={target}
          >
            {STEPS[target].label}
            <span>
              {target + 1}/{STEPS.length}
            </span>
          </span>
          <ol className="onboarding-progress" aria-label="Progress">
            {STEPS.map(({ id, label }, index) => (
              <li
                key={id}
                data-state={
                  index === target
                    ? "current"
                    : index < target
                      ? "done"
                      : undefined
                }
              >
                <button
                  type="button"
                  onClick={() => go(index)}
                  aria-current={index === target ? "step" : undefined}
                  aria-label={`Step ${index + 1} of ${STEPS.length}: ${label}`}
                  title={label}
                />
              </li>
            ))}
          </ol>
        </div>
        <button
          type="button"
          className="onboarding-next"
          onClick={next}
          data-final={last || undefined}
        >
          <span key={nextLabel(target, last, mode)}>
            {nextLabel(target, last, mode)}
          </span>
          <ArrowRight size={16} aria-hidden="true" />
        </button>
      </footer>
    </div>
  );
}

function Intro() {
  return (
    <>
      <LomiSymbol size={84} className="onboarding-symbol" />
      <h1 style={at(1)}>
        Build side
        <br />
        by side.
      </h1>
      <p style={at(2)}>
        Lomi is the workspace where you and your AI agents build in one window.
        Three quick choices and you are in.
      </p>
      <p className="onboarding-prompt" style={at(3)}>
        <span aria-hidden="true">›</span> ready when you are
        <i className="onboarding-cursor" aria-hidden="true" />
      </p>
    </>
  );
}

function Mode({
  mode,
  onChange,
}: {
  mode: WorkMode;
  onChange: (mode: WorkMode) => void;
}) {
  return (
    <>
      <span className="onboarding-eyebrow" style={at(0)}>
        Workflow
      </span>
      <h2 style={at(1)}>How do you like to work?</h2>
      <p style={at(2)}>
        This only sets what opens first. Terminals and chats are always in the +
        menu.
      </p>
      <div
        className="onboarding-modes"
        role="radiogroup"
        aria-label="Workflow"
        style={at(3)}
      >
        {MODES.map(({ value, label, text, Icon }) => (
          <button
            key={value}
            type="button"
            role="radio"
            aria-checked={mode === value}
            tabIndex={mode === value ? 0 : -1}
            className="onboarding-choice onboarding-mode"
            data-mode={value}
            onClick={() => onChange(value)}
          >
            <span className="onboarding-mode-preview" aria-hidden="true">
              {value === "cli" ? (
                <>
                  <b>
                    <em>$</em> claude
                  </b>
                  <i />
                  <i />
                  <b data-done>
                    <em>✓</em> 3 files changed
                  </b>
                  <b>
                    <em>›</em>
                    <span className="onboarding-cursor" />
                  </b>
                </>
              ) : (
                <>
                  <i data-from="you" />
                  <i data-from="agent" />
                  <i data-from="agent" />
                  <u>
                    <span className="onboarding-cursor" />
                  </u>
                </>
              )}
            </span>
            <span className="onboarding-mode-text">
              <strong>
                <Icon size={15} aria-hidden="true" />
                {label}
                <Check
                  size={14}
                  className="onboarding-tick"
                  aria-hidden="true"
                />
              </strong>
              <span>{text}</span>
            </span>
          </button>
        ))}
      </div>
    </>
  );
}

function Appearance() {
  const { preferences, select } = useThemes();
  return (
    <>
      <span className="onboarding-eyebrow" style={at(0)}>
        Appearance
      </span>
      <h2 style={at(1)}>Pick your light.</h2>
      <p style={at(2)}>You can change it any time in Settings.</p>
      <div
        className="onboarding-appearances"
        role="radiogroup"
        aria-label="Appearance"
        style={at(3)}
      >
        {APPEARANCES.map(({ value, label, Icon }) => (
          <button
            key={value}
            type="button"
            role="radio"
            aria-checked={preferences.appearance === value}
            tabIndex={preferences.appearance === value ? 0 : -1}
            className="onboarding-choice onboarding-appearance"
            data-preview={value}
            onClick={() => void select(preferences.active, value)}
          >
            <span className="onboarding-preview" aria-hidden="true">
              <i />
              <b />
            </span>
            <span className="onboarding-appearance-label">
              <Icon size={14} aria-hidden="true" />
              {label}
              <Check size={14} className="onboarding-tick" aria-hidden="true" />
            </span>
          </button>
        ))}
      </div>
    </>
  );
}

function Agents({ mode }: { mode: WorkMode }) {
  const [clients, setClients] = useState<InstalledCli[] | null>(null);
  useEffect(() => {
    let live = true;
    getInfo()
      .then((info) => {
        const profile = info.profiles[0];
        return profile ? loadInstalledAgentClis(profile, info.home) : [];
      })
      .catch(() => [])
      .then((found) => {
        if (live) setClients(found);
      });
    return () => {
      live = false;
    };
  }, []);
  return (
    <>
      <span className="onboarding-eyebrow" style={at(0)}>
        Agents
      </span>
      <h2 style={at(1)}>
        Your agents,
        <br />
        in plain sight.
      </h2>
      <p style={at(2)}>
        {mode === "chat"
          ? "Chat through the agent CLIs you already use, with your own login. No new accounts."
          : "Run Claude Code, Codex, Gemini CLI or any CLI agent next to your own work."}
      </p>
      <div className="onboarding-agents" style={at(3)} aria-live="polite">
        {clients === null ? (
          <div className="onboarding-scanning">
            <span className="onboarding-bars" aria-hidden="true">
              <i />
              <i />
              <i />
            </span>
            Looking for agents on this computer…
          </div>
        ) : clients.length === 0 ? (
          <div className="onboarding-scanning">
            No agent CLIs found yet. Install one any time; Lomi picks it up in a
            new terminal.
          </div>
        ) : (
          <ul>
            {clients.map((client, index) => (
              <li key={client.cli} style={{ "--j": index } as CSSProperties}>
                <CliAgentIcon cli={client.cli} />
                <span>{client.name}</span>
                <code>{client.command}</code>
                <i className="onboarding-live" aria-label="Installed" />
              </li>
            ))}
          </ul>
        )}
      </div>
    </>
  );
}

function Keys({ mode }: { mode: WorkMode }) {
  const { actions, bindings } = useKeybindings();
  const [pressed, setPressed] = useState<ActionId[]>([]);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const shortcut = shortcutFromEvent(event);
      const id = KEYS[mode].find(
        (key) => bindings[key] && bindings[key] === shortcut,
      );
      if (!id) return;
      event.preventDefault();
      setPressed((list) => (list.includes(id) ? list : [...list, id]));
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [bindings, mode]);
  const rows = KEYS[mode].filter((id) => bindings[id]);
  return (
    <>
      <span className="onboarding-eyebrow" style={at(0)}>
        Shortcuts
      </span>
      <h2 style={at(1)}>
        Keep your hands
        <br />
        on the keys.
      </h2>
      <p style={at(2)}>
        Try one now. Every shortcut is listed, and can be changed, in Settings
        under Keybinds.
      </p>
      <ul className="onboarding-keys" style={at(3)}>
        {rows.map((id, index) => (
          <li
            key={id}
            style={{ "--j": index } as CSSProperties}
            data-pressed={pressed.includes(id) || undefined}
          >
            <span>{actions.find((action) => action.id === id)?.label}</span>
            <Keycaps binding={bindings[id]} />
            <Check size={14} className="onboarding-tick" aria-hidden="true" />
          </li>
        ))}
      </ul>
      <p
        className="onboarding-keys-status"
        style={at(4)}
        aria-live="polite"
        data-complete={
          (rows.length > 0 && pressed.length === rows.length) || undefined
        }
      >
        {rows.length > 0 && pressed.length === rows.length
          ? "All of them, first try. Nice."
          : `${pressed.length} of ${rows.length} tried`}
      </p>
    </>
  );
}

function Trust({ mode }: { mode: WorkMode }) {
  const { preferences } = useThemes();
  const appearance = APPEARANCES.find(
    ({ value }) => value === preferences.appearance,
  );
  const chosen = MODES.find(({ value }) => value === mode);
  return (
    <>
      <span className="onboarding-eyebrow" style={at(0)}>
        Ready
      </span>
      <h2 style={at(1)}>
        Nothing runs
        <br />
        without you.
      </h2>
      <ul
        className="onboarding-summary"
        style={at(2)}
        aria-label="Your choices"
      >
        {chosen && (
          <li>
            <chosen.Icon size={13} aria-hidden="true" />
            {chosen.label}
          </li>
        )}
        {appearance && (
          <li>
            <appearance.Icon size={13} aria-hidden="true" />
            {appearance.label}
          </li>
        )}
      </ul>
      <ul className="onboarding-trust">
        {TRUST.map(({ Icon, title, text }, index) => (
          <li key={title} style={at(index + 3)}>
            <span>
              <Icon size={16} aria-hidden="true" />
            </span>
            <div>
              <strong>{title}</strong>
              <p>{text}</p>
            </div>
          </li>
        ))}
      </ul>
    </>
  );
}
