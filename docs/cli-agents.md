# CLI agent integrations

Run installed agents in ordinary Lomi terminal panels. Lomi recognizes the 30
agents below plus the existing Antigravity CLI on macOS and Linux. It recognizes
native executables, documented Node/Bun launchers and Python console scripts;
inline code, prompt arguments and remote SSH commands are not agent identities.
This does not install the agents or restore their conversations after restart.

Settings → Agent control → MCP clients lists Claude, Codex, Gemini, Copilot,
Cursor, OpenCode, OpenClaw, Hermes, Kilo, Qwen, Kiro, Vibe, Kimi, Grok and
Antigravity. **Install** registers Lomi in the selected user configuration, and
**Install for all supported clients** applies only to this list. Local running agents also offer
applicable integrations in the status bar. Inspection never writes configuration;
installation requires a click. Client approval and Lomi pairing still apply.

## MCP adapters

Paths below are defaults. Documented environment overrides are resolved from the
running process for status-bar setup and from Lomi's environment for Settings.
Project settings may override user settings. For newly supported agents started
with an explicit configuration/profile flag, use that agent's own setup or
Settings for its default user configuration; Lomi does not guess the active file.

| Agent                                                                                                           | Command                    | Automatic user MCP configuration                                                                                      |
| --------------------------------------------------------------------------------------------------------------- | -------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| Claude Code                                                                                                     | `claude`                   | `~/.claude.json`                                                                                                      |
| OpenAI Codex CLI                                                                                                | `codex`                    | `~/.codex/config.toml`                                                                                                |
| [Gemini CLI](https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/mcp-server.md)                    | `gemini`                   | `~/.gemini/settings.json`                                                                                             |
| [GitHub Copilot CLI](https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-mcp-servers)  | `copilot`                  | `~/.copilot/mcp-config.json`; local server type and tools list                                                        |
| Cursor CLI                                                                                                      | `cursor-agent`             | `~/.cursor/mcp.json`                                                                                                  |
| [OpenCode](https://opencode.ai/v2/docs/mcp-servers)                                                             | `opencode`                 | `~/.config/opencode/opencode.json` or `.jsonc`; current v2 `mcp.servers` with command array                           |
| [OpenClaw](https://docs.openclaw.ai/gateway/config-extensions)                                                  | `openclaw`                 | `~/.openclaw/openclaw.json`; JSON5 `mcp.servers`                                                                      |
| [Hermes Agent](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/features/mcp.md)  | `hermes`                   | `~/.hermes/config.yaml`; `mcp_servers` map                                                                            |
| [Pi Coding Agent](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/README.md)               | `pi`                       | Requires an MCP extension; no built-in MCP registry                                                                   |
| [Aider](https://aider.chat/docs/config/options.html)                                                            | `aider`                    | No documented native MCP client                                                                                       |
| [Goose](https://github.com/aaif-goose/goose/blob/main/documentation/docs/guides/config-files.md)                | `goose`                    | `~/.config/goose/config.yaml`; `extensions`, `cmd`, `envs`                                                            |
| [Cline CLI](https://docs.cline.bot/getting-started/config)                                                      | `cline`                    | Existing `~/.cline/data/settings/cline_mcp_settings.json` or legacy `~/.cline/mcp.json`; see path qualification below |
| [Kilo Code CLI](https://kilo.ai/docs/automate/mcp/using-in-kilo-code)                                           | `kilo`, `kilocode`         | `~/.config/kilo/kilo.jsonc`; `mcp` with command array                                                                 |
| [Qwen Code](https://qwenlm.github.io/qwen-code-docs/en/users/features/mcp/)                                     | `qwen`                     | `~/.qwen/settings.json`                                                                                               |
| [Kiro CLI](https://kiro.dev/docs/mcp/configuration/)                                                            | `kiro-cli`                 | `~/.kiro/settings/mcp.json`                                                                                           |
| [Factory Droid](https://docs.factory.ai/harness/mcp)                                                            | `droid`                    | `~/.factory/mcp.json`                                                                                                 |
| [OpenHands CLI](https://docs.openhands.dev/openhands/usage/cli/mcp-servers)                                     | `openhands`                | `~/.openhands/mcp.json`; targets the deprecated v1 CLI                                                                |
| [Continue CLI](https://docs.continue.dev/reference)                                                             | `cn`                       | `~/.continue/config.yaml`; `mcpServers` list                                                                          |
| [Amp CLI](https://ampcode.com/docs/customize/mcp)                                                               | `amp`                      | `~/.config/amp/settings.json` or `.jsonc`; `amp.mcpServers`                                                           |
| [Auggie](https://docs.augmentcode.com/cli/integrations)                                                         | `auggie`                   | `~/.augment/settings.json`                                                                                            |
| [Crush](https://github.com/charmbracelet/crush/blob/main/docs/config/README.md)                                 | `crush`                    | Manual setup in `crushrc`; Lomi does not rewrite executable shell configuration                                       |
| [Mistral Vibe](https://docs.mistral.ai/vibe/code/cli/mcp-servers)                                               | `vibe`                     | `~/.vibe/config.toml`; `[[mcp_servers]]` entries                                                                      |
| [Kimi Code CLI](https://github.com/MoonshotAI/kimi-code/blob/main/docs/en/customization/mcp.md)                 | `kimi`                     | `~/.kimi-code/mcp.json`; targets the successor, not legacy Kimi CLI                                                   |
| [Open Interpreter](https://www.openinterpreter.com/docs/terminal/mcp)                                           | `interpreter`              | `~/.openinterpreter/config.toml`; `mcp_servers` table                                                                 |
| [Grok Build](https://docs.x.ai/build/features/mcp-servers)                                                      | `grok`                     | `~/.grok/config.toml`; `mcp_servers` table                                                                            |
| [Junie CLI](https://junie.labs.jb.gg/docs/junie-cli-mcp-configuration.html)                                     | `junie`                    | `~/.junie/mcp/mcp.json`                                                                                               |
| [Deep Agents Code](https://github.com/langchain-ai/deepagents/blob/main/openwiki/workflows/deep-agents-code.md) | `dcode`, `deepagents-code` | `~/.deepagents/.mcp.json`                                                                                             |
| [Freebuff CLI](https://github.com/CodebuffAI/freebuff/blob/main/sdk/src/agents/load-mcp-config.ts)              | `freebuff`                 | `~/.agents/mcp.json`, separate from Freebuff's profile directory                                                      |
| [Trae Agent](https://github.com/bytedance/trae-agent/blob/main/trae_agent/utils/config.py)                      | `trae-cli`                 | Manual project/selected `trae_config.yaml`; no shared user registry                                                   |
| [SWE-agent](https://github.com/SWE-agent/SWE-agent/blob/main/docs/config/config.md)                             | `sweagent`                 | No documented native MCP client                                                                                       |
| Antigravity CLI                                                                                                 | `agy`                      | `~/.gemini/config/mcp_config.json`                                                                                    |

Supported environment overrides include `CODEX_HOME`, `GEMINI_CLI_HOME`,
`COPILOT_HOME`, `XDG_CONFIG_HOME` (OpenCode, Kilo, Amp and Goose), `OPENCODE_CONFIG`,
`OPENCLAW_CONFIG_PATH`, `OPENCLAW_STATE_DIR`, `HERMES_HOME`, `GOOSE_PATH_ROOT`,
`CLINE_DATA_DIR`, `KILO_CONFIG`, `QWEN_HOME`, `KIRO_HOME`, `VIBE_HOME`, `KIMI_CODE_HOME`,
`INTERPRETER_HOME`, `GROK_HOME`, `JUNIE_HOME` and `DEEPAGENTS_HOME`.
`GEMINI_CLI_HOME` is the parent of `.gemini`. `GOOSE_PATH_ROOT` contains the
`config/config.yaml` path. Relative overrides are rejected.

Cline's documentation names two layouts. Without `CLINE_DATA_DIR`, Lomi only
writes when exactly one documented file already exists; otherwise initialize MCP
with `cline mcp`, then refresh. OpenCode/Amp configurations with both JSON and
JSONC files require manual setup. Legacy OpenCode MCP maps require migration to v2 or manual setup.
Inline OpenCode/Kilo configuration, OpenCode/Kilo's
additional config directory, custom OpenClaw profiles/homes, custom Junie config locations and custom Claude MCP
homes are not guessed. Vibe with `VIBE_CLI=rust` requires manual qualification;
the upstream Rust MCP manager documents OAuth-only additions.

Configuration updates preserve unrelated values, reject a foreign server named
`lomi`, check file revisions, save exact original backups and replace files
atomically. JSON/JSONC/JSON5 edits preserve unrelated comments and text. YAML
updates reformat the file and remove comments; the original remains in the backup.
YAML aliases, merge keys, unsupported tags and duplicate keys are rejected rather
than silently rewritten. Parsing is bounded, with no include-file loading or
property expansion by Lomi.

## Titles and notifications

All terminal panels display titles and supported terminal notification sequences
emitted by their programs. Automatic title setup is available for Codex, Claude,
Cursor, Antigravity, Gemini and Qwen. Gemini/Qwen retain their enabled defaults;
Lomi offers setup only when title settings are disabled. Gemini uses
`ui.hideWindowTitle` / `ui.dynamicWindowTitle`, and Qwen uses
`ui.hideWindowTitle` / `ui.showStatusInTitle`. Their
[Gemini schema](https://github.com/google-gemini/gemini-cli/blob/main/schemas/settings.schema.json)
and [Qwen settings](https://qwenlm.github.io/qwen-code-docs/en/users/configuration/settings/)
define these fields.

Automatic notification setup remains available for Codex and Claude. Other
agents may already send terminal bells/titles or provide their own desktop
notifications. Their notification hooks are not installed by Lomi. No agent is
restarted automatically after configuration changes.

## Manual MCP clients

Start Lomi's MCP server in Settings and use **MCP JSON configuration for this
running instance** as the source of the executable and argument values. That
manual connection uses the separate `lomi-mcp` helper and must be copied again
after a server restart, as described in [MCP usage](mcp/USAGE.md).

For Trae, put the copied command/args in `mcp_servers.lomi` in the selected
`trae_config.yaml`, and include `lomi` in `allow_mcp_servers`. For Crush, use its
`crushrc` MCP configuration builtins from the linked official documentation. Pi
needs a separately installed MCP extension. Aider and official SWE-agent remain
usable as terminal programs, but cannot be given a native MCP configuration that
their current contracts do not provide.

## Qualification

The adapters follow official documentation/source reviewed on 2026-09-24.
Automated tests cover registration formats, preservation/conflicts, process
identification, environment paths and the opt-in Settings/status-bar workflow.
This is not an end-to-end qualification of installed/authenticated versions of
all agents. Process inspection is macOS/Linux only; WSL and Windows agent
inspection are not added by this change. Existing generic terminal behavior is
unchanged on those platforms.
