# README screenshots

These PNGs were captured from the running Lomi Tauri application on macOS 27
ARM64 on 25 September 2026. They use the default built-in Lomi theme in Dark
mode, except `workbench-light.png`, which uses Light mode. The application
layout and styling were not changed for the captures.

| Image                                          | Native window size | PNG size    | Content                                                                                   |
| ---------------------------------------------- | ------------------ | ----------- | ----------------------------------------------------------------------------------------- |
| [Workbench](images/workbench.png)              | 1440 × 900         | 2880 × 1800 | Explorer, `src/model.ts`, and `pnpm test` in a native zsh terminal below the editor.      |
| [Workbench, light](images/workbench-light.png) | 1440 × 900         | 2880 × 1800 | The same session in Light mode.                                                           |
| [Browser](images/browser.png)                  | 1440 × 900         | 2880 × 1800 | Workspaces sidebar, `bun run dev` for `lomi-web`, and its page in a native browser panel. |
| [Source Control](images/source-control.png)    | 1440 × 900         | 2880 × 1800 | History, commit `54f1636`, and the diff for `src-tauri/src/terminal_preferences.rs`.      |
| [Agent control](images/agent-control.png)      | 920 × 680          | 1840 × 1360 | The Settings window's MCP client list.                                                    |
| [Themes](images/themes.png)                    | 920 × 680          | 1840 × 1360 | The Settings window's Themes page with Lomi active.                                       |

The application was a debug build of the working tree based on `ab5fdd3`, with
the frontend embedded, the separate application ID `dev.lomi.readme-capture`
and its own application data directory. It ran from an ad hoc signed `.app`
bundle so browser panels used the normal WebKit networking path. The projects
were disposable local clones of `lomi` at `ab5fdd3` and `lomi-web`; the
`lomi-web` clone used a placeholder `PUBLIC_PLUNK_API_KEY` so its signup form
shows the configured state. No form was submitted.

Terminals ran real zsh sessions through native PTYs. A capture-only `ZDOTDIR`
set a minimal prompt and ran the displayed command once in each project
directory. Views were selected through macOS Accessibility actions on the real
interface: pressing buttons, focusing panels and submitting the browser address
after the dev server was ready. No script was injected into the webviews.
macOS `screencapture -l -o` captured each window without shadows. No generated
UI, image compositing or text replacement was applied to the PNGs.

## Refreshing the images

1. Run a current desktop build with a separate application ID and a disposable
   project copy. Use the bundled Lomi theme, real commands and real Git history.
2. Wait for files, fonts, terminal output and browser pages to finish loading.
   Browser panels only lay out while their window is visible on the current
   Space.
3. Capture each native window without unrelated desktop content, dialogs,
   tooltips, focus rings or private information. Preserve logical sizes and
   native scale.
4. Replace the PNGs and update this provenance. Inspect the actual images.

For browser, PTY and native-window imagery, capture the desktop application;
Playwright's mocked native bridge is not evidence of those native features.
