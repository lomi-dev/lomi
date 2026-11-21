import { msg, t } from "./i18n";
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
  general: msg("General"),
  themes: msg("Themes"),
  keybinds: msg("Keybinds"),
  notifications: msg("Notifications"),
  terminal: msg("Terminal"),
  android: msg("Android"),
  plugins: msg("Plugins"),
  "chat-providers": msg("Providers"),
  "chat-defaults": msg("Defaults"),
  account: msg("Account"),
  remote: msg("Remote"),
  "agent-control": msg("Agent control"),
  about: msg("About"),
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
    msg("Workflow"),
    "mode, cli first, chat first, terminal, ai chat",
    [
      [
        msg("Start with"),
        msg("Terminals or chat up front."),
        "cli first, chat first, default tab",
      ],
    ],
  ],
  [
    msg("Projects"),
    "new project, clone, folder",
    [
      [
        msg("Default location"),
        msg("Where new and cloned projects go."),
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
      msg("Appearance"),
      "look",
      [
        [
          msg("Color mode"),
          msg("System, light or dark appearance."),
          "dark mode, light mode, appearance, night, system theme, brightness",
        ],
      ],
    ],
    [
      msg("Theme library"),
      "themes, vs code, icons",
      [
        [msg("Colors"), msg("Color themes."), "color theme, palette"],
        [
          msg("File icons"),
          msg("File icon themes."),
          "icon theme, explorer icons",
        ],
        [
          msg("Interface icons"),
          msg("Interface icon themes."),
          "product icons, ui icons",
        ],
        [
          msg("Create theme"),
          msg("Create your own theme."),
          "new theme, custom theme",
        ],
        [
          msg("Import VS Code…"),
          msg("Import a VS Code theme."),
          "vscode, visual studio code, extension",
        ],
        [msg("Import folder…"), msg("Import themes from a folder.")],
        [
          msg("Manage themes"),
          msg("Create or import themes."),
          "ellipsis, menu",
        ],
        [msg("Search themes"), undefined, "filter themes"],
        [msg("Export to VS Code"), undefined, "vsix, share theme"],
        [msg("Duplicate theme"), undefined, "copy theme, clone"],
        [msg("Open theme folder"), undefined, "reveal in finder, files"],
      ],
    ],
  ],
);

const keybinds = build(
  "keybinds",
  "shortcut, hotkey, keyboard, keys, bindings",
  [
    [
      msg("Actions"),
      "shortcut, hotkey, keyboard",
      [
        [
          msg("Add keybinding"),
          msg("Assign a shortcut to an action."),
          "new shortcut, hotkey, assign",
        ],
        [msg("Search keybindings"), undefined, "find shortcut"],
        [msg("Reset to defaults"), undefined, "restore shortcuts"],
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
      msg("Interaction"),
      "mouse, pointer, click",
      [
        [
          msg("Focus follows pointer"),
          msg("Typing, paste and shortcuts go to the panel under the mouse."),
          "hover focus, mouse focus, panel focus, sloppy focus",
        ],
        [
          msg("Open with one click"),
          msg(
            "One click opens after a brief pause; a double click selects without opening.",
          ),
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
      msg("On this computer"),
      "desktop, system notifications",
      [
        [
          msg("Notify when tasks finish"),
          msg("Show system notifications when tasks finish."),
          "desktop notification, alert, completion, done",
        ],
        [
          msg("Only when the main Lomi window is inactive"),
          undefined,
          "background, focus, window inactive",
        ],
      ],
    ],
    [
      msg("On my phones"),
      "mobile, push",
      [
        [
          msg("Push notifications on my phones"),
          msg(
            "Send completion notifications from this computer to your phones.",
          ),
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
      msg("Your phones"),
      "emulator, avd, device, virtual device",
      [
        [
          msg("Create device"),
          msg("Create a new Android phone."),
          "new phone, new emulator, avd, virtual device, add phone",
        ],
        [
          msg("Android version"),
          undefined,
          "api level, system image, os version",
        ],
        [
          msg("Phone profile"),
          msg("Screen size and density."),
          "device model, pixel, resolution",
        ],
        [
          msg("Advanced settings"),
          undefined,
          "graphics, gpu, cold boot, audio, camera",
        ],
        [
          msg("Enable Lomi text input"),
          msg("Installs the Lomi keyboard in this phone."),
          "keyboard, ime, typing",
        ],
      ],
    ],
    [
      msg("Android versions"),
      "system images, downloads",
      [
        [
          msg("Available to download"),
          undefined,
          "install image, system image, api level",
        ],
        [msg("Remove image"), undefined, "delete android version, uninstall"],
        [msg("Filter Android version"), undefined, "api level"],
        [msg("Filter included apps"), undefined, "google play, google apis"],
      ],
    ],
    [
      msg("Android tools"),
      "sdk, adb, emulator, tools",
      [
        [
          msg("Hardware acceleration available"),
          undefined,
          "hax, hvf, whpx, kvm, virtualization, gpu",
        ],
        [
          msg("Managed SDK"),
          msg("Android SDK managed by Lomi."),
          "sdk, platform tools, adb, download, cmdline tools",
        ],
        [msg("Repair tools"), undefined, "fix sdk, reinstall"],
        [msg("Roll back"), undefined, "previous sdk, undo update"],
      ],
    ],
    [msg("Repair Android versions"), "fix image", [[msg("Repair image")]]],
    [
      msg("Storage and troubleshooting"),
      "repair, cleanup, diagnostics, reset",
      [
        [msg("Repair interrupted operation"), undefined, "fix, recover"],
        [msg("Clean temporary files"), undefined, "cache, free space, disk"],
        [msg("Export diagnostics"), undefined, "logs, debug, support"],
        [msg("Restore backup"), undefined, "recover preferences"],
        [msg("Reset preferences"), undefined, "defaults, clear"],
      ],
    ],
  ],
);

const plugins = build("plugins", "extensions, add-ons, marketplace, install", [
  [
    msg("Installed plugins"),
    "extensions, enable, disable, uninstall",
    [
      [
        msg("Import plugin"),
        msg("Install a plugin from a folder."),
        "install plugin, add extension, load, package",
      ],
      [msg("Search plugins"), undefined, "find extension"],
      [msg("Plugin categories"), undefined, "filter by type"],
      [msg("Plugin status"), undefined, "enabled, disabled, filter"],
      [msg("Enable plugin"), undefined, "trust plugin, turn on, toggle"],
      [msg("Uninstall plugin"), undefined, "remove extension, delete"],
      [msg("Restart Lomi"), undefined, "reload plugin"],
      [msg("Plugin ID"), undefined, "details, source, trust, runtime"],
    ],
  ],
]);

const account = build(
  "account",
  "login, sign in, sign out, github, profile, session, user",
  [
    [
      msg("Your account"),
      "profile, name, email",
      [
        [msg("Name"), undefined, "profile"],
        [msg("GitHub"), undefined, "username, identity"],
        [msg("Email"), undefined, "address"],
        [
          msg("Storage"),
          msg("You will need to sign in again after closing Lomi."),
          "session, credential store",
        ],
      ],
    ],
    [
      msg("Sign in with GitHub"),
      "login, oauth, github",
      [
        [
          msg("Sign in with GitHub"),
          msg("Connect your GitHub identity to Lomi."),
          "login, log in, oauth, authenticate",
        ],
        [
          msg("Remember me on this device"),
          undefined,
          "stay signed in, keep session, credential store, keychain",
        ],
      ],
    ],
    [
      msg("Finish signing in"),
      "browser approval",
      [
        [msg("Open browser again"), undefined, "login, approve"],
        [msg("Cancel sign-in"), undefined, "abort login"],
      ],
    ],
    [
      msg("Account access"),
      "sessions, sign out, logout",
      [
        [
          msg("Manage account"),
          msg("Manage sessions in your browser."),
          "portal, profile, subscription, billing",
        ],
        [
          msg("Sign out of this device"),
          undefined,
          "log out, logout, disconnect, sign off",
        ],
        [msg("Retry connection"), undefined, "reconnect, offline"],
      ],
    ],
  ],
);

const remote = build(
  "remote",
  "host, share, relay, collaborate, workspace sharing, online",
  [
    [
      msg("Connect to a shared workspace"),
      "join, guest",
      [
        [
          msg("Choose an online host"),
          msg("Request approval and open an existing workspace."),
          "join workspace, guest, connect host",
        ],
      ],
    ],
    [
      msg("Host account"),
      "sign in, host login",
      [
        [msg("Sign in to the host"), undefined, "login, publish, relay"],
        [msg("Sign-in status"), undefined, "host login state"],
        [msg("Sign out of the host"), undefined, "logout"],
        [msg("Open verification page"), undefined, "device code, approve"],
      ],
    ],
    [msg("Workspace access"), "share, stop sharing", []],
    [msg("Shared workspaces"), "shared by the local host", []],
    [
      msg("Background sessions"),
      "keep running, quit, tray",
      [
        [msg("Host sharing"), undefined, "status"],
        [
          msg("Keep Lomi running"),
          msg("Closing a Lomi window keeps host-owned terminals running."),
          "background, quit, close window, show workspace",
        ],
        [msg("Quit Lomi"), undefined, "exit, close app"],
      ],
    ],
  ],
);

const agentControl = build(
  "agent-control",
  "mcp, agent, claude code, codex, gemini cli, cursor, automation, yolo, sessions",
  [
    [
      msg("Connect an agent"),
      "mcp, claude code, codex, gemini cli, coding agent",
      [
        [
          msg("Choose your agent"),
          undefined,
          "claude code, codex, gemini, cursor, opencode, cli, mcp client",
        ],
        [msg("Search agents"), undefined, "find client"],
        [msg("Back to agents"), undefined, "all clients"],
        [
          msg("Lomi connection"),
          msg("Add Lomi to a coding agent."),
          "mcp, install, setup, configure",
        ],
        [msg("Connection setup"), undefined, "add lomi, mcp config"],
        [
          msg("Set up all eligible clients"),
          undefined,
          "add lomi to every client, install everywhere",
        ],
        [msg("Setup details"), undefined, "configuration path, revision"],
      ],
      "connect",
    ],
    [
      msg("Local connection"),
      "server, mcp",
      [
        [
          msg("Connection server"),
          undefined,
          "mcp server, start server, turn off server, enable, port",
        ],
      ],
      "sessions",
    ],
    [
      msg("New session requests"),
      "approve, reject, pending",
      [[msg("Review requests"), undefined, "approve session, pending"]],
      "sessions",
    ],
    [
      msg("Active sessions"),
      "connected agents",
      [
        [
          msg("Stop all access"),
          msg("Ends every session and rejects pending requests."),
          "disconnect agents, revoke, kill sessions",
        ],
      ],
      "sessions",
    ],
    [msg("Terminal input requests"), "approve, prompt", [], "sessions"],
    [msg("Preference change requests"), "approve", [], "sessions"],
    [msg("Project folder requests"), "open project", [], "sessions"],
    [msg("Android setup and device requests"), "emulator", [], "sessions"],
    [msg("APK installation requests"), "install apk", [], "sessions"],
    [
      msg("Server preferences"),
      "startup, autostart",
      [
        [
          msg("Start MCP server when Lomi opens"),
          undefined,
          "autostart, launch, startup, on open, mcp server",
        ],
      ],
      "preferences",
    ],
    [
      msg("Automatic approvals (YOLO)"),
      "yolo, auto approve",
      [
        [
          msg("Automatically approve requests (YOLO)"),
          msg("Approves requests from all local clients without asking."),
          "yolo mode, auto approve, skip permissions, trust agents, unattended",
        ],
      ],
      "preferences",
    ],
    [
      msg("Manual configuration"),
      "mcp config, json",
      [
        [
          msg("Copy configuration"),
          msg("MCP configuration for this server."),
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
      msg("Application"),
      "version, license",
      [
        [msg("Version"), undefined, "update, release, build number"],
        [
          msg("Check for updates"),
          msg("Look for a newer version of Lomi."),
          "update, upgrade, new version, release",
        ],
        [msg("License"), undefined, "apache, open source, legal"],
      ],
    ],
    [
      msg("Resources"),
      "links, help",
      [
        [
          msg("Source code"),
          "lomi-dev/lomi",
          "github, repository, repo, view on github",
        ],
        [msg("Website"), "lomi.dev", "open website, homepage, site"],
        [msg("Documentation"), "docs.lomi.dev", "docs, manual, help, guide"],
        [
          msg("Feedback"),
          msg("Report a bug or suggest an improvement."),
          "report an issue, bug, feature request, support",
        ],
      ],
    ],
    [
      msg("Build details"),
      "platform, architecture",
      [
        [msg("Build"), undefined, "platform, arch, version"],
        [msg("Bundle ID"), undefined, "identifier"],
      ],
    ],
  ],
);

const terminal = build(
  "terminal",
  "shell, console, command line, tty, font, cursor, colors",
  [
    [
      msg("Font"),
      "typeface, monospace",
      [
        [
          msg("Terminal font"),
          msg("Used by every terminal."),
          "typeface, jetbrains mono, monospace, theme default, custom font",
        ],
        [msg("Font size"), undefined, "text size, zoom, bigger, smaller"],
        [
          msg("Font family"),
          msg("An installed font. Separate fallbacks with commas."),
          "typeface, custom font, fallback",
        ],
        [msg("Cursor style"), undefined, "bar, block, underline, caret"],
        [
          msg("Always show terminal titles"),
          msg("Otherwise hold Control to reveal titles."),
          "tab title, header, label",
        ],
      ],
    ],
    [
      msg("Text"),
      "typography",
      [
        [msg("Font weight"), undefined, "bold, thin, light, normal"],
        [msg("Bold font weight"), undefined, "heavier text"],
        [msg("Line height"), undefined, "spacing, leading"],
        [msg("Letter spacing"), undefined, "tracking, kerning"],
      ],
    ],
    [
      msg("Cursor"),
      "caret",
      [
        [msg("Inactive cursor"), undefined, "unfocused cursor, outline"],
        [msg("Blink cursor"), undefined, "blinking, flash, caret"],
        [msg("Cursor width"), msg("Bar cursor only."), "caret thickness"],
      ],
    ],
    [
      msg("Scrolling"),
      "history, buffer",
      [
        [
          msg("Scrollback lines"),
          msg("Up to 100,000. Lowering it drops older output."),
          "history, buffer, scroll history, memory",
        ],
        [msg("Scroll speed"), undefined, "sensitivity, mouse wheel"],
        [msg("Fast scroll speed (Alt)"), undefined, "sensitivity, option"],
        [
          msg("Scroll animation time"),
          msg("0 disables animation; up to 1,000 ms."),
          "smooth scrolling, duration",
        ],
        [
          msg("Scroll to bottom on input"),
          undefined,
          "follow output, autoscroll",
        ],
        [
          msg("Keep cleared screen in scrollback"),
          undefined,
          "clear, erase display",
        ],
      ],
    ],
    [
      msg("Input"),
      "keyboard, mouse, shell",
      [
        [
          msg("Default shell"),
          msg("Applies to new terminals."),
          "powershell, cmd, windows shell",
        ],
        [
          msg("Tab width"),
          msg("Spaces per tab stop in terminal output."),
          "tab stop, indentation",
        ],
        [msg("Alt-click moves cursor"), undefined, "mouse, option click"],
        [msg("Right-click selects a word"), undefined, "context menu, mouse"],
        [
          msg("Use Option as Meta (macOS)"),
          undefined,
          "alt key, escape sequences, emacs",
        ],
        [
          msg("Option-click forces selection (macOS)"),
          undefined,
          "mouse reporting, vim",
        ],
        [
          msg("Word separators"),
          msg("Characters that end a word on double-click."),
          "double click, selection, delimiters",
        ],
      ],
    ],
    [
      msg("Rendering"),
      "display, accessibility, gpu",
      [
        [
          msg("Minimum contrast ratio"),
          msg("1 keeps exact colors; up to 21."),
          "accessibility, readability, wcag",
        ],
        [msg("Use bright colors for bold text"), undefined, "bold, ansi"],
        [
          msg("Screen reader support"),
          undefined,
          "accessibility, a11y, voiceover, nvda",
        ],
        [
          msg("Draw continuous box and block characters"),
          msg("Accelerated renderer only."),
          "custom glyphs, box drawing, webgl",
        ],
        [
          msg("Rescale overlapping characters"),
          msg("Accelerated renderer only."),
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
      msg("Providers"),
      "chat ai, accounts, cli, api",
      [
        [
          msg("Add provider"),
          msg(
            "Add an API provider; installed CLIs are detected automatically.",
          ),
          "new provider, connect",
        ],
        [
          msg("API key"),
          msg("Stored in the system key store."),
          "secret, token, credential",
        ],
        [msg("Base URL"), undefined, "endpoint, host, openai compatible"],
        [msg("Models"), undefined, "fetch models, add model"],
        [msg("Test connection"), undefined, "probe, verify api"],
        [msg("Remove provider"), undefined, "delete"],
      ],
    ],
  ],
);

const chatDefaults = build(
  "chat-defaults",
  "chat ai, defaults, new threads, send key, system prompt, tokens",
  [
    [
      msg("New threads"),
      "model, permissions, workspace",
      [
        [
          msg("Model"),
          msg("Default model for new threads."),
          "default model, provider, effort, reasoning",
        ],
        [
          msg("Fallback model"),
          msg("Used when the default model's provider is unavailable."),
          "fallback, backup",
        ],
        [
          msg("Permissions"),
          msg("What CLI agents may do in new threads."),
          "read only, edit files, full access, sandbox",
        ],
        [
          msg("Workspace"),
          msg("Where new threads start."),
          "project folder, private folder, cwd",
        ],
      ],
    ],
    [
      msg("Composer"),
      "send",
      [
        [
          msg("Send message with"),
          msg("Enter or Cmd+Enter."),
          "send key, shortcut",
        ],
      ],
    ],
    [
      msg("Responses"),
      "instructions, tokens",
      [
        [
          msg("System instructions"),
          msg("Sent to API providers and Claude Code."),
          "system prompt",
        ],
        [msg("Max output tokens"), undefined, "max tokens, limit, length"],
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
  tr: (text: string) => string,
) {
  const label = fold(tr(entry.label));
  const section = fold(entry.section ? tr(entry.section) : "");
  const pageLabel = fold(tr(settingsPageLabels[entry.page]));
  const keywords = fold((entry.keywords ?? []).join(" "));
  const description = fold(entry.description ? tr(entry.description) : "");
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
      // Matches the translated text as well as the English source text.
      score: Math.max(
        scoreEntry(entry, terms, query, (text) => text),
        scoreEntry(entry, terms, query, t),
      ),
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
  const names = (text?: string) =>
    text ? [normalize(t(text)), normalize(text)] : [];
  const label = names(entry.label);
  const all = <T extends Element>(selector: string) =>
    Array.from(root.querySelectorAll<T>(selector)).filter(visible);
  if (
    !entry.section &&
    label.includes(normalize(settingsPageLabels[entry.page]))
  )
    return root.querySelector<HTMLElement>("h1");
  const sectionHeading = (title?: string) =>
    all<HTMLElement>("h2").find((heading) =>
      names(title).includes(normalize(heading.firstChild?.textContent)),
    );
  const header = (heading?: HTMLElement) =>
    heading?.closest<HTMLElement>(".settings-section-heading") ??
    heading ??
    null;
  const row = all<HTMLElement>(".setting-row").find((item) =>
    label.includes(
      normalize(
        item.querySelector(".setting-label")?.firstElementChild?.textContent,
      ),
    ),
  );
  if (row) return row;
  const heading = header(sectionHeading(entry.label));
  if (heading) return heading;
  const exact = all<HTMLElement>(clickable).find(
    (item) =>
      label.includes(normalize(item.textContent)) ||
      label.includes(normalize(item.getAttribute("aria-label"))),
  );
  if (exact) return exact;
  if (!loose) return null;
  return (
    all<HTMLElement>(`h1, h2, ${clickable}`).find((item) =>
      label.some((name) => normalize(item.textContent).includes(name)),
    ) ??
    header(sectionHeading(entry.section)) ??
    // Unassigned keybinding actions have no row; point at the way to add one.
    (entry.page === "keybinds"
      ? all<HTMLElement>("button").find((item) =>
          names("Add keybinding").some((name) =>
            normalize(item.textContent).includes(name),
          ),
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
