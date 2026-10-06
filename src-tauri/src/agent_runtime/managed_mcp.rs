//! Private MCP launch metadata and the explicit native task observation ceiling.
//! Tool grants still apply. Native peers never gain global application authority.
use crate::cli_mcp::Registration;
use lomi_control_protocol::{control::Request, ErrorCode};
use std::path::{Path, PathBuf};
use tauri::AppHandle;

pub(super) fn registration(app: &AppHandle, storage_root: &Path) -> Result<Registration, String> {
    #[cfg(target_os = "macos")]
    {
        let mut registration = crate::cli_mcp::registration(app, true)?
            .ok_or("The authenticated Lomi MCP registration is unavailable.")?;
        registration.command = super::host_boundary::staged_worker(storage_root)?
            .to_str()
            .ok_or("The private MCP worker path is not UTF-8.")?
            .to_owned();
        discovery_read_paths(&registration)?;
        Ok(registration)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, storage_root);
        Err("Managed native MCP is unavailable on this platform.".into())
    }
}

/// Only this public, signed discovery document is read by the worker. The public
/// signing key travels as an argument; the private broker signing key is excluded.
pub(super) fn discovery_read_paths(registration: &Registration) -> Result<Vec<PathBuf>, String> {
    if registration.args.len() != 5
        || registration.args[0] != "--mcp"
        || registration.args[1] != "--discovery-file"
        || registration.args[3] != "--discovery-key"
        || registration.args[4].is_empty()
    {
        return Err("The private MCP registration differs from the reviewed entry point.".into());
    }
    let path = PathBuf::from(&registration.args[2]);
    if !path.is_absolute()
        || path.components().any(|c| {
            !matches!(
                c,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
        || path.file_name() != Some(std::ffi::OsStr::new(lomi_control_core::discovery::FILE))
    {
        return Err("The private MCP discovery path is invalid.".into());
    }
    Ok(vec![path])
}

/// Returns the opaque workspace alias only for tools whose implementation uses
/// descriptor-relative bounded project reads or the fixed read-only Git worker.
/// No mutation, terminal, browser, Chat, settings or global event tool is eligible.
pub(super) fn observation_workspace(request: &Request) -> Result<Option<&str>, ErrorCode> {
    use lomi_control_core::project_files::validate_relative;
    let relative = |value: &str, allow_empty: bool| {
        if value.is_empty() && allow_empty {
            Ok(())
        } else {
            validate_relative(value)
        }
    };
    let alias = match request {
        Request::Status(_) => return Ok(None),
        Request::Connect(input) => &input.workspace_id,
        Request::FilesRead(input) => {
            relative(&input.relative_path, false)?;
            &input.workspace_id
        }
        Request::FilesList(input) => {
            relative(&input.relative_directory, true)?;
            &input.workspace_id
        }
        Request::FilesSearch(input) => {
            relative(&input.relative_directory, true)?;
            &input.workspace_id
        }
        Request::GitStatus(input) => {
            relative(&input.repository_relative, true)?;
            &input.workspace_id
        }
        Request::GitDiff(input) => {
            relative(&input.repository_relative, true)?;
            relative(&input.relative_path, false)?;
            &input.workspace_id
        }
        Request::GitHistory(input) => {
            relative(&input.repository_relative, true)?;
            &input.workspace_id
        }
        Request::GitCommit(input) => {
            relative(&input.repository_relative, true)?;
            &input.workspace_id
        }
        Request::GitRemotes(input) => {
            relative(&input.repository_relative, true)?;
            &input.workspace_id
        }
        _ => return Err(ErrorCode::ScopeDenied),
    };
    if alias.is_empty() || alias.len() > 512 || alias.chars().any(char::is_control) {
        return Err(ErrorCode::ScopeDenied);
    }
    Ok(Some(alias))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(tool: &str, arguments: serde_json::Value) -> Request {
        serde_json::from_value(serde_json::json!({"tool":tool,"arguments":arguments})).unwrap()
    }
    #[test]
    fn ceiling_accepts_only_bounded_project_observations() {
        let read = request(
            "lomi_files_read",
            serde_json::json!({"workspaceId":"opaque","relativePath":"src/main.rs"}),
        );
        assert_eq!(observation_workspace(&read).unwrap(), Some("opaque"));
        let list = request(
            "lomi_files_list",
            serde_json::json!({"workspaceId":"opaque"}),
        );
        assert_eq!(observation_workspace(&list).unwrap(), Some("opaque"));
        let history = request(
            "lomi_git_history",
            serde_json::json!({"workspaceId":"opaque"}),
        );
        assert_eq!(observation_workspace(&history).unwrap(), Some("opaque"));
        for path in [
            "../other",
            "/private/key",
            ".git/config",
            ".env",
            "src/../key",
        ] {
            let read = request(
                "lomi_files_read",
                serde_json::json!({"workspaceId":"opaque","relativePath":path}),
            );
            assert_eq!(
                observation_workspace(&read).err(),
                Some(ErrorCode::ScopeDenied)
            );
        }
        let input = request(
            "lomi_git_open",
            serde_json::json!({"workspaceId":"opaque","view":{"type":"commit","commit":"abc"},"expectedRevision":"r","retryEpoch":"e","requestKey":"k"}),
        );
        assert_eq!(
            observation_workspace(&input).err(),
            Some(ErrorCode::ScopeDenied)
        );
    }
    #[test]
    fn registration_exposes_only_public_discovery_document() {
        let registration = Registration {
            command: "/private/worker".into(),
            args: vec![
                "--mcp".into(),
                "--discovery-file".into(),
                format!("/private/control/{}", lomi_control_core::discovery::FILE),
                "--discovery-key".into(),
                "public-key".into(),
            ],
        };
        assert_eq!(discovery_read_paths(&registration).unwrap().len(), 1);
        let mut extra = registration.clone();
        extra.args.push("--unsafe".into());
        assert!(discovery_read_paths(&extra).is_err());
        let mut relative = registration;
        relative.args[2] = "../discovery.json".into();
        assert!(discovery_read_paths(&relative).is_err());
    }
}
