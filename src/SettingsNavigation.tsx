import { Fragment, useLayoutEffect, useRef, useState } from "react";
import {
  Bell,
  Globe,
  Info,
  Keyboard,
  MessageSquare,
  Palette,
  Puzzle,
  Search,
  ShieldCheck,
  Smartphone,
  Terminal,
  X,
} from "./icons";
import type { SettingsPageId } from "./settings-pages";

const groups = [
  {
    label: "Personalize",
    pages: [
      {
        id: "themes",
        label: "Themes",
        icon: Palette,
        keywords: "appearance color colour light dark",
      },
      {
        id: "keybinds",
        label: "Keybinds",
        icon: Keyboard,
        keywords: "keyboard shortcuts keys focus explorer",
      },
      {
        id: "notifications",
        label: "Notifications",
        icon: Bell,
        keywords: "alerts sounds agent",
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
        keywords: "shell font cursor scrollback",
      },
      {
        id: "chat-ai",
        label: "Chat AI",
        icon: MessageSquare,
        keywords: "models providers api connections assistant",
      },
      {
        id: "android",
        label: "Android",
        icon: Smartphone,
        keywords: "phone device emulator simulator sdk",
      },
      {
        id: "plugins",
        label: "Plugins",
        icon: Puzzle,
        keywords: "extensions install packages",
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
        keywords: "sign in login profile authentication",
      },
      {
        id: "remote",
        label: "Remote",
        icon: Globe,
        keywords: "sharing browser access hosting",
      },
      {
        id: "agent-control",
        label: "Agent control",
        icon: ShieldCheck,
        keywords: "mcp permissions pairing cli tools",
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
        keywords: "version updates application lomi",
      },
    ],
  },
] as const;

interface SettingsNavigationProps {
  active: boolean;
  page: SettingsPageId;
  requestRevision: number;
  remoteAvailable: boolean;
  onSelect: (page: SettingsPageId) => void;
}

export default function SettingsNavigation({
  active,
  page,
  requestRevision,
  remoteAvailable,
  onSelect,
}: SettingsNavigationProps) {
  const [query, setQuery] = useState("");
  const navigation = useRef<HTMLElement>(null);
  const search = useRef<HTMLInputElement>(null);
  const pendingFocus = useRef(false);
  const terms = query.toLocaleLowerCase().trim().split(/\s+/).filter(Boolean);
  const visibleGroups = groups.map((group) => ({
    ...group,
    pages: group.pages.filter((entry) =>
      terms.every((term) =>
        `${entry.label} ${entry.keywords}`.toLocaleLowerCase().includes(term),
      ),
    ),
  }));
  const hasMatches = visibleGroups.some((group) => group.pages.length > 0);
  const enabledButtons = () =>
    Array.from(
      navigation.current?.querySelectorAll<HTMLButtonElement>(
        ".settings-nav-item:not(:disabled)",
      ) ?? [],
    );
  useLayoutEffect(() => {
    if (!active) return;
    pendingFocus.current = true;
    setQuery("");
  }, [active, page, requestRevision]);
  useLayoutEffect(() => {
    if (!active || !pendingFocus.current || query) return;
    pendingFocus.current = false;
    if (!document.querySelector("dialog[open]")) {
      const current = navigation.current?.querySelector<HTMLButtonElement>(
        '[aria-current="page"]',
      );
      current?.focus({ preventScroll: true });
      current?.scrollIntoView({
        block: "nearest",
        inline: "nearest",
        behavior: "instant",
      });
    }
  }, [active, page, requestRevision, query]);
  const clearSearch = () => {
    setQuery("");
    search.current?.focus();
  };

  return (
    <nav
      ref={navigation}
      className="settings-navigation"
      aria-label="Settings pages"
    >
      <div className="settings-navigation-header">
        <div className="settings-navigation-title">Settings</div>
        <p className="settings-navigation-subtitle">Make Lomi yours.</p>
      </div>
      <div className="settings-navigation-search">
        <Search
          className="settings-navigation-search-icon"
          size={14}
          aria-hidden="true"
        />
        <input
          ref={search}
          className="settings-navigation-search-input"
          type="text"
          aria-label="Find a settings page"
          placeholder="Find a page…"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.nativeEvent.isComposing) return;
            if (event.key === "Escape") {
              event.preventDefault();
              event.stopPropagation();
              clearSearch();
            } else if (event.key === "ArrowDown" || event.key === "Enter") {
              event.preventDefault();
              event.stopPropagation();
              const first = enabledButtons()[0];
              if (event.key === "Enter") first?.click();
              else first?.focus();
            }
          }}
        />
        {query && (
          <button
            type="button"
            className="settings-navigation-search-clear"
            aria-label="Clear page search"
            onClick={clearSearch}
          >
            <X size={12} aria-hidden="true" />
          </button>
        )}
      </div>
      {visibleGroups.map(
        (group) =>
          group.pages.length > 0 && (
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
                  title={
                    id === "remote" && !remoteAvailable
                      ? "Sign in to use remote sharing"
                      : undefined
                  }
                  onClick={() => {
                    if (query) {
                      pendingFocus.current = true;
                      setQuery("");
                    }
                    onSelect(id);
                  }}
                  onKeyDown={(event) => {
                    if (
                      !["ArrowUp", "ArrowDown", "Home", "End"].includes(
                        event.key,
                      )
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
          ),
      )}
      {!hasMatches && (
        <div className="settings-navigation-empty">
          <p role="status">No matching pages.</p>
          <button
            type="button"
            className="text-button settings-navigation-clear"
            onClick={clearSearch}
          >
            Clear search
          </button>
        </div>
      )}
    </nav>
  );
}
