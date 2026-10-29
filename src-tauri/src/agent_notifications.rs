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
    if window.label() != "main" {
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
    pane_id: Option<String>,
) -> Result<bool, String> {
    main_window(&window)?;
    if matches!(kind, NotificationKind::Info) {
        return Err("Info notifications are reserved for application events.".into());
    }
    let terminals = app.state::<crate::terminal::Terminals>();
    if context.chars().count() > 300 || context.chars().any(char::is_control) {
        return Err("Invalid notification context.".into());
    }
    if pane_id
        .as_deref()
        .is_some_and(|pane_id| !crate::notifications::valid_id(pane_id))
    {
        return Err("Invalid notification pane ID.".into());
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
        (_, NotificationKind::Info) => unreachable!("Info was rejected before delivery"),
    };
    let agent = source.agent(&terminals, session_id.as_deref());
    let (requested, recorded) = crate::push::terminal_local::deliver(
        &app,
        &window,
        kind,
        agent,
        pane_id.as_deref(),
        title,
        &context,
    )?;
    recorded?;
    forwarded?;
    Ok(requested)
}
