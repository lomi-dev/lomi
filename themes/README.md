# JSONC themes

Settings → Themes offers **Create theme**, **Import folder**, **Duplicate theme**,
**Edit theme**, **Preview**, and theme selection. Preview changes only that
window; cancel/close restores its previous working layers. Save checks the file
revision, writes atomically, and notifies every window. Unsaved drafts remain
available after validation, resource, revision, or disk errors.

## Package and format

A local folder contains `theme.jsonc`, optional ordered CSS files and local assets.
[`theme.schema.json`](theme.schema.json) describes version 2; runtime validation
also checks CSS values, contribution paths, duplicates and resource availability.
Create theme writes `theme.jsonc` and its local schema. The new adaptive theme
inherits Lomi until overrides are added; it contains no demonstration assets.
[`lomi.json`](lomi.json) is the default built-in theme. It maps the Lomi Brandbook
and Design System v1.0.0 to the desktop interface: Electric Lime actions, graphite
dark surfaces, Chalk light surfaces, semantic status colors, and bundled Manrope.
Controls use the system’s 8 px radius and keyboard focus 2 px. Desktop dialogs
use 16 px corners, layered shadows, a dimmed backdrop, and semantic warning or
danger actions. Terminal surfaces use graphite in dark mode and white in light
mode, with additional inset space around the text.
Desktop density and monospace JetBrains Mono in code/terminals are retained.
Syntax and ANSI colors extend the semantic palette for readable code.

[`deepmono.json`](deepmono.json) preserves DeepMono 1.1.0 Mono/Mono Light with
Graphite as a second read-only built-in theme. Both themes support System, Light
and Dark modes, duplication into editable local packages, and VS Code export.
Lomi is represented by `active: null`; DeepMono uses `active: "@builtin-deepmono"`.
Existing default selections adopt Lomi; explicit local and plugin selections
remain unchanged. Built-ins are embedded in the application and need no external
theme folder. Manrope and its OFL license ship in `public/fonts/manrope/`.
`src/theme/baseline.css` provides the same startup baseline before JavaScript runs.

```jsonc
{
  "version": 2,
  "name": "My graphite",
  "appearance": "adaptive", // adaptive (default), light, or dark
  "common": {
    "tokens": { "--radius-control": "6px", "--terminal-padding": "12px" },
    "layout": {
      "tabs": "below",
      "statusbar": "bottom",
      "settingsNavigation": "left",
    },
    "editor": {
      "colors": { "selection": "#33445588" },
      "syntax": { "comment": { "fontStyle": "italic" } },
    },
    "terminal": { "preset": "AdventureTime", "fontSize": 14 },
    "plugins": { "lomi.context": { "surface": "var(--color-surface)" } },
  },
  "light": { "editor": { "colors": { "background": "#fafafa" } } },
  "dark": { "editor": { "colors": { "background": "#101010" } } },
  "resources": {
    "stylesheets": ["theme.css"],
    "assets": { "wall": "images/wall.svg" },
  },
}
```

Comments and trailing commas are allowed. Duplicate keys, unknown fields, JSON5
syntax, non-finite numbers, and unknown syntax names are errors. Raw documents
are bounded to 256 KiB and 32 nested levels. Diagnostics include file, location,
and the offending property where available. Form edits use jsonc-parser edits;
formatting retains comments. Controls and the JSONC editor share one draft.

`common`, `light`, and `dark` accept:

- `tokens`: CSS custom properties for semantic colors, typography, spacing,
  density, borders and radii. Use the built-in token catalogue. Color values
  accept hex RGB/RGBA, `transparent`, `var(--token)`, and
  `color-mix(in srgb, var(--token) 70%, transparent)` (0–100%). Other valid CSS
  color expressions can be used in optional CSS.
- `styles`: selector → CSS declaration map, validated/serialized through CSSOM.
- `layout`: tabs `inline|above|below`, statusbar `top|bottom`, settingsNavigation
  `left|right|top|bottom`. These affect presentation, not saved workspaces.
- `backgrounds`: `app|terminal|sidebar|titlebar|statusbar|settings|modal`, with
  `image`, `opacity` (0–1), `size`, `position`, `repeat`, `blur` (0–100), `overlay`,
  and `blendMode`. Image opacity affects the background layer, not terminal text.
- `editor`: `fontFamily`, `fontSize` (6–72), `colors` (`background`, `foreground`,
  `gutterBackground`, `gutterForeground`, `activeLine`, `selection`, `cursor`),
  and `syntax`. Supported syntax names: `keyword`, `string`, `number`, `comment`,
  `type`, `function`, `variable`, `property`, `operator`, `punctuation`, `heading`,
  `strong`, `emphasis`, `link`, `invalid`. Styles accept `color`, `fontWeight`
  (`normal|bold` or 100–900 in steps of 100), `fontStyle` (`normal|italic`), `textDecoration` (`none|underline`).
  They map to a fixed Lezer tag table, never executable property expressions.
  Search, diff/commit and Markdown use the existing `--color-*` semantic tokens.
- `terminal`: appearance defaults and `colors`; the schema lists all 16 ANSI
  colors, cursor/accent, foreground/background, active/inactive selection,
  optional selection foreground and four search-decoration colors. The optional
  `preset` selects xterm-theme 1.1.0 palette data, loaded only when needed. In the
  editor use **Load terminal presets** to discover names. Explicit colors win.
- `plugins`: owner → token map. Tokens cannot target core properties through this
  field. The CSS name is `--plugin-<encoded-owner>-<token>`: encode `-` as `-h`
  and `.` as `-d`, in that order. Thus `lomi.context.surface` maps to
  `--plugin-lomi-dcontext-surface`. This encoding avoids owner collisions.

## Resolution and resources

Resolution order: built-in mode baseline → common → matching mode variant →
optional stylesheets in manifest order → user terminal appearance overrides.
Nested syntax styles, background properties, selector declarations and terminal
colors merge by property. Removing an override restores inheritance. A fixed
appearance takes precedence over System/Light/Dark; returning to an adaptive
theme restores the saved choice. Shell behavior and editor language/indentation
are independent preferences.

The runtime stages CSS, backgrounds and declared image/font assets with bounded,
cancelable loads before replacing working layers. Declare required fonts under
`resources.assets`, and use `@font-face` in CSS. URLs inside a stylesheet resolve
relative to that stylesheet, including `@import`. Network resources are blocked
by the application CSP. A failed optional nested CSS import cannot always be
reported by the engine's outer `load` event; declare required assets explicitly.

Paths must stay inside the package: no symlinks, traversal, encoded separators,
absolute/Windows paths, reserved DOS names, alternate streams or trailing dots.
Packages are bounded to 64 MiB, 1024 entries, 16 directory levels and 20 MiB per
asset. Themes cannot serve executable scripts. CSS can hide UI, so it is not an
isolation mechanism; use safe startup to recover.

After styles are ready, one runtime reads the final custom properties and updates
DOM/Dockview, CodeMirror appearance compartments and existing xterm instances.
`--syntax-<name>-color|font-style|font-weight|text-decoration` are public CSS
syntax overrides. Terminal user overrides use inline `!important` values.
Hidden editor buffers keep history; hidden terminals continue parsing output.

Each process has one native source revision, shared by main and Settings. It
includes selected preferences, manifest content and local resource metadata.
Frontend snapshots also have a local application revision for previews, system
mode and terminal preference changes. Tauri events plus a scoped active-folder
watch refresh other windows; late loads cannot replace newer selections. Failed
loads keep each window's last working appearance and show an error. Refresh and
window focus retry disk reads.

## Migration and recovery

Legacy `theme.json` version 1 is a bounded data input only. Import converts it to
version 2 `theme.jsonc` beside the original. Editing an already-installed legacy
theme writes `theme.jsonc` on the first save. The original file remains intact.
Unsupported legacy data produces an error instead of being silently dropped.
`src/themes.ts` and `src/theme-runtime.ts` were removed; there is one active engine
under `src/theme/`. Theme selection and System/Light/Dark settings retain their
existing `theme-settings.json` format. Terminal preference files are unchanged.

Start `lomi --safe-mode` (or `LOMI_SAFE_MODE=1 lomi`) to skip
third-party code and themes before evaluation. `--disable-plugins` also selects
the baseline; legacy `LOMI_SAFE_THEME=1` skips only custom themes. In this
mode use Settings → Themes and select Lomi, then restart normally. Invalid
files stay intact until an explicit recovery/save. Plugin-supplied themes are
available without enabling code, remain immutable, and can be duplicated before
editing. Uninstalling their owner requires selecting a fallback first.

Rust parses raw JSONC with jsonc-parser and independently checks callers, syntax,
structure, paths, resource limits and revision/atomic persistence. The frontend
owns CSS/color/syntax semantic validation and engine resource readiness. Rust
storage acceptance alone is not permission to activate a theme. Shared grammar
fixtures exercise the overlap; the two validators do not claim identical roles.

The `xterm-theme` 1.1.0 npm manifest lists ISC, while its upstream license is MIT
and the npm tarball omits that license file. The upstream MIT notice is retained
in `public/licenses/xterm-theme-MIT.txt`. The package supplies palette data only;
Lomi uses a single `@xterm/xterm` runtime.

## VS Code color-theme exchange

Settings → Themes can import VS Code color files, TextMate themes, extension
folders / `package.json`, and VSIX packages. Export creates an installable local
VSIX with one fixed appearance or separate dark/light entries. Imported data is
retained under the optional `vscode` field; common/light/dark overrides apply
above its mapping. Extension code is never evaluated.

See [the compatibility analysis](../docs/vscode-theme-compatibility.md) for the
VS Code loading pipeline, supported mappings, import/export behavior and limits.
CodeMirror syntax is an approximation of TextMate; semantic rules are retained
for export. Lomi CSS, layouts and assets have no standard VS Code
color-theme equivalent. This is not full visual or semantic parity.

File and interface icon themes use separate selections in Settings → Themes.
A version 2 wrapper references a standard VS Code icon document with
`"iconTheme": { "kind": "file", "path": "icons.json" }` (or `"product"`).
Create or duplicate an icon theme, then open its folder to edit its document and
scoped assets. Export includes all referenced SVG/raster images and glyph fonts.
See the [compatibility analysis](../docs/vscode-theme-compatibility.md#file-and-product-icon-themes)
for matching rules, resource bounds and language-extension limitations.
