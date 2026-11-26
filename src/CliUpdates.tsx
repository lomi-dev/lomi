import { useCallback, useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { api, errorMessage, native } from "./api";
import { Download } from "./icons";
import { useT } from "./i18n";
import { Menu } from "./ui";

interface CliUpdate {
  cli: string;
  name: string;
  installed: string;
  latest: string;
  command: string | null;
}

const STORAGE_KEY = "lomi.cliUpdates.dismissed.v1";
const FIRST_CHECK = 20_000;
const INTERVAL = 6 * 60 * 60 * 1000;
const RECHECK_AFTER_UPDATE = 60_000;
const identity = (update: CliUpdate) => `${update.cli}@${update.latest}`;

function loadDismissed() {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "");
    return new Set(Array.isArray(parsed) ? parsed.map(String) : []);
  } catch {
    return new Set<string>();
  }
}

/** Offers outdated agent CLIs; updating runs visibly in a new terminal tab. */
export function useCliUpdates(
  runInTerminal: (command: string) => void,
  onError: (message: string) => void,
) {
  const { t, tp } = useT();
  const [updates, setUpdates] = useState<CliUpdate[]>([]);
  const [dismissed, setDismissed] = useState(loadDismissed);
  const [open, setOpen] = useState(false);
  const checked = useRef(0);
  const latest = useRef({ runInTerminal, onError });
  latest.current = { runInTerminal, onError };

  const check = useCallback(async (force: boolean) => {
    if (!native) return;
    try {
      const found = await api<CliUpdate[]>("check_cli_updates", { force });
      checked.current = Date.now();
      setUpdates(found);
    } catch {
      // Offline or an unsupported shell: stay quiet and retry on the next cycle.
    }
  }, []);

  useEffect(() => {
    const first = setTimeout(() => void check(false), FIRST_CHECK);
    const every = setInterval(() => void check(false), INTERVAL);
    const focused = () => {
      if (Date.now() - checked.current > INTERVAL) void check(false);
    };
    window.addEventListener("focus", focused);
    return () => {
      clearTimeout(first);
      clearInterval(every);
      window.removeEventListener("focus", focused);
    };
  }, [check]);

  const dismiss = (update: CliUpdate) => {
    const next = new Set(dismissed).add(identity(update));
    setDismissed(next);
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify([...next]));
    } catch {
      // The dismissal then lasts only for this session.
    }
  };
  const run = (command: string) => {
    setOpen(false);
    try {
      latest.current.runInTerminal(command);
    } catch (error) {
      latest.current.onError(errorMessage(error));
      return;
    }
    setTimeout(() => void check(true), RECHECK_AFTER_UPDATE);
  };

  const visible = updates.filter((update) => !dismissed.has(identity(update)));
  const close = useCallback(() => setOpen(false), []);
  return {
    bar: visible.length > 0 && (
      <div className="cli-integrations cli-updates">
        <div className="cli-integration-item">
          <button
            className="cli-integration-button cli-updates-trigger"
            data-menu-trigger
            aria-haspopup="true"
            aria-expanded={open}
            onClick={() => setOpen((shown) => !shown)}
          >
            <Download size={12} aria-hidden="true" />
            {visible.length === 1
              ? t("{name} update available", { name: visible[0].name })
              : tp(
                  visible.length,
                  "{count} CLI update available",
                  "{count} CLI updates available",
                )}
          </button>
        </div>
        {open && (
          <Menu className="cli-updates-menu" onClose={close}>
            {visible.map((update) => (
              <div key={identity(update)} className="cli-updates-row">
                <span className="cli-updates-name">{update.name}</span>
                <span className="cli-updates-version">
                  {update.installed} → {update.latest}
                </span>
                {update.command ? (
                  <button
                    className="cli-integration-button"
                    title={update.command}
                    onClick={() => run(update.command!)}
                  >
                    {t("Update")}
                  </button>
                ) : (
                  <span className="cli-updates-version">
                    {t("Update it with the tool you installed it with")}
                  </span>
                )}
                <button
                  className="cli-integration-dismiss"
                  aria-label={t("Hide the {name} update", {
                    name: update.name,
                  })}
                  title={t("Hide until a newer version is released")}
                  onClick={() => dismiss(update)}
                >
                  <X size={12} aria-hidden="true" />
                </button>
              </div>
            ))}
          </Menu>
        )}
      </div>
    ),
  };
}
