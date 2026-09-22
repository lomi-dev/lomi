# Built-in portable icons

SVGs and the glyph font come from `lucide-static@1.41.0`, matching the installed
Lucide React version. The upstream ISC/MIT notices are in `LICENSE.txt` and are
included in exported icon extensions. These resources are embedded by Rust for
export and duplication; the normal application continues rendering Lucide React
SVGs and does not load the export font.

Source: [Lucide](https://github.com/lucide-icons/lucide) /
[static package](https://www.npmjs.com/package/lucide-static/v/1.41.0).

To regenerate definitions from `src/theme/product-icons.ts`, obtain and extract
that version's npm package, then run:

```sh
node --experimental-strip-types scripts/update-builtin-icon-themes.mjs /path/to/extracted/package
pnpm exec prettier --write themes/icons/*.json
```

The script copies only the listed SVGs, font and license. Product definitions
include every Lomi control mapping; VS Code uses its own default for other
identifiers. All font glyphs remain available for authors extending their copy.
