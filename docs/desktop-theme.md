# Lomi desktop theme

The built-in Lomi theme follows Lomi Brand Guidelines 4.0 (`brandbook/` in the
Lomi workspace). It follows system appearance by default. Light and dark remain
explicit user overrides, independent from the selected theme.

## Visual direction

Lomi matches current agent-first desktop apps (Codex, Claude Code, Devin):
quiet navigation chrome and one raised content surface.

- Titlebar, sidebars and status bar share the window background and have no
  separators. The central stage is a 12 px rounded surface with a hairline
  border, inset 8 px from the window edges.
- Selection and hover use neutral fills. Signal Lime (dark) and Agent Violet
  (light, where Signal fails contrast on white) mark only primary actions and
  live states. Violet 300/500 marks keyboard focus and text selection.
- Mona Sans carries the interface (Expanded 700-800 for display headlines);
  Geist Mono carries code, terminals, labels and anything the machine says.
  Labels use sentence case; mono eyebrows use capitals with +4% tracking.
- Motion explains change: 180 ms `cubic-bezier(.2,.8,.2,1)` entrances, 120-160
  ms state changes, no bounce except the Signal landing on the logo. Reduced
  motion swaps movement for a fade.

- Dialogs are compact (400 px, no symbol, 15 px title, 30 px actions) on the
  stage surface. Their primary action is neutral (light on dark, ink on
  light); destructive actions keep the danger fill.

## Surfaces

| Role                       | Dark      | Light     |
| -------------------------- | --------- | --------- |
| Window chrome (background) | `#0e0e11` | `#f2f2f5` |
| Content surface            | `#111114` | `#ffffff` |
| Containers / hover         | `#1a1a1e` | `#ededf1` |
| Structural borders         | `#27272d` | `#e2e2e8` |
| Primary text               | `#f7f7f9` | `#18181c` |
| Muted text                 | `#9696a2` | `#686874` |
| Primary action             | `#b9f500` | `#723bfa` |
| Focus                      | `#ae8dff` | `#723bfa` |
| Text selection             | `#3a1594` | `#e6dbff` |

Neutrals come from the brand scale, which carries a slight violet tint.
Semantic colors use the brand success, warning and danger values, darkened in
light mode to keep 4.5:1 contrast.

## Welcome and onboarding

`src/Welcome.tsx` shows the Lomi symbol (`src/LomiSymbol.tsx`, geometry from
the brandbook), one heading and two buttons on the plain stage:
Add project opens a menu of project sources, New file picks a folder. `src/Onboarding.tsx`
runs once on first launch: intro, a CLI-first or Chat-first workflow choice,
appearance, detected agent CLIs, a shortcut step where pressing a listed
shortcut lights its row, and the trust model with a summary of the choices.
Chat first opens new projects with a chat tab and lists New chat first in the
new-tab menu; CLI first (default) keeps the terminal. Onboarding uses the brand Aurora, staged entrances and keyboard control
(Enter and arrow keys navigate, progress dots jump, Escape skips). Completion
and the workflow choice are stored in webview storage
(`src/onboarding-state.ts`).

## Theme implementation

- `themes/lomi.json` is the sole built-in definition; `src/theme/baseline.css`
  supplies the same startup values and registers the fonts before JavaScript.
- Mona Sans and Geist Mono variable fonts and their OFL licenses ship in
  `public/fonts/`. The built-in theme reads nothing from the brandbook at runtime.
- `src/styles.css` ends with the brand shell layer (stage surface, dividers,
  neutral selection, tabs). Custom themes still override every token.

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
