import { prepareIcons, type PreparedIcons } from "./icon-theme";
import { terminalPresets } from "./palette";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { ITerminalOptions, ITheme } from "@xterm/xterm";
import { native } from "../api";
import {
  builtinTheme,
  kebab,
  parseThemeText,
  resolveTheme,
  resolveAppearance,
  terminalColors,
} from "./format";
import type {
  AppearancePreference,
  ThemeBundle,
  ThemeManifest,
  ResolvedTheme,
  SyntaxName,
  SyntaxStyle,
  ThemePreferences,
} from "./format";

import { defaultTerminalPreferences } from "../terminal-preferences";
import type { TerminalPreferences } from "../terminal-preferences";

const defaultSyntax: Record<SyntaxName, SyntaxStyle> = {
  keyword: { color: "var(--color-accent-text)", fontWeight: 600 },
  string: { color: "var(--color-info)" },
  number: { color: "var(--color-warning)" },
  comment: { color: "var(--color-surface-variant-text)", fontStyle: "italic" },
  type: { color: "var(--color-muted-text)" },
  function: { color: "var(--color-background-text)" },
  variable: {},
  property: {},
  operator: {},
  punctuation: {},
  heading: { fontWeight: "bold" },
  strong: { fontWeight: "bold" },
  emphasis: { fontStyle: "italic" },
  link: { textDecoration: "underline" },
  invalid: { color: "var(--color-error)" },
};
let snapshot = {
  fileIcons: null as PreparedIcons | null,
  productIcons: null as PreparedIcons | null,
  highContrast: false,
  sourceRevision: 0,
  revision: 0,
  appearance:
    document.documentElement.dataset.appearance === "light"
      ? ("light" as const)
      : ("dark" as const),
  syntax: defaultSyntax,
  terminal: {} as ITerminalOptions,
  search: {} as Record<string, string>,
  tokens: {} as Record<string, string>,
};
const listeners = new Set<() => void>();
export const effectiveTheme = () => snapshot;
export const subscribeTheme = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};
let activeResolved: ResolvedTheme = {};
let activeIcons: { file: PreparedIcons | null; product: PreparedIcons | null } =
  { file: null, product: null };
let highContrast = false;
let sourceRevision = 0;
function publish() {
  const root = getComputedStyle(document.documentElement);
  const tokens: Record<string, string> = {};
  for (let i = 0; i < root.length; i++) {
    const key = root.item(i);
    if (key.startsWith("--")) tokens[key] = root.getPropertyValue(key).trim();
  }
  const syntax = Object.fromEntries(
    Object.entries(defaultSyntax).map(([key, value]) => [key, { ...value }]),
  ) as Record<SyntaxName, SyntaxStyle>;
  for (const [name, style] of Object.entries(
    activeResolved.editor?.syntax ?? {},
  ))
    syntax[name as SyntaxName] = { ...syntax[name as SyntaxName], ...style };
  for (const [name, style] of Object.entries(syntax))
    for (const property of [
      "color",
      "fontStyle",
      "fontWeight",
      "textDecoration",
    ] as const) {
      const value = root
        .getPropertyValue(`--syntax-${name}-${kebab(property)}`)
        .trim();
      if (value) (style as Record<string, string>)[property] = value;
    }
  snapshot = {
    fileIcons: activeIcons.file,
    productIcons: activeIcons.product,
    highContrast,
    sourceRevision,
    revision: snapshot.revision + 1,
    appearance:
      document.documentElement.dataset.appearance === "light"
        ? "light"
        : "dark",
    syntax,
    terminal: terminalOptions!,
    search: { ...searchPalette },
    tokens,
  };
  document.documentElement.dataset.themeSourceRevision = String(
    snapshot.sourceRevision,
  );
  document.documentElement.dataset.hideExplorerArrows = String(
    !!activeIcons.file?.data.hidesExplorerArrows,
  );
  for (const listener of listeners) listener();
  window.dispatchEvent(new Event(themeAppliedEvent));
}
let terminalPreferences = defaultTerminalPreferences;
export function applyTerminalPreferences(next: TerminalPreferences) {
  const root = document.documentElement.style;
  for (const [key, value] of Object.entries(terminalPreferences.appearance)) {
    if (key === "colors") {
      for (const name of Object.keys(value))
        root.removeProperty(`--terminal-${kebab(name)}`);
    } else root.removeProperty(`--terminal-${kebab(key)}`);
  }
  terminalPreferences = next;
  for (const [key, value] of Object.entries(next.appearance)) {
    if (key === "colors") {
      for (const [name, color] of Object.entries(next.appearance.colors ?? {}))
        root.setProperty(`--terminal-${kebab(name)}`, color, "important");
    } else
      root.setProperty(
        `--terminal-${kebab(key)}`,
        `${typeof value === "boolean" ? Number(value) : value}${key === "fontSize" || key === "letterSpacing" ? "px" : ""}`,
        "important",
      );
  }
  refreshTerminalAppearance();
}

export function terminalPalette() {
  terminalAppearance();
  return { ...searchPalette };
}

export const themeAppliedEvent = "lomi-theme-applied";
let revision = 0;
let appliedRevision = 0;
let terminalOptions: ITerminalOptions | undefined;
let searchPalette: Record<string, string> = {};

export function initializeAppearance(
  preference: AppearancePreference = "system",
) {
  document.documentElement.dataset.appearance = resolveAppearance(
    preference,
    window.matchMedia("(prefers-color-scheme: dark)").matches
      ? "dark"
      : "light",
  );
}

export function applyAppearance(preference: AppearancePreference) {
  terminalAppearance();
  initializeAppearance(preference);
  refreshTerminalAppearance();
}

function nextStyleFrame() {
  return new Promise<void>((resolve) => {
    const finish = () => {
      cancelAnimationFrame(frame);
      clearTimeout(timer);
      resolve();
    };
    const frame = requestAnimationFrame(finish);
    // Hidden WebKit views suspend frames; their theme watchers still need to start.
    const timer = setTimeout(finish, 50);
  });
}

function refreshTerminalAppearance() {
  const request = ++appliedRevision;
  // WebKit applies media changes after this task. Canvas fonts need an explicit load.
  void nextStyleFrame().then(() => {
    const next = readTerminalAppearance();
    void loadTerminalFonts(next).then(() => {
      if (request !== appliedRevision) return;
      terminalOptions = next;
      publish();
    });
  });
}

export async function loadTerminalFonts(options: ITerminalOptions) {
  await Promise.allSettled([
    ...["normal", "italic"].flatMap((style) =>
      [options.fontWeight, options.fontWeightBold].flatMap((weight) =>
        [options.fontFamily, '"JetBrains Mono"'].map((family) =>
          document.fonts.load(
            `${style} ${weight} ${options.fontSize}px ${family}`,
            // Canvas glyphs need their fallback fonts loaded before xterm caches them.
            "M⠋\ue0b0\uf013",
          ),
        ),
      ),
    ),
    // WebKit's FontFaceSet.load may only load the first family in a list.
    document.fonts.load('16px "Noto Sans Symbols"', "⚙"),
    document.fonts.load('16px "Noto Sans Symbols 2"', "⠋"),
    document.fonts.load('16px "Symbols Nerd Font Mono"', "\uf013"),
  ]);
}

export function themeAssetUrl(id: string, path: string, revision: string) {
  const base = native ? convertFileSrc("", "theme") : "/theme-assets/";
  return `${base}${encodeURIComponent(id)}/${revision}/${path.split("/").map(encodeURIComponent).join("/")}`;
}

function setDeclaration(
  style: CSSStyleDeclaration,
  property: string,
  value: string,
) {
  const important = /\s*!important\s*$/.test(value);
  const plain = value.replace(/\s*!important\s*$/, "");
  if (!property.startsWith("--") && !CSS.supports(property, plain))
    throw new Error(`Invalid CSS value for ${property}: ${value}`);
  style.setProperty(property, plain, important ? "important" : "");
  if (!style.getPropertyValue(property))
    throw new Error(`Invalid CSS value for ${property}: ${value}`);
}

export function compileTheme(
  theme: ResolvedTheme,
  asset: (path: string) => string,
) {
  const sheet = new CSSStyleSheet();
  const rule = (selector: string, declarations: Record<string, string>) => {
    // CSSOM validates selectors and serializes values without interpolating executable stylesheet syntax.
    let rule: CSSRule;
    try {
      const index = sheet.insertRule(`${selector} {}`, sheet.cssRules.length);
      rule = sheet.cssRules[index];
    } catch {
      throw new Error(`Invalid CSS selector: ${selector}`);
    }
    if (!(rule instanceof CSSStyleRule))
      throw new Error(`Invalid CSS selector: ${selector}`);
    const style = rule.style;
    for (const [property, value] of Object.entries(declarations))
      setDeclaration(style, property, value);
  };
  const tokens: Record<string, string> = { ...theme.tokens };
  for (const [name, path] of Object.entries(theme.assets ?? {}))
    tokens[`--asset-${name}`] = `url("${asset(path)}")`;
  for (const [key, value] of Object.entries(theme.terminal ?? {})) {
    if (key === "colors" || key === "preset") continue;
    tokens[`--terminal-${kebab(key)}`] =
      `${typeof value === "boolean" ? Number(value) : value}${key === "fontSize" || key === "letterSpacing" ? "px" : ""}`;
  }
  for (const [name, color] of Object.entries(theme.terminal?.colors ?? {})) {
    if (!CSS.supports("color", color))
      throw new Error(`Invalid terminal color: ${name}`);
    tokens[`--terminal-${kebab(name)}`] = color;
  }
  for (const [area, background] of Object.entries(theme.backgrounds ?? {})) {
    const prefix = `--background-${area}-`;
    if (background.image)
      tokens[`${prefix}image`] = `url("${asset(background.image)}")`;
    if (background.opacity !== undefined)
      tokens[`${prefix}opacity`] = String(background.opacity);
    if (background.blur !== undefined)
      tokens[`${prefix}blur`] = `${background.blur}px`;
    for (const [key, property] of Object.entries({
      size: "background-size",
      position: "background-position",
      repeat: "background-repeat",
      overlay: "background-color",
      blendMode: "mix-blend-mode",
    })) {
      const value = background[key as keyof typeof background];
      if (typeof value === "string") {
        if (!CSS.supports(property, value))
          throw new Error(`Invalid backgrounds.${area}.${key}: ${value}`);
        tokens[`${prefix}${kebab(key)}`] = value;
      }
    }
  }
  for (const [owner, values] of Object.entries(theme.plugins ?? {}))
    for (const [name, value] of Object.entries(values))
      tokens[
        `--plugin-${owner.replaceAll("-", "-h").replaceAll(".", "-d")}-${name}`
      ] = value;
  for (const [name, value] of Object.entries(theme.editor?.colors ?? {}))
    tokens[`--editor-${kebab(name)}`] = value;
  if (theme.editor?.fontFamily)
    tokens["--editor-font-family"] = theme.editor.fontFamily;
  if (theme.editor?.fontSize)
    tokens["--editor-font-size"] = `${theme.editor.fontSize}px`;
  for (const [name, style] of Object.entries({
    ...defaultSyntax,
    ...theme.editor?.syntax,
  }))
    for (const [property, value] of Object.entries(style))
      tokens[`--syntax-${name}-${kebab(property)}`] = String(value);
  rule(":root", tokens);
  const layout = theme.layout;
  if (layout?.tabs && layout.tabs !== "inline") {
    const titlebar = ".app-shell:not(.settings-window) > .titlebar";
    rule(`${titlebar}:has(.tab-bar)`, {
      height: "auto",
      "min-height": "var(--titlebar-height)",
      "flex-wrap": "wrap",
    });
    rule(`${titlebar} > .tab-bar`, {
      order: layout.tabs === "above" ? "-1" : "1",
      flex: "1 0 100%",
      height: "calc(var(--tab-height) + var(--space-8))",
    });
    rule(`${titlebar} > .titlebar-space`, { flex: "1" });
    rule(`${titlebar} > .window-controls`, {
      height: "var(--titlebar-height)",
    });
  }
  if (layout?.statusbar === "top") {
    rule(".app-shell > .titlebar", { order: "-3" });
    rule(".app-shell > .notice", { order: "-2" });
    rule(".app-shell > .statusbar", {
      order: "-1",
      "border-top-width": "0",
      "border-bottom":
        "var(--statusbar-border-width) solid var(--color-outline)",
    });
  }
  const navigation = layout?.settingsNavigation;
  if (navigation && navigation !== "left") {
    rule(".settings-layout", {
      "flex-direction":
        navigation === "right"
          ? "row-reverse"
          : navigation === "top"
            ? "column"
            : "column-reverse",
    });
    rule(".settings-navigation", {
      "border-right-width": "0",
      [navigation === "right"
        ? "border-left"
        : navigation === "top"
          ? "border-bottom"
          : "border-top"]: "var(--border-width) solid var(--color-outline)",
      ...(navigation !== "right"
        ? {
            display: "flex",
            "flex-wrap": "wrap",
            flex: "0 0 auto",
            gap: "var(--space-4)",
          }
        : {}),
    });
    if (navigation !== "right")
      rule(".settings-navigation > .settings-nav-item", {
        width: "auto",
        margin: "0",
      });
  }
  for (const [selector, declarations] of Object.entries(theme.styles ?? {}))
    rule(selector, declarations);
  return Array.from(sheet.cssRules, (rule) => rule.cssText).join("\n");
}

export interface PreparedTheme {
  manifest: ThemeManifest;
  appearance: AppearancePreference;
  commit: (onVisible?: () => void) => Promise<void>;
  dispose: () => void;
}
export async function prepareTheme(
  bundle: ThemeBundle | null,
  preferences: ThemePreferences,
  signal?: AbortSignal,
  nextSourceRevision = sourceRevision,
  iconBundles?: { file: ThemeBundle | null; product: ThemeBundle | null },
): Promise<PreparedTheme> {
  const manifest = bundle ? parseThemeText(bundle.raw) : builtinTheme;
  if (manifest.iconTheme)
    throw new Error("Select icon themes in their own category.");
  const appearance =
    manifest.appearance && manifest.appearance !== "adaptive"
      ? manifest.appearance
      : preferences.appearance;
  const mode = resolveAppearance(
    appearance,
    window.matchMedia("(prefers-color-scheme: dark)").matches
      ? "dark"
      : "light",
  );
  const resolved = resolveTheme(manifest, mode);
  if (resolved.terminal?.preset) {
    const palettes = await terminalPresets();
    const palette = palettes[resolved.terminal.preset];
    if (!palette)
      throw new Error(`Unknown terminal preset: ${resolved.terminal.preset}`);
    resolved.terminal.colors = { ...palette, ...resolved.terminal.colors };
  }
  const currentRevision = `${Date.now()}-${++revision}`;
  const asset = (path: string) =>
    themeAssetUrl(bundle!.id, path, currentRevision);
  const style = document.createElement("style");
  style.dataset.themeLayer = "tokens";
  style.textContent = compileTheme(resolved, asset);
  const links: HTMLLinkElement[] = [];
  const cancelLoads = new Set<() => void>();
  const stagedIcons: {
    file: PreparedIcons | null;
    product: PreparedIcons | null;
  } = { file: null, product: null };
  let iconsCommitted = false;
  const dispose = () => {
    signal?.removeEventListener("abort", dispose);
    for (const cancel of cancelLoads) cancel();
    if (!iconsCommitted) {
      stagedIcons.file?.dispose();
      stagedIcons.product?.dispose();
    }
    style.remove();
    for (const link of links) link.remove();
  };
  signal?.addEventListener("abort", dispose, { once: true });
  if (signal?.aborted) throw new Error("Theme loading was canceled.");
  try {
    if (iconBundles)
      for (const kind of ["file", "product"] as const) {
        const iconBundle = iconBundles[kind];
        if (!iconBundle) continue;
        if (
          parseThemeText(iconBundle.raw).iconTheme?.kind !== kind ||
          !iconBundle.iconTheme
        )
          throw new Error(`Expected a ${kind} icon theme.`);
        const prepared = await prepareIcons(
          iconBundle.iconTheme,
          (path) => themeAssetUrl(iconBundle.id, path, currentRevision),
          `${currentRevision} ${kind}`,
          signal,
        );
        if (signal?.aborted) {
          prepared.dispose();
          throw new Error("Icon loading was canceled.");
        }
        stagedIcons[kind] = prepared;
      }
    const paths = resolved.stylesheets ?? [];
    await Promise.all(
      paths.map(
        (path) =>
          new Promise<void>((resolve, reject) => {
            const link = document.createElement("link");
            link.rel = "stylesheet";
            link.media = "not all";
            link.href = asset(path);
            links.push(link);
            const finish = (error?: Error) => {
              clearTimeout(timeout);
              link.onload = link.onerror = null;
              cancelLoads.delete(cancel);
              if (error) reject(error);
              else resolve();
            };
            const cancel = () =>
              finish(new Error("Theme loading was cancelled."));
            const timeout = setTimeout(
              () => finish(new Error(`Loading ${path} timed out.`)),
              10000,
            );
            cancelLoads.add(cancel);
            link.onload = () => finish();
            link.onerror = () =>
              finish(
                new Error(
                  `Cannot load ${path}. The previous theme is still active.`,
                ),
              );
            // Append in manifest order; completion order must not change the cascade.
            document.head.append(link);
          }),
      ),
    );
    const images = new Set(
      Object.values(resolved.backgrounds ?? {}).flatMap((background) =>
        background.image ? [background.image] : [],
      ),
    );
    const fonts: string[] = [];
    for (const path of Object.values(resolved.assets ?? {})) {
      if (/\.(png|jpe?g|gif|webp|svg|avif)$/i.test(path)) images.add(path);
      else if (/\.(woff2?|ttf|otf)$/i.test(path)) fonts.push(path);
    }
    await Promise.all(
      [...images].map(
        (path) =>
          new Promise<void>((resolve, reject) => {
            const image = new Image();
            let finished = false;
            const finish = (error?: Error) => {
              if (finished) return;
              finished = true;
              clearTimeout(timer);
              cancelLoads.delete(cancel);
              image.onload = image.onerror = null;
              if (error) {
                image.src = "";
                reject(error);
              } else resolve();
            };
            const cancel = () =>
              finish(new Error("Theme loading was canceled."));
            const timer = setTimeout(
              () =>
                finish(new Error(`Cannot load background or image: ${path}`)),
              10000,
            );
            cancelLoads.add(cancel);
            image.onload = () => finish();
            image.onerror = () =>
              finish(new Error(`Cannot decode background or image: ${path}`));
            image.src = asset(path);
          }),
      ),
    );
    await Promise.all(
      fonts.map(
        (path) =>
          new Promise<void>((resolve, reject) => {
            let finished = false;
            const face = new FontFace(
              "Lomi theme preflight",
              `url("${asset(path)}")`,
            );
            const finish = (error?: unknown) => {
              if (finished) return;
              finished = true;
              clearTimeout(timer);
              cancelLoads.delete(cancel);
              if (error)
                reject(
                  new Error(
                    `Cannot load required font ${path}: ${String(error)}`,
                  ),
                );
              else resolve();
            };
            const cancel = () => finish("loading canceled");
            const timer = setTimeout(() => finish("loading timed out"), 10000);
            cancelLoads.add(cancel);
            void face.load().then(() => finish(), finish);
          }),
      ),
    );
  } catch (error) {
    dispose();
    throw error;
  }
  return {
    manifest,
    appearance,
    dispose,
    commit: async (onVisible) => {
      if (signal?.aborted) throw new Error("Theme loading was canceled.");
      signal?.removeEventListener("abort", dispose);
      terminalAppearance();
      document
        .querySelectorAll("[data-theme-layer]:not([data-theme-held])")
        .forEach((node) => node.remove());
      document
        .querySelectorAll<HTMLStyleElement | HTMLLinkElement>(
          "[data-theme-held]",
        )
        .forEach((node) => (node.media = "not all"));
      if (links.length) {
        // Moving a loaded link restarts stylesheet loading in WebKitGTK.
        document.head.insertBefore(style, links[0]);
        for (const link of links) {
          link.dataset.themeLayer = "css";
          // WebKit may defer @import until a staged sheet's media becomes active.
          // Re-read the final cascade when those imports finish, including in hidden views.
          link.onload = () => {
            if (link.isConnected && link.media === "all")
              refreshTerminalAppearance();
          };
          link.media = "all";
        }
      } else document.head.append(style);
      document.documentElement.dataset.theme = bundle?.id ?? "deepmono";
      activeResolved = resolved;
      highContrast =
        manifest.vscode?.type === "hcDark" ||
        manifest.vscode?.type === "hcLight";
      const previousIcons = iconBundles ? activeIcons : null;
      if (iconBundles) {
        activeIcons = stagedIcons;
        iconsCommitted = true;
      }
      sourceRevision = nextSourceRevision;
      initializeAppearance(appearance);
      terminalOptions = readTerminalAppearance();
      onVisible?.();
      const request = ++appliedRevision;
      await nextStyleFrame();
      const options = readTerminalAppearance();
      await loadTerminalFonts(options);
      if (request === appliedRevision) {
        terminalOptions = options;
        publish();
      }
      // Retain old faces until React has replaced glyphs using the old families.
      if (previousIcons) {
        await nextStyleFrame();
        previousIcons.file?.dispose();
        previousIcons.product?.dispose();
      }
    },
  };
}

export function terminalAppearance(): ITerminalOptions {
  return (terminalOptions ??= readTerminalAppearance());
}

function readTerminalAppearance(): ITerminalOptions {
  const root = getComputedStyle(document.documentElement);
  const token = (name: string) =>
    root.getPropertyValue(`--terminal-${kebab(name)}`).trim();
  const probe = document.createElement("span");
  probe.style.position = "fixed";
  probe.style.visibility = "hidden";
  document.body.append(probe);
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = 1;
  const context = canvas.getContext("2d", { willReadFrequently: true })!;
  const colors: Record<string, string> = {};
  for (const name of terminalColors) {
    if (!token(name) || token(name) === "none") continue;
    probe.style.color = `var(--terminal-${kebab(name)})`;
    context.clearRect(0, 0, 1, 1);
    context.fillStyle = getComputedStyle(probe).color;
    context.fillRect(0, 0, 1, 1);
    colors[name] =
      `#${Array.from(context.getImageData(0, 0, 1, 1).data, (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
  }
  probe.remove();
  searchPalette = colors;
  const numeric = (
    name: string,
    fallback: number,
    min: number,
    max: number,
  ) => {
    const value = parseFloat(token(name));
    return Number.isFinite(value)
      ? Math.min(max, Math.max(min, value))
      : fallback;
  };
  const weight = (name: string, fallback: "normal" | "bold") => {
    const value = token(name);
    return value === "normal" || value === "bold"
      ? value
      : numeric(name, fallback === "bold" ? 700 : 400, 1, 1000);
  };
  // Keep named fonts first, but insert bundled fallbacks before generic families
  // can select system fonts. Quoted family names may contain commas.
  const families = (
    token("fontFamily").match(
      /(?:[^,"']|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*')+/g,
    ) ?? []
  )
    .map((family) => family.trim())
    .filter(Boolean);
  const generic = families.findIndex((family) =>
    /^(serif|sans-serif|monospace|cursive|fantasy|system-ui|ui-serif|ui-sans-serif|ui-monospace|ui-rounded|emoji|math|fangsong)$/i.test(
      family,
    ),
  );
  families.splice(
    generic < 0 ? families.length : generic,
    0,
    '"JetBrains Mono"',
    '"Noto Sans Symbols"',
    '"Noto Sans Symbols 2"',
    '"Symbols Nerd Font Mono"',
  );
  return {
    ...terminalPreferences.behavior,
    fontFamily: families.join(", "),
    fontSize: numeric("fontSize", 16, 6, 72),
    fontWeight: weight("fontWeight", "normal"),
    fontWeightBold: weight("fontWeightBold", "bold"),
    lineHeight: numeric("lineHeight", 1.25, 1, 3),
    letterSpacing: numeric("letterSpacing", 0, -2, 20),
    cursorBlink: token("cursorBlink") !== "0",
    cursorWidth: numeric("cursorWidth", 1, 1, 10),
    cursorStyle: (["bar", "block", "underline"].includes(token("cursorStyle"))
      ? token("cursorStyle")
      : "bar") as ITerminalOptions["cursorStyle"],
    cursorInactiveStyle: ([
      "outline",
      "bar",
      "block",
      "underline",
      "none",
    ].includes(token("cursorInactiveStyle"))
      ? token("cursorInactiveStyle")
      : "outline") as ITerminalOptions["cursorInactiveStyle"],
    minimumContrastRatio: numeric("minimumContrastRatio", 1, 1, 21),
    drawBoldTextInBrightColors: token("drawBoldTextInBrightColors") !== "0",
    // Preserve the pane's RGB for xterm's contrast calculations and background
    // queries; zero alpha lets CSS paint the background and images only once.
    theme: {
      ...Object.fromEntries(
        Object.entries(colors).filter(([key]) => !key.startsWith("search")),
      ),
      background: `${colors.background.slice(0, 7)}00`,
    } as ITheme,
  };
}

export function terminalSearchColors() {
  terminalAppearance();
  const color = (name: string) => searchPalette[name];
  return {
    matchBackground: color("searchMatchBackground"),
    activeMatchBackground: color("searchActiveMatchBackground"),
    matchBorder: color("searchMatchBorder"),
    activeMatchBorder: color("searchActiveMatchBorder"),
    matchOverviewRuler: color("searchMatchBorder"),
    activeMatchColorOverviewRuler: color("searchActiveMatchBorder"),
  };
}

export function holdThemePreview() {
  const nodes = [
    ...document.querySelectorAll<HTMLStyleElement | HTMLLinkElement>(
      "[data-theme-layer]",
    ),
  ];
  const previous = {
    highContrast,
    sourceRevision,
    resolved: activeResolved,
    appearance: document.documentElement.dataset.appearance!,
    id: document.documentElement.dataset.theme,
  };
  for (const node of nodes) node.dataset.themeHeld = "true";
  let restored = false;
  return async () => {
    if (restored) return;
    restored = true;
    document
      .querySelectorAll("[data-theme-layer]:not([data-theme-held])")
      .forEach((node) => node.remove());
    for (const node of nodes) {
      delete node.dataset.themeHeld;
      node.media = "all";
    }
    activeResolved = previous.resolved;
    highContrast = previous.highContrast;
    sourceRevision = previous.sourceRevision;
    document.documentElement.dataset.appearance = previous.appearance;
    if (previous.id) document.documentElement.dataset.theme = previous.id;
    else delete document.documentElement.dataset.theme;
    const request = ++appliedRevision;
    await nextStyleFrame();
    const options = readTerminalAppearance();
    await loadTerminalFonts(options);
    if (request === appliedRevision) {
      terminalOptions = options;
      publish();
    }
  };
}
