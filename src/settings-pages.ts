export const settingsPageIds = [
  "general",
  "keybinds",
  "notifications",
  "terminal",
  "speech",
  "themes",
  "plugins",
  "chat-providers",
  "chat-defaults",
  "android",
  "agent-control",
  "remote",
  "account",
  "about",
] as const;

export type SettingsPageId = (typeof settingsPageIds)[number];

export const defaultSettingsPage: SettingsPageId = "general";

export function parseSettingsPage(value: unknown): SettingsPageId | null {
  return typeof value === "string" &&
    settingsPageIds.some((page) => page === value)
    ? (value as SettingsPageId)
    : null;
}
