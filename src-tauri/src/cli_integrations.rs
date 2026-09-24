use crate::{
    cli_config, cli_mcp,
    cli_titles::{self, CliTitleConfig, TitleCli, TitleProcess},
    terminal::Terminals,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{Emitter, State, Window};

#[derive(Default)]
pub struct CliIntegrations(Mutex<HashSet<(TitleCli, Feature)>>);

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Feature {
    Notifications,
    Mcp,
    Titlebar,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureStatus {
    feature: Feature,
    configured: bool,
    path: String,
    revision: Option<String>,
    error: Option<String>,
}

#[derive(Serialize)]
pub struct CliStatus {
    cli: TitleCli,
    features: Vec<FeatureStatus>,
}

pub(crate) fn name(cli: TitleCli) -> &'static str {
    cli.name()
}

fn inspect_feature(
    feature: Feature,
    path: Result<PathBuf, String>,
    inspect: impl FnOnce(&Path, Option<&str>) -> Result<bool, String>,
) -> FeatureStatus {
    let mut status = FeatureStatus {
        feature,
        configured: false,
        path: String::new(),
        revision: None,
        error: None,
    };
    let result = (|| {
        let path = path?;
        status.path = path.to_string_lossy().into_owned();
        let source = cli_config::read(&path)?;
        status.revision = cli_config::revision(source.as_deref());
        status.configured = inspect(&path, source.as_deref())?;
        Ok::<_, String>(())
    })();
    status.error = result.err();
    status
}

fn inspect_features(
    cli: TitleCli,
    titles: Result<(PathBuf, bool), String>,
    mcp: Option<Result<PathBuf, String>>,
    registration: Result<Option<cli_mcp::Registration>, String>,
) -> Vec<FeatureStatus> {
    let disabled = titles
        .as_ref()
        .map(|(_, disabled)| *disabled)
        .unwrap_or(false);
    let path = titles.map(|(path, _)| path);
    let mut features = vec![];
    if crate::cli_notifications::supported(cli) {
        features.push(inspect_feature(
            Feature::Notifications,
            path.clone(),
            |path, _| crate::cli_notifications::inspect(cli, path),
        ));
    }
    if let Some(mcp) = mcp.filter(|_| cli_mcp::supported(cli)) {
        features.push(inspect_feature(Feature::Mcp, mcp, |_, source| {
            cli_mcp::configured(cli, source, registration?.as_ref())
        }));
    }
    if cli.supports_titles() {
        features.push(inspect_feature(Feature::Titlebar, path, |_, source| {
            cli_titles::configured(cli, source, disabled)
        }));
    }
    features
}

#[tauri::command]
pub fn inspect_cli_integrations(
    window: Window,
    app: tauri::AppHandle,
    terminals: State<'_, Terminals>,
    config: State<'_, CliTitleConfig>,
    state: State<'_, CliIntegrations>,
    id: String,
    process: TitleProcess,
) -> Result<CliStatus, String> {
    crate::files::main_window(&window)?;
    let dismissed = state.0.lock().map_err(|error| error.to_string())?.clone();
    let _guard = config.0.lock().map_err(|error| error.to_string())?;
    terminals.check_title_process(&id, process)?;
    let features = inspect_features(
        process.cli,
        cli_titles::configuration(process),
        (crate::agent_control::supported_host() && cli_mcp::supported(process.cli))
            .then(|| cli_titles::mcp_configuration(process)),
        cli_mcp::registration(&app, false),
    )
    .into_iter()
    .filter(|feature| !dismissed.contains(&(process.cli, feature.feature)))
    .collect();
    Ok(CliStatus {
        cli: process.cli,
        features,
    })
}

#[tauri::command]
pub fn dismiss_cli_integrations(
    window: Window,
    state: State<'_, CliIntegrations>,
    cli: TitleCli,
    feature: Feature,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    state
        .0
        .lock()
        .map_err(|error| error.to_string())?
        .insert((cli, feature));
    Ok(())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn enable_cli_integration(
    window: Window,
    app: tauri::AppHandle,
    terminals: State<'_, Terminals>,
    config: State<'_, CliTitleConfig>,
    id: String,
    process: TitleProcess,
    feature: Feature,
    path: String,
    revision: Option<String>,
) -> Result<String, String> {
    crate::files::main_window(&window)?;
    {
        let _guard = config.0.lock().map_err(|error| error.to_string())?;
        terminals.check_title_process(&id, process)?;
        let current = match feature {
            Feature::Mcp => cli_titles::mcp_configuration(process)?,
            _ => cli_titles::configuration(process)?.0,
        };
        if current != Path::new(&path) {
            return Err("The CLI configuration location changed. Check it again before enabling this feature.".into());
        }
        match feature {
            Feature::Titlebar => cli_titles::enable(process.cli, &current, revision.as_deref())?,
            Feature::Notifications => {
                crate::cli_notifications::enable(process.cli, &current, revision.as_deref())?;
                crate::terminal_preferences::enable_notifications(&app)?;
            }
            Feature::Mcp => {
                let registration = cli_mcp::registration(&app, true)?
                    .ok_or("Cannot prepare the Lomi MCP registration.")?;
                cli_mcp::enable(process.cli, &current, revision.as_deref(), &registration)?;
            }
        }
    }
    let _ = app.emit("cli-integrations-changed", ());
    if matches!(feature, Feature::Mcp) {
        crate::agent_control::enable_for_cli_setup(&app)
            .await
            .map_err(|error| {
                format!("Lomi MCP was registered, but its server could not start: {error}")
            })?;
    }
    Ok(match feature {
        Feature::Titlebar if process.cli == TitleCli::Agy => "Titlebar enabled. Enter /title on in Antigravity CLI, or restart it, to apply the change.".into(),
        Feature::Mcp => format!("Lomi MCP is registered for {}. Restart or reload the CLI, approve Lomi in the client if prompted, then pair in Settings → Agent control.", name(process.cli)),
        _ => format!("{} configured. Restart the CLI and resume your conversation to apply the change.", name(process.cli)),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpClientStatus {
    cli: TitleCli,
    manual_reason: Option<&'static str>,
    notice: Option<&'static str>,
    name: &'static str,
    configured: bool,
    path: String,
    revision: Option<String>,
    error: Option<String>,
}

fn settings(window: &Window) -> Result<(), String> {
    if window.label() == "settings" {
        Ok(())
    } else {
        Err("MCP client installation requires Settings.".into())
    }
}

fn default_mcp_path(cli: TitleCli) -> Result<PathBuf, String> {
    let home = crate::shell::home();
    let path = match cli {
        TitleCli::Codex => std::env::var_os("CODEX_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"))
            .join("config.toml"),
        TitleCli::Claude => {
            if std::env::var_os("CLAUDE_CONFIG_DIR").is_some_and(|value| !value.is_empty()) {
                return Err("Claude Code uses a custom configuration directory. Use claude mcp add --scope user for this profile.".into());
            }
            home.join(".claude.json")
        }
        TitleCli::Cursor => home.join(".cursor/mcp.json"),
        TitleCli::Agy => home.join(".gemini/config/mcp_config.json"),
        _ => {
            let entries: Vec<Vec<u8>> = std::env::vars_os()
                .map(|(key, value)| {
                    #[cfg(unix)]
                    {
                        use std::os::unix::ffi::OsStrExt;
                        [key.as_bytes(), b"=", value.as_bytes()].concat()
                    }
                    #[cfg(not(unix))]
                    {
                        format!("{}={}", key.to_string_lossy(), value.to_string_lossy())
                            .into_bytes()
                    }
                })
                .collect();
            return cli_mcp::configuration_path(
                cli,
                &entries.iter().map(Vec::as_slice).collect::<Vec<_>>(),
            );
        }
    };
    cli_mcp::resolve(&path)
}

#[tauri::command]
pub fn inspect_mcp_clients(
    window: Window,
    app: tauri::AppHandle,
    config: State<'_, CliTitleConfig>,
) -> Result<Vec<McpClientStatus>, String> {
    settings(&window)?;
    let _guard = config.0.lock().map_err(|error| error.to_string())?;
    Ok(TitleCli::MCP_CLIENTS
        .into_iter()
        .map(|cli| {
            if let Some(reason) = cli_mcp::manual_reason(cli) {
                return McpClientStatus {
                    cli,
                    name: name(cli),
                    manual_reason: Some(reason),
                    notice: None,
                    configured: false,
                    path: String::new(),
                    revision: None,
                    error: None,
                };
            }
            let feature = inspect_feature(Feature::Mcp, default_mcp_path(cli), |_, source| {
                cli_mcp::configured(cli, source, cli_mcp::registration(&app, false)?.as_ref())
            });
            McpClientStatus {
                cli,
                manual_reason: None,
                notice: matches!(cli, TitleCli::Hermes | TitleCli::Goose | TitleCli::Continue)
                    .then_some(
                    "YAML is reformatted and comments are removed; the original file is backed up.",
                ),
                name: name(cli),
                configured: feature.configured,
                path: feature.path,
                revision: feature.revision,
                error: feature.error,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn install_mcp_client(
    window: Window,
    app: tauri::AppHandle,
    config: State<'_, CliTitleConfig>,
    cli: TitleCli,
    path: String,
    revision: Option<String>,
) -> Result<(), String> {
    settings(&window)?;
    {
        let _guard = config.0.lock().map_err(|error| error.to_string())?;
        let current = default_mcp_path(cli)?;
        if current != Path::new(&path) {
            return Err(
                "The CLI configuration location changed. Refresh this page before installing."
                    .into(),
            );
        }
        let registration = cli_mcp::registration(&app, true)?
            .ok_or("Cannot prepare the Lomi MCP registration.")?;
        cli_mcp::enable(cli, &current, revision.as_deref(), &registration)?;
    }
    let _ = app.emit("cli-integrations-changed", ());
    crate::agent_control::enable_for_cli_setup(&app)
        .await
        .map_err(|error| {
            format!("The client was configured, but the MCP server could not start: {error}")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broken_title_location_does_not_hide_the_separate_mcp_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let mcp = temp.path().join("mcp.json");
        std::fs::write(&mcp, "{}").unwrap();
        let features = inspect_features(
            TitleCli::Cursor,
            Err("The title configuration directory is inaccessible.".into()),
            Some(Ok(mcp.clone())),
            Ok(None),
        );
        let mcp_status = features
            .iter()
            .find(|f| matches!(f.feature, Feature::Mcp))
            .unwrap();
        assert!(mcp_status.error.is_none());
        assert_eq!(mcp_status.path, mcp.to_string_lossy());
        assert!(mcp_status.revision.is_some());
        assert!(!mcp_status.configured);
        assert!(features
            .iter()
            .find(|f| matches!(f.feature, Feature::Titlebar))
            .unwrap()
            .error
            .is_some());

        let title = temp.path().join("cli-config.json");
        std::fs::write(&title, r#"{"display":{"showStatusIndicators":true}}"#).unwrap();
        let features = inspect_features(
            TitleCli::Cursor,
            Ok((title, false)),
            Some(Err("MCP location unavailable".into())),
            Ok(None),
        );
        assert!(
            features
                .iter()
                .find(|f| matches!(f.feature, Feature::Titlebar))
                .unwrap()
                .configured
        );
        assert!(features
            .iter()
            .find(|f| matches!(f.feature, Feature::Mcp))
            .unwrap()
            .error
            .is_some());
    }
    #[test]
    fn unsupported_features_are_never_offered_for_new_cli_clients() {
        let features = inspect_features(
            TitleCli::Kilo,
            Err("No title setup".into()),
            Some(Ok(PathBuf::from("/not-created/kilo.jsonc"))),
            Ok(None),
        );
        assert_eq!(features.len(), 1);
        assert!(matches!(features[0].feature, Feature::Mcp));
        let features = inspect_features(TitleCli::Pi, Err("No title setup".into()), None, Ok(None));
        assert!(features.is_empty());
        for cli in TitleCli::MCP_CLIENTS {
            assert_ne!(
                cli_mcp::supported(cli),
                cli_mcp::manual_reason(cli).is_some(),
                "{cli:?}"
            );
        }
    }
}
