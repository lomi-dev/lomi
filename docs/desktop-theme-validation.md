# Desktop theme validation

Verified on 20 October 2026 using Node 24 on macOS. Interface tests use Playwright
with mocked native commands. The new built-in theme follows system appearance;
manual light/dark choices and independent custom themes remain available.

## Initial rollout checks

| Check                           | Result                                                                                          |
| ------------------------------- | ----------------------------------------------------------------------------------------------- |
| `pnpm check`                    | Passed SDK contract verification and both TypeScript checks.                                    |
| `pnpm test`                     | Passed 306 application tests and 45 AI runtime tests.                                           |
| `pnpm build`                    | Passed production frontend build. Vite reported large chunks and a mixed static/dynamic import. |
| Focused Prettier check          | Passed for changed theme, chrome, content styles, components, tests and documentation.          |
| `git diff --check`              | Passed.                                                                                         |
| Chromium theme/workbench suites | 31 unique tests passed.                                                                         |
| Chromium content suites         | 59 tests passed.                                                                                |
| WebKit theme/workbench suites   | 24 tests passed, no skips.                                                                      |
| Representative preview capture  | Both appearances passed.                                                                        |
| Independent review              | No remaining material findings after fixes.                                                     |

The Chromium theme/workbench suites were `builtin-themes`, `themes`,
`lomi-surfaces`, `settings-layout`, `editor-appearance` and
`desktop-accessibility`. Content coverage was `agent-runtime`, `chat-layout` and
`welcome`. WebKit covered `builtin-themes`, `lomi-surfaces`, `editor-appearance`,
`settings-layout` and `welcome`.

Runs used isolated configurations and ports 1425, 1426 and 1427 to avoid other
development sessions. Viewports include 1440 x 900 and the native minimum
800 x 420; dialog tests also exercise a narrower 560 x 420 viewport.

## Reference refinement and restored sections

The supplied reference informs the charcoal surfaces, rounded tab pills, raised
composer and restrained tool/account cards. The File Explorer, terminal desktop,
Workspaces list and Source Control retain their pre-reference section styles and
geometry, following the later scope correction.

- The old tab regression was reproduced: tabs sat 5 px off the strip center and
  the project picker, plus button and overflow arrows had different centerlines.
  They now share the same 21.5 px center in a 44 px header. Long labels and the
  close/new-tab controls remain visible at 800 x 420.
- Explorer/Source Control component CSS matches the pre-reference snapshot.
  Workspaces item spacing, active marker and sidebar seams match that snapshot.
  Restored defaults include 30 px tree rows, 12 px by 14 px terminal padding,
  6 px control corners and zero extra desktop pane border, radius or spacing.
- Chromium: 90 unique regression cases passed across tabs, dragging, Settings,
  themes, editor appearance and chat. One library-width assertion was corrected
  to measure the container content box, then passed on rerun.
- WebKit: all 21 selected tab and surface cases passed. A separate four-case
  capture verified tab centerlines in light/dark and Linux/macOS fixtures.
- All 233 current light/dark startup tokens match the resolved definition.
  Independent review confirmed custom and duplicated theme overrides, retained
  panel behavior and built-in opaque-background eligibility.
- Production build, TypeScript and focused formatting checks passed.

These checks use mocked native commands. The protected-section restoration
concerns component styling and layout defaults; the shared theme palette remains
consistent across the application.

## Settings header cleanup

Generic Refresh controls and the Refreshed completion message are removed from
Settings page headers. Existing native change events and window-focus loading
continue to update themes and plugins.

- All 23 selected themes, plugins and settings-layout tests passed. External
  theme/plugin changes are simulated through native events in the fixtures.
- Production build and TypeScript checks passed.
- Light/dark Settings previews were recaptured after removing the controls.

The follow-up removes the remaining status/action groups from the Keybinds,
Terminal and Account headers, including Saved, Reset all and Check connection.
Per-setting resets remain available; corrupt preferences and account connection
failures expose contextual recovery actions in their notices.

- All 23 focused cases passed: 9 Keybinds, 4 Terminal preferences and 10 Account.
  Coverage includes cross-window persistence, retained PTYs, failed saves,
  corrupt-file recovery and stale account responses after logout.
- Three additional affected cases passed for pointer-focus recovery, terminal
  title visibility and Windows shell preferences.
- Production build, both TypeScript checks and focused formatting checks passed.
- Rendered Keybinds, Terminal and Account recovery previews were inspected.

## Behavior and visual checks

- System appearance changes, saved manual overrides, built-in duplication and
  migration from a removed built-in selection pass.
- Theme editing exposes shared tokens, saves layout changes across windows and
  retains terminal sessions and panel identities. Editor selection and undo
  history survive appearance changes.
- Partial custom themes retain their primary colors on hover, chosen input
  borders, scalar tab radii and window radius. Preview cancellation, failed
  resources and invalid edits retain the previous working appearance.
- All 233 resolved startup tokens match the built-in definition in both modes.
  Geist Latin and Latin Extended load locally, including Polish text.
- Light and dark settings, command search, chat, account/task views and dialogs
  were inspected. Focus, long content, scrolling and footer controls remain usable.
- Checked enabled text pairs exceed 4.5:1; checked control borders
  exceed 3:1 against adjacent input and dialog surfaces. Primary action text is
  16.55:1. Late-loaded chat styles preserve built-in borders and focus colors.
- The built-in shell remains square, opaque and eligible for the existing Linux
  background optimization. Custom shell geometry remains configurable.

## Previews

These are rendered application views with fixture project/chat content, captured
in Chromium. They are browser previews, not native window screenshots.

| View             | Light                                                      | Dark                                                      |
| ---------------- | ---------------------------------------------------------- | --------------------------------------------------------- |
| Workbench        | [Preview](images/desktop-theme/light-workbench.png)        | [Preview](images/desktop-theme/dark-workbench.png)        |
| Editor           | [Preview](images/desktop-theme/light-editor.png)           | [Preview](images/desktop-theme/dark-editor.png)           |
| Chat             | [Preview](images/desktop-theme/light-chat.png)             | [Preview](images/desktop-theme/dark-chat.png)             |
| Settings         | [Preview](images/desktop-theme/light-settings.png)         | [Preview](images/desktop-theme/dark-settings.png)         |
| Compact settings | [Preview](images/desktop-theme/light-compact-settings.png) | [Preview](images/desktop-theme/dark-compact-settings.png) |
| Welcome          | [Preview](images/desktop-theme/light-welcome.png)          | [Preview](images/desktop-theme/dark-welcome.png)          |

## Limits

No native Tauri executable, installer, PTY transport or native child-webview
rendering was qualified in this theme task. Native Windows and Linux rendering
remain unverified. Existing runtime and native behavior were preserved rather
than replaced. Browser WebKit coverage does not claim full WKWebView parity.

The repository contained substantial unrelated changes before this task and
received concurrent edits. Those changes were preserved and were outside the
theme review. No dependencies were added and no commit was created.
