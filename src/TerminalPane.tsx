import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import {
  ArrowDown,
  ArrowUp,
  CaseSensitive,
  Code,
  Command,
  Copy,
  Maximize2,
  Minimize2,
  Play,
  RotateCcw,
  Search,
  X,
} from "./icons";
import type { Pane, ShellProfile } from "./model";
import type { SharedHostTerminalBinding } from "./shared-host";
import {
  getTerminalAgents,
  subscribeTerminalAgents,
  terminalFor,
} from "./terminal-runtime";
import { cliTerminalDisplayTitle } from "./cli-terminal-title";
import { terminalProgramStatusActivity } from "./cli-activity";
import { IconButton } from "./ui";
import { actionForEvent, formatShortcut } from "./keybindings";
import { useKeybindings } from "./KeybindingsProvider";
import { useTerminalPreferences } from "./TerminalPreferencesProvider";
import { useT } from "./i18n";

interface Props {
  pane: Pane;
  profile?: ShellProfile;
  active: boolean;
  overview: boolean;
  revealTitle: boolean;
  canMove: boolean;
  canMaximize: boolean;
  maximized: boolean;
  onToggleMaximize: () => void;
  onFocus: () => void;
  onRestart: (useProjectDirectory?: boolean) => void;
  sharedHost?: SharedHostTerminalBinding;
}

export default function TerminalPane(props: Props) {
  const { t } = useT();
  const { bindings } = useKeybindings();
  if (!props.profile)
    return (
      <section className="terminal-pane">
        <div className="empty-message">
          <h2>{t("Shell unavailable")}</h2>
          <p>
            {t("Use {shortcut} to choose an installed terminal environment.", {
              shortcut: formatShortcut(bindings.changeEnvironment),
            })}
          </p>
        </div>
      </section>
    );
  return <LiveTerminal {...props} profile={props.profile} />;
}

function LiveTerminal({
  pane,
  profile,
  active,
  overview,
  revealTitle,
  canMove,
  canMaximize,
  maximized,
  onToggleMaximize,
  onFocus,
  onRestart,
  sharedHost,
}: Props & { profile: ShellProfile }) {
  const { t } = useT();
  const [runtime] = useState(() =>
    terminalFor(pane, profile, undefined, undefined, sharedHost),
  );
  runtime.configureSharedHostBinding(sharedHost);
  useEffect(() => runtime.setSharedHostFocused(active), [active, runtime]);
  const snapshot = useSyncExternalStore(runtime.subscribe, runtime.getSnapshot);
  const terminalAgents = useSyncExternalStore(
    subscribeTerminalAgents,
    getTerminalAgents,
  );
  const { value: preferences } = useTerminalPreferences();
  const { bindings } = useKeybindings();
  const container = useRef<HTMLDivElement>(null);
  const overviewCard = useRef<HTMLDivElement>(null);
  const searchInput = useRef<HTMLInputElement>(null);
  const composerInput = useRef<HTMLTextAreaElement>(null);
  const searchOpen = snapshot.searchOpen;
  const setSearchOpen = (open: boolean) => runtime.setView("searchOpen", open);
  const [query, setQuery] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [regex, setRegex] = useState(false);
  const composer = snapshot.composerOpen;
  const [command, setCommand] = useState("");
  const blocks = snapshot.blocksOpen;
  const setBlocks = (open: boolean) => runtime.setView("blocksOpen", open);

  useLayoutEffect(() => {
    // Overview covers the current layout without rebuilding its WebGL renderers.
    runtime.attach(container.current!);
    return () => runtime.detach();
  }, [runtime]);
  useEffect(() => {
    if (active && overview) {
      overviewCard.current?.focus();
      return;
    }
    if (
      active &&
      !container.current
        ?.closest(".terminal-pane")
        ?.contains(document.activeElement)
    )
      runtime.terminal.focus();
  }, [active, overview, runtime]);
  useEffect(() => {
    if (searchOpen) {
      searchInput.current?.focus();
      searchInput.current?.select();
    } else runtime.searchAddon.clearDecorations();
    runtime.scheduleFit();
  }, [searchOpen, runtime]);
  useEffect(() => {
    if (searchOpen) runtime.find(query, false, caseSensitive, regex);
  }, [query, caseSensitive, regex, searchOpen, runtime]);
  useEffect(() => {
    runtime.scheduleFit();
    if (composer) composerInput.current?.focus();
  }, [composer, runtime]);

  const execute = () => {
    runtime.execute(command);
    setCommand("");
  };
  const title =
    snapshot.status === "running"
      ? [
          cliTerminalDisplayTitle(
            snapshot.title,
            snapshot.foregroundProgram,
            terminalAgents[pane.id],
          ),
          snapshot.accountLabel,
        ]
          .filter(Boolean)
          .join(" · ")
      : "";
  const programStatus =
    terminalAgents[pane.id] === "codex" ? snapshot.programStatus : null;
  const activity =
    snapshot.status === "running"
      ? programStatus
        ? programStatus.state === "done"
          ? "finished"
          : terminalProgramStatusActivity(programStatus)
        : snapshot.agentSignal || (snapshot.titleBusy ? "working" : null)
      : null;
  const activityLabel =
    activity === "working"
      ? t("Working")
      : activity === "attention"
        ? t("Needs input")
        : activity === "finished"
          ? t("Done")
          : activity === "error"
            ? t("Error")
            : "";
  const fallbackTitle = pane.cwd || profile.name;
  const displayTitle = title || (canMove || revealTitle ? fallbackTitle : "");
  const headingVisible =
    snapshot.agentControlled ||
    ((preferences.alwaysShowTitles || revealTitle || canMove) &&
      !!(displayTitle || activity || maximized));
  return (
    <section
      className={`terminal-pane${active ? " is-active" : ""}${overview ? " is-overview" : ""}`}
      data-pane-id={pane.id}
      aria-label={t("Terminal {name}", { name: profile.name })}
      onPointerDownCapture={onFocus}
      onFocusCapture={onFocus}
    >
      {overview && (
        <div
          className="terminal-overview"
          ref={overviewCard}
          tabIndex={0}
          onClick={() => overviewCard.current?.focus()}
        >
          <span dir="auto">{title || fallbackTitle}</span>
        </div>
      )}
      {searchOpen && (
        <div className="terminal-search" inert={overview}>
          <Search size={14} />
          <input
            ref={searchInput}
            aria-label={t("Search terminal output")}
            placeholder={t("Find in terminal…")}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (
                event.key === "Escape" ||
                actionForEvent(event.nativeEvent, bindings) === "searchTerminal"
              ) {
                event.preventDefault();
                setSearchOpen(false);
                runtime.terminal.focus();
                return;
              }
              if (event.key === "Enter")
                runtime.find(query, event.shiftKey, caseSensitive, regex);
            }}
          />
          <span className="search-result">{snapshot.searchResult}</span>
          <IconButton
            title={t("Match case")}
            aria-pressed={caseSensitive}
            onClick={() => setCaseSensitive(!caseSensitive)}
          >
            <CaseSensitive size={16} />
          </IconButton>
          <IconButton
            title={t("Regular expression")}
            aria-pressed={regex}
            onClick={() => setRegex(!regex)}
          >
            <span className="regex-icon">.*</span>
          </IconButton>
          <IconButton
            title={t("Previous match")}
            onClick={() => runtime.find(query, true, caseSensitive, regex)}
          >
            <ArrowUp size={14} />
          </IconButton>
          <IconButton
            title={t("Next match")}
            onClick={() => runtime.find(query, false, caseSensitive, regex)}
          >
            <ArrowDown size={14} />
          </IconButton>
          <IconButton
            title={t("Close search")}
            onClick={() => setSearchOpen(false)}
          >
            <X size={14} />
          </IconButton>
        </div>
      )}
      <div className="terminal-body" inert={overview}>
        <div className="terminal-mount" ref={container} />
        <div
          className="terminal-heading"
          aria-hidden={!headingVisible}
          inert={!headingVisible}
        >
          <div
            className={`terminal-title-box${canMove ? " is-movable" : ""}`}
            title={
              canMove
                ? t("Alt+drag or Ctrl+drag to move terminal · Esc to cancel")
                : undefined
            }
          >
            {displayTitle && (
              <span
                className="terminal-title"
                title={canMove ? undefined : title}
                dir="auto"
              >
                {displayTitle}
              </span>
            )}
            {snapshot.agentControlled && (
              <button
                type="button"
                className="terminal-control"
                title={t(
                  "Agent input is enabled. Take control without sending a key or stopping the shell.",
                )}
                onClick={() => void runtime.takeControl()}
              >
                {t("Agent input · Take control")}
              </button>
            )}
            {activity && (
              <span
                className="terminal-activity"
                data-state={activity}
                role="status"
                aria-label={activityLabel}
                aria-atomic="true"
              >
                <span className="terminal-activity-dots" aria-hidden="true">
                  <span />
                  <span />
                  <span />
                </span>
                {activityLabel}
              </span>
            )}
            {canMaximize && (
              <IconButton
                title={
                  maximized
                    ? t("Restore terminal size")
                    : t("Maximize terminal")
                }
                aria-pressed={maximized}
                onClick={() => {
                  onToggleMaximize();
                  runtime.terminal.focus();
                }}
              >
                {maximized ? <Minimize2 size={13} /> : <Maximize2 size={13} />}
              </IconButton>
            )}
          </div>
        </div>
        {snapshot.status === "error" && (
          <div className="terminal-error">
            <p>{snapshot.error}</p>
            <button className="button" onClick={() => onRestart()}>
              <RotateCcw size={14} />
              {t("Retry terminal")}
            </button>
            <button className="button" onClick={() => onRestart(true)}>
              {t("Start in project folder")}
            </button>
          </div>
        )}
        {snapshot.status === "exited" && (
          <button
            className="button restart-terminal"
            onClick={() => onRestart()}
          >
            <RotateCcw size={14} />
            {t("Restart")}
          </button>
        )}
        {blocks && (
          <aside className="command-blocks" aria-label={t("Command blocks")}>
            <header>
              <span>{t("COMMANDS")}</span>
              <IconButton
                title={t("Close command blocks")}
                onClick={() => setBlocks(false)}
              >
                <X size={14} />
              </IconButton>
            </header>
            {!snapshot.blocks.length && (
              <p className="muted">
                {t("Run a command to see its output block here.")}
              </p>
            )}
            {[...snapshot.blocks].reverse().map((block) => (
              <button
                className="command-block"
                key={block.id}
                onClick={() => runtime.jumpTo(block)}
                title={t("Jump to command output")}
              >
                <span className="command-block-text">
                  <Command size={13} />
                  <code>{block.command}</code>
                </span>
                <span className="command-block-meta">
                  <span className={block.exitCode ? "text-error" : ""}>
                    {block.finished
                      ? block.exitCode === undefined
                        ? t("Completed")
                        : t("Exit {code}", { code: block.exitCode })
                      : t("Running")}
                  </span>
                  <span>
                    {block.finished
                      ? `${((block.finished - block.started) / 1000).toFixed(1)}s`
                      : ""}
                  </span>
                </span>
              </button>
            ))}
          </aside>
        )}
      </div>
      {composer && (
        <div className="command-composer" inert={overview}>
          <div className="composer-heading">
            <Code size={14} />
            <span>{t("Command input")}</span>
            <span className="muted">
              {bindings.runCommand
                ? t("{shortcut} to run", {
                    shortcut: formatShortcut(bindings.runCommand),
                  })
                : t("Run command")}
            </span>
            <IconButton
              title={t("Close command input")}
              onClick={() => runtime.setView("composerOpen", false)}
            >
              <X size={14} />
            </IconButton>
            <IconButton
              title={t("Copy terminal selection")}
              onClick={() => void runtime.copy()}
            >
              <Copy size={13} />
            </IconButton>
          </div>
          <textarea
            ref={composerInput}
            aria-label={t("Command input")}
            placeholder={t("Write a command…")}
            spellCheck={false}
            value={command}
            onChange={(event) => setCommand(event.target.value)}
            onKeyDown={(event) => {
              if (
                actionForEvent(event.nativeEvent, bindings) === "runCommand"
              ) {
                event.preventDefault();
                if (!event.repeat) execute();
                return;
              }
              if (event.key === "Tab") {
                event.preventDefault();
                const target = event.currentTarget;
                const start = target.selectionStart;
                setCommand(
                  command.slice(0, start) +
                    "  " +
                    command.slice(target.selectionEnd),
                );
                requestAnimationFrame(() =>
                  target.setSelectionRange(start + 2, start + 2),
                );
              }
              if (
                event.key === "ArrowUp" &&
                !command &&
                snapshot.blocks.length
              ) {
                event.preventDefault();
                setCommand(snapshot.blocks.at(-1)!.command);
              }
            }}
          />
          <button
            className="button composer-run"
            disabled={!command.trim() || snapshot.status !== "running"}
            onClick={execute}
          >
            <Play size={13} />
            {t("Run")}
          </button>
        </div>
      )}
    </section>
  );
}
