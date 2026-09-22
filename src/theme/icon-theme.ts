export type IconKind = "file" | "product";
export interface IconDefinition {
  iconPath?: string;
  fontCharacter?: string;
  fontColor?: string;
  fontSize?: string;
  fontId?: string;
}
export interface IconFont {
  id: string;
  src: { path: string; format: string }[];
  weight?: string;
  style?: string;
  size?: string;
}
export interface IconAssociations {
  file?: string;
  folder?: string;
  folderExpanded?: string;
  rootFolder?: string;
  rootFolderExpanded?: string;
  fileNames?: Record<string, string>;
  fileExtensions?: Record<string, string>;
  languageIds?: Record<string, string>;
  folderNames?: Record<string, string>;
  folderNamesExpanded?: Record<string, string>;
  rootFolderNames?: Record<string, string>;
  rootFolderNamesExpanded?: Record<string, string>;
}
export interface IconTheme extends IconAssociations {
  iconDefinitions: Record<string, IconDefinition>;
  fonts?: IconFont[];
  light?: IconAssociations;
  highContrast?: IconAssociations;
  hidesExplorerArrows?: boolean;
  showLanguageModeIcons?: boolean;
  usesCurrentColor?: boolean;
}
export interface FileIconRequest {
  path: string;
  folder?: boolean;
  expanded?: boolean;
  root?: boolean;
  language?: string;
}
export interface PreparedIcons {
  data: IconTheme;
  asset: (path: string) => string;
  fonts: Record<string, string>;
  dispose: () => void;
}

const languages: Record<string, string> = {
  js: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  jsx: "javascriptreact",
  ts: "typescript",
  mts: "typescript",
  cts: "typescript",
  tsx: "typescriptreact",
  rs: "rust",
  py: "python",
  pyw: "python",
  lua: "lua",
  go: "go",
  c: "c",
  h: "c",
  cc: "cpp",
  cpp: "cpp",
  cxx: "cpp",
  hpp: "cpp",
  hxx: "cpp",
  java: "java",
  html: "html",
  htm: "html",
  css: "css",
  scss: "scss",
  less: "less",
  json: "json",
  jsonc: "jsonc",
  md: "markdown",
  markdown: "markdown",
  yaml: "yaml",
  yml: "yaml",
  sql: "sql",
  xml: "xml",
  svg: "xml",
  toml: "toml",
  sh: "shellscript",
  bash: "shellscript",
  zsh: "shellscript",
  ps1: "powershell",
  bat: "bat",
  cmd: "bat",
  txt: "plaintext",
  rb: "ruby",
  php: "php",
  cs: "csharp",
  swift: "swift",
  kt: "kotlin",
  dart: "dart",
  vue: "vue",
  svelte: "svelte",
  r: "r",
  ipynb: "jupyter",
};
export function iconLanguage(path: string) {
  const name = path.split(/[\\/]/).pop()!.toLowerCase();
  if (/^(dockerfile|containerfile)(\.|$)/.test(name)) return "dockerfile";
  if (/^(makefile|gnumakefile)$/.test(name)) return "makefile";
  if (/^\.(gitignore|gitattributes|gitmodules)$/.test(name)) return "ignore";
  return languages[name.split(".").pop()!] ?? "plaintext";
}

// Compile case-insensitive associations once, not for every visible tree row.
const normalized = new WeakMap<IconAssociations, IconAssociations>();
function normalize(value: IconAssociations): IconAssociations {
  const existing = normalized.get(value);
  if (existing) return existing;
  const result = Object.fromEntries(
    Object.entries(value).map(([key, entry]) => [
      key,
      entry && typeof entry === "object" && !Array.isArray(entry)
        ? Object.fromEntries(
            Object.entries(entry).map(([name, id]) => [name.toLowerCase(), id]),
          )
        : entry,
    ]),
  );
  normalized.set(value, result);
  return result;
}

const associationOrder = [
  "folder",
  "folderExpanded",
  "rootFolder",
  "rootFolderExpanded",
  "file",
  "folderNames",
  "folderNamesExpanded",
  "rootFolderNames",
  "rootFolderNamesExpanded",
  "languageIds",
  "fileExtensions",
  "fileNames",
] as const;
const orders = new WeakMap<IconTheme, Map<string, number>>();
function definitionOrder(theme: IconTheme) {
  let order = orders.get(theme);
  if (!order) {
    order = new Map();
    for (const variant of [theme, theme.light, theme.highContrast])
      if (variant) {
        for (const field of associationOrder) {
          const value =
            variant[field] ??
            (field === "rootFolder"
              ? variant.folder
              : field === "rootFolderExpanded"
                ? variant.folderExpanded
                : undefined);
          for (const id of typeof value === "string"
            ? [value]
            : Object.values(value ?? {})) {
            if (!order.has(id)) order.set(id, order.size);
          }
        }
      }
    orders.set(theme, order);
  }
  return order;
}

// Match VS Code's generated selector specificity, including mode qualifiers,
// parent directories and expanded folders. Equal scores follow definition rule order.
export function fileIconMatches(
  theme: IconTheme,
  request: FileIconRequest,
  appearance: "light" | "dark",
  highContrast = false,
) {
  const order = definitionOrder(theme);
  const matches: { id: string; score: number }[] = [];
  const parts = request.path.toLowerCase().split(/[\\/]/).filter(Boolean);
  const name = parts.pop() ?? "";
  const parent = parts.pop();
  const extensions = name
    .split(".")
    .slice(1)
    .map((_, i, all) => all.slice(i).join("."));
  for (const [index, value] of [
    theme,
    !highContrast && appearance === "light" ? theme.light : undefined,
    highContrast ? theme.highContrast : undefined,
  ].entries()) {
    if (!value) continue;
    const variant = normalize(value);
    const add = (id: string | undefined, score: number) => {
      if (id && theme.iconDefinitions[id])
        matches.push({ id, score: score + (index ? 1 : 0) });
    };
    const named = (
      field: keyof IconAssociations,
      name: string,
      score: number,
      withParent = true,
    ) => {
      const values = variant[field] as Record<string, string> | undefined;
      add(values?.[name], score);
      if (parent && withParent) add(values?.[`${parent}/${name}`], score + 1);
    };
    if (request.folder) {
      if (request.root) {
        add(variant.rootFolder ?? variant.folder, 2);
        if (request.expanded)
          add(variant.rootFolderExpanded ?? variant.folderExpanded, 6);
        named("rootFolderNames", name, 3, false);
        if (request.expanded) named("rootFolderNamesExpanded", name, 7, false);
      } else {
        add(variant.folder, 2);
        if (request.expanded) add(variant.folderExpanded, 6);
        named("folderNames", name, 3);
        if (request.expanded) named("folderNamesExpanded", name, 7);
      }
    } else {
      add(variant.file, 2);
      const language = request.language ?? iconLanguage(request.path);
      add(
        variant.languageIds?.[language] ??
          (language === "jsonc" ? variant.languageIds?.json : undefined),
        3,
      );
      for (const extension of extensions)
        named("fileExtensions", extension, 3 + extension.split(".").length);
      named("fileNames", name, 5 + extensions.length);
    }
  }
  return matches.sort(
    (a, b) =>
      a.score - b.score || (order.get(a.id) ?? 0) - (order.get(b.id) ?? 0),
  );
}

export function fileIconId(
  theme: IconTheme,
  request: FileIconRequest,
  appearance: "light" | "dark",
  highContrast = false,
) {
  return fileIconMatches(theme, request, appearance, highContrast).at(-1)?.id;
}

export function fileIconDefinition(
  theme: IconTheme,
  request: FileIconRequest,
  appearance: "light" | "dark",
  highContrast = false,
) {
  const matches = fileIconMatches(theme, request, appearance, highContrast);
  if (!matches.length) return undefined;
  const definition: IconDefinition = {};
  for (const { id } of matches) {
    const next = theme.iconDefinitions[id];
    if (next.iconPath) definition.iconPath = next.iconPath;
    else {
      if (next.fontCharacter) {
        definition.fontCharacter = next.fontCharacter;
        delete definition.iconPath;
      }
      for (const field of ["fontId", "fontColor", "fontSize"] as const)
        if (next[field]) definition[field] = next[field];
    }
  }
  return { id: matches.at(-1)!.id, definition };
}

export function iconCharacter(value: string) {
  return value.replace(/\\([a-f\d]{1,6})\s?/gi, (_, hex: string) => {
    const code = parseInt(hex, 16);
    return code > 0 && code <= 0x10ffff ? String.fromCodePoint(code) : "\ufffd";
  });
}

export function productIconDefinition(
  theme: IconTheme,
  ids: readonly string[],
) {
  for (const id of ids) {
    const definition = theme.iconDefinitions[id];
    if (definition?.fontCharacter) return { id, definition };
  }
  return undefined;
}

export async function prepareIcons(
  data: IconTheme,
  asset: (path: string) => string,
  namespace: string,
  signal?: AbortSignal,
): Promise<PreparedIcons> {
  const faces: FontFace[] = [];
  const fonts: Record<string, string> = Object.create(null);
  const dispose = () => {
    for (const face of faces) document.fonts.delete(face);
  };
  const abortable = (promise: Promise<FontFace>) =>
    new Promise<FontFace>((resolve, reject) => {
      const finish = (error?: unknown, face?: FontFace) => {
        clearTimeout(timer);
        signal?.removeEventListener("abort", cancel);
        if (error) reject(error);
        else resolve(face!);
      };
      const cancel = () => finish(new Error("Icon loading was canceled."));
      const timer = setTimeout(
        () => finish(new Error("Icon font loading timed out.")),
        10000,
      );
      signal?.addEventListener("abort", cancel, { once: true });
      if (signal?.aborted) cancel();
      void promise.then(
        (face) => finish(undefined, face),
        (error) => finish(error),
      );
    });
  try {
    for (const [index, font] of (data.fonts ?? []).entries()) {
      const family = `Lomi icons ${namespace} ${index}`;
      const face = new FontFace(
        family,
        font.src
          .map(
            (source) =>
              `url(${JSON.stringify(asset(source.path))}) format(${JSON.stringify(source.format)})`,
          )
          .join(","),
        {
          weight: font.weight ?? "normal",
          style: font.style ?? "normal",
        },
      );
      faces.push(face);
      try {
        await abortable(face.load());
      } catch (error) {
        throw new Error(
          `Cannot load icon font "${font.id}": ${error instanceof Error ? error.message : String(error)}`,
        );
      }
      if (signal?.aborted) throw new Error("Icon loading was canceled.");
      document.fonts.add(face);
      fonts[font.id] = family;
    }
    return { data, asset, fonts, dispose };
  } catch (error) {
    dispose();
    throw error;
  }
}
