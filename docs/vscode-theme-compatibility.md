# VS Code themes and Lomi

## VS Code's theme pipeline

VS Code separates three extension contributions: color themes, file icon themes,
and product icon themes. This integration supports all three as independent
selections. The color pipeline below is followed by the file/product icon
pipeline and its compatibility boundaries.

1. A data-only extension declares `contributes.themes` in `package.json`. Each
   entry supplies a label, a relative theme path, and a `uiTheme` baseline:
   `vs`, `vs-dark`, `hc-black`, or `hc-light`. Labels may reference
   `package.nls.json`. One extension may contain many variants.
2. The color-theme file is JSON with comments. `include` loads a base file before
   the current file. `colors` overrides workbench color identifiers; unassigned
   identifiers use VS Code's registered defaults for the baseline. `"default"`
   removes an inherited color override. Token rule arrays append in include
   order. Semantic rules inherit by selector; VS Code combines the included
   `semanticHighlighting` flags with logical OR.
3. TextMate grammars assign scope stacks to source text. `tokenColors` assigns
   colors and styles to scope selectors. A token may match several selectors;
   specificity and rule order resolve each style property. An explicit empty
   `fontStyle` clears inherited font styling. `tokenColors` may also reference a
   TextMate `.tmTheme` plist. No executable theme extension is required.
4. Language services may provide semantic token types and modifiers. When enabled,
   `semanticTokenColors` styles these tokens on top of grammar highlighting.
   Semantic selectors can qualify a language and modifiers. If a semantic rule
   is absent, VS Code can map a semantic token to TextMate scopes. This depends
   on the installed grammar, language extension, document, and project context.
5. User workbench/token/semantic customizations overlay the chosen theme.
   Fonts, editor behavior, icon themes, and the arrangement of workbench views
   are separate settings. Standard color themes cannot execute CSS or redesign
   the application's layout.

Primary references, consulted 2026-09-17:

- [Theming capabilities](https://code.visualstudio.com/api/extension-capabilities/theming)
- [Color theme authoring](https://code.visualstudio.com/api/extension-guides/color-theme)
- [Theme color identifiers](https://code.visualstudio.com/api/references/theme-color)
- [Syntax highlighting](https://code.visualstudio.com/api/language-extensions/syntax-highlight-guide)
- [Semantic highlighting](https://code.visualstudio.com/api/language-extensions/semantic-highlight-guide)
- [Theme contributions](https://code.visualstudio.com/api/references/contribution-points#contributes.themes)
- [VS Code's color-theme loader](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/services/themes/common/colorThemeData.ts)

## Existing Lomi pipeline

Lomi v2 themes have common/light/dark sections, semantic CSS variables,
component declarations, layout, backgrounds, local resources, editor syntax,
terminal palettes, and plugin tokens. The runtime applies a complete validated
appearance transaction and reads computed CSS values. CodeMirror compartments
and retained xterm instances update in place. Main and Settings use the same
native source revisions; previews stay local. Invalid themes keep the last
working appearance.

Lomi's editor uses Lezer/CodeMirror tags, not TextMate scope stacks or
VS Code language-service tokens. The theme change does not replace its editor,
install a language server, or execute VS Code extensions.

## Implemented exchange

In **Settings → Themes**:

- **Import VS Code** accepts color or icon `.json`/`.jsonc`, `.tmTheme`, an
  extension's `package.json`, or `.vsix`.
- **Import folder** accepts either a Lomi package or a VS Code extension
  folder. Dropping one file/folder follows the same import path.
- Each contributed color or icon theme becomes a separate local theme. Imports do not
  select a theme automatically. Extension code is neither copied nor executed.
- **Export to VS Code** writes a new `.vsix` into the chosen folder. Existing files
  are never overwritten. Use VS Code's **Extensions: Install from VSIX** command.
  Adaptive Lomi themes export separate Dark and Light entries; fixed
  themes export one entry. High contrast metadata is retained for imported themes.
- **Edit theme → VS Code compatibility** explains which data is mapped and which
  is retained only for export. The original resolved VS Code data lives in the
  `vscode` property of the installed `theme.jsonc`; ordinary Lomi overrides
  remain editable in common/light/dark sections.

The order is baseline → imported VS Code mapping → common → appearance variant
→ local stylesheets → user terminal appearance preferences. Removing a local
override restores the imported value. System appearance and preference storage
retain their existing behavior.

Native import resolves includes and TextMate references inside the selected
extension root. For a standalone file, its containing directory is the root;
choose the extension folder when references need its parent directory. It checks
cycles, symlinks, traversal, JSON duplicates, archive paths, entry counts, depth,
and size before installing a batch. Association object order is preserved because
VS Code uses rule order to break equal-specificity icon matches. An invalid later variant leaves no earlier
variant installed. A VSIX is read as bounded data; executable contents are ignored.
Limits remain 256 KiB per resolved theme, 16 include levels, 64 include loads,
64 variants, and 8192 archive entries / 64 MiB uncompressed data.

Export resolves theme CSS variables and hex/alpha colors in a sandboxed, hidden
rendering document. It does not switch the selected theme, mutate preferences,
apply terminal user overrides, or touch editor/PTY lifetimes. Required outer
stylesheets have bounded loads; a failure aborts export and removes the temporary
document. Supported component colors are sampled from representative elements
and exact mapped JSON declarations. Original unmapped colors, contextual rules,
and semantic rules survive export; changed mapped colors and syntax overrides
are added without replacing the source rule arrays. Included files are flattened,
so formatting/comments and the original file structure are not reproduced.

## Compatibility boundaries

| Area                              | Current behavior                                                                                                                                                                                                                                                                                                                |
| --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Workbench colors                  | Explicit mapping for existing Lomi surfaces and states. Missing colors inherit Lomi's baseline, not VS Code's full color registry.                                                                                                                                                                                              |
| Terminal                          | Foreground/background, cursor, selection, all 16 ANSI colors, and search decorations map to xterm. Terminal rendering and user overrides can affect the final appearance.                                                                                                                                                       |
| Editor colors                     | Editor background/foreground, gutter, line/selection/cursor colors and selected decorations map to CodeMirror.                                                                                                                                                                                                                  |
| TextMate syntax                   | Approximation to Lomi's existing 16 syntax categories. Prefix specificity, per-property inheritance, array/comma scopes and style resets are handled. Contextual selectors do not leak into global categories. Language-specific grammar distinctions and strikethrough are retained in source data but are not fully rendered. |
| Semantic tokens                   | Preserved and exported. Lomi has no VS Code semantic-token provider, so these rules do not claim semantic rendering in Lomi.                                                                                                                                                                                                    |
| Unknown color IDs                 | Preserved, exported, and exposed as `--vscode-*` variables; no invented UI component is created.                                                                                                                                                                                                                                |
| High contrast                     | Light/dark mode metadata and mapped contrast borders are retained. This does not reproduce every VS Code accessibility state.                                                                                                                                                                                                   |
| CSS/layout/images/fonts/plugin UI | Remain Lomi features. Export can capture mapped colors, but a VS Code color extension cannot express their layout or resource effects. Pseudo-state or structural CSS without a mapped representative may need manual adjustment.                                                                                               |
| File/product icons                | Independent data-only imports, selections, scoped assets, live rendering and VSIX exports; details below.                                                                                                                                                                                                                       |
| Licensing/publishing              | The VSIX is a local extension under `lomi-local`. It is not published or installed automatically. Original author metadata is retained in Lomi; review the original license before redistributing an imported theme.                                                                                                            |

Consequently, this is a bidirectional color and icon theme bridge with retained source
data, **not full visual or semantic parity** between both applications. Achieving
that parity for source code would also require compatible TextMate grammars,
selector evaluation, semantic-token providers, and per-language extension
behavior. CSS-driven Lomi layouts would still have no representation in
standard VS Code color themes.

## File and product icon themes

VS Code registers `contributes.iconThemes` and `contributes.productIconThemes`
separately from `contributes.themes`. Lomi imports all three lists from a
single extension atomically. **Colors**, **File icons**, and **Interface icons**
in Settings select independent preferences. Selecting an icon theme preserves
colors, appearance, the other icon selection, editor history and PTY instances.
The same native revision and application transaction apply all three selections.
Missing resources or fonts leave the last working appearance active. Safe startup
skips all three; plugin removal checks every selected theme.

An installed icon package contains a small native wrapper:

```json
{
  "version": 2,
  "name": "My file icons",
  "iconTheme": { "kind": "file", "path": "icons.json" }
}
```

`icons.json` contains the VS Code icon document. Its image and font paths are
resolved relative to that document, confined to the package. Import rewrites
paths into a scoped `assets/` directory. Definitions, unknown metadata, unused
icons, associations, light/high-contrast overrides and font declarations survive
export. File documents can be 2 MiB, resources up to 20 MiB each, a package up to
64 MiB; fonts are limited to 32 with at most 8 alternative sources each. SVG/raster
images and WOFF/WOFF2/TTF/OTF fonts are data only. Extension code is not installed.

File icon matching follows VS Code's generated CSS specificity: filename and
parent-folder associations, multipart extensions, language IDs, root folders,
expanded folders, light/high-contrast qualifiers, and rule order for ties.
`jsonc` inherits the `json` language association when absent. SVGs render as
images or current-color masks; font icons honor characters, family, size and
color. `hidesExplorerArrows` hides the decoration while preserving accessible
expansion buttons. File icons apply to Explorer, resource tabs and headings,
project folders, search results, rename/create rows and unsaved-file prompts.

Product icons render the declared glyph fonts, inherit control colors and fall
back to the built-in Lucide icon for missing definitions. The explicit semantic
identifier/default alias mapping is in `src/theme/product-icons.ts`; all existing
application icon controls use it. This does not add VS Code controls absent from
Lomi or replace icons drawn inside third-party web pages or plugin views.
Resource icons have an independent built-in fallback and are not changed by
product themes.

Language associations use Lomi's filename language detection and may take
an explicit language ID. Lomi does not install VS Code language extensions
or their separately registered language-mode icons; `showLanguageModeIcons` is
retained, but those external fallback images are unavailable. The High Contrast
variant follows an imported high-contrast color theme. There is no separate new
high-contrast preference or automatic VS Code settings import.

**Export to VS Code** includes the selected icon document and every referenced
image/font in a data-only VSIX. Built-in Lomi file and interface icons also
export using Lucide SVGs and the matching glyph font, with its license. **Create
icon theme** and **Duplicate** create local editable packages; **Open icon
definitions** opens their folder for editing the standard JSONC/JSON document.
Filesystem watches reload the selected package in open windows. Editing native
icon definitions externally can add icons for VS Code surfaces that Lomi
does not display; they remain present in the exported extension.

Primary references:

- [File icon themes](https://code.visualstudio.com/api/extension-guides/file-icon-theme)
- [Product icon themes](https://code.visualstudio.com/api/extension-guides/product-icon-theme)
- [Product icon identifiers](https://code.visualstudio.com/api/references/icons-in-labels)
- [File icon loader and specificity](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/services/themes/browser/fileIconThemeData.ts)
- [Product icon loader](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/services/themes/browser/productIconThemeData.ts)

## Validation

The unit tests cover mapping, source preservation, overrides, selector
specificity, malformed data, native include/TextMate resolution, archive safety,
atomic batch import, and VSIX reimport. UI tests cover the real theme/export
runtime with mocked native commands, both appearances, layout at narrow widths,
error cleanup, and editor/terminal retention.

On macOS, `node --experimental-strip-types tests/native/run-theme-smoke.mjs`
launches an isolated Tauri development instance. It requires local VS Code
(default `/Applications/Visual Studio Code.app`, or pass its theme-defaults
extension folder as an argument). The test imports and exports through native
commands, parses installed VS Code default variants, exercises WKWebView export,
checks cross-window updates, verifies native PTY identity/output and dirty editor
undo, renders file SVGs and product glyph fonts, round-trips both imported and
built-in icon VSIX assets, and rejects color/icon export from main. It prints isolated data and artifact paths;
it does not use the normal Lomi data directory.
