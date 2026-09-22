# Agent instruction files

Claude Code and Gemini CLI share project instructions through
[LOMI.md](../LOMI.md). [AGENTS.md](../AGENTS.md) contains the same
rules for agents that discover that filename directly. Keep these two files
identical when changing repository conventions, validation commands, or
application invariants.

| File                      | Purpose                                                                                      |
| ------------------------- | -------------------------------------------------------------------------------------------- |
| [AGENTS.md](../AGENTS.md) | Project instructions for agents that discover this filename, including Zed's built-in agent. |
| [LOMI.md](../LOMI.md)     | Shared instructions imported by Claude Code and Gemini CLI.                                  |
| [CLAUDE.md](../CLAUDE.md) | Claude Code entry point containing only `@LOMI.md`.                                          |
| [GEMINI.md](../GEMINI.md) | Gemini CLI entry point containing only `@LOMI.md`.                                           |

Both entry points use native imports. The CLI loads the contents of `LOMI.md`
into its instruction context; the line is not merely a Markdown link or a request
for the model to open another file. See
[Claude Code imports](https://code.claude.com/docs/en/memory#import-additional-files)
and [Gemini CLI imports](https://geminicli.com/docs/cli/gemini-md/#modularize-context-with-imports).

These files guide agents working on the Lomi repository. Each agent is
responsible for loading them, including when it runs in a Lomi terminal.

## How Zed organizes its instructions

Examined `zed-industries/zed` on 2026-09-15 at commit
[`a95da07db20cf36bd750ecc4bdfecdf2b29e3f0f`](https://github.com/zed-industries/zed/tree/a95da07db20cf36bd750ecc4bdfecdf2b29e3f0f).

```text
zed/
  .rules          Shared repository instructions
  AGENTS.md       Symlink to .rules
  CLAUDE.md       Symlink to .rules
  GEMINI.md       Symlink to .rules
  docs/
    .rules        Documentation conventions
    AGENTS.md     Documentation automation instructions
  .agents/skills/ Task-specific workflows
```

The three root aliases are Git symbolic links (mode `120000`) whose target is
`.rules`. They expose one file under the names different agents recognize. The
root `.rules` contains Zed's Rust/GPUI conventions and contribution rules;
`docs/.rules` and `docs/AGENTS.md` are separate regular files with documentation
guidance. Their discovery depends on the consuming agent and directory scope.
Sources: [Zed's Git tree](https://api.github.com/repos/zed-industries/zed/git/trees/a95da07db20cf36bd750ecc4bdfecdf2b29e3f0f?recursive=1)
and [shared rules](https://github.com/zed-industries/zed/blob/a95da07db20cf36bd750ecc4bdfecdf2b29e3f0f/.rules).

Zed's built-in agent selects the first matching project instruction file in this
order:

```text
.rules → .cursorrules → .windsurfrules → .clinerules
       → .github/copilot-instructions.md → AGENT.md → AGENTS.md
       → CLAUDE.md → GEMINI.md
```

This order is defined in
[`RULES_FILE_NAMES`](https://github.com/zed-industries/zed/blob/a95da07db20cf36bd750ecc4bdfecdf2b29e3f0f/crates/prompt_store/src/prompts.rs#L22).
Zed also loads personal instructions from `~/.config/zed/AGENTS.md`
(`%APPDATA%\Zed\AGENTS.md` on Windows); project instructions take precedence when
they conflict. External agents and terminal CLIs use their own instruction
loaders. Zed's filename priority does not determine Claude Code's or Gemini CLI's
loading behavior. See [Zed's instruction documentation](https://zed.dev/docs/ai/instructions).

Zed separates persistent instructions from reusable workflows in
[`.agents/skills/`](https://github.com/zed-industries/zed/tree/a95da07db20cf36bd750ecc4bdfecdf2b29e3f0f/.agents/skills).
Those workflows include GPUI tests, benchmarks, lint creation, and cherry-picking.
They are specific to Zed's development process.

## Lomi adaptation

Lomi retains its existing `AGENTS.md` for native discovery, preserving
its Tauri, React, terminal, editor, persistence, permissions, and Git conventions.
`LOMI.md` contains the same instructions. `CLAUDE.md` and `GEMINI.md` are
ordinary text files with a single native import of `LOMI.md`.
This shares instructions between those two CLIs without requiring symlink
support during checkout. The two full instruction files require synchronization.
Claude's documentation specifically recommends imports
on Windows because creating symlinks requires Developer Mode or administrator
privileges. See [Claude Code's AGENTS.md guidance](https://code.claude.com/docs/en/memory#agentsmd).

Zed can read `AGENTS.md` directly, so Lomi needs no `.rules` compatibility
file. Adding a higher-priority filename from the list above would change which
file Zed selects. Preserve both import lines and keep `AGENTS.md` and
`LOMI.md` identical when editing instructions.

## Validation

Run the existing formatter from the repository root:

```sh
pnpm exec prettier --check AGENTS.md LOMI.md CLAUDE.md GEMINI.md README.md docs/agent-instructions.md
node --input-type=module -e 'import { readFileSync as read } from "node:fs"; import { strictEqual } from "node:assert"; strictEqual(read("AGENTS.md", "utf8"), read("LOMI.md", "utf8"))'
git diff --check
```

To inspect actual loading, start a fresh agent session in the repository root.
In Claude Code, use `/context` to inspect memory files. In Gemini CLI, use
`/memory show` to inspect the expanded instructions, or `/memory reload` after an
edit. These commands are documented in the CLI references linked above.
