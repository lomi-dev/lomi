# macOS disk image background

`src-tauri/dmg/background.html` is the editable source for the Finder install
window background. It uses the original Lomi dark wordmark from the brandbook
(`brandbook/assets/logos/lomi-primary-dark.svg`), the supplied arrow-right path
(`brandbook/assets/icons/arrow-right.svg`), the Manrope font already bundled in
`public/fonts/manrope/Manrope.woff`, and the brandbook colors Ink `#0B0D0C`,
Soft Chalk `#F7F8F3`, and Electric Lime `#C8FF3D`. The original wordmark SVG is
vendored unchanged in `src-tauri/dmg/lomi-primary-dark.svg`; the arrow keeps the
source path geometry with Ink stroke styling.

Regenerate the checked-in 1x PNG with:

```sh
pnpm dmg:background
```

The renderer uses Playwright Chromium at a 720x480 CSS-pixel viewport and waits
for the local Manrope font before saving `src-tauri/dmg/background.png`. Install
Chromium first when Playwright has no browser available with
`pnpm exec playwright install chromium`. The PNG is checked in, so normal DMG
builds do not need Chromium or regeneration. Keep
the HTML canvas dimensions, this viewport, and the `bundle.macOS.dmg.windowSize`
setting in `src-tauri/tauri.macos.conf.json` in sync. The DMG layout coordinates
are configured there as well; Finder positions the app and Applications folder
over this background, so do not add their icons or labels to the PNG.
Finder's window bounds include its title bar, so the bottom of the canvas is
deliberately blank. Verify the mounted DMG in Finder after changing the layout.

Build the DMG with `pnpm tauri build --bundles dmg` on macOS.
