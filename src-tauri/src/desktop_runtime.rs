//! The workspace owner is fixed for the lifetime of a desktop process.
//! A failed remote request must never be replayed by the local backend.

pub(crate) const REMOTE_DISABLED: &str =
    "Remote is disabled. Local projects, files, Git, and terminals are available.";

pub(crate) const fn shared_host_enabled() -> bool {
    cfg!(feature = "remote-host")
}

pub(crate) const fn workspace_backend() -> &'static str {
    if shared_host_enabled() {
        "shared-host"
    } else {
        "local"
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeInfo {
    workspace_backend: &'static str,
}

#[tauri::command]
pub(crate) fn desktop_runtime_info(
    window: tauri::Window,
    webview: tauri::Webview,
) -> Result<RuntimeInfo, String> {
    crate::shared_host::trusted_full_window(&window, &webview)?;
    Ok(RuntimeInfo {
        workspace_backend: workspace_backend(),
    })
}

pub(crate) fn require_shared_host() -> Result<(), String> {
    if shared_host_enabled() {
        Ok(())
    } else {
        Err(REMOTE_DISABLED.into())
    }
}

pub(crate) fn remote_command(command: &str) -> bool {
    command.starts_with("shared_host_") || command.starts_with("remote_v2_")
}

/// Old Git records do not retain a trustworthy path scope. Preserve their
/// uncertainty across backend changes without connecting to the former Host.
pub(crate) fn pending_host_git_effect(app: &tauri::AppHandle) -> Result<bool, String> {
    if crate::native_git_effects::pending_host_effect(app)? {
        return Ok(true);
    }
    crate::shared_host::pending_native_git_effect(app)
}

pub(crate) fn ensure_local_git_write_safe(
    app: &tauri::AppHandle,
    root: &str,
) -> Result<(), String> {
    let root = std::fs::canonicalize(root)
        .map_err(|_| "The selected Git repository is unavailable.".to_owned())?;
    if crate::native_file_effects::local_host_effect_conflicts(app, &[root], true)? {
        return Err("A previous Host operation is unresolved. Recover it in a Remote-enabled build before changing this repository.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_owner_matches_the_build_capability() {
        assert_eq!(
            workspace_backend(),
            if cfg!(feature = "remote-host") {
                "shared-host"
            } else {
                "local"
            }
        );
        assert_eq!(require_shared_host().is_ok(), shared_host_enabled());
    }

    #[test]
    fn remote_authority_is_separate_from_local_application_commands() {
        for command in [
            "shared_host_import_workspace",
            "shared_host_rpc",
            "remote_v2_authorize_consumer",
            "remote_v2_open_channel",
        ] {
            assert!(remote_command(command), "{command}");
        }
        for command in [
            "start_terminal",
            "save_editor_file",
            "git_commit",
            "auth_begin_login",
            "push_get_desktop_state",
            "agent_task_send",
        ] {
            assert!(!remote_command(command), "{command}");
        }
    }
}
