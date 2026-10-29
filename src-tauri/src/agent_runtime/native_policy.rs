//! Fixed launch restrictions for the reviewed native artifacts.
//! These options limit customization; they do not prove lifecycle settlement.
use crate::cli_catalog::TitleCli;

const CLAUDE_SETTINGS: &str = r#"{"disableAllHooks":true,"disableClaudeAiConnectors":true,"disableBundledSkills":true,"disableSkillShellExecution":true,"enabledPlugins":{"cc-plugin-agents-md@builtin":false,"cc-plugin-plugin-authoring@builtin":false}}"#;
const EMPTY_MCP: &str = r#"{"mcpServers":{}}"#;
const CLAUDE_TRANSPORT: &[&str] = &[
    "--print",
    "--verbose",
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
    "--include-partial-messages",
    "--include-hook-events",
    "--permission-prompt-tool",
    "stdio",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Version,
    Turn,
    Collector,
    AccountTerminal,
}

pub(super) fn reviewed(cli: TitleCli, version: &str) -> Result<(), String> {
    match (cli, version) {
        (TitleCli::Codex, "0.160.0") | (TitleCli::Claude, "2.1.287") => Ok(()),
        _ => Err("Native argument policy requires the reviewed Codex 0.160.0 or Claude 2.1.287 artifact.".into()),
    }
}

/// Place this prefix before the separately validated native transport arguments.
/// The version must come from artifact admission, before executing `--version`.
pub(super) fn arguments(
    cli: TitleCli,
    exact_version: &str,
    _mode: Mode,
) -> Result<Vec<String>, String> {
    reviewed(cli, exact_version)?;
    // Public artifact help/init fixtures reviewed 2026-10-06 establish these
    // exact-version flags. No empty-map override is claimed to erase Codex's
    // inherited configuration. ManagedConfiguration replaces Claude's empty
    // strict MCP map with the exact owned registration only for managed turns.
    let args: &[&str] = match cli {
        TitleCli::Codex => &[
            "--no-daemon",
            "--disable",
            "hooks",
            "--disable",
            "plugins",
            "--disable",
            "remote_plugin",
            "--disable",
            "apps",
            "--enable",
            "skip_host_skill_discovery",
            "-c",
            "notify=[]",
            "-c",
            "analytics.enabled=false",
            "--disable",
            "remote_control",
        ],
        TitleCli::Claude => &[
            "--restricted",
            // An attached value prevents variadic --tools from consuming a
            // subsequent positional argument. Prompts travel through stdin.
            "--tools=Bash,Read,Edit",
            "--disable-slash-commands",
            "--strict-mcp-config",
            "--mcp-config",
            EMPTY_MCP,
            "--settings",
            CLAUDE_SETTINGS,
            "--no-chrome",
        ],
        _ => unreachable!("reviewed family checked above"),
    };
    Ok(args.iter().map(|arg| (*arg).to_owned()).collect())
}

/// Validate the complete suffix, excluding the fixed policy prefix and the
/// supervisor's protected ownership configuration. Never append extras after
/// validation: native transport, model and session are the only turn options.
pub(super) fn validate_native_arguments(
    cli: TitleCli,
    exact_version: &str,
    mode: Mode,
    args: &[String],
) -> Result<(), String> {
    reviewed(cli, exact_version)?;
    let exact = |expected: &[&str]| {
        args.len() == expected.len()
            && args
                .iter()
                .zip(expected)
                .all(|(actual, expected)| actual == expected)
    };
    let allowed = match mode {
        Mode::Version => exact(&["--version"]),
        Mode::AccountTerminal => match cli {
            TitleCli::Codex => exact(&["login"]),
            TitleCli::Claude => exact(&["auth", "login"]),
            _ => false,
        },
        Mode::Collector => match cli {
            TitleCli::Codex => exact(&["login", "status"]),
            TitleCli::Claude => exact(&["auth", "status"]),
            _ => false,
        },
        Mode::Turn => match cli {
            TitleCli::Codex => exact(&["app-server", "--listen", "stdio://"]),
            TitleCli::Claude => claude_turn(args),
            _ => false,
        },
    };
    if allowed {
        Ok(())
    } else {
        Err("Native arguments differ from the reviewed managed launch; extra configuration, tools, plugins, MCP servers and positional prompts are not allowed.".into())
    }
}

fn claude_turn(args: &[String]) -> bool {
    if args.len() < CLAUDE_TRANSPORT.len()
        || !args
            .iter()
            .zip(CLAUDE_TRANSPORT)
            .all(|(arg, expected)| arg == expected)
    {
        return false;
    }
    let mut suffix = &args[CLAUDE_TRANSPORT.len()..];
    if suffix.first().is_some_and(|arg| arg == "--model") {
        if suffix.len() < 2 || !value(&suffix[1], 512) {
            return false;
        }
        suffix = &suffix[2..];
    }
    match suffix {
        [] => true,
        [flag, session] if flag == "--session-id" || flag == "--resume" => {
            // Both are concrete session IDs, never the interactive resume
            // picker, a search query, or a flag-like value.
            session.len() == 36
                && session.bytes().enumerate().all(|(i, b)| {
                    if matches!(i, 8 | 13 | 18 | 23) {
                        b == b'-'
                    } else {
                        b.is_ascii_hexdigit()
                    }
                })
        }
        _ => false,
    }
}

fn value(value: &str, limit: usize) -> bool {
    !value.is_empty()
        && value.len() <= limit
        && !value.starts_with('-')
        && !value.chars().any(char::is_control)
}
