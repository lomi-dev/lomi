# Lomi desktop theme

The built-in Lomi theme is rebuilt around the current website identity and the
v2 brandbook/design system. It follows system appearance by default. Light and
dark remain explicit user overrides, independent from the selected theme.

## Visual direction

The workbench keeps its existing workspace structure. Rounded active tabs share
one vertical centerline with the project picker, close buttons and strip controls. Geist Variable carries interface text; code and terminals
retain their existing monospace fonts. Light uses Chalk, Paper and Ink. Dark
uses subtly cool charcoal surfaces, with bright, readable foregrounds.
Navigation, content and raised menus/dialogs use distinct surface levels. Electric
Lime identifies primary actions and keyboard focus where appropriate. It is
not a general-purpose success color or a border around every active region.

Explorer, terminal desktop, Workspaces and Source Control retain their previous
section layout, spacing, pane framing and selection treatments. The reference
style applies to tabs, chat composition, tool/account cards, menus and dialogs.
Content remains
opaque; the theme does not imitate native Liquid Glass by blurring terminals.
States use semantic colors and text or actual interface icons. New UI copy uses
plain punctuation and does not add ornamental text glyphs.

These choices are adaptations for Lomi, rather than copied interfaces or claims
that the linked products use Lomi's token values.

## References and resulting decisions

| Reference                                                                                                       | Applied principle                                                                                                                                   |
| --------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| [VS Code modern UI](https://code.visualstudio.com/blogs/2026/10/15/modern-ui)                                   | Shared surface/control tokens, compact density, sentence-case labels and clear active-tab states.                                                   |
| [Linear design refresh](https://linear.app/now/behind-the-latest-design-refresh)                                | Quieter navigation, fewer competing treatments and separators, and visual evaluation in the application.                                            |
| [Claude Code desktop redesign](https://claude.com/resources/articles/claude-code-desktop-redesign)              | Keep terminal, editor, changes and preview in a coherent workspace with contextual actions.                                                         |
| [Introducing the Codex app](https://openai.com/pl-PL/index/introducing-the-codex-app/)                          | Preserve project context while moving between work, coordination and review. The article does not prescribe exact styling values.                   |
| [Motion React](https://motion.dev/docs/react)                                                                   | Use CSS for small visual feedback and retain existing runtime-aware motion infrastructure.                                                          |
| [Motion layout animations](https://motion.dev/docs/react-layout-animations)                                     | Avoid blanket transform-based layout animation on retained terminals, native browser panes or editor ancestors.                                     |
| [VS Code custom layout](https://code.visualstudio.com/docs/configure/custom-layout)                             | Preserve resizable, movable, keyboard-accessible workspace regions and existing layout preferences.                                                 |
| [VS Code UX guidelines](https://code.visualstudio.com/api/ux-guidelines/overview)                               | Keep navigation, editor content, contextual tools and status hierarchies distinct.                                                                  |
| [Apple: designing for macOS](https://developer.apple.com/design/human-interface-guidelines/designing-for-macos) | Dense but readable desktop content, resizable windows, keyboard use and customization.                                                              |
| [Apple: materials](https://developer.apple.com/design/human-interface-guidelines/materials)                     | Separate navigation and content roles; keep code and terminal surfaces readable and opaque.                                                         |
| [Radix introduction](https://www.radix-ui.com/primitives/docs/overview/introduction)                            | Preserve accessible control semantics, focus handling and keyboard behavior. A visual refresh alone does not justify replacing existing primitives. |
| [Designing Grok Bot](https://x.ai/news/designing-grok-bot)                                                      | Make identity, current activity and required attention understandable without permanent verbose chrome.                                             |
| [Devin gets a Mac](https://devin.ai/blog/devin-gets-a-mac)                                                      | Verify visible state and actual interaction sequences, rather than treating successful compilation as visual acceptance.                            |

References were reviewed on 20 October 2026. Apple's official Markdown versions
were also used where the web pages depend on JavaScript. Motion's layout URL
appeared twice in the supplied list and represents one reference.

## Theme implementation

- `themes/lomi.json` is the sole built-in definition. Its shared tokens and
  appearance variants replace the previous Manrope/graphite identity.
- `src/theme/baseline.css` supplies the same startup values before the runtime
  theme commits and remains a fallback for partial custom themes.
- Geist and its OFL license ship locally in `public/fonts/geist/`. The application
  has no runtime dependency on either documentation repository or a font CDN.
- The existing theme format, runtime, custom/plugin theme support and native
  appearance synchronization remain the authority for loading and switching.
- `src/styles.css` owns shared workbench chrome. Content, workflow, dialog, chat,
  task, account and authentication styles use the same semantic hooks.

The built-in selection remains `active: null`. The existing migration from
historical built-in selections remains intact. There is no second legacy
built-in theme left for users to select.

## Reference-led appearance

The supplied desktop screenshot adds a concrete visual direction: restrained
charcoal surfaces, softly rounded tab pills, bordered tool/account cards and a
raised composer with the draft above its controls. Geist and Electric Lime
retain Lomi's identity. The dark neutrals have a subtle cool undertone. Light
appearance retains the source palette.

The Explorer, terminal desktop, Workspaces list and Source Control use their
pre-reference component styles and workspace geometry. Their tree rows remain
30 px, terminal padding remains 12 px by 14 px, and desktop panes retain zero
extra spacing, border and radius. The tab strip is centered and uses 10 px
corners with 148-220 px widths. The outer native shell remains square and opaque.

The desktop palette adapts the identity and the supplied reference. The website's
scoped green dark surface remains documented separately in the brandbook.

| Role                           | Value     |
| ------------------------------ | --------- |
| Canvas                         | `#151618` |
| Content                        | `#191a1d` |
| Primary text                   | `#ededf0` |
| Navigation                     | `#1b1c1f` |
| Muted text                     | `#adb0b8` |
| Raised menus and dialogs       | `#25262a` |
| Selection                      | `#303237` |
| Structural borders             | `#3b3d43` |
| Control borders                | `#777a83` |
| Primary action / Electric Lime | `#c8ff3d` |

## Runtime constraints

A theme switch reconfigures the retained editor appearance compartment and
existing xterm instances. It does not restart PTYs, replace editor buffers or
reset undo history. User terminal overrides retain precedence. Keep theme
preview restoration, failed-load recovery and custom resource cancellation.

The built-in outer shell stays square and covers the viewport without transforms,
filters or animation. This preserves the existing Linux opaque-background
optimization. Do not animate browser pane geometry or terminal/editor ancestry.
Existing pane motion retains its cancellation, fit and reduced-motion guards.
Small hover/focus feedback must also respect reduced motion.

Settings pages use their existing initial loads, native events and focus updates.
Generic catalog refresh buttons and their completion messages are removed from
page headers. Keybinds, Terminal and Account also omit header actions and status
labels such as Saved, Reset all and Check connection. Individual resets remain
beside their fields. Recovery actions appear only alongside the relevant error
or connection notice. Task-specific actions stay with their existing controls.

## Validation

Use the existing mocked desktop suites to inspect both appearances at 1440 x 900
and the native minimum 800 x 420. Include workbench/editor content, long names,
settings, command search and dialogs. Check contrast, focus, system changes,
custom-theme switching and retained terminal/editor state.

Browser fixtures do not qualify native PTY transport or native materials on
other operating systems. Record actual commands, screenshots and platform
limits in the final validation report rather than presenting this design
reference as a completed test result.

See [the validation report and screenshots](desktop-theme-validation.md) for the
completed checks and representative views.
