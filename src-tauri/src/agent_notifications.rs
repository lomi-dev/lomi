use crate::{
    cli_config, cli_titles::CliTitleConfig, files::main_window, notifications::NotificationKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tauri::{Emitter, Manager, State, Window};

const EVENTS: [(&str, &str); 3] = [
    ("UserPromptSubmit", "working"),
    ("Notification", "attention"),
    ("Stop", "finished"),
];

fn command(signal: &str) -> String {
    format!(
        r#"[ "$TERM_PROGRAM" = "Lomi" ] && printf '%s' '{{"terminalSequence":"\u001b]777;notify;Lomi;claude;{signal}\u0007"}}' || true"#
    )
}

fn configuration_path() -> Result<PathBuf, String> {
    let directory = std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::shell::home().join(".claude"));
    if !directory.is_absolute() {
        return Err("CLAUDE_CONFIG_DIR must be an absolute path for notification setup.".into());
    }
    resolve_path(&directory.join("settings.json"))
}

fn resolve_path(path: &Path) -> Result<PathBuf, String> {
    let mut ancestor = path;
    let mut missing = Vec::new();
    loop {
        match ancestor.symlink_metadata() {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(
                    ancestor
                        .file_name()
                        .ok_or("Invalid Claude configuration path.")?,
                );
                ancestor = ancestor
                    .parent()
                    .ok_or("Invalid Claude configuration path.")?;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    let mut resolved = ancestor.canonicalize().map_err(|error| error.to_string())?;
    for component in missing.iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

fn merge(source: Option<&str>) -> Result<Value, String> {
    let mut data: Value = serde_json::from_str(source.unwrap_or("{}"))
        .map_err(|_| "Claude configuration is not valid JSON. The file was left intact.")?;
    let root = data
        .as_object_mut()
        .ok_or("Claude configuration must be an object.")?;
    if root.get("disableAllHooks").and_then(Value::as_bool) == Some(true) {
        return Err("Claude Code has disabled all hooks. Enable hooks in Claude Code before configuring notifications.".into());
    }
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("Claude hooks must be an object. The file was left intact.")?;
    for (event, signal) in EVENTS {
        let groups = hooks
            .entry(event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or("Claude hook groups must be arrays. The file was left intact.")?;
        let command = command(signal);
        let already_configured = groups.iter().any(|group| {
            let matcher = group.get("matcher").and_then(Value::as_str).unwrap_or("");
            (matcher.is_empty() || matcher == "*")
                && group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|hooks| {
                        hooks.iter().any(|hook| {
                            hook["type"] == "command"
                                && hook["command"] == command
                                && hook["async"] != true
                        })
                    })
        });
        if !already_configured {
            groups.push(json!({"hooks": [{"type": "command", "command": command}]}));
        }
    }
    Ok(data)
}

#[derive(Serialize)]
pub(crate) struct NotificationSetup {
    pub(crate) path: String,
    pub(crate) revision: Option<String>,
    pub(crate) configured: bool,
}

pub(crate) fn inspect(path: &Path) -> Result<NotificationSetup, String> {
    let source = cli_config::read(path)?;
    let merged = merge(source.as_deref())?;
    let configured = source
        .as_deref()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .is_some_and(|data| data == merged);
    Ok(NotificationSetup {
        path: path.to_string_lossy().into_owned(),
        revision: cli_config::revision(source.as_deref()),
        configured,
    })
}

pub(crate) fn enable(path: &Path, expected: Option<&str>) -> Result<(), String> {
    let source = cli_config::read(path)?;
    if cli_config::revision(source.as_deref()).as_deref() != expected {
        return Err(
            "Claude configuration changed. Review it again before enabling notifications.".into(),
        );
    }
    let data = merge(source.as_deref())?;
    if source
        .as_deref()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .is_some_and(|previous| previous == data)
    {
        return Ok(());
    }
    let output = format!(
        "{}\n",
        serde_json::to_string_pretty(&data).map_err(|error| error.to_string())?
    );
    cli_config::write(path, source.as_deref(), output)
}

#[tauri::command]
pub fn request_agent_notification_setup(
    window: Window,
    app: tauri::AppHandle,
) -> Result<(), String> {
    if window.label() != "settings" {
        return Err("Notification setup can only be requested from settings.".into());
    }
    let main = app.get_window("main").ok_or("The workspace is not open.")?;
    main.unminimize().map_err(|error| error.to_string())?;
    main.show().map_err(|error| error.to_string())?;
    main.set_focus().map_err(|error| error.to_string())?;
    app.emit_to("main", "agent-notification-setup", ())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn inspect_agent_notifications(
    window: Window,
    state: State<'_, CliTitleConfig>,
) -> Result<NotificationSetup, String> {
    main_window(&window)?;
    let _guard = state.0.lock().map_err(|error| error.to_string())?;
    inspect(&configuration_path()?)
}

#[tauri::command]
pub fn enable_agent_notifications(
    window: Window,
    state: State<'_, CliTitleConfig>,
    path: String,
    revision: Option<String>,
) -> Result<(), String> {
    main_window(&window)?;
    let _guard = state.0.lock().map_err(|error| error.to_string())?;
    let current = configuration_path()?;
    if current != Path::new(&path) {
        return Err("The Claude configuration location changed. Review it again.".into());
    }
    enable(&current, revision.as_deref())
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationSource {
    #[default]
    Claude,
    Terminal,
}

impl NotificationSource {
    fn qualify_cloud(
        &self,
        terminals: &crate::terminal::Terminals,
        session: Option<&str>,
        id: Option<&str>,
    ) -> Result<(), String> {
        // Both source labels are presentation only. Native session ownership is
        // required even for Claude; renderer-provided attribution grants nothing.
        terminals.validate_notification_session(session)?;
        crate::push::terminal::validate_id(id.ok_or("Notification identity is required.")?)
    }
    fn agent(
        self,
        terminals: &crate::terminal::Terminals,
        session_id: Option<&str>,
    ) -> Option<crate::cli_catalog::TitleCli> {
        match self {
            Self::Claude => Some(crate::cli_catalog::TitleCli::Claude),
            Self::Terminal => terminals.notification_agent(session_id),
        }
    }
}

#[tauri::command]
pub fn notify_agent(
    window: Window,
    app: tauri::AppHandle,
    kind: NotificationKind,
    context: String,
    source: Option<NotificationSource>,
    session_id: Option<String>,
    notification_id: Option<String>,
) -> Result<bool, String> {
    main_window(&window)?;
    let terminals = app.state::<crate::terminal::Terminals>();
    if context.chars().count() > 300 || context.chars().any(char::is_control) {
        return Err("Invalid notification context.".into());
    }
    let source = source.unwrap_or_default();
    // OSC9/777 signals are attention events, not proof of task completion.
    // Cloud failures must not suppress local delivery.
    let forwarded = (|| {
        source.qualify_cloud(
            &terminals,
            session_id.as_deref(),
            notification_id.as_deref(),
        )?;
        crate::push::terminal::observe(
            &app,
            session_id.as_deref().unwrap(),
            notification_id.as_deref().unwrap(),
        )
    })();
    let title = match (&source, kind) {
        (NotificationSource::Claude, NotificationKind::Attention) => "Claude Code needs your input",
        (NotificationSource::Claude, NotificationKind::Finished) => {
            "Claude Code finished responding"
        }
        (NotificationSource::Terminal, NotificationKind::Attention) => "Agent needs your input",
        (NotificationSource::Terminal, NotificationKind::Finished) => "Agent finished responding",
    };
    let agent = source.agent(&terminals, session_id.as_deref());
    let (requested, recorded) =
        crate::push::terminal_local::deliver(&app, &window, kind, agent, title, &context)?;
    #[cfg(feature = "native-smoke")]
    if std::env::var_os("LOMI_NOTIFICATION_SMOKE_DIRECTORY").is_some() {
        let _ = app.emit_to(
            "main",
            "notification-smoke-result",
            json!({"requested": requested, "context": context, "title": title}),
        );
    }
    recorded?;
    forwarded?;
    Ok(requested)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn both_signal_sources_require_native_ownership_for_cloud() {
        let terminals = crate::terminal::Terminals::default();
        for source in [NotificationSource::Claude, NotificationSource::Terminal] {
            assert!(source
                .qualify_cloud(
                    &terminals,
                    None,
                    Some("12345678-1234-4234-8234-123456789abc")
                )
                .is_err());
            assert!(source
                .qualify_cloud(
                    &terminals,
                    Some("renderer-spoof"),
                    Some("12345678-1234-4234-8234-123456789abc")
                )
                .is_err());
        }
    }

    #[test]
    fn attributes_known_claude_sources_and_leaves_missing_terminals_unknown() {
        let terminals = crate::terminal::Terminals::default();
        assert_eq!(
            NotificationSource::default().agent(&terminals, None),
            Some(crate::cli_catalog::TitleCli::Claude)
        );
        assert_eq!(
            NotificationSource::Claude.agent(&terminals, Some("missing")),
            Some(crate::cli_catalog::TitleCli::Claude)
        );
        assert_eq!(NotificationSource::Terminal.agent(&terminals, None), None);
        assert_eq!(
            NotificationSource::Terminal.agent(&terminals, Some("missing")),
            None
        );
    }

    #[test]
    fn merges_hooks_idempotently_without_removing_user_configuration() {
        let source = json!({"env":{"CUSTOM":"keep"}, "hooks":{
            "Stop":[{"hooks":[{"type":"command","command":"echo user-hook"}],"matcher":"*"}],
            "PreToolUse":[{"hooks":[{"type":"command","command":"echo validate"}]}]
        }});
        let merged = merge(Some(&source.to_string())).unwrap();
        assert_eq!(merged["env"], source["env"]);
        assert_eq!(merged["hooks"]["PreToolUse"], source["hooks"]["PreToolUse"]);
        assert_eq!(merged["hooks"]["Stop"][0], source["hooks"]["Stop"][0]);
        assert_eq!(merged["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(merge(Some(&merged.to_string())).unwrap(), merged);
    }

    #[test]
    fn atomic_install_preserves_backup_and_rejects_conflicts_and_invalid_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let source = "{\r\n  \"env\": {\"CUSTOM\":\"keep\"}\r\n}\r\n";
        fs::write(&path, source).unwrap();
        let before = inspect(&path).unwrap();
        assert!(!before.configured);
        enable(&path, before.revision.as_deref()).unwrap();
        assert!(inspect(&path).unwrap().configured);
        let installed = fs::read_to_string(&path).unwrap();
        assert!(installed.contains("\r\n"));
        let backups: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains("lomi-backup")
            })
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read_to_string(&backups[0]).unwrap(), source);
        assert!(enable(&path, before.revision.as_deref()).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), installed);
        enable(&path, inspect(&path).unwrap().revision.as_deref()).unwrap();
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
        for invalid in [
            "broken",
            "[]",
            "{\"hooks\":[]}",
            "{\"hooks\":{\"Stop\":false}}",
            "{\"disableAllHooks\":true}",
        ] {
            fs::write(&path, invalid).unwrap();
            assert!(inspect(&path).is_err());
            assert!(enable(&path, cli_config::revision(Some(invalid)).as_deref()).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), invalid);
        }
    }

    #[test]
    fn installs_into_new_directories_and_rejects_stale_creation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude/settings.json");
        let resolved = resolve_path(&path).unwrap();
        assert!(!inspect(&resolved).unwrap().configured);
        enable(&resolved, None).unwrap();
        assert!(inspect(&resolved).unwrap().configured);
        assert!(enable(&resolved, None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn hook_returns_terminal_sequence_only_inside_lomi() {
        for (_, signal) in EVENTS {
            for terminal in ["Lomi", "Other"] {
                let output = std::process::Command::new("sh")
                    .args(["-c", &command(signal)])
                    .env("TERM_PROGRAM", terminal)
                    .output()
                    .unwrap();
                assert!(output.status.success());
                assert!(output.stderr.is_empty());
                if terminal == "Other" {
                    assert!(output.stdout.is_empty());
                } else {
                    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
                    assert_eq!(
                        output["terminalSequence"],
                        format!("\x1b]777;notify;Lomi;claude;{signal}\x07")
                    );
                }
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn installation_preserves_symlinks_and_permissions() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let target = dir.path().join("target.json");
        fs::write(&target, "{}").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        symlink(&target, &path).unwrap();
        let resolved = resolve_path(&path).unwrap();
        enable(&resolved, inspect(&resolved).unwrap().revision.as_deref()).unwrap();
        assert!(path.is_symlink());
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        fs::set_permissions(&target, fs::Permissions::from_mode(0o440)).unwrap();
        fs::write(dir.path().join("locked.json"), "{}").unwrap();
        let locked = dir.path().join("locked.json");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o440)).unwrap();
        assert!(enable(&locked, inspect(&locked).unwrap().revision.as_deref()).is_err());
        assert_eq!(fs::read_to_string(&locked).unwrap(), "{}");
    }
}
