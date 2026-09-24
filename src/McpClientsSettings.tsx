import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage, native } from "./api";
import { CliAgentIcon } from "./CliAgentIcon";
import { ArrowLeft, Check, ChevronRight, RefreshCw, Search } from "./icons";
import type { TitleProcess } from "./terminal-runtime";
import { DisclosureSummary } from "./ui";

interface McpClient {
  cli: TitleProcess["cli"];
  name: string;
  configured: boolean;
  manualReason?: string | null;
  notice?: string | null;
  path: string;
  revision: string | null;
  error: string | null;
}

function clientStatus(client: McpClient) {
  if (client.configured) return "Configured";
  if (client.manualReason) return "Manual setup";
  if (client.error) return "Setup unavailable";
  return "Not configured";
}

function rowStatus(client: McpClient) {
  if (client.configured) return "Configured";
  if (client.manualReason) return "Manual setup";
  if (client.error) return "Setup unavailable";
  return "";
}

export function McpClientsSettings({
  yoloMode,
  onInstalled,
}: {
  yoloMode: boolean;
  onInstalled: () => Promise<void> | void;
}) {
  const [clients, setClients] = useState<McpClient[]>([]);
  const [selectedCli, setSelectedCli] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(Boolean(native));
  const [busy, setBusy] = useState(false);
  const [loadError, setLoadError] = useState("");
  const [error, setError] = useState("");
  const [status, setStatus] = useState("");
  const saving = useRef(false);
  const generation = useRef(0);
  const rowButtons = useRef(new Map<string, HTMLButtonElement>());
  const backButton = useRef<HTMLButtonElement>(null);
  const searchInput = useRef<HTMLInputElement>(null);
  const focusIntent = useRef<"setup" | "chooser" | null>(null);
  const returnFocusCli = useRef<string | null>(null);
  const selected = clients.find((client) => client.cli === selectedCli);
  const matchingClients = useMemo(
    () =>
      clients.filter((client) => {
        const query = search.trim().toLowerCase();
        return (
          !query ||
          client.name.toLowerCase().includes(query) ||
          client.cli.toLowerCase().includes(query)
        );
      }),
    [clients, search],
  );
  const eligible = useMemo(
    () =>
      clients.filter(
        (client) => !client.configured && !client.manualReason && !client.error,
      ),
    [clients],
  );

  const refresh = useCallback(async () => {
    if (!native) {
      setLoading(false);
      return;
    }
    const request = ++generation.current;
    setLoading(true);
    setLoadError("");
    try {
      const next = await api<McpClient[]>("inspect_mcp_clients");
      if (request !== generation.current) return;
      setClients(next);
    } catch (cause) {
      if (request === generation.current) setLoadError(errorMessage(cause));
    } finally {
      if (request === generation.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (selectedCli && !selected) {
      focusIntent.current = "chooser";
      returnFocusCli.current = null;
      setSelectedCli(null);
      return;
    }

    const intent = focusIntent.current;
    if (!intent) return;
    focusIntent.current = null;

    if (intent === "setup") {
      backButton.current?.scrollIntoView({ block: "start" });
      backButton.current?.focus();
      return;
    }

    const row = returnFocusCli.current
      ? rowButtons.current.get(returnFocusCli.current)
      : undefined;
    (row ?? searchInput.current)?.focus();
    returnFocusCli.current = null;
  }, [selected, selectedCli]);

  useEffect(() => {
    if (!native) return;
    void refresh();
    const focused = () => {
      if (!saving.current) void refresh();
    };
    const stop = listen("cli-integrations-changed", focused);
    window.addEventListener("focus", focused);
    return () => {
      ++generation.current;
      window.removeEventListener("focus", focused);
      void stop.then((unlisten) => unlisten()).catch(() => {});
    };
  }, [refresh]);

  const install = async (targets: McpClient[]) => {
    if (saving.current || !native) return;
    const installable = targets.filter(
      (client) => !client.configured && !client.manualReason && !client.error,
    );
    if (!installable.length) return;

    saving.current = true;
    ++generation.current;
    setBusy(true);
    setError("");
    setStatus("");
    const installed: string[] = [];
    const failures: string[] = [];
    try {
      for (const client of installable) {
        try {
          await api("install_mcp_client", {
            cli: client.cli,
            path: client.path,
            revision: client.revision,
          });
          installed.push(client.name);
        } catch (cause) {
          failures.push(`${client.name}: ${errorMessage(cause)}`);
        }
      }
      if (installed.length) {
        const names = installed.join(", ");
        setStatus(
          yoloMode
            ? `Lomi MCP was added to ${names}. Restart ${names} to connect; YOLO mode will approve its Lomi session automatically.`
            : `Lomi MCP was added to ${names}. Restart ${names}, then approve its session in Sessions.`,
        );
      }
      setError(failures.join("\n"));
      await refresh();
      if (installed.length) await onInstalled();
    } finally {
      saving.current = false;
      setBusy(false);
    }
  };

  const selectedStatus = selected ? clientStatus(selected) : "";
  return (
    <section
      aria-labelledby={selected ? "mcp-setup-heading" : "mcp-clients-heading"}
      aria-busy={busy || loading}
    >
      {!selected ? (
        <div className="agent-client-chooser">
          <div className="agent-control-section-heading">
            <h2 id="mcp-clients-heading">Choose a client</h2>
            <button
              type="button"
              className="icon-button agent-client-refresh"
              aria-label="Refresh"
              disabled={!native || busy || loading}
              onClick={() => void refresh()}
            >
              <RefreshCw size={15} aria-hidden="true" />
            </button>
          </div>

          {!native && (
            <p className="agent-control-empty">
              Client setup is available in the Lomi desktop app.
            </p>
          )}
          {loadError && (
            <div className="keybindings-error" role="alert">
              Could not read client configurations: {loadError}
            </div>
          )}
          {error && (
            <div
              className="keybindings-error"
              role="alert"
              style={{ whiteSpace: "pre-line" }}
            >
              {error}
            </div>
          )}
          {status && (
            <p className="agent-control-notice" role="status">
              {status}
            </p>
          )}

          {loading && !clients.length ? (
            <p className="agent-control-empty" role="status">
              Checking client configurations…
            </p>
          ) : !clients.length ? (
            !loadError &&
            native && (
              <p className="agent-control-empty">
                No supported client configurations were found.
              </p>
            )
          ) : (
            <>
              <div className="agent-client-picker-toolbar">
                <div className="agent-client-search-control">
                  <label
                    className="agent-client-visually-hidden"
                    htmlFor="mcp-client-search"
                  >
                    Search agents
                  </label>
                  <div className="agent-client-search-field">
                    <Search aria-hidden="true" />
                    <input
                      ref={searchInput}
                      id="mcp-client-search"
                      type="search"
                      aria-label="Search agents"
                      placeholder="Search agents"
                      value={search}
                      disabled={busy || loading}
                      onChange={(event) => setSearch(event.currentTarget.value)}
                    />
                  </div>
                </div>
                <span
                  className="agent-client-result-count"
                  role="status"
                  aria-live="polite"
                  aria-atomic="true"
                >
                  {search.trim()
                    ? `${matchingClients.length} ${matchingClients.length === 1 ? "result" : "results"}`
                    : `${clients.length} ${clients.length === 1 ? "agent" : "agents"}`}
                </span>
              </div>

              {matchingClients.length ? (
                <ul className="agent-client-list" aria-label="Coding agents">
                  {matchingClients.map((client) => {
                    const descriptionId = `mcp-client-status-${client.cli}`;
                    const detail = rowStatus(client);
                    return (
                      <li key={client.cli}>
                        <button
                          ref={(element) => {
                            if (element)
                              rowButtons.current.set(client.cli, element);
                            else rowButtons.current.delete(client.cli);
                          }}
                          type="button"
                          className="agent-client-row"
                          aria-label={client.name}
                          aria-describedby={detail ? descriptionId : undefined}
                          disabled={busy || loading}
                          onClick={() => {
                            focusIntent.current = "setup";
                            setStatus("");
                            setError("");
                            setSelectedCli(client.cli);
                          }}
                        >
                          <CliAgentIcon
                            cli={client.cli}
                            className="agent-client-option-icon"
                          />
                          <span className="agent-client-option-name">
                            {client.name}
                          </span>
                          {detail && (
                            <span
                              id={descriptionId}
                              className="agent-client-option-status"
                              data-state={
                                client.configured
                                  ? "configured"
                                  : client.manualReason
                                    ? "manual"
                                    : "error"
                              }
                            >
                              {client.configured && (
                                <Check
                                  className="agent-client-option-check"
                                  aria-hidden="true"
                                />
                              )}
                              {detail}
                            </span>
                          )}
                          <ChevronRight
                            className="agent-client-row-chevron"
                            aria-hidden="true"
                          />
                        </button>
                      </li>
                    );
                  })}
                </ul>
              ) : (
                <div className="agent-client-empty-search">
                  <p>No agents match “{search.trim()}”.</p>
                  <button
                    type="button"
                    className="button"
                    onClick={() => setSearch("")}
                  >
                    Clear search
                  </button>
                </div>
              )}

              <details className="agent-control-details">
                <DisclosureSummary>All client configurations</DisclosureSummary>
                <div className="agent-control-details-content">
                  <ul className="agent-setup-client-list">
                    {clients.map((client) => (
                      <li key={client.cli}>
                        <span className="agent-setup-client-list-identity">
                          <CliAgentIcon
                            cli={client.cli}
                            className="agent-setup-client-list-icon"
                          />
                          <span>{client.name}</span>
                        </span>
                        <span className="agent-setup-client-status">
                          {clientStatus(client)}
                        </span>
                      </li>
                    ))}
                  </ul>
                  <p className="settings-help">
                    Set up every eligible client in one go. This updates their
                    user configurations for all projects and starts the local
                    server. Client applications are not installed.
                  </p>
                  <button
                    type="button"
                    className="button"
                    disabled={!native || busy || loading || !eligible.length}
                    onClick={() => void install(eligible)}
                  >
                    {busy
                      ? "Setting up…"
                      : `Set up all eligible clients (${eligible.length})`}
                  </button>
                </div>
              </details>
            </>
          )}
        </div>
      ) : (
        <div className="agent-setup-client">
          <div className="agent-setup-navigation">
            <button
              ref={backButton}
              type="button"
              className="agent-setup-back"
              disabled={busy || loading}
              onClick={() => {
                returnFocusCli.current = selected.cli;
                focusIntent.current = "chooser";
                setSelectedCli(null);
              }}
            >
              <ArrowLeft size={16} aria-hidden="true" />
              Back to agents
            </button>
            <button
              type="button"
              className="icon-button agent-client-refresh"
              aria-label="Refresh"
              disabled={!native || busy || loading}
              onClick={() => void refresh()}
            >
              <RefreshCw size={15} aria-hidden="true" />
            </button>
          </div>
          <header className="agent-setup-client-header">
            <CliAgentIcon
              cli={selected.cli}
              className="agent-setup-client-icon"
            />
            <div className="agent-setup-client-identity">
              <h2 id="mcp-setup-heading">{selected.name}</h2>
              <span
                className="agent-setup-client-status"
                data-state={
                  selected.configured
                    ? "configured"
                    : selected.manualReason
                      ? "manual"
                      : selected.error
                        ? "error"
                        : "ready"
                }
              >
                {selectedStatus}
              </span>
            </div>
          </header>

          <p className="agent-setup-client-description">
            {selected.configured
              ? `Lomi is configured for ${selected.name}.`
              : selected.manualReason
                ? selected.manualReason
                : selected.error
                  ? "Automatic setup is unavailable for this client until the reported issue is resolved."
                  : `Adds Lomi to ${selected.name} for all projects and starts the local server.`}
          </p>
          {loadError && (
            <div className="keybindings-error" role="alert">
              Could not read client configurations: {loadError}
            </div>
          )}
          {error && (
            <div
              className="keybindings-error"
              role="alert"
              style={{ whiteSpace: "pre-line" }}
            >
              {error}
            </div>
          )}
          {status && (
            <p className="agent-control-notice" role="status">
              {status}
            </p>
          )}
          {selected.notice && (
            <p className="agent-control-notice">{selected.notice}</p>
          )}
          {selected.error && (
            <p className="keybindings-error" role="alert">
              {selected.error}
            </p>
          )}
          {selected.configured ? (
            <p className="settings-help" role="status">
              {yoloMode
                ? "Next: restart the client. YOLO mode approves its Lomi session automatically."
                : "Next: restart the client, then approve its Lomi session in Sessions."}
            </p>
          ) : selected.manualReason ? (
            <p className="settings-help">
              Open Preferences → Manual configuration to connect this client.
            </p>
          ) : (
            <div className="agent-setup-client-controls">
              <button
                type="button"
                className="button button-primary"
                disabled={busy || loading || Boolean(selected.error)}
                aria-label={`Install Lomi MCP for ${selected.name}`}
                onClick={() => void install([selected])}
              >
                {busy ? "Setting up…" : `Add Lomi to ${selected.name}`}
              </button>
              <p className="settings-help">
                Your existing configuration is backed up.
              </p>
            </div>
          )}

          <details className="agent-control-details">
            <DisclosureSummary>Setup details</DisclosureSummary>
            <div className="agent-control-details-content">
              <dl>
                <dt>Configuration path</dt>
                <dd>{selected.path || "Not reported"}</dd>
                <dt>Revision</dt>
                <dd>{selected.revision ?? "Not tracked"}</dd>
              </dl>
            </div>
          </details>
        </div>
      )}
    </section>
  );
}
