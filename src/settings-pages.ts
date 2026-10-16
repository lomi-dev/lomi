export const settingsPageIds = [
  "keybinds",
  "notifications",
  "terminal",
  "themes",
  "plugins",
  "chat-ai",
  "android",
  "agent-control",
  "remote",
  "account",
  "about",
] as const;

export type SettingsPageId = (typeof settingsPageIds)[number];

export const defaultSettingsPage: SettingsPageId = "keybinds";

export function parseSettingsPage(value: unknown): SettingsPageId | null {
  return typeof value === "string" &&
    settingsPageIds.some((page) => page === value)
    ? (value as SettingsPageId)
    : null;
}
