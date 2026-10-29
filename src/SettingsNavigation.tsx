import { Fragment, useLayoutEffect, useRef } from "react";
import {
  Bell,
  Globe,
  Info,
  Keyboard,
  MessageSquare,
  Palette,
  Puzzle,
  ShieldCheck,
  Smartphone,
  Terminal,
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
        id: "chat-ai",
        label: "Chat AI",
        icon: MessageSquare,
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

interface SettingsNavigationProps {
  active: boolean;
  page: SettingsPageId;
  requestRevision: number;
  remoteAvailable: boolean;
  remoteDisabledReason: string;
  onSelect: (page: SettingsPageId) => void;
}

export default function SettingsNavigation({
  active,
  page,
  requestRevision,
  remoteAvailable,
  remoteDisabledReason,
  onSelect,
}: SettingsNavigationProps) {
  const navigation = useRef<HTMLElement>(null);
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
      <div className="settings-navigation-header">
        <div className="settings-navigation-title">Settings</div>
        <p className="settings-navigation-subtitle">Make Lomi yours.</p>
      </div>
      {groups.map(
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
    </nav>
  );
}
