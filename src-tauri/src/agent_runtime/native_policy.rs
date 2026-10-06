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

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn native_policy_exact_artifact_versions_and_modes() {
        for mode in [
            Mode::Version,
            Mode::Turn,
            Mode::Collector,
            Mode::AccountTerminal,
        ] {
            assert!(arguments(TitleCli::Codex, "0.160.0", mode).is_ok());
            assert!(arguments(TitleCli::Claude, "2.1.287", mode).is_ok());
            assert!(arguments(TitleCli::Codex, "0.160.1", mode).is_err());
            assert!(arguments(TitleCli::Claude, "2.1.287 (Claude Code)", mode).is_err());
            assert!(arguments(TitleCli::Pi, "1.0.1", mode).is_err());
        }
        assert!(validate_native_arguments(
            TitleCli::Codex,
            "0.160.1",
            Mode::Version,
            &owned(&["--version"])
        )
        .is_err());
    }

    #[test]
    fn native_policy_codex_globals_precede_transport_without_empty_map_claim() {
        let mut args = arguments(TitleCli::Codex, "0.160.0", Mode::Turn).unwrap();
        assert_eq!(
            args,
            owned(&[
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
                "remote_control"
            ])
        );
        let suffix = owned(&["app-server", "--listen", "stdio://"]);
        validate_native_arguments(TitleCli::Codex, "0.160.0", Mode::Turn, &suffix).unwrap();
        args.extend(suffix);
        assert_eq!(args[args.len() - 3], "app-server");
        for extra in [
            "--enable",
            "hooks",
            "--config",
            "notify=[\"sh\"]",
            "--profile",
            "unsafe",
            "prompt",
        ] {
            let mut suffix = owned(&["app-server", "--listen", "stdio://"]);
            suffix.push(extra.into());
            assert!(
                validate_native_arguments(TitleCli::Codex, "0.160.0", Mode::Turn, &suffix).is_err()
            );
        }
    }

    #[test]
    fn native_policy_claude_fixed_tools_settings_and_empty_mcp_ceiling() {
        let args = arguments(TitleCli::Claude, "2.1.287", Mode::Turn).unwrap();
        assert_eq!(
            args,
            owned(&[
                "--restricted",
                "--tools=Bash,Read,Edit",
                "--disable-slash-commands",
                "--strict-mcp-config",
                "--mcp-config",
                EMPTY_MCP,
                "--settings",
                CLAUDE_SETTINGS,
                "--no-chrome"
            ])
        );
        assert!(!args.iter().any(|arg| arg == "--tools"));
        let settings: serde_json::Value = serde_json::from_str(CLAUDE_SETTINGS).unwrap();
        for setting in [
            "disableAllHooks",
            "disableClaudeAiConnectors",
            "disableBundledSkills",
            "disableSkillShellExecution",
        ] {
            assert_eq!(settings[setting], true);
        }
        assert_eq!(
            settings["enabledPlugins"]["cc-plugin-agents-md@builtin"],
            false
        );
        assert_eq!(
            settings["enabledPlugins"]["cc-plugin-plugin-authoring@builtin"],
            false
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(EMPTY_MCP).unwrap(),
            serde_json::json!({"mcpServers": {}})
        );
    }

    #[test]
    fn native_policy_claude_model_resume_cannot_widen_or_swallow_prompt() {
        let session = "550e8400-e29b-41d4-a716-446655440000";
        for flag in ["--session-id", "--resume"] {
            let mut args = owned(CLAUDE_TRANSPORT);
            args.extend(owned(&["--model", "claude-opus-5-5", flag, session]));
            validate_native_arguments(TitleCli::Claude, "2.1.287", Mode::Turn, &args).unwrap();
            for extra in [
                "--tools=default",
                "--settings={}",
                "--mcp-config={}",
                "--plugin-dir=/tmp/plugin",
                "--dangerously-skip-permissions",
                "prompt",
            ] {
                let mut bad = args.clone();
                bad.push(extra.into());
                assert!(
                    validate_native_arguments(TitleCli::Claude, "2.1.287", Mode::Turn, &bad)
                        .is_err()
                );
            }
        }
        for suffix in [
            owned(&["--model", "--tools=default"]),
            owned(&["--resume", "--settings"]),
            owned(&["--resume"]),
            owned(&["--session-id", session, "--resume", session]),
            owned(&["--model", "x\n--tools=default"]),
        ] {
            let mut args = owned(CLAUDE_TRANSPORT);
            args.extend(suffix);
            assert!(
                validate_native_arguments(TitleCli::Claude, "2.1.287", Mode::Turn, &args).is_err()
            );
        }
    }

    #[test]
    fn native_policy_collector_terminal_version_have_no_turn_extras() {
        for (cli, version, status) in [
            (TitleCli::Codex, "0.160.0", &["login", "status"][..]),
            (TitleCli::Claude, "2.1.287", &["auth", "status"][..]),
        ] {
            validate_native_arguments(cli, version, Mode::Collector, &owned(status)).unwrap();
            validate_native_arguments(cli, version, Mode::Version, &owned(&["--version"])).unwrap();
            let login = match cli {
                TitleCli::Codex => owned(&["login"]),
                TitleCli::Claude => owned(&["auth", "login"]),
                _ => unreachable!(),
            };
            validate_native_arguments(cli, version, Mode::AccountTerminal, &login).unwrap();
            assert!(validate_native_arguments(cli, version, Mode::AccountTerminal, &[]).is_err());
            for mode in [Mode::Collector, Mode::Version, Mode::AccountTerminal] {
                let mut args = match mode {
                    Mode::Collector => owned(status),
                    Mode::Version => owned(&["--version"]),
                    _ => login.clone(),
                };
                args.extend(owned(&["--settings", "{}"]));
                assert!(validate_native_arguments(cli, version, mode, &args).is_err());
            }
            assert!(validate_native_arguments(cli, version, Mode::Turn, &owned(status)).is_err());
        }
    }
}
