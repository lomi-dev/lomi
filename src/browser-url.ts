import { t } from "./i18n.ts";
export function restoreBrowserUrl(value: unknown): string {
  if (value === "about:blank") return value;
  if (typeof value !== "string" || value.length > 16_384) return "about:blank";
  try {
    const url = new URL(value);
    return ["http:", "https:"].includes(url.protocol) &&
      !url.username &&
      !url.password &&
      !["tauri.localhost", "ipc.localhost", "theme.localhost"].includes(
        url.hostname,
      )
      ? url.href
      : "about:blank";
  } catch {
    return "about:blank";
  }
}

export function browserAddress(value: string): string {
  const text = value.trim();
  if (!text || text === "about:blank") return "about:blank";
  const local = /^(localhost|127\.\d+\.\d+\.\d+|\[::1\])(?=[:/]|$)/i.test(text);
  const hostWithPort = /^(?:[^:/\s]+|\[[a-f\d:]+\]):\d+(?:[/?#]|$)/i.test(text);
  const explicit = /^[a-z][a-z\d+.-]*:/i.test(text) && !local && !hostWithPort;
  const address = explicit
    ? text
    : local || hostWithPort
      ? `http://${text}`
      : !/\s/.test(text) && text.includes(".")
        ? `https://${text}`
        : `https://duckduckgo.com/?q=${encodeURIComponent(text)}`;
  const url = restoreBrowserUrl(address);
  if (url === "about:blank")
    throw new Error(t("Enter an HTTP or HTTPS address."));
  return url;
}
