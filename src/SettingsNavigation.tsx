import { Fragment, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  Bell,
  Globe,
  Info,
  Keyboard,
  Layers,
  MessageSquare,
  Palette,
  Puzzle,
  Search,
  Settings,
  ShieldCheck,
  Smartphone,
  Terminal,
} from "./icons";
import type { SettingsPageId } from "./settings-pages";
import {
  highlightParts,
  searchSettings,
  settingsPageLabels,
  type SettingsSearchEntry,
} from "./settings-search";
import "./settings-search.css";

const groups = [
  {
    label: "Personalize",
    pages: [
      {
        id: "general",
        label: "General",
        icon: Settings,
      },
      {
        id: "themes",
        label: "Themes",
        icon: Palette,
      },
      {
        id: "keybinds",
        label: "Keybinds",
        icon: Keyboard,
      },
      {
        id: "notifications",
        label: "Notifications",
        icon: Bell,
      },
    ],
  },
  {
    label: "Workspace",
    pages: [
      {
        id: "terminal",
        label: "Terminal",
        icon: Terminal,
      },
      {
        id: "android",
        label: "Android",
        icon: Smartphone,
      },
      {
        id: "plugins",
        label: "Plugins",
        icon: Puzzle,
      },
    ],
  },
  {
    label: "Chat AI",
    pages: [
      {
        id: "chat-providers",
        label: "Providers",
        icon: MessageSquare,
      },
      {
        id: "chat-defaults",
        label: "Defaults",
        icon: Layers,
      },
    ],
  },
  {
    label: "Access",
    pages: [
      {
        id: "account",
        label: "Account",
        icon: ShieldCheck,
      },
      {
        id: "remote",
        label: "Remote",
        icon: Globe,
      },
      {
        id: "agent-control",
        label: "Agent control",
        icon: ShieldCheck,
      },
    ],
  },
  {
    label: null,
    pages: [
      {
        id: "about",
        label: "About",
        icon: Info,
      },
    ],
  },
] as const;

export function settingsPageLabel(id: SettingsPageId) {
  return settingsPageLabels[id] ?? "Settings";
}

const pageIcons = Object.fromEntries(
  groups.flatMap((group) => group.pages.map((page) => [page.id, page.icon])),
) as Record<SettingsPageId, (typeof groups)[number]["pages"][number]["icon"]>;

function Highlighted({ text, query }: { text: string; query: string }) {
  return highlightParts(text, query).map((part, index) =>
    part.match ? <mark key={index}>{part.text}</mark> : part.text,
  );
}

interface SettingsNavigationProps {
  active: boolean;
  page: SettingsPageId;
  requestRevision: number;
  remoteAvailable: boolean;
  remoteDisabledReason: string;
  onSelect: (page: SettingsPageId) => void;
  onReveal: (entry: SettingsSearchEntry) => void;
}

export default function SettingsNavigation({
  active,
  page,
  requestRevision,
  remoteAvailable,
  remoteDisabledReason,
  onSelect,
  onReveal,
}: SettingsNavigationProps) {
  const navigation = useRef<HTMLElement>(null);
  const [query, setQuery] = useState("");
  const [activeResult, setActiveResult] = useState(0);
  const searching = query.trim() !== "";
  const results = useMemo(() => searchSettings(query), [query]);
  const resultId = (index: number) => `settings-search-result-${index}`;
  const open = (entry: SettingsSearchEntry) => {
    if (entry.page === "remote" && !remoteAvailable) return;
    setQuery("");
    onReveal(entry);
  };
  const enabledButtons = () =>
    Array.from(
      navigation.current?.querySelectorAll<HTMLButtonElement>(
        ".settings-nav-item:not(:disabled)",
      ) ?? [],
    );
  useLayoutEffect(() => {
    if (!active || document.querySelector("dialog[open]")) return;
    const current = navigation.current?.querySelector<HTMLButtonElement>(
      '[aria-current="page"]',
    );
    current?.focus({ preventScroll: true });
    current?.scrollIntoView({
      block: "nearest",
      inline: "nearest",
      behavior: "instant",
    });
  }, [active, page, requestRevision]);

  return (
    <nav
      ref={navigation}
      className="settings-navigation"
      aria-label="Settings pages"
    >
      <label className="settings-nav-search">
        <Search size={14} aria-hidden="true" />
        <input
          type="search"
          aria-label="Search settings"
          placeholder="Search"
          value={query}
          role="combobox"
          aria-expanded={searching}
          aria-controls="settings-search-results"
          aria-autocomplete="list"
          aria-activedescendant={
            searching && results.length ? resultId(activeResult) : undefined
          }
          onChange={(event) => {
            setQuery(event.target.value);
            setActiveResult(0);
          }}
          onKeyDown={(event) => {
            if (event.key === "Escape" && searching) {
              event.preventDefault();
              event.stopPropagation();
              setQuery("");
            } else if (
              searching &&
              results.length &&
              (event.key === "ArrowDown" || event.key === "ArrowUp")
            ) {
              event.preventDefault();
              setActiveResult(
                (activeResult +
                  (event.key === "ArrowDown" ? 1 : -1) +
                  results.length) %
                  results.length,
              );
            } else if (event.key === "Enter" && searching) {
              event.preventDefault();
              const entry = results[activeResult];
              if (entry) open(entry);
            }
          }}
        />
      </label>
      {searching ? (
        <div
          id="settings-search-results"
          className="settings-search-results"
          role="listbox"
          aria-label="Search results"
        >
          {results.length === 0 && (
            <p className="settings-search-empty" role="status">
              No results for “{query.trim()}”
            </p>
          )}
          {results.map((entry, index) => {
            const Icon = pageIcons[entry.page];
            const disabled = entry.page === "remote" && !remoteAvailable;
            return (
              <div
                key={`${entry.page}/${entry.tab ?? ""}/${entry.section ?? ""}/${entry.label}`}
                id={resultId(index)}
                role="option"
                className="settings-search-result"
                aria-selected={index === activeResult}
                aria-disabled={disabled || undefined}
                title={disabled ? remoteDisabledReason : undefined}
                onMouseEnter={() => setActiveResult(index)}
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => open(entry)}
              >
                <Icon size={16} aria-hidden="true" />
                <span className="settings-search-text">
                  <span className="settings-search-label">
                    <Highlighted text={entry.label} query={query} />
                  </span>
                  <span className="settings-search-crumb">
                    {settingsPageLabels[entry.page]}
                    {entry.section && ` › ${entry.section}`}
                  </span>
                </span>
              </div>
            );
          })}
        </div>
      ) : (
        groups.map((group) => (
          <Fragment key={group.label ?? "footer"}>
            {group.label && (
              <div className="settings-nav-group-label">{group.label}</div>
            )}
            {group.pages.map(({ id, label, icon: Icon }) => (
              <button
                key={id}
                type="button"
                className={`settings-nav-item${id === "account" ? " settings-nav-account" : id === "about" ? " settings-nav-about" : ""}`}
                aria-current={page === id ? "page" : undefined}
                disabled={id === "remote" && !remoteAvailable}
                aria-description={
                  id === "remote" && !remoteAvailable
                    ? remoteDisabledReason
                    : undefined
                }
                title={
                  id === "remote" && !remoteAvailable
                    ? remoteDisabledReason
                    : undefined
                }
                onClick={() => onSelect(id)}
                onKeyDown={(event) => {
                  if (
                    !["ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)
                  )
                    return;
                  event.preventDefault();
                  event.stopPropagation();
                  const buttons = enabledButtons();
                  const index = buttons.indexOf(event.currentTarget);
                  const next =
                    event.key === "Home"
                      ? 0
                      : event.key === "End"
                        ? buttons.length - 1
                        : (index +
                            (event.key === "ArrowDown" ? 1 : -1) +
                            buttons.length) %
                          buttons.length;
                  buttons[next]?.focus();
                }}
              >
                <Icon size={16} aria-hidden="true" />
                <span className="settings-nav-label">{label}</span>
              </button>
            ))}
          </Fragment>
        ))
      )}
    </nav>
  );
}
