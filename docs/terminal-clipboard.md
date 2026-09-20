# Terminal image paste

`paste_terminal_clipboard` is restricted to the main application webview and a
live native terminal session. It reads the system clipboard only after an
explicit paste, preferring its image representation over text. The existing
clipboard plugin decodes native formats off the GUI thread. Images never cross
the webview IPC boundary.

For a foreground local `agy` process on macOS/Linux, native process inspection
selects the CLI's existing Ctrl+V image-paste action. The foreground process is
checked again while holding the PTY writer lock before sending the control byte.
No shell hooks, CLI settings or clipboard contents are changed. agy imports the
system clipboard and owns the resulting attachment. Its own clipboard support
and default Ctrl+V binding must be available. WSL is excluded from this process
check; Windows/WSL currently use the generic file-path route.

For other programs, Rust encodes RGBA as PNG and returns a shell-quoted path.
Codex recognizes the pasted path as an image attachment. Other CLIs need support
for pasted image paths; the app does not claim a universal attachment protocol.

Files use unpredictable names in the app cache's `terminal-clipboard` directory.
On Unix, the directory is private (0700) and files are 0600. Failed encodes are
removed, previous pastes are never overwritten, and the cache is bounded to
512 MiB and 2048 files. Automatic eviction is avoided because another terminal
or a saved CLI draft may still need a file. Remove unneeded images manually from
the directory reported by a full-cache error. Images are not encrypted.

The native session's shell profile determines path quoting and WSL translation.
The frontend runs clipboard work in the existing input queue, then lets xterm
normalize and bracket returned text. The native agy action returns no text, so
it cannot also paste a path or send the shortcut twice. Later typing and Enter
follow the paste action in PTY order. Closing the originating runtime drops a
pending text result; native delivery checks the original session's writer.
Moving or hiding a runtime preserves its queue. The app never modifies CLI
configuration, presses Enter as part of paste, or uploads image contents.

This does not transfer files over SSH or into containers. WSL uses the existing
native `wslpath` flow for file paths.
Other platforms and agents require their own native qualification.

## Verification

- `cargo test --manifest-path src-tauri/Cargo.toml --locked terminal::clipboard`
  checks pixel/alpha preservation, private files, cache bounds, failed writes and
  symlink rejection.
- `pnpm test:ui tests/ui/terminal-clipboard.spec.ts tests/ui/terminal-input.spec.ts`
  checks shortcut routing, native menu paste events, text fallback, bracketed
  input, asynchronous ordering, errors and closing during preparation. Native
  commands are mocked in these browser tests.
- `node tests/native/run-terminal-clipboard-smoke.mjs` is an opt-in macOS fixture
  behind `native-smoke`. It requires Python 3 and Swift.
  It uses isolated SimpleBench data and an empty temporary project, saves and
  restores all clipboard representations unless the user copies something else,
  and tests native clipboard reads through real PTYs, including the actual
  AppKit Paste menu action with an image-only clipboard.
- Add `--agents` to exercise installed and authenticated `codex` and `agy` in
  this repository, which must already be trusted by both CLIs. The fixture does
  not grant directory trust or modify CLI configuration. It verifies a Codex
  attachment and an agy native media attachment, saves screenshots, then discards the
  unsent input. No model prompt is submitted.

On the tested macOS ARM64 host, Codex 0.155.1 recognizes the pasted PNG path as
`[Image #1]` and agy 1.2.7 reports `1 media attached`. No provider request is sent
by the fixture. Windows, WSL and Linux have not been tested natively for this
feature; their shortcut routing is covered by mocked interface tests.
