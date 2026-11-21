import { actions as keybindingActions } from "./keybindings";
import type { SettingsPageId } from "./settings-pages";

export interface SettingsSearchEntry {
  page: SettingsPageId;
  section?: string;
  label: string;
  description?: string;
  keywords?: string[];
  /** Agent control tab to open before revealing. */
  tab?: string;
}

export const settingsPageLabels: Record<SettingsPageId, string> = {
  general: "General",
  themes: "Themes",
  keybinds: "Keybinds",
  notifications: "Notifications",
  terminal: "Terminal",
  android: "Android",
  plugins: "Plugins",
  "chat-providers": "Providers",
  "chat-defaults": "Defaults",
  account: "Account",
  remote: "Remote",
  "agent-control": "Agent control",
  about: "About",
};

type Row = [
  label: string,
  description?: string,
  keywords?: string,
  tab?: string,
];

// Each page lists its sections; the section title itself is an entry, and so
// is every row. Keywords are space separated synonyms.
function build(
  page: SettingsPageId,
  keywords: string,
  sections: [title: string, keywords: string, rows: Row[], tab?: string][],
): SettingsSearchEntry[] {
  const words = (value?: string) => (value ? value.split(/\s{2,}|,\s*/) : []);
  return [
    { page, label: settingsPageLabels[page], keywords: words(keywords) },
    ...sections.flatMap(([title, sectionKeywords, rows, tab]) => [
      { page, label: title, keywords: words(sectionKeywords), tab },
      ...rows.map(([label, description, rowKeywords, rowTab]) => ({
        page,
        section: title,
        label,
        description,
        keywords: words(rowKeywords),
        tab: rowTab ?? tab,
      })),
    ]),
  ];
}

const general = build("general", "general, preferences, basics", [
  [
    "Workflow",
    "mode, cli first, chat first, terminal, ai chat",
    [
      [
        "Start with",
        "Terminals or chat up front.",
        "cli first, chat first, default tab",
      ],
    ],
  ],
  [
    "Projects",
    "new project, clone, folder",
    [
      [
        "Default location",
        "Where new and cloned projects go.",
        "parent folder, path, directory",
      ],
    ],
  ],
]);

const themes = build(
  "themes",
  "appearance, look, color, theme, icons, customize",
  [
    [
      "Appearance",
      "look",
      [
        [
          "Color mode",
          "System, light or dark appearance.",
          "dark mode, light mode, appearance, night, system theme, brightness",
        ],
      ],
    ],
    [
      "Theme library",
      "themes, vs code, icons",
      [
        ["Colors", "Color themes.", "color theme, palette"],
        ["File icons", "File icon themes.", "icon theme, explorer icons"],
        [
          "Interface icons",
          "Interface icon themes.",
          "product icons, ui icons",
        ],
        ["Create theme", "Create your own theme.", "new theme, custom theme"],
        [
          "Import VS Code…",
          "Import a VS Code theme.",
          "vscode, visual studio code, extension",
        ],
        ["Import folder…", "Import themes from a folder."],
        ["Manage themes", "Create or import themes.", "ellipsis, menu"],
        ["Search themes", undefined, "filter themes"],
        ["Export to VS Code", undefined, "vsix, share theme"],
        ["Duplicate theme", undefined, "copy theme, clone"],
        ["Open theme folder", undefined, "reveal in finder, files"],
      ],
    ],
  ],
);

const keybinds = build(
  "keybinds",
  "shortcut, hotkey, keyboard, keys, bindings",
  [
    [
      "Actions",
      "shortcut, hotkey, keyboard",
      [
        [
          "Add keybinding",
          "Assign a shortcut to an action.",
          "new shortcut, hotkey, assign",
        ],
        ["Search keybindings", undefined, "find shortcut"],
        ["Reset to defaults", undefined, "restore shortcuts"],
      ],
    ],
    ...[...new Set(keybindingActions.map((action) => action.group))].map(
      (group): [string, string, Row[]] => [
        group,
        "shortcut, hotkey, keyboard",
        keybindingActions
          .filter((action) => action.group === group)
          .map((action): Row => [
            action.label,
            action.description,
            "shortcut, hotkey",
          ]),
      ],
    ),
    [
      "Interaction",
      "mouse, pointer, click",
      [
        [
          "Focus follows pointer",
          "Typing, paste and shortcuts go to the panel under the mouse.",
          "hover focus, mouse focus, panel focus, sloppy focus",
        ],
        [
          "Open with one click",
          "One click opens after a brief pause; a double click selects without opening.",
          "single click, double click, explorer, open files",
        ],
      ],
    ],
  ],
);

const notifications = build(
  "notifications",
  "alerts, notify, push, inbox, sound",
  [
    [
      "On this computer",
      "desktop, system notifications",
      [
        [
          "Notify when tasks finish",
          "Show system notifications when tasks finish.",
          "desktop notification, alert, completion, done",
        ],
        [
          "Only when the main Lomi window is inactive",
          undefined,
          "background, focus, window inactive",
        ],
      ],
    ],
    [
      "On my phones",
      "mobile, push",
      [
        [
          "Push notifications on my phones",
          "Send completion notifications from this computer to your phones.",
          "mobile, ios, android, push, remote",
        ],
      ],
    ],
  ],
);

const android = build(
  "android",
  "emulator, avd, phone, device, virtual device, sdk, adb, mobile",
  [
    [
      "Your phones",
      "emulator, avd, device, virtual device",
      [
        [
          "Create device",
          "Create a new Android phone.",
          "new phone, new emulator, avd, virtual device, add phone",
        ],
        ["Android version", undefined, "api level, system image, os version"],
        [
          "Phone profile",
          "Screen size and density.",
          "device model, pixel, resolution",
        ],
        [
          "Advanced settings",
          undefined,
          "graphics, gpu, cold boot, audio, camera",
        ],
        [
          "Enable Lomi text input",
          "Installs the Lomi keyboard in this phone.",
          "keyboard, ime, typing",
        ],
      ],
    ],
    [
      "Android versions",
      "system images, downloads",
      [
        [
          "Available to download",
          undefined,
          "install image, system image, api level",
        ],
        ["Remove image", undefined, "delete android version, uninstall"],
        ["Filter Android version", undefined, "api level"],
        ["Filter included apps", undefined, "google play, google apis"],
      ],
    ],
    [
      "Android tools",
      "sdk, adb, emulator, tools",
      [
        [
          "Hardware acceleration available",
          undefined,
          "hax, hvf, whpx, kvm, virtualization, gpu",
        ],
        [
          "Managed SDK",
          "Android SDK managed by Lomi.",
          "sdk, platform tools, adb, download, cmdline tools",
        ],
        ["Repair tools", undefined, "fix sdk, reinstall"],
        ["Roll back", undefined, "previous sdk, undo update"],
      ],
    ],
    ["Repair Android versions", "fix image", [["Repair image"]]],
    [
      "Storage and troubleshooting",
      "repair, cleanup, diagnostics, reset",
      [
        ["Repair interrupted operation", undefined, "fix, recover"],
        ["Clean temporary files", undefined, "cache, free space, disk"],
        ["Export diagnostics", undefined, "logs, debug, support"],
        ["Restore backup", undefined, "recover preferences"],
        ["Reset preferences", undefined, "defaults, clear"],
      ],
    ],
  ],
);

const plugins = build("plugins", "extensions, add-ons, marketplace, install", [
  [
    "Installed plugins",
    "extensions, enable, disable, uninstall",
    [
      [
        "Import plugin",
        "Install a plugin from a folder.",
        "install plugin, add extension, load, package",
      ],
      ["Search plugins", undefined, "find extension"],
      ["Plugin categories", undefined, "filter by type"],
      ["Plugin status", undefined, "enabled, disabled, filter"],
      ["Enable plugin", undefined, "trust plugin, turn on, toggle"],
      ["Uninstall plugin", undefined, "remove extension, delete"],
      ["Restart Lomi", undefined, "reload plugin"],
      ["Plugin ID", undefined, "details, source, trust, runtime"],
    ],
  ],
]);

const account = build(
  "account",
  "login, sign in, sign out, github, profile, session, user",
  [
    [
      "Your account",
      "profile, name, email",
      [
        ["Name", undefined, "profile"],
        ["GitHub", undefined, "username, identity"],
        ["Email", undefined, "address"],
        [
          "Storage",
          "You will need to sign in again after closing Lomi.",
          "session, credential store",
        ],
      ],
    ],
    [
      "Sign in with GitHub",
      "login, oauth, github",
      [
        [
          "Sign in with GitHub",
          "Connect your GitHub identity to Lomi.",
          "login, log in, oauth, authenticate",
        ],
        [
          "Remember me on this device",
          undefined,
          "stay signed in, keep session, credential store, keychain",
        ],
      ],
    ],
    [
      "Finish signing in",
      "browser approval",
      [
        ["Open browser again", undefined, "login, approve"],
        ["Cancel sign-in", undefined, "abort login"],
      ],
    ],
    [
      "Account access",
      "sessions, sign out, logout",
      [
        [
          "Manage account",
          "Manage sessions in your browser.",
          "portal, profile, subscription, billing",
        ],
        [
          "Sign out of this device",
          undefined,
          "log out, logout, disconnect, sign off",
        ],
        ["Retry connection", undefined, "reconnect, offline"],
      ],
    ],
  ],
);

const remote = build(
  "remote",
  "host, share, relay, collaborate, workspace sharing, online",
  [
    [
      "Connect to a shared workspace",
      "join, guest",
      [
        [
          "Choose an online host",
          "Request approval and open an existing workspace.",
          "join workspace, guest, connect host",
        ],
      ],
    ],
    [
      "Host account",
      "sign in, host login",
      [
        ["Sign in to the host", undefined, "login, publish, relay"],
        ["Sign-in status", undefined, "host login state"],
        ["Sign out of the host", undefined, "logout"],
        ["Open verification page", undefined, "device code, approve"],
      ],
    ],
    ["Workspace access", "share, stop sharing", []],
    ["Shared workspaces", "shared by the local host", []],
    [
      "Background sessions",
      "keep running, quit, tray",
      [
        ["Host sharing", undefined, "status"],
        [
          "Keep Lomi running",
          "Closing a Lomi window keeps host-owned terminals running.",
          "background, quit, close window, show workspace",
        ],
        ["Quit Lomi", undefined, "exit, close app"],
      ],
    ],
  ],
);

const agentControl = build(
  "agent-control",
  "mcp, agent, claude code, codex, gemini cli, cursor, automation, yolo, sessions",
  [
    [
      "Connect an agent",
      "mcp, claude code, codex, gemini cli, coding agent",
      [
        [
          "Choose your agent",
          undefined,
          "claude code, codex, gemini, cursor, opencode, cli, mcp client",
        ],
        ["Search agents", undefined, "find client"],
        ["Back to agents", undefined, "all clients"],
        [
          "Lomi connection",
          "Add Lomi to a coding agent.",
          "mcp, install, setup, configure",
        ],
        ["Connection setup", undefined, "add lomi, mcp config"],
        [
          "Set up all eligible clients",
          undefined,
          "add lomi to every client, install everywhere",
        ],
        ["Setup details", undefined, "configuration path, revision"],
      ],
      "connect",
    ],
    [
      "Local connection",
      "server, mcp",
      [
        [
          "Connection server",
          undefined,
          "mcp server, start server, turn off server, enable, port",
        ],
      ],
      "sessions",
    ],
    [
      "New session requests",
      "approve, reject, pending",
      [["Review requests", undefined, "approve session, pending"]],
      "sessions",
    ],
    [
      "Active sessions",
      "connected agents",
      [
        [
          "Stop all access",
          "Ends every session and rejects pending requests.",
          "disconnect agents, revoke, kill sessions",
        ],
      ],
      "sessions",
    ],
    ["Terminal input requests", "approve, prompt", [], "sessions"],
    ["Preference change requests", "approve", [], "sessions"],
    ["Project folder requests", "open project", [], "sessions"],
    ["Android setup and device requests", "emulator", [], "sessions"],
    ["APK installation requests", "install apk", [], "sessions"],
    [
      "Server preferences",
      "startup, autostart",
      [
        [
          "Start MCP server when Lomi opens",
          undefined,
          "autostart, launch, startup, on open, mcp server",
        ],
      ],
      "preferences",
    ],
    [
      "Automatic approvals (YOLO)",
      "yolo, auto approve",
      [
        [
          "Automatically approve requests (YOLO)",
          "Approves requests from all local clients without asking.",
          "yolo mode, auto approve, skip permissions, trust agents, unattended",
        ],
      ],
      "preferences",
    ],
    [
      "Manual configuration",
      "mcp config, json",
      [
        [
          "Copy configuration",
          "MCP configuration for this server.",
          "mcp json, config, clipboard, claude code, codex",
        ],
      ],
      "preferences",
    ],
  ],
);

const about = build(
  "about",
  "update, version, license, build, help, support, docs",
  [
    [
      "Application",
      "version, license",
      [
        ["Version", undefined, "update, release, build number"],
        [
          "Check for updates",
          "Look for a newer version of Lomi.",
          "update, upgrade, new version, release",
        ],
        ["License", undefined, "apache, open source, legal"],
      ],
    ],
    [
      "Resources",
      "links, help",
      [
        [
          "Source code",
          "lomi-dev/lomi",
          "github, repository, repo, view on github",
        ],
        ["Website", "lomi.dev", "open website, homepage, site"],
        ["Documentation", "docs.lomi.dev", "docs, manual, help, guide"],
        [
          "Feedback",
          "Report a bug or suggest an improvement.",
          "report an issue, bug, feature request, support",
        ],
      ],
    ],
    [
      "Build details",
      "platform, architecture",
      [
        ["Build", undefined, "platform, arch, version"],
        ["Bundle ID", undefined, "identifier"],
      ],
    ],
  ],
);

const terminal = build(
  "terminal",
  "shell, console, command line, tty, font, cursor, colors",
  [
    [
      "Font",
      "typeface, monospace",
      [
        [
          "Terminal font",
          "Used by every terminal.",
          "typeface, jetbrains mono, monospace, theme default, custom font",
        ],
        ["Font size", undefined, "text size, zoom, bigger, smaller"],
        [
          "Font family",
          "An installed font. Separate fallbacks with commas.",
          "typeface, custom font, fallback",
        ],
        ["Cursor style", undefined, "bar, block, underline, caret"],
        [
          "Always show terminal titles",
          "Otherwise hold Control to reveal titles.",
          "tab title, header, label",
        ],
      ],
    ],
    [
      "Text",
      "typography",
      [
        ["Font weight", undefined, "bold, thin, light, normal"],
        ["Bold font weight", undefined, "heavier text"],
        ["Line height", undefined, "spacing, leading"],
        ["Letter spacing", undefined, "tracking, kerning"],
      ],
    ],
    [
      "Cursor",
      "caret",
      [
        ["Inactive cursor", undefined, "unfocused cursor, outline"],
        ["Blink cursor", undefined, "blinking, flash, caret"],
        ["Cursor width", "Bar cursor only.", "caret thickness"],
      ],
    ],
    [
      "Scrolling",
      "history, buffer",
      [
        [
          "Scrollback lines",
          "Up to 100,000. Lowering it drops older output.",
          "history, buffer, scroll history, memory",
        ],
        ["Scroll speed", undefined, "sensitivity, mouse wheel"],
        ["Fast scroll speed (Alt)", undefined, "sensitivity, option"],
        [
          "Scroll animation time",
          "0 disables animation; up to 1,000 ms.",
          "smooth scrolling, duration",
        ],
        ["Scroll to bottom on input", undefined, "follow output, autoscroll"],
        [
          "Keep cleared screen in scrollback",
          undefined,
          "clear, erase display",
        ],
      ],
    ],
    [
      "Input",
      "keyboard, mouse, shell",
      [
        [
          "Default shell",
          "Applies to new terminals.",
          "powershell, cmd, windows shell",
        ],
        [
          "Tab width",
          "Spaces per tab stop in terminal output.",
          "tab stop, indentation",
        ],
        ["Alt-click moves cursor", undefined, "mouse, option click"],
        ["Right-click selects a word", undefined, "context menu, mouse"],
        [
          "Use Option as Meta (macOS)",
          undefined,
          "alt key, escape sequences, emacs",
        ],
        [
          "Option-click forces selection (macOS)",
          undefined,
          "mouse reporting, vim",
        ],
        [
          "Word separators",
          "Characters that end a word on double-click.",
          "double click, selection, delimiters",
        ],
      ],
    ],
    [
      "Rendering",
      "display, accessibility, gpu",
      [
        [
          "Minimum contrast ratio",
          "1 keeps exact colors; up to 21.",
          "accessibility, readability, wcag",
        ],
        ["Use bright colors for bold text", undefined, "bold, ansi"],
        [
          "Screen reader support",
          undefined,
          "accessibility, a11y, voiceover, nvda",
        ],
        [
          "Draw continuous box and block characters",
          "Accelerated renderer only.",
          "custom glyphs, box drawing, webgl",
        ],
        [
          "Rescale overlapping characters",
          "Accelerated renderer only.",
          "glyphs, webgl, ligatures",
        ],
      ],
    ],
  ],
);

const chatProviders = build(
  "chat-providers",
  "chat ai, models, llm, openai, anthropic, claude, codex, api key",
  [
    [
      "Providers",
      "chat ai, accounts, cli, api",
      [
        [
          "Add provider",
          "Add an API provider; installed CLIs are detected automatically.",
          "new provider, connect",
        ],
        [
          "API key",
          "Stored in the system key store.",
          "secret, token, credential",
        ],
        ["Base URL", undefined, "endpoint, host, openai compatible"],
        ["Models", undefined, "fetch models, add model"],
        ["Test connection", undefined, "probe, verify api"],
        ["Remove provider", undefined, "delete"],
      ],
    ],
  ],
);

const chatDefaults = build(
  "chat-defaults",
  "chat ai, defaults, new threads, send key, system prompt, tokens",
  [
    [
      "New threads",
      "model, permissions, workspace",
      [
        [
          "Model",
          "Default model for new threads.",
          "default model, provider, effort, reasoning",
        ],
        [
          "Fallback model",
          "Used when the default model's provider is unavailable.",
          "fallback, backup",
        ],
        [
          "Permissions",
          "What CLI agents may do in new threads.",
          "read only, edit files, full access, sandbox",
        ],
        [
          "Workspace",
          "Where new threads start.",
          "project folder, private folder, cwd",
        ],
      ],
    ],
    [
      "Composer",
      "send",
      [["Send message with", "Enter or Cmd+Enter.", "send key, shortcut"]],
    ],
    [
      "Responses",
      "instructions, tokens",
      [
        [
          "System instructions",
          "Sent to API providers and Claude Code.",
          "system prompt",
        ],
        ["Max output tokens", undefined, "max tokens, limit, length"],
      ],
    ],
  ],
);

export const settingsSearchIndex: SettingsSearchEntry[] = [
  ...general,
  ...themes,
  ...keybinds,
  ...notifications,
  ...terminal,
  ...android,
  ...plugins,
  ...chatProviders,
  ...chatDefaults,
  ...account,
  ...remote,
  ...agentControl,
  ...about,
];

const fold = (value: string) => value.toLowerCase();

function scoreEntry(
  entry: SettingsSearchEntry,
  terms: string[],
  query: string,
) {
  const label = fold(entry.label);
  const section = fold(entry.section ?? "");
  const pageLabel = fold(settingsPageLabels[entry.page]);
  const keywords = fold((entry.keywords ?? []).join(" "));
  const description = fold(entry.description ?? "");
  const haystack = `${label} ${section} ${pageLabel} ${keywords} ${description}`;
  if (!terms.every((term) => haystack.includes(term))) return 0;
  let score = 1;
  if (label === query) score = 100;
  else if (label.startsWith(query)) score = 90;
  else if (terms.every((term) => label.includes(term))) score = 80;
  else if (terms.every((term) => `${label} ${section}`.includes(term)))
    score = 60;
  else if (
    terms.every((term) => `${label} ${section} ${pageLabel}`.includes(term))
  )
    score = 50;
  else if (terms.every((term) => `${label} ${keywords}`.includes(term)))
    score = 40;
  else score = 20;
  // Pages and sections sort ahead of rows with the same match quality.
  if (!entry.section)
    score += entry.label === settingsPageLabels[entry.page] ? 5 : 3;
  return score;
}

export function searchSettings(
  rawQuery: string,
  limit = 30,
): SettingsSearchEntry[] {
  const query = fold(rawQuery.trim().replace(/\s+/g, " "));
  if (!query) return [];
  const terms = query.split(" ");
  return settingsSearchIndex
    .map((entry, order) => ({
      entry,
      order,
      score: scoreEntry(entry, terms, query),
    }))
    .filter((hit) => hit.score > 0)
    .sort((a, b) => b.score - a.score || a.order - b.order)
    .slice(0, limit)
    .map((hit) => hit.entry);
}

/** Splits text into plain and matched parts for highlighting. */
export function highlightParts(text: string, rawQuery: string) {
  const terms = [...new Set(fold(rawQuery).split(/\s+/).filter(Boolean))].sort(
    (a, b) => b.length - a.length,
  );
  if (!terms.length) return [{ text, match: false }];
  const pattern = new RegExp(
    `(${terms.map((term) => term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|")})`,
    "gi",
  );
  return text
    .split(pattern)
    .map((part, index) => ({ text: part, match: index % 2 === 1 }))
    .filter((part) => part.text);
}

const normalize = (text: string | null | undefined) =>
  (text ?? "").replace(/\s+/g, " ").replace(/…$/, "").trim().toLowerCase();
const visible = (element: Element) => element.getClientRects().length > 0;
const clickable = "button, label, summary, legend, dt, h3, [aria-label]";

/** Finds the element for an entry in a mounted page; `loose` allows substring matches. */
export function findRevealTarget(
  root: HTMLElement,
  entry: SettingsSearchEntry,
  loose: boolean,
): HTMLElement | null {
  const label = normalize(entry.label);
  const all = <T extends Element>(selector: string) =>
    Array.from(root.querySelectorAll<T>(selector)).filter(visible);
  if (!entry.section && label === normalize(settingsPageLabels[entry.page]))
    return root.querySelector<HTMLElement>("h1");
  const sectionHeading = (title?: string) =>
    all<HTMLElement>("h2").find(
      (heading) =>
        normalize(heading.firstChild?.textContent) === normalize(title),
    );
  const header = (heading?: HTMLElement) =>
    heading?.closest<HTMLElement>(".settings-section-heading") ??
    heading ??
    null;
  const row = all<HTMLElement>(".setting-row").find(
    (item) =>
      normalize(
        item.querySelector(".setting-label")?.firstElementChild?.textContent,
      ) === label,
  );
  if (row) return row;
  const heading = header(sectionHeading(entry.label));
  if (heading) return heading;
  const exact = all<HTMLElement>(clickable).find(
    (item) =>
      normalize(item.textContent) === label ||
      normalize(item.getAttribute("aria-label")) === label,
  );
  if (exact) return exact;
  if (!loose) return null;
  return (
    all<HTMLElement>(`h1, h2, ${clickable}`).find((item) =>
      normalize(item.textContent).includes(label),
    ) ??
    header(sectionHeading(entry.section)) ??
    // Unassigned keybinding actions have no row; point at the way to add one.
    (entry.page === "keybinds"
      ? all<HTMLElement>("button").find((item) =>
          normalize(item.textContent).includes("add keybinding"),
        )
      : undefined) ??
    root.querySelector<HTMLElement>("h1")
  );
}

export function showRevealTarget(target: HTMLElement) {
  if (!visible(target)) {
    // Some pages hide their title; the page entry then just returns to the top.
    target.closest(".settings-page")?.scrollTo({ top: 0, behavior: "smooth" });
    return;
  }
  const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
  target.scrollIntoView({
    behavior: reduced ? "auto" : "smooth",
    block: "center",
  });
  target.classList.remove("settings-search-hit");
  void target.offsetWidth; // restart the animation
  target.classList.add("settings-search-hit");
  window.setTimeout(() => target.classList.remove("settings-search-hit"), 1600);
  const control = target.matches("button, input, select, textarea")
    ? target
    : target.querySelector<HTMLElement>(
        "input:not(:disabled), select:not(:disabled), textarea:not(:disabled), button:not(:disabled)",
      );
  control?.focus({ preventScroll: true });
}
