import baseline from "./baseline.css?inline";
import applicationStyles from "../styles.css?inline";
import {
  kebab,
  resolveTheme,
  type Appearance,
  type ThemeBundle,
  type ThemeManifest,
} from "./format";
import { compileTheme, themeAssetUrl } from "./runtime";
import { terminalPresets } from "./palette";
import {
  componentColors,
  syntaxScopes,
  terminalColorIds,
  workbenchColors,
  type TokenRule,
  type VSCodeTheme,
} from "./vscode";

interface Sample {
  colors: Record<string, string>;
  syntax: Record<string, TokenRule["settings"]>;
}
async function sample(
  manifest: ThemeManifest,
  appearance: Appearance,
  bundle: ThemeBundle | null,
): Promise<Sample> {
  const values = resolveTheme(manifest, appearance);
  if (values.terminal?.preset) {
    const palettes = await terminalPresets();
    const palette = palettes[values.terminal.preset];
    if (!palette)
      throw new Error(`Unknown terminal preset: ${values.terminal.preset}`);
    values.terminal.colors = { ...palette, ...values.terminal.colors };
  }
  // Resolve CSS colors in an isolated document without changing either application's theme or user preferences.
  const frame = document.createElement("iframe");
  frame.setAttribute("sandbox", "allow-same-origin");
  frame.setAttribute("aria-hidden", "true");
  frame.tabIndex = -1;
  frame.style.cssText =
    "position:fixed;left:-20000px;top:0;width:1440px;height:900px;visibility:hidden;pointer-events:none";
  document.body.append(frame);
  const cancelLoads = new Set<() => void>();
  try {
    const doc = frame.contentDocument!;
    const view = frame.contentWindow!;
    doc.documentElement.dataset.appearance = appearance;
    doc.documentElement.dataset.theme = bundle?.id ?? "lomi";
    const asset = (path: string) =>
      themeAssetUrl(bundle!.id, path, bundle!.revision);
    for (const css of [
      baseline,
      applicationStyles,
      compileTheme(values, asset),
    ]) {
      const style = doc.createElement("style");
      style.textContent = css;
      doc.head.append(style);
    }
    await Promise.all(
      (values.stylesheets ?? []).map(
        (path) =>
          new Promise<void>((resolve, reject) => {
            const link = doc.createElement("link");
            link.rel = "stylesheet";
            link.href = asset(path);
            const finish = (error?: Error) => {
              clearTimeout(timer);
              cancelLoads.delete(cancel);
              link.onload = link.onerror = null;
              if (error) reject(error);
              else resolve();
            };
            const cancel = () => finish(new Error("Theme export cancelled."));
            const timer = setTimeout(
              () =>
                finish(new Error(`Cannot export: loading ${path} timed out.`)),
              10000,
            );
            cancelLoads.add(cancel);
            link.onload = () => finish();
            link.onerror = () =>
              finish(new Error(`Cannot export: loading ${path} failed.`));
            doc.head.append(link);
          }),
      ),
    );
    doc.body.innerHTML =
      '<div class="app-shell"><div class="titlebar"><div class="tab-bar"><div class="tab active-tab"></div><div class="tab"></div></div></div><div class="work-area"><div class="sidebar"><div class="sidebar-heading"></div></div></div><div class="statusbar"></div><div class="menu"></div><input/><textarea></textarea><button class="button"></button><button class="button button-primary"></button><a></a></div>';
    const probe = doc.createElement("span");
    doc.body.append(probe);
    const root = view.getComputedStyle(doc.documentElement);
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 1;
    const context = canvas.getContext("2d", { willReadFrequently: true })!;
    const color = (value: string) => {
      if (!value || value === "none") return undefined;
      probe.style.color = "";
      probe.style.color = value;
      if (!probe.style.color) return undefined;
      context.clearRect(0, 0, 1, 1);
      context.fillStyle = view.getComputedStyle(probe).color;
      context.fillRect(0, 0, 1, 1);
      const bytes = context.getImageData(0, 0, 1, 1).data;
      return `#${Array.from(bytes.slice(0, bytes[3] === 255 ? 3 : 4), (b) => b.toString(16).padStart(2, "0")).join("")}`;
    };
    const colors: Record<string, string> = {};
    for (const [id, names] of Object.entries(workbenchColors)) {
      const token =
        id === "editor.background" ? "--editor-background" : names[0];
      const value = color(root.getPropertyValue(token).trim());
      if (value) colors[id] = value;
    }
    for (const [name, id] of Object.entries(terminalColorIds)) {
      const value = color(
        root.getPropertyValue(`--terminal-${kebab(name)}`).trim(),
      );
      if (value) colors[id] = value;
    }
    for (const [id, [selector, property]] of Object.entries(componentColors)) {
      const declaration = values.styles?.[selector]?.[property];
      const node = doc.querySelector(selector);
      const value = color(
        declaration ??
          (node ? view.getComputedStyle(node).getPropertyValue(property) : ""),
      );
      if (value) colors[id] = value;
    }
    const syntax = Object.fromEntries(
      Object.keys(syntaxScopes).map((name) => {
        const token = (key: string) =>
          root.getPropertyValue(`--syntax-${name}-${key}`).trim();
        const foreground = color(token("color") || "var(--editor-foreground)");
        const fontStyle = [
          token("font-style") === "italic" ? "italic" : "",
          token("font-weight") === "bold" || Number(token("font-weight")) >= 600
            ? "bold"
            : "",
          token("text-decoration").includes("underline") ? "underline" : "",
        ]
          .filter(Boolean)
          .join(" ");
        return [name, { foreground, fontStyle }];
      }),
    );
    return { colors, syntax };
  } finally {
    for (const cancel of cancelLoads) cancel();
    frame.remove();
  }
}
export async function exportVSCodeThemes(
  manifest: ThemeManifest,
  bundle: ThemeBundle | null,
) {
  const modes: Appearance[] =
    manifest.appearance === "dark" || manifest.appearance === "light"
      ? [manifest.appearance]
      : ["dark", "light"];
  const result: { name: string; theme: VSCodeTheme }[] = [];
  for (const mode of modes) {
    const current = await sample(manifest, mode, bundle);
    const original = manifest.vscode
      ? await sample(
          { version: 2, name: manifest.name, vscode: manifest.vscode },
          mode,
          null,
        )
      : null;
    const name =
      modes.length > 1
        ? `${manifest.name} ${mode === "dark" ? "Dark" : "Light"}`
        : manifest.name;
    const theme: VSCodeTheme = structuredClone(manifest.vscode ?? {});
    theme.name = name;
    theme.type = manifest.vscode?.type?.startsWith("hc")
      ? mode === "light"
        ? "hcLight"
        : "hcDark"
      : mode;
    theme.colors = { ...theme.colors };
    for (const [id, value] of Object.entries(current.colors))
      if (!original || original.colors[id] !== value) theme.colors[id] = value;
    const rules: TokenRule[] = [];
    for (const [key, scopes] of Object.entries(syntaxScopes))
      if (
        !original ||
        JSON.stringify(original.syntax[key]) !==
          JSON.stringify(current.syntax[key])
      )
        rules.push({
          name: `Lomi ${key}`,
          scope: scopes,
          settings: current.syntax[key],
        });
    if (rules.length)
      theme.tokenColors = [...(theme.tokenColors ?? []), ...rules];
    result.push({ name, theme });
  }
  return result;
}
