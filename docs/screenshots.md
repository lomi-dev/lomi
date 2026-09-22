# README screenshots

These PNGs were captured from the running Lomi Tauri application on macOS ARM64
on 22 September 2026, after the local rebrand. They use the bundled DeepMono dark
theme. The application layout and styling were not changed for the captures.

| Image                                       | Native window size | PNG size    | Content                                                                            |
| ------------------------------------------- | ------------------ | ----------- | ---------------------------------------------------------------------------------- |
| [Workbench](images/workbench.png)           | 1440 × 900         | 2880 × 1800 | Explorer, `src/browser-url.ts`, and eight passing tests in a real native terminal. |
| [Source Control](images/source-control.png) | 1440 × 900         | 2880 × 1800 | The native Git diff for a local demonstration comment in `src/browser-url.ts`.     |
| [Keybinds](images/keybindings.png)          | 920 × 680          | 1840 × 1360 | The separate Settings window, including the Lomi product name.                     |

The session used a disposable clone named `lomi`, a separate application ID
`dev.lomi.rebrand-demo`, and its own application data directory. The desktop
frontend and Rust backend were built from the rebranded working tree. The
candidate SDK archive was installed in the disposable application checkout.
The editor, shell, filesystem and Git commands used the real native backend.
The terminal ran:

```sh
node --experimental-strip-types --test --test-reporter=spec tests/browser.test.ts tests/editor-text.test.ts tests/editor-preferences.test.ts
```

A temporary capture helper in the disposable checkout selected views and sent
this command to the terminal. It did not replace native commands or alter UI
rendering and is not part of the application sources. macOS `screencapture`
captured each window without shadows. No generated UI, image compositing or
text replacement was applied to the PNGs.

## Refreshing the images

1. Run the current desktop build with a separate demonstration profile and a
   disposable project copy.
2. Use the bundled dark theme, real commands and a local Git diff. Wait for
   files, fonts and terminal output to finish loading.
3. Capture each native window without unrelated desktop content, dialogs,
   tooltips or private information. Preserve logical sizes and native scale.
4. Replace the PNGs and update this provenance. Inspect the actual images.

For browser, PTY and native-window imagery, capture the desktop application;
Playwright's mocked native bridge is not evidence of those native features.
