# Lomi agent instructions

## Project

Lomi is a desktop ADE application developed incrementally around project
folders, named workspaces, and terminal tabs. A project owns workspaces sharing
its folder; each workspace owns tabs; each terminal tab owns a default shell
environment and a tree of terminal, file editor, and browser panels. Browser, file, diff, and commit tabs
do not own shells. There is no hardcoded tab count limit.

The current milestone includes project selection, workspaces, tabs, a file
explorer, CodeMirror file editing, conditional Git Source Control, native terminals with splits and
background streaming, session restoration, and configurable shortcuts in the
separate settings window's Keybinds page. Add further product functionality only
when requested. Do not infer a detailed ADE feature set from the project name
or acronym.

## Technology and structure

- Tauri 2 hosts the desktop application and its native window.
- Rust implements the native application in `src-tauri/src/`.
- React and TypeScript implement the interface in `src/`.
- Vite serves the frontend during development and builds it into `dist/`.
- Plain CSS defines the interface and theme in `src/styles.css`.
- `src/model.ts` owns the persisted layout and pure layout transformations.
  Workspace tabs are terminals with pane layouts, file editors with project-relative
  paths and positions, read-only file diffs with a repository path and staged/working
  comparison, browsers with an HTTP(S) URL, or commit views with a repository
  path and full commit hash.
  Dragging terminal, browser, or file tabs into a terminal layout preserves pane IDs,
  editor buffers, and running PTYs; moved terminal panes may override the tab's default shell profile, including after restoration.
  Keep terminal operations scoped to
  terminal tabs and preserve compatibility with saved tabs without a type.
- `src/Workbench.tsx` coordinates projects, workspaces, tabs, and persistence.
  Workspaces, Explorer, and Source Control have independently assigned sidebar sides; opposite
  sides can stay open together. `src/Sidebar.tsx` resizes them, and
  `src/SidebarToggle.tsx` exposes placement through the status-bar context menus.
  `src/Workspaces.tsx` lists workspaces across all project folders. Adding a
  workspace from this panel may reuse a folder with separate tabs and terminals.
  Workspace creation and context-menu rename/delete actions live only in this
  sidebar. Removing the last workspace removes its project from the session,
  preserving files on disk and checking unsaved editor buffers before closing.
  Preserve legacy left-sidebar settings when restoring older sessions.
- `src/editor-service.ts` loads the editor on demand; `src/editor-runtime.ts`
  keeps shared CodeMirror buffers and history outside React. `src/FileEditor.tsx`
  mounts visible file panels. `src/editor-text.ts` preserves exact line endings.
  `src-tauri/src/files/editor.rs` implements scoped reads, atomic saves with
  revision checks, encoding preservation, and native file watches.
- `src/editor-preferences.ts` validates indentation preferences;
  `src/EditorPreferencesProvider.tsx` synchronizes them across windows.
  Settings → Editor owns writes to `editor-preferences.json` through
  `src-tauri/src/editor_preferences.rs`. Apply changes to shared buffers through
  CodeMirror compartments without replacing text, selections, or undo history.
  The editor status bar changes indentation and language for the current shared
  buffer only. Keep these overrides across tab/workspace switches and let users
  restore the defaults; language selection must not rename the file. Load parsers
  on demand and ignore stale results after language switches or buffer disposal.
- `src/plugins/` owns trusted local plugin metadata, lifecycle, view/command/fill
  registries, placeholders and Settings management. `@lomi-dev/plugin-sdk` from the separate
  `lomi-dev/plugin-sdk` repository is the external author contract. The host
  imports its installed pure exports; do not reintroduce SDK source copies.
  `pnpm sdk:verify` checks the Rust fixture snapshot against that dependency. Only main imports executable ESM through the plugin
  protocol; settings reads metadata and approves immutable revision trust.
  `src-tauri/src/plugins.rs` serializes bounded installs/removals and checks
  caller, path containment, content identity and close approvals. Themes supplied
  by packages are data-only, available without code, immutable, and duplicable.
  Disable/uninstall preserves unavailable descriptors after dirty-view guards.
  Safe startup skips third-party code and themes before evaluation.
- `src/DockviewLayout.tsx` and `src/dockview-layout.ts` render the actual mixed
  central layout through Dockview core and stable React portals. The domain
  tree is authoritative; do not independently persist Dockview state or dispose
  runtimes on its transient remove/add events. Floating/popout/native DnD are
  disabled; pointer and keyboard operations update the domain atomically.
- Session v2 adds plugin tabs/panes and per-workspace plugin sidebars. Preserve
  v1 compatibility and its atomic backup before upgrading the session file.
  Unknown plugin views retain valid state; never interpret them as shells.
- Theme v2 uses raw JSONC, common/light/dark surfaces and scoped resources.
  Keep one effective runtime, native source revisions, cancelable resource
  staging, local previews and scoped filesystem watches. Legacy JSON is a
  bounded data converter only. Reconfigure CodeMirror compartments and retained
  xterm instances from final computed tokens; never reset editor history or PTYs.
- `src/keybindings.ts` defines shortcut actions, defaults, and validation;
  `src/KeybindingsProvider.tsx` synchronizes them between native windows.
  `src/SettingsWindow.tsx` edits them on the Keybinds page.
  The same preferences persist the optional focus-follows-pointer mode.
  `src/usePointerFocus.ts` focuses panels for typing, paste, and shortcuts;
  preserve dialogs, ordinary form fields, dragging, and input composition.
- `src/terminal-preferences.ts` validates terminal appearance overrides and bounded
  behavior settings; `src/TerminalPreferencesProvider.tsx` synchronizes them across
  windows. Settings → Terminal owns writes to `terminal-preferences.json` through
  `src-tauri/src/terminal_preferences.rs`. Apply changes to live and hidden xterm
  instances without restarting PTYs; unset appearance options inherit the theme.
- `src/BrowserPane.tsx` provides browser controls; `src/browser-runtime.ts` positions
  native child webviews and retains them across tab/workspace switches and docking.
  `src-tauri/src/browser.rs` uses the system Tauri engine, with a GTK overlay on Linux.
  `src-tauri/src/browser/servers.rs` discovers local HTTP listeners for address
  suggestions; probe only loopback addresses and keep discovery in the main view.
  Stream confirmed addresses without waiting for other probes; share the last results
  and the in-flight scan across browser panels in `src/browser-runtime.ts`.
  Restore addresses lazily; close webviews only when their last panel is removed.
  Browser pages have a separate storage profile and no application command privileges.
  Their only IPC permission reports events for their own browser panel. Scope native
  capabilities by webview label: child browser views share the main window. Extract
  `Window`, not `WebviewWindow`, in commands because main contains multiple webviews;
  preserve the caller webview check in the application invoke handler.
- xterm.js renders terminals; `src/terminal-runtime.ts` owns their lifecycle and
  streaming independently of React. Rust `portable-pty` owns native processes.
- `src/CliIntegrations.tsx` offers missing titlebar, MCP, and supported notification
  integrations in the status bar for local macOS/Linux CLI processes. Explicit
  clicks authorize native process-scoped configuration; each suggestion has its own
  dismissal scoped to the CLI and feature for the native app process. Settings lists the CLI catalog and installs MCP per supported client or in bulk.
  `cli_catalog.rs` owns identities; `cli_mcp.rs` and its YAML adapter handle
  verified formats. Preserve capability gating and manual-only clients; see
  `docs/cli-agents.md` for paths and qualification.
  `cli_titles.rs` resolves the process configuration; `cli_config.rs` preserves
  other settings, revisions, and backups. `cli_mcp.rs` registers the app's headless
  `--mcp` entry point with a pinned signing key for authenticated broker discovery.
  Pairing grants remain connection-scoped. The `--agy-terminal-title` entry point formats
  agy's supplied JSON state, reading current conversation names from its local
  annotation files without starting the GUI or reading transcripts. Never
  configure a CLI silently or restart it automatically.
- `src-tauri/src/shell.rs` discovers shell environments and quotes dropped paths;
  `src-tauri/shell/` contains integration hooks. Do not edit user shell profiles.
- `src-tauri/src/files.rs` handles file access and session saving;
  `src-tauri/src/files/search.rs` searches scoped text with bounded results and
  cancellation. `src-tauri/src/files/operations.rs` handles explicit Explorer
  mutations, serialized with editor saves. `src/explorer-model.ts` updates paths
  and closes removed views; migrate shared editor aliases before retaining the
  updated session, preserving dirty text and undo history across renames.
  `src-tauri/src/git.rs` handles Git through argument-based CLI calls.
  `src/FileDiff.tsx` opens Source Control comparisons in separate read-only tabs.
  `src-tauri/src/git/diff.rs` provides full-file context, including new and deleted
  files, with explicit binary and size-limit notices. Diff tabs do not own shells
  or editor buffers and restore by loading fresh comparisons.
  `src-tauri/src/git/history.rs` provides paginated history and commit details.
  History and commit tabs are read-only; merge diffs use the first parent.
- pnpm manages frontend dependencies; Cargo manages Rust dependencies.
- `src/Updater.tsx` checks signed GitHub release metadata after startup and on
  requests from Settings → About. `src-tauri/src/updater.rs` detects Linux update
  instructions and routes manual checks to the main view. Windows/macOS install
  only after the close guard and session save succeed; Linux only shows external
  update instructions. Keep updater installation permissions scoped to the main
  webview on Windows/macOS, preserve Tauri's restart exit code on macOS, and never
  commit the updater private key. Publish complete signed releases with `latest.json`.
- `src-tauri/tauri.conf.json` connects Vite to Tauri and configures the window.
- `src-tauri/capabilities/` defines the native commands available to the frontend.
- `src-tauri/src/main.rs` contains a startup workaround for WebKitGTK's Wayland
  Error 71 on NVIDIA. Keep it scoped to that environment, preserve explicit user
  overrides, and set the process environment before Tauri starts GUI threads.
- Commit `pnpm-lock.yaml` and `src-tauri/Cargo.lock` when dependencies change.
  Do not add lockfiles from other JavaScript package managers.

## Scope and simplicity

- Implement the smallest complete solution for the requested milestone.
- Avoid overengineering: do not add speculative features, premature abstractions,
  generic frameworks, unused dependencies, or infrastructure for hypothetical needs.
- Prefer straightforward code and the existing stack. Extract shared code only
  when real duplication or current complexity justifies it.
- Do not prebuild editors, AI integrations, remote services, or additional
  settings before the corresponding feature is requested.
- Keep changes focused. Do not mix unrelated refactors into a task.
- Add native commands, plugins, and permissions only when a current feature needs
  them, and grant only the access that feature requires.

## Terminal and persistence invariants

- Keep terminal output outside React state. Send binary output directly through
  Tauri channels and acknowledge it after xterm parses it. Preserve bounded flow
  control, ordered input/output, and the ordered end-of-stream marker.
- Switching tabs or workspaces must not restart running PTYs or stop parsing
  their output. Release hidden WebGL renderers and dispose closed PTYs.
- Preserve WebGL initialization and context-loss fallback. Do not impose an
  arbitrary tab cap; retain bounded scrollback and command metadata.
- Shell commands, control keys, terminal escape sequences, and Unicode must pass
  through without speculative interpretation or automatic command execution.
- Dropped paths must use the selected shell's quoting rules, including WSL path
  translation. Never append Enter when inserting a dropped path.
- Restore layouts and working directories with fresh shells when tabs are first
  visited. Never replay commands or claim to restore live processes or output.
- Serialize session saves and replace the layout file atomically. Preserve an
  unreadable or unsupported saved session until the user chooses recovery.
- Retain dirty file buffers and undo history across tab and workspace switches.
  Closing their final view, deleting their workspace, or exiting must offer
  save/discard/cancel. Failed saves must retain edits and prevent save-and-close.
  External changes may reload clean buffers; dirty buffers require an explicit
  reload or overwrite choice. Session restoration loads fresh file contents,
  not unsaved editor buffers. Preserve encodings, line endings, and permissions.
- Capture configured application shortcuts before xterm forwards input to the
  PTY. Ctrl+D opens a panel in the current tab; Ctrl+W closes the active panel.
  Unassigned keys, composition, and ordinary form editing must remain intact.
- Persist shortcuts separately in `keybindings.json`, apply changes across
  windows without restarting terminals, and preserve invalid settings until
  explicit recovery. Prevent conflicting shortcut assignments.
- Keep project, editor, Git, and terminal commands restricted to the main window.
  Settings may manage keybindings, editor preferences, terminal preferences, and theme packages, listen for updates, and
  use its own window controls. Only settings may write these preferences or
  import/create theme packages; both windows may read and apply themes.
- Git mutations must follow an explicit UI action. Preserve the user's identity
  and exact commit text; never silently stage, commit, push, or add attribution.

## Language and comments

- Keep this entire `AGENTS.md` file in English.
- Write all code comments and documentation comments in English.
- Comments must be technical and refer directly to the code: explain behavior,
  constraints, invariants, non-obvious decisions, or platform-specific workarounds.
- Do not add filler, narration of the task, obvious restatements of the code,
  promotional text, or AI attribution to comments. Omit unnecessary comments.

## Appearance

- Lomi is the default built-in theme, defined in `themes/lomi.json` and the
  matching startup tokens in `src/theme/baseline.css`. It follows the Lomi
  Brandbook and Design System v1.0.0: Electric Lime, graphite/chalk surfaces,
  semantic status colors, and bundled Manrope. Keep code and terminals monospace.
- Preserve `themes/deepmono.json` as the optional built-in DeepMono 1.1.0
  Mono/Mono Light palette with Graphite. The default selection is `active: null`;
  DeepMono uses `active: "@builtin-deepmono"`. Both are read-only and duplicable.
- Reuse semantic tokens instead of inventing additional colors. Built-in themes
  must work offline without files from the source brandbook or design system.
- Follow system appearance by default, including startup and live changes.
  Persist manual Light/Dark overrides separately from the selected theme.
  Explicit theme appearances take precedence; adaptive themes inherit the
  chosen mode. Keep native windows transparent so CSS paints the background
  once. The app icon source is `public/Lomi.icon`; run `pnpm icon:generate` on
  macOS with Xcode 26+ after editing it. Commit the generated desktop icons and
  `public/app-icon.png`. See `docs/app-icons.md` for appearance and packaging checks.
- User-selected themes may override the default appearance through versioned
  `theme.jsonc` files, scoped local assets, JSON styles, and optional CSS.
  `src/theme/format.ts`, `src/theme/runtime.ts`, and `src/ThemeProvider.tsx` own theme
  validation and live application; `src-tauri/src/themes.rs` owns theme folders,
  the resource protocol, and atomic preferences. Keep the last working theme
  after load failures, and preserve invalid files until explicit recovery.
- The application must work without access to the original local theme file.
  Keep the selected colors in the repository; do not read that path at runtime.
- Keep the initial interface minimal and usable at the configured minimum window
  size. Use semantic HTML and preserve readable contrast.
- Keep terminal pages free of permanent pane toolbars and environment selectors,
  except for the centered terminal title and its maximize/restore control.
  Search, command input, command blocks, and environment selection are shown
  only when requested through their configurable shortcuts.

## Development and validation

- Install dependencies: `pnpm install --frozen-lockfile`.
- Start the desktop app: `pnpm tauri dev`.
- Start only the browser frontend: `pnpm dev`.
- Check TypeScript: `pnpm check`.
- Run model tests: `pnpm test`.
- Run interface tests: `pnpm test:ui` (install Chromium with
  `pnpm exec playwright install chromium` first).
- Check frontend/document formatting: `pnpm format:check`.
- Build the frontend: `pnpm build`.
- Check Rust: `cargo check --manifest-path src-tauri/Cargo.toml --locked`.
- Run Rust and native PTY tests:
  `cargo test --manifest-path src-tauri/Cargo.toml --locked`.
- Check Rust formatting: `cargo fmt --manifest-path src-tauri/Cargo.toml --check`.
- Check Rust lints:
  `cargo clippy --manifest-path src-tauri/Cargo.toml --locked -- -D warnings`.
- Build the desktop executable: `pnpm tauri build --no-bundle`.
- Build platform installers: `pnpm tauri build`.
- Run checks appropriate to the changed code. For UI changes, inspect the rendered
  result. Add tests for meaningful behavior, not static markup or trivial wrappers.
- Playwright uses mocked native commands. Verify PTY transport, native windows,
  renderer behavior, and platform-specific shells in the desktop application
  when changing those paths. State which operating systems were actually tested.
- Report what was verified and any checks that could not run. Never claim a check
  passed unless it actually ran successfully.

## Commit and GitHub conventions

- Use English for commit subjects, commit bodies, pull request titles, and pull
  request descriptions.
- Use this subject format for every commit and pull request title:
  `type(scope): short imperative summary`.
- Use a lowercase type and scope. Allowed types: `feat`, `fix`, `docs`, `style`,
  `refactor`, `test`, `build`, `ci`, `chore`, and `perf`.
- Choose a short scope that identifies the changed area, such as `app`, `ui`,
  `rust`, `deps`, or `repo`.
- Keep the subject at most 72 characters, without a trailing period. Use an
  imperative verb such as `add`, `fix`, or `remove`.
- Each commit should represent one coherent change.
- After a blank line, include a short body explaining what changed and why, then
  a `Validation:` section listing the checks and their results. If checks were
  not run, state that and give the reason. Wrap body text at about 72 characters.
- Describe breaking changes in a `BREAKING CHANGE:` footer when applicable.
- Never add an AI agent as an author or co-author. Never add an AI
  `Co-authored-by:` trailer, agent signature, or "generated by" attribution to
  commits or pull requests. Preserve the user's configured Git identity.
- Use `.github/pull_request_template.md` for pull request descriptions. Describe
  the final behavior, its reason, and actual validation results. Link an existing
  issue when relevant; do not invent issue numbers.

Example commit:

```text
feat(app): initialize the desktop foundation

Add a Tauri window and an empty React workspace using the DeepMono palette.
Keep the first milestone small so ADE features can be added incrementally.

Validation:
- pnpm build: passed
- cargo check --manifest-path src-tauri/Cargo.toml --locked: passed
```
