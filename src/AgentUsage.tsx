import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import type { ReactNode } from "react";
import { createPortal } from "react-dom";
import { CircleAlert } from "lucide-react";
import { api, native } from "./api";
import { t, tp, useT } from "./i18n";
import { CliAgentIcon } from "./CliAgentIcon";
import {
  agentUsageTargetBatches,
  agentUsageSignature,
  agentUsageTargets,
  groupAgentUsage,
  type AgentUsageEntry,
  type AgentUsageTarget,
  type AgentUsageWindow,
  worstRemainingWindow,
  validRemainingPercent,
} from "./agent-usage";
import { cliNames } from "./cli-agents";
import type { CliAgent } from "./cli-agents";
import type { TerminalContext } from "./terminal-runtime";
import "./agent-usage.css";

interface UsageReply {
  entries: AgentUsageEntry[];
}

interface InFlightRequest {
  signature: string;
  epoch: number;
  promise: Promise<void>;
}

interface PendingRefresh {
  requested: boolean;
  force: boolean;
}

const staleAfterMs = 45_000;

function sameProcess(
  left: { id: string; process: { cli: string; pid: number } },
  right: { id: string; process: { cli: string; pid: number } },
) {
  return (
    left.id === right.id &&
    left.process.cli === right.process.cli &&
    left.process.pid === right.process.pid
  );
}

function formatPercent(value: number) {
  return `${new Intl.NumberFormat(undefined, {
    maximumFractionDigits: 1,
    minimumFractionDigits: value > 0 && value < 1 ? 1 : 0,
  }).format(value)}%`;
}

function formatQuantity(value: number | null, unit: string | null) {
  if (value === null || !Number.isFinite(value)) return null;
  const amount = new Intl.NumberFormat(undefined, {
    maximumFractionDigits: 1,
  }).format(value);
  return unit ? `${amount} ${unit}` : amount;
}

function formatReset(timestamp: number | null) {
  if (timestamp === null || !Number.isFinite(timestamp)) return null;
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return null;
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date);
}

function hasUsageValues(entry: AgentUsageEntry | null) {
  return Boolean(
    entry?.windows.some(
      (window) =>
        validRemainingPercent(window.remainingPercent) ||
        window.used !== null ||
        window.limit !== null,
    ),
  );
}

function isStale(
  entry: AgentUsageEntry | null,
  requestFailed: boolean,
  now: number,
) {
  if (!entry || !hasUsageValues(entry)) return false;
  return (
    requestFailed ||
    entry.status !== "ready" ||
    entry.updatedAt === null ||
    !Number.isFinite(entry.updatedAt) ||
    now - entry.updatedAt > staleAfterMs
  );
}

function entryStatus(
  entry: AgentUsageEntry | null,
  failed: boolean,
  now: number,
) {
  if (!entry) return failed ? t("Refresh failed") : t("Checking");
  if (failed)
    return hasUsageValues(entry)
      ? t("Refresh failed · showing saved values")
      : t("Refresh failed");
  switch (entry.status) {
    case "ready": {
      if (!hasUsageValues(entry))
        return entry.updatedAt === null
          ? t("Not checked yet")
          : t("No quotas available");
      const stale =
        entry.updatedAt === null ||
        !Number.isFinite(entry.updatedAt) ||
        now - entry.updatedAt > staleAfterMs;
      return stale ? t("Out of date · showing saved values") : t("Current");
    }
    case "keychain-access-required":
      return t("Keychain access needed");
    case "unauthenticated":
      return t("Account not connected");
    case "unsupported":
      return t("Usage unavailable");
    case "error":
      return hasUsageValues(entry)
        ? t("Could not refresh · showing saved values")
        : t("Could not refresh usage");
    case "rate-limited":
      return hasUsageValues(entry)
        ? t("Rate limited · showing saved values")
        : t("Rate limited");
  }
}

function entryDescription(entry: AgentUsageEntry | null) {
  if (!entry) return t("Checking this agent's account usage.");
  switch (entry.status) {
    case "ready":
      return entry.windows.length
        ? ""
        : t("No quota windows are currently available.");
    case "keychain-access-required":
      return t(
        "Allow Lomi to read Cursor’s saved account from macOS Keychain.",
      );
    case "unauthenticated":
      return t("Sign in to this CLI account to view usage.");
    case "unsupported":
      return t("Account usage is unavailable in this CLI mode.");
    case "error":
      return hasUsageValues(entry)
        ? t(
            "The latest check failed. The last successful values are shown below.",
          )
        : t("The account usage check failed.");
    case "rate-limited":
      return hasUsageValues(entry)
        ? t(
            "The account limited this check. The last successful values are shown below.",
          )
        : t("The account is temporarily limiting usage checks.");
  }
}

function safeMessage(message: string | null) {
  if (!message) return null;
  const bounded = message
    .replace(/[\u0000-\u001f\u007f]/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 240);
  return bounded || null;
}

function windowTone(window: AgentUsageWindow) {
  if (!validRemainingPercent(window.remainingPercent)) return "unknown";
  if (window.remainingPercent <= 10) return "critical";
  if (window.remainingPercent <= 25) return "warning";
  return "normal";
}

export function useAgentUsage() {
  const contexts = useRef<Record<string, TerminalContext>>({});
  const signature = useRef(agentUsageSignature([]));
  const epoch = useRef(0);
  const inFlight = useRef<InFlightRequest | null>(null);
  const pending = useRef<PendingRefresh>({ requested: false, force: false });
  const alive = useRef(false);
  const authorization = useRef<AgentUsageTarget | null>(null);
  const [authorizing, setAuthorizing] = useState<AgentUsageTarget | null>(null);
  const [authorizationError, setAuthorizationError] = useState<{
    target: AgentUsageTarget;
    message: string;
  } | null>(null);
  const refreshRef = useRef<(force?: boolean) => Promise<void>>(() =>
    Promise.resolve(),
  );
  const [targets, setTargets] = useState<AgentUsageTarget[]>([]);
  const [entries, setEntries] = useState<AgentUsageEntry[]>([]);
  const [requestFailed, setRequestFailed] = useState(false);
  const [now, setNow] = useState(Date.now());

  const refresh = useCallback((force = false): Promise<void> => {
    const currentTargets = agentUsageTargets(contexts.current);
    if (!native || currentTargets.length === 0) {
      return Promise.resolve();
    }

    if (authorization.current) {
      pending.current.requested = true;
      pending.current.force ||= force;
      return Promise.resolve();
    }

    const currentSignature = agentUsageSignature(currentTargets);
    const currentEpoch = epoch.current;
    if (inFlight.current) {
      if (inFlight.current.epoch !== currentEpoch || force) {
        pending.current.requested = true;
        pending.current.force ||= force;
      }
      return inFlight.current.promise;
    }

    const request: InFlightRequest = {
      signature: currentSignature,
      epoch: currentEpoch,
      promise: Promise.resolve(),
    };
    request.promise = (async () => {
      try {
        const received: AgentUsageEntry[] = [];
        for (const batch of agentUsageTargetBatches(currentTargets)) {
          const result = await api<UsageReply>("inspect_cli_usage", {
            targets: batch,
            force,
          });
          if (
            !alive.current ||
            request.epoch !== epoch.current ||
            request.signature !== signature.current
          )
            return;
          received.push(...result.entries);
        }

        const latestTargets = agentUsageTargets(contexts.current);
        const accepted = received.filter((entry) =>
          latestTargets.some((target) => sameProcess(target, entry)),
        );
        setEntries(accepted);
        setRequestFailed(false);
        setNow(Date.now());
      } catch {
        if (
          alive.current &&
          request.epoch === epoch.current &&
          request.signature === signature.current
        ) {
          setRequestFailed(true);
          setNow(Date.now());
        }
      } finally {
        if (inFlight.current === request) inFlight.current = null;
        if (!alive.current) return;
        const next = pending.current;
        pending.current = { requested: false, force: false };
        if (next.requested) {
          void refreshRef.current(next.force);
        }
      }
    })();
    inFlight.current = request;
    return request.promise;
  }, []);
  refreshRef.current = refresh;

  const authorize = useCallback(async (target: AgentUsageTarget) => {
    if (
      !native ||
      authorization.current ||
      target.process.cli !== "cursor" ||
      !agentUsageTargets(contexts.current).some((current) =>
        sameProcess(current, target),
      )
    )
      return;
    authorization.current = target;
    setAuthorizing(target);
    setAuthorizationError(null);
    // Invalidate passive reads started before the intentional permission request.
    const requestEpoch = ++epoch.current;
    const requestSignature = signature.current;
    const isCurrent = () =>
      alive.current &&
      epoch.current === requestEpoch &&
      signature.current === requestSignature;
    try {
      const result = await api<UsageReply>("authorize_cursor_usage_keychain", {
        target,
      });
      if (isCurrent()) {
        const accepted = result.entries.filter((entry) =>
          sameProcess(target, entry),
        );
        setEntries((current) => [
          ...current.filter((entry) => !sameProcess(target, entry)),
          ...accepted,
        ]);
        setRequestFailed(false);
        setNow(Date.now());
      }
    } catch {
      if (isCurrent())
        setAuthorizationError({
          target,
          message: t(
            "The Keychain access request could not finish. Try again.",
          ),
        });
    } finally {
      authorization.current = null;
      if (alive.current) {
        setAuthorizing(null);
        pending.current = { requested: false, force: false };
        // Check the latest processes using the normal native reader; helper completion is not a grant.
        void refreshRef.current(true);
      }
    }
  }, []);

  const observe = useCallback((next: Record<string, TerminalContext>) => {
    contexts.current = next;
    const nextTargets = agentUsageTargets(next);
    const nextSignature = agentUsageSignature(nextTargets);
    if (nextSignature === signature.current) return;

    signature.current = nextSignature;
    epoch.current += 1;
    setTargets(nextTargets);
    setAuthorizationError(null);
    setEntries((current) =>
      current.filter((entry) =>
        nextTargets.some((target) => sameProcess(target, entry)),
      ),
    );
    setRequestFailed(false);
    setNow(Date.now());

    if (inFlight.current) {
      pending.current.requested = true;
      pending.current.force = false;
    } else if (nextTargets.length) {
      void refreshRef.current(false);
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    const focused = () => {
      if (agentUsageTargets(contexts.current).length)
        void refreshRef.current(false);
    };
    focused();
    window.addEventListener("focus", focused);
    return () => {
      alive.current = false;
      epoch.current += 1;
      window.removeEventListener("focus", focused);
    };
  }, []);

  useEffect(() => {
    if (!targets.length) return;
    const timer = window.setInterval(() => {
      setNow(Date.now());
      void refreshRef.current(false);
    }, 15_000);
    return () => window.clearInterval(timer);
  }, [targets.length]);

  const bar: ReactNode = (
    <AgentUsageBar
      targets={targets}
      entries={entries}
      requestFailed={requestFailed}
      now={now}
      authorizing={authorizing}
      authorizationError={authorizationError}
      onAuthorize={authorize}
      onOpen={() => void refreshRef.current(false)}
    />
  );

  return { observe, bar, usage: { entries, now, failed: requestFailed } };
}

function AgentUsageBar({
  targets,
  entries,
  requestFailed,
  now,
  authorizing,
  authorizationError,
  onAuthorize,
  onOpen,
}: {
  targets: AgentUsageTarget[];
  entries: AgentUsageEntry[];
  requestFailed: boolean;
  now: number;
  authorizing: AgentUsageTarget | null;
  authorizationError: { target: AgentUsageTarget; message: string } | null;
  onAuthorize: (target: AgentUsageTarget) => Promise<void>;
  onOpen: () => void;
}) {
  useT();
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const agentList = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState({ left: 0, top: 0 });

  const entryFor = (target: AgentUsageTarget) =>
    entries.find((entry) => sameProcess(target, entry)) ?? null;
  const groups = groupAgentUsage(targets, entries);
  const providers = (Object.keys(cliNames) as CliAgent[]).flatMap((cli) => {
    const providerTargets = targets.filter(
      (target) => target.process.cli === cli,
    );
    if (providerTargets.length === 0) return [];

    const providerEntries = providerTargets.map((target) => entryFor(target));
    const matchingEntries = providerEntries.filter(
      (entry): entry is AgentUsageEntry => entry !== null,
    );
    const worst = worstRemainingWindow(matchingEntries);
    const remainingPercent = worst?.window.remainingPercent ?? null;
    const stale = providerEntries.some((entry) =>
      isStale(entry, requestFailed, now),
    );
    const statuses = [
      ...new Set(
        providerEntries.map((entry) => entryStatus(entry, requestFailed, now)),
      ),
    ];
    const reasons = [
      ...new Set(
        providerEntries
          .map(
            (entry) =>
              safeMessage(entry?.message ?? null) ?? entryDescription(entry),
          )
          .filter((reason): reason is string => Boolean(reason)),
      ),
    ];
    const otherStatuses = statuses.filter((status) => status !== t("Current"));
    const status =
      remainingPercent === null
        ? statuses.join(", ")
        : `${
            stale
              ? t("{percent} remaining, stale data", {
                  percent: formatPercent(remainingPercent),
                })
              : t("{percent} remaining", {
                  percent: formatPercent(remainingPercent),
                })
          }${otherStatuses.length ? `; ${otherStatuses.join(", ")}` : ""}`;
    const tone = worst ? windowTone(worst.window) : "unknown";
    const summary = `${t("{cli} · {status}", { cli: cliNames[cli], status })}${remainingPercent === null && reasons.length ? ` · ${reasons.join("; ")}` : ""}`;

    return [
      {
        cli,
        remainingPercent,
        stale,
        status,
        tone,
        summary,
        title: tp(
          providerTargets.length,
          "{summary} · {count} active session",
          "{summary} · {count} active sessions",
          { summary },
        ),
      },
    ];
  });
  const count = targets.length;
  const title = t("{summaries}. Open account usage details.", {
    summaries: providers.map(({ title }) => title).join("; "),
  });
  const label = t(
    "CLI account usage. {summaries}. Activate to open account usage details.",
    { summaries: providers.map(({ summary }) => summary).join("; ") },
  );

  const close = useCallback((restoreFocus: boolean) => {
    setOpen(false);
    if (restoreFocus)
      requestAnimationFrame(() =>
        trigger.current?.focus({ preventScroll: true }),
      );
  }, []);

  const show = useCallback(() => {
    setOpen(true);
    onOpen();
  }, [onOpen]);

  useLayoutEffect(() => {
    if (!open || !trigger.current || !menu.current) return;
    const triggerBounds = trigger.current.getBoundingClientRect();
    const menuBounds = menu.current.getBoundingClientRect();
    const left = Math.max(
      8,
      Math.min(
        triggerBounds.right - menuBounds.width,
        innerWidth - menuBounds.width - 8,
      ),
    );
    const below = triggerBounds.bottom + 6;
    const top =
      below + menuBounds.height <= innerHeight - 8
        ? below
        : Math.max(8, triggerBounds.top - menuBounds.height - 6);
    setPosition({ left, top });
  }, [entries, open, targets, authorizing, authorizationError]);

  useLayoutEffect(() => {
    if (!open) return;
    agentList.current?.focus({ preventScroll: true });
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const outside = (event: Event) => {
      const target = event.target as Node;
      if (trigger.current?.contains(target) || menu.current?.contains(target))
        return;
      close(false);
    };
    const dismiss = () => close(false);
    document.addEventListener("pointerdown", outside);
    window.addEventListener("resize", dismiss);
    window.addEventListener("blur", dismiss);
    window.visualViewport?.addEventListener("resize", dismiss);
    return () => {
      document.removeEventListener("pointerdown", outside);
      window.removeEventListener("resize", dismiss);
      window.removeEventListener("blur", dismiss);
      window.visualViewport?.removeEventListener("resize", dismiss);
    };
  }, [close, open]);

  useLayoutEffect(() => {
    if (!count && open) setOpen(false);
  }, [count, open]);

  if (!count) return null;

  return (
    <>
      <button
        ref={trigger}
        type="button"
        className="agent-usage-trigger"
        title={title}
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => (open ? close(false) : show())}
        onContextMenu={(event) => {
          event.preventDefault();
          show();
        }}
        onKeyDown={(event) => {
          if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
            const list = event.currentTarget.querySelector<HTMLElement>(
              ".agent-usage-trigger-list",
            );
            if (list && list.scrollWidth > list.clientWidth) {
              event.preventDefault();
              event.stopPropagation();
              list.scrollBy({
                left: event.key === "ArrowRight" ? 48 : -48,
                behavior: "auto",
              });
            }
            return;
          }
          if (
            event.key === "ArrowDown" ||
            event.key === "ContextMenu" ||
            (event.shiftKey && event.key === "F10")
          ) {
            event.preventDefault();
            event.stopPropagation();
            show();
          }
        }}
      >
        <span className="agent-usage-trigger-list" aria-hidden="true">
          {providers.map((provider) => (
            <span
              key={provider.cli}
              className="agent-usage-summary"
              data-cli={provider.cli}
              data-tone={provider.tone}
              data-stale={provider.stale || undefined}
              title={provider.title}
            >
              <CliAgentIcon cli={provider.cli} />
              <span className="agent-usage-trigger-value">
                {provider.remainingPercent === null
                  ? "—"
                  : formatPercent(provider.remainingPercent)}
              </span>
              {provider.stale && <span className="agent-usage-stale-dot" />}
            </span>
          ))}
        </span>
      </button>
      {open &&
        createPortal(
          <div
            ref={menu}
            className="menu agent-usage-menu"
            role="menu"
            aria-label={t("CLI account usage")}
            style={position}
            onContextMenu={(event) => event.preventDefault()}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.preventDefault();
                event.stopPropagation();
                close(true);
                return;
              }
              if (event.key === "Tab") {
                event.preventDefault();
                event.stopPropagation();
                const controls = [
                  agentList.current,
                  ...event.currentTarget.querySelectorAll<HTMLButtonElement>(
                    'button[role="menuitem"]:not(:disabled)',
                  ),
                ].filter(
                  (element): element is HTMLDivElement | HTMLButtonElement =>
                    element !== null,
                );
                const index = controls.indexOf(
                  document.activeElement as HTMLButtonElement,
                );
                controls[
                  (index + (event.shiftKey ? -1 : 1) + controls.length) %
                    controls.length
                ]?.focus({ preventScroll: true });
                return;
              }
              if (
                event.target === agentList.current &&
                [
                  "ArrowUp",
                  "ArrowDown",
                  "Home",
                  "End",
                  "PageUp",
                  "PageDown",
                  " ",
                ].includes(event.key)
              )
                return;
              if (!["ArrowUp", "ArrowDown", "Home", "End"].includes(event.key))
                return;
              event.preventDefault();
              const items = [
                ...event.currentTarget.querySelectorAll<HTMLButtonElement>(
                  'button[role="menuitem"]:not(:disabled)',
                ),
              ];
              const index = items.indexOf(
                document.activeElement as HTMLButtonElement,
              );
              items[
                event.key === "Home"
                  ? 0
                  : event.key === "End"
                    ? items.length - 1
                    : (index +
                        (event.key === "ArrowDown" ? 1 : -1) +
                        items.length) %
                      items.length
              ]?.focus();
            }}
          >
            <div className="agent-usage-menu-heading" role="presentation">
              <strong>{t("Account usage")}</strong>
            </div>
            <div
              ref={agentList}
              className="agent-usage-list"
              role="region"
              aria-label={t("Usage details by agent")}
              tabIndex={0}
            >
              {groups.map(({ target, entry }, index) => {
                const stale = isStale(entry, requestFailed, now);
                const statusText = entryStatus(entry, requestFailed, now);
                const message = safeMessage(entry?.message ?? null);
                const description = message ? "" : entryDescription(entry);
                return (
                  <section
                    className="agent-usage-agent"
                    role="group"
                    aria-label={t("{cli} usage {index}", {
                      cli: cliNames[target.process.cli],
                      index: index + 1,
                    })}
                    key={`${target.id}:${target.process.cli}:${target.process.pid}`}
                  >
                    <div className="agent-usage-agent-heading">
                      <CliAgentIcon cli={target.process.cli} />
                      <strong>{cliNames[target.process.cli]}</strong>
                      {(stale ||
                        entry?.status === "keychain-access-required") && (
                        <span
                          className="agent-usage-status"
                          data-stale={stale || undefined}
                          data-attention={
                            entry?.status === "keychain-access-required" ||
                            undefined
                          }
                        >
                          {stale && (
                            <CircleAlert size={12} aria-hidden="true" />
                          )}
                          {statusText}
                        </span>
                      )}
                    </div>
                    {description && (
                      <p className="agent-usage-description">{description}</p>
                    )}
                    {message && (
                      <p className="agent-usage-description">{message}</p>
                    )}
                    {target.process.cli === "cursor" &&
                      entry?.status === "keychain-access-required" && (
                        <div className="agent-usage-keychain">
                          <button
                            type="button"
                            role="menuitem"
                            tabIndex={-1}
                            className="menu-item"
                            disabled={authorizing !== null}
                            aria-busy={
                              Boolean(
                                authorizing && sameProcess(authorizing, entry),
                              ) || undefined
                            }
                            onClick={() =>
                              void onAuthorize({
                                id: entry.id,
                                process: entry.process,
                              })
                            }
                          >
                            {authorizing && sameProcess(authorizing, entry)
                              ? t("Waiting for macOS…")
                              : t("Allow Keychain access…")}
                          </button>
                          <p className="agent-usage-description">
                            {t(
                              "Choose Always Allow in the macOS dialog for future checks.",
                            )}
                          </p>
                          <p className="agent-usage-description" role="status">
                            {authorizing && sameProcess(authorizing, entry)
                              ? t("Waiting for macOS…")
                              : authorizationError &&
                                  sameProcess(authorizationError.target, entry)
                                ? authorizationError.message
                                : ""}
                          </p>
                        </div>
                      )}
                    {entry?.windows.length ? (
                      <div className="agent-usage-windows">
                        {entry.windows.map((window, windowIndex) => (
                          <UsageWindow
                            key={`${window.label}:${windowIndex}`}
                            window={window}
                            now={now}
                          />
                        ))}
                      </div>
                    ) : !description && !message ? (
                      <p className="agent-usage-no-windows">
                        {entry
                          ? t("No quota windows to show.")
                          : t("Usage data is loading.")}
                      </p>
                    ) : null}
                    {entry?.retryAt !== null &&
                      entry?.retryAt !== undefined && (
                        <p className="agent-usage-retry">
                          {t("Try again after {time}.", {
                            time:
                              formatReset(entry.retryAt) ?? t("the next check"),
                          })}
                        </p>
                      )}
                  </section>
                );
              })}
            </div>
          </div>,
          document.body,
        )}
    </>
  );
}

const meterCells = 24;

function formatCountdown(timestamp: number | null, now: number) {
  if (timestamp === null || !Number.isFinite(timestamp)) return null;
  const minutes = Math.max(0, Math.round((timestamp - now) / 60_000));
  const days = Math.floor(minutes / 1440);
  const hours = Math.floor((minutes % 1440) / 60);
  if (days) return `${days}d ${hours}h`;
  if (hours) return `${hours}h ${minutes % 60}m`;
  return `${minutes}m`;
}

/** Remaining share of the window's time, where an even pace would leave quota. */
function paceRemaining(window: AgentUsageWindow, now: number) {
  const length = window.windowSeconds;
  if (!length || length <= 0 || window.resetsAt === null) return null;
  const left = (window.resetsAt - now) / 1000;
  if (!Number.isFinite(left) || left < 0 || left > length) return null;
  return (left / length) * 100;
}

function UsageWindow({
  window,
  now,
}: {
  window: AgentUsageWindow;
  now: number;
}) {
  const { t } = useT();
  const remaining = validRemainingPercent(window.remainingPercent)
    ? window.remainingPercent
    : null;
  const used = formatQuantity(window.used, window.unit);
  const limit = formatQuantity(window.limit, window.unit);
  const reset = formatReset(window.resetsAt);
  const countdown = formatCountdown(window.resetsAt, now);
  const pace = remaining === null ? null : paceRemaining(window, now);
  const label = window.label || t("Quota window");
  const amount =
    used && limit
      ? t("{used} used / {limit}", { used, limit })
      : used
        ? t("{used} used", { used })
        : limit
          ? t("Limit {limit}", { limit })
          : null;

  return (
    <div
      className="agent-usage-window"
      data-tone={windowTone(window)}
      title={[
        amount,
        reset
          ? t("Resets {time}", { time: reset })
          : t("Reset time unavailable"),
      ]
        .filter(Boolean)
        .join(" · ")}
    >
      <div className="agent-usage-window-heading">
        <span className="agent-usage-window-label">{label}</span>
        <span className="agent-usage-window-reset">
          {countdown
            ? t("resets in {time}", { time: countdown })
            : t("reset unknown")}
        </span>
      </div>
      <div className="agent-usage-window-body">
        <strong className="agent-usage-window-value">
          {remaining === null ? "—" : formatPercent(remaining)}
        </strong>
        <div
          className="agent-usage-meter"
          role={remaining === null ? undefined : "meter"}
          aria-label={t("{label} quota", { label })}
          aria-valuemin={remaining === null ? undefined : 0}
          aria-valuemax={remaining === null ? undefined : 100}
          aria-valuenow={remaining ?? undefined}
          aria-valuetext={
            remaining === null
              ? undefined
              : reset
                ? t("{percent} remaining, resets {time}", {
                    percent: formatPercent(remaining),
                    time: reset,
                  })
                : t("{percent} remaining", {
                    percent: formatPercent(remaining),
                  })
          }
        >
          <span className="agent-usage-meter-cells">
            <span
              style={{
                // Whole cells only; any quota left keeps at least one lit.
                width: `${(Math.ceil(((remaining ?? 0) / 100) * meterCells) / meterCells) * 100}%`,
              }}
            />
          </span>
          {pace !== null && (
            <span
              className="agent-usage-meter-pace"
              style={{ left: `${pace}%` }}
              title={t("Even pace for the time left in this window")}
            />
          )}
        </div>
      </div>
    </div>
  );
}
