import { useCallback, useEffect, useId, useRef, useState } from "react";
import { errorMessage } from "./api";
import { CliAgentIcon } from "./CliAgentIcon";
import type { CliAgent } from "./cli-agents";
import { maximumAgentCount, type ShellProfile } from "./model";
import type { RefObject } from "react";
import useAgentLayoutSize from "./useAgentLayoutSize";
import { Folder, Minus, Plus, RefreshCw } from "./icons";
import { Modal } from "./ui";

import {
  cachedInstalledAgentClis,
  loadInstalledAgentClis,
  type InstalledCli,
} from "./installed-agent-clis";

function defaultCli(clients: InstalledCli[]) {
  return (clients.find((client) => client.cli === "cursor") ?? clients[0])?.cli;
}

export default function AgentsDialog({
  profile,
  cwd,
  stage,
  onClose,
  onLaunch,
  onTask,
}: {
  profile: ShellProfile;
  cwd: string;
  stage: RefObject<HTMLElement | null>;
  onClose: () => void;
  onLaunch: (cli: CliAgent, count: number) => Promise<void>;
  onTask?: () => void;
}) {
  const id = useId();
  const refreshButton = useRef<HTMLButtonElement>(null);
  const request = useRef(0);
  const launching = useRef(false);
  const [clients, setClients] = useState<InstalledCli[]>(
    () => cachedInstalledAgentClis(profile, cwd) ?? [],
  );
  const [selected, setSelected] = useState<CliAgent | undefined>(() =>
    defaultCli(cachedInstalledAgentClis(profile, cwd) ?? []),
  );
  const size = useAgentLayoutSize(stage);
  const maximum = maximumAgentCount(size);
  const [count, setCount] = useState(() =>
    String(Math.max(1, Math.min(4, maximum))),
  );
  const [loading, setLoading] = useState(
    () => cachedInstalledAgentClis(profile, cwd) === undefined,
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const amount = Number(count);
  const validCount =
    Number.isSafeInteger(amount) && amount >= 1 && amount <= maximum;
  useEffect(() => {
    if (maximum < 1) return;
    setCount((previous) => {
      const value = Number(previous);
      return Number.isSafeInteger(value) && value > maximum
        ? String(maximum)
        : previous;
    });
  }, [maximum]);

  function adjustCount(change: -1 | 1) {
    setCount((previous) => {
      if (maximum < 1) return previous;
      const current = Number(previous);
      if (!Number.isSafeInteger(current) || current < 1) return "1";
      return String(Math.max(1, Math.min(maximum, current + change)));
    });
  }

  const refresh = useCallback(
    async (force = false) => {
      const generation = ++request.current;
      setLoading(true);
      setError("");
      try {
        const installed = await loadInstalledAgentClis(profile, cwd, force);
        if (generation !== request.current) return;
        setClients(installed);
        setSelected((previous) =>
          installed.some((client) => client.cli === previous)
            ? previous
            : defaultCli(installed),
        );
      } catch (cause) {
        if (generation === request.current) setError(errorMessage(cause));
      } finally {
        if (generation === request.current) setLoading(false);
      }
    },
    [profile, cwd],
  );

  useEffect(() => {
    const cached = cachedInstalledAgentClis(profile, cwd) ?? [];
    setClients(cached);
    setSelected(defaultCli(cached));
    void refresh();
    return () => {
      ++request.current;
    };
  }, [refresh]);

  return (
    <Modal
      title="Agents"
      className="agents-dialog"
      wide
      descriptionId={`${id}-description`}
      initialFocus={refreshButton}
      closeDisabled={busy}
      onClose={onClose}
    >
      <form
        className="dialog-form"
        noValidate
        onSubmit={async (event) => {
          event.preventDefault();
          if (launching.current || !clients.length) return;
          const amount = Number(count);
          if (!Number.isSafeInteger(amount) || amount < 1) {
            setError("Enter a whole number of terminals, at least 1.");
            return;
          }
          if (amount > maximum) {
            setError(
              `Only ${maximum} ${maximum === 1 ? "agent fits" : "agents fit"} in the current window.`,
            );
            return;
          }
          if (!selected || !clients.some((client) => client.cli === selected))
            return;
          launching.current = true;
          setBusy(true);
          setError("");
          try {
            await onLaunch(selected, amount);
            onClose();
          } catch (cause) {
            setError(errorMessage(cause));
          } finally {
            launching.current = false;
            setBusy(false);
          }
        }}
      >
        <div className="agents-dialog-body">
          <p id={`${id}-description`} className="agents-description">
            Choose an agent and how many terminals to open.
          </p>
          <section className="agents-chooser">
            <div className="agents-chooser-heading">
              <span id={`${id}-cli-label`}>Installed CLI</span>
              <button
                ref={refreshButton}
                type="button"
                className="icon-button"
                aria-label="Refresh installed CLI"
                title="Refresh installed CLI"
                disabled={loading || busy}
                onClick={() => void refresh(true)}
              >
                <RefreshCw size={15} aria-hidden="true" />
              </button>
            </div>
            <div className="agents-scan-status" role="status">
              {loading
                ? clients.length
                  ? ""
                  : "Looking for installed agents…"
                : !error && !clients.length
                  ? "No supported agent CLI found. Install a CLI, then refresh."
                  : ""}
            </div>
            {!!clients.length && (
              <fieldset
                className="agents-cli-list"
                aria-labelledby={`${id}-cli-label`}
                disabled={busy}
              >
                {clients.map((client) => (
                  <label
                    className="agents-cli-option"
                    key={client.cli}
                    title={client.name}
                  >
                    <input
                      type="radio"
                      name={`${id}-cli`}
                      value={client.cli}
                      aria-label={client.name}
                      checked={selected === client.cli}
                      onChange={() => setSelected(client.cli)}
                    />
                    <CliAgentIcon cli={client.cli} />
                  </label>
                ))}
              </fieldset>
            )}
          </section>
          <section className="agents-count">
            <div className="agents-count-heading">
              <label id={`${id}-count-label`} htmlFor={`${id}-count`}>
                Number of terminals
              </label>
              <div className="agents-count-controls">
                <button
                  type="button"
                  className="agents-count-step"
                  aria-label="Decrease terminal count"
                  disabled={
                    busy || maximum === 0 || (validCount && amount === 1)
                  }
                  onClick={() => adjustCount(-1)}
                >
                  <Minus size={16} aria-hidden="true" />
                </button>
                <input
                  id={`${id}-count`}
                  type="number"
                  min="1"
                  max={maximum}
                  aria-invalid={count !== "" && !validCount}
                  step="1"
                  required
                  value={count}
                  disabled={busy || maximum === 0}
                  onChange={(event) => setCount(event.target.value)}
                />
                <button
                  type="button"
                  className="agents-count-step"
                  aria-label="Increase terminal count"
                  disabled={busy || maximum === 0 || amount >= maximum}
                  onClick={() => adjustCount(1)}
                >
                  <Plus size={16} aria-hidden="true" />
                </button>
              </div>
            </div>
            <div
              className="agents-count-presets"
              aria-label="Terminal count presets"
            >
              {[1, 2, 4, 6, 8].map((preset) => (
                <button
                  key={preset}
                  type="button"
                  aria-label={`Use ${preset} ${preset === 1 ? "terminal" : "terminals"}`}
                  aria-pressed={validCount && amount === preset}
                  disabled={busy || preset > maximum}
                  title={
                    preset > maximum
                      ? `Only ${maximum} ${maximum === 1 ? "agent fits" : "agents fit"} in the current window.`
                      : undefined
                  }
                  onClick={() => setCount(String(preset))}
                >
                  {preset}
                </button>
              ))}
            </div>
          </section>
          <div className="agents-launch-context">
            <div className="agents-layout-summary" aria-live="polite">
              {maximum === 0
                ? "Not enough space for a terminal."
                : validCount
                  ? `One tab · ${amount} ${amount === 1 ? "panel" : "panels"} · Up to ${maximum} fit`
                  : amount > maximum
                    ? `Only ${maximum} ${maximum === 1 ? "agent fits" : "agents fit"} in the current window.`
                    : "Choose a whole number, at least 1"}
            </div>
            <div className="agents-directory" title={cwd}>
              <Folder size={14} aria-hidden="true" />
              <code>{cwd}</code>
            </div>
          </div>
          {error && (
            <p className="text-error" role="alert">
              {error}
            </p>
          )}
        </div>
        <div className="dialog-actions">
          {onTask && (
            <button
              type="button"
              className="button"
              disabled={busy}
              onClick={onTask}
            >
              New CLI task
            </button>
          )}
          <button
            type="button"
            className="button"
            disabled={busy}
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            type="submit"
            className="button button-primary"
            disabled={
              busy || !clients.length || maximum === 0 || amount > maximum
            }
          >
            {busy ? "Launching…" : "Launch agents"}
          </button>
        </div>
      </form>
    </Modal>
  );
}
