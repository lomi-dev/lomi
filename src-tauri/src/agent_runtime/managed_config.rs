//! RESTRICTED launches preserve account configuration and admit only understood
//! passive settings. Inherited configuration is denied by the child sandbox;
//! immutable snapshots are checked again at the owner's dispatch fence.
use super::native_policy::{self, Mode};
use crate::{cli_catalog::TitleCli, cli_mcp::Registration};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_CONFIG: u64 = 1024 * 1024;
// Public 0.160.0 config/read schema, qualified with an owned no-account probe.
const CODEX_EFFECTIVE_KEYS: &[&str] = &[
    "agents",
    "allow_login_shell",
    "allow_symlinked_codex_home",
    "analytics",
    "approval_policy",
    "approvals_reviewer",
    "apps",
    "apps_mcp_product_sku",
    "audio",
    "auto_review",
    "background_terminal_max_timeout",
    "browser_use",
    "chatgpt_base_url",
    "check_for_update_on_startup",
    "cli_auth_credentials_store",
    "cloud",
    "compact_prompt",
    "computer_use",
    "default_permissions",
    "desktop",
    "developer_instructions",
    "disable_paste_burst",
    "experimental_compact_prompt_file",
    "experimental_realtime_start_instructions",
    "experimental_realtime_webrtc_call_base_url",
    "experimental_realtime_ws_backend_prompt",
    "experimental_realtime_ws_base_url",
    "experimental_realtime_ws_model",
    "experimental_realtime_ws_startup_context",
    "experimental_thread_store",
    "experimental_thread_store_endpoint",
    "experimental_use_unified_exec_tool",
    "features",
    "feedback",
    "file_opener",
    "forced_chatgpt_workspace_id",
    "forced_login_method",
    "ghost_snapshot",
    "goals",
    "hide_agent_reasoning",
    "history",
    "hooks",
    "include_apps_instructions",
    "include_collaboration_mode_instructions",
    "include_environment_context",
    "include_permissions_instructions",
    "instructions",
    "js_repl_node_module_dirs",
    "js_repl_node_path",
    "log_dir",
    "marketplaces",
    "mcp_enterprise_managed_auth",
    "mcp_oauth_callback_port",
    "mcp_oauth_callback_url",
    "mcp_oauth_credentials_store",
    "mcp_optional_startup_grace_ms",
    "mcp_servers",
    "memories",
    "model",
    "model_auto_compact_token_limit",
    "model_auto_compact_token_limit_scope",
    "model_catalog_json",
    "model_context_window",
    "model_instructions_file",
    "model_post_turn_compact_threshold_percent",
    "model_provider",
    "model_providers",
    "model_reasoning_effort",
    "model_reasoning_summary",
    "model_verbosity",
    "notice",
    "notify",
    "openai_base_url",
    "orchestrator",
    "oss_provider",
    "otel",
    "permissions",
    "personality",
    "plan_mode_reasoning_effort",
    "plugins",
    "profile",
    "profiles",
    "project_doc_fallback_filenames",
    "project_doc_max_bytes",
    "project_root_markers",
    "projects",
    "realtime",
    "responses_api_metadata",
    "review_model",
    "sandbox_mode",
    "sandbox_workspace_write",
    "service_tier",
    "shell_environment_policy",
    "show_raw_agent_reasoning",
    "skills",
    "sqlite_home",
    "suppress_unstable_features_warning",
    "thread_unload_delay_secs",
    "tool_output_token_limit",
    "tool_suggest",
    "tools",
    "tui",
    "web_search",
    "windows",
];

type SnapshotIdentity = (u64, u64, u32, u32, u64, i64, i64, i64, i64);

#[derive(Clone, PartialEq, Eq)]
struct Snapshot {
    path: PathBuf,
    identity: Option<SnapshotIdentity>,
    digest: Vec<u8>,
    contents: Option<Vec<u8>>,
}

pub(crate) struct ManagedConfiguration {
    cli: TitleCli,
    version: String,
    registration: Option<Registration>,
    snapshots: Vec<Snapshot>,
    protected: Vec<PathBuf>,
    blocked: Vec<PathBuf>,
}
fn reject(reason: &str) -> String {
    format!("RESTRICTED native configuration: {reason}. Original configuration was preserved.")
}
fn snapshot(path: &Path) -> Result<Snapshot, String> {
    let absent = || Snapshot {
        path: path.into(),
        identity: None,
        digest: Vec::new(),
        contents: None,
    };
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(absent()),
        Err(_) => return Err(reject("configuration cannot be inspected")),
    };
    if meta.file_type().is_symlink() || (!meta.is_file() && !meta.is_dir()) {
        return Err(reject("configuration uses a link or unsupported file type"));
    }
    let identity = |m: &fs::Metadata| {
        if m.is_dir() {
            (m.dev(), m.ino(), m.mode(), m.uid(), 0, 0, 0, 0, 0)
        } else {
            (
                m.dev(),
                m.ino(),
                m.mode(),
                m.uid(),
                m.len(),
                m.mtime(),
                m.mtime_nsec(),
                m.ctime(),
                m.ctime_nsec(),
            )
        }
    };
    let mut digest = Vec::new();
    let mut contents = None;
    if meta.is_file() {
        if meta.len() > MAX_CONFIG || meta.nlink() != 1 {
            return Err(reject("configuration is oversized or multiply linked"));
        }
        let mut file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path)
            .map_err(|_| reject("configuration cannot be opened"))?;
        if identity(
            &file
                .metadata()
                .map_err(|_| reject("configuration cannot be inspected"))?,
        ) != identity(&meta)
        {
            return Err(reject("configuration changed during inspection"));
        }
        let mut bytes = Vec::new();
        file.by_ref()
            .take(MAX_CONFIG + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| reject("configuration cannot be read"))?;
        if bytes.len() as u64 != meta.len()
            || identity(
                &file
                    .metadata()
                    .map_err(|_| reject("configuration cannot be inspected"))?,
            ) != identity(&meta)
            || identity(
                &fs::symlink_metadata(path).map_err(|_| reject("configuration disappeared"))?,
            ) != identity(&meta)
        {
            return Err(reject("configuration changed during inspection"));
        }
        digest = Sha256::digest(&bytes).to_vec();
        contents = Some(bytes);
    }
    Ok(Snapshot {
        path: path.into(),
        identity: Some(identity(&meta)),
        digest,
        contents,
    })
}
fn private(path: &Path) -> Result<(), String> {
    let m =
        fs::symlink_metadata(path).map_err(|_| reject("private account root is unavailable"))?;
    if !m.is_dir()
        || m.file_type().is_symlink()
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o077 != 0
    {
        return Err(reject(
            "account root must be an owned physical private directory",
        ));
    }
    Ok(())
}
fn passive_codex(bytes: &str) -> Result<(), String> {
    let doc = bytes
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| reject("private Codex TOML is malformed"))?;
    for (key, item) in doc.iter() {
        match key {
            "cli_auth_credentials_store" if matches!(item.as_str(),Some("file"|"keyring"|"auto")) => {},
            "model" | "model_reasoning_effort" | "model_verbosity" | "personality" if item.as_str().is_some_and(|v| !v.is_empty() && v.len()<512 && !v.chars().any(char::is_control)) => {},
            _ => return Err(reject(&format!("private Codex setting '{key}' is unsupported; hooks, helpers, MCP, provider routes, plugins, telemetry and remote execution cannot be inherited"))),
        }
    }
    Ok(())
}
fn passive_claude(bytes: &str) -> Result<(), String> {
    let value: Value = serde_json::from_str(bytes)
        .map_err(|_| reject("private Claude settings JSON is malformed"))?;
    let object = value
        .as_object()
        .ok_or_else(|| reject("private Claude settings must be an object"))?;
    for (key, value) in object {
        match key.as_str() {
            "model" | "theme"
                if value
                    .as_str()
                    .is_some_and(|s| !s.chars().any(char::is_control)) => {}
            "disableAllHooks"
            | "disableClaudeAiConnectors"
            | "disableBundledSkills"
            | "disableSkillShellExecution"
                if value == true => {}
            _ => {
                return Err(reject(&format!(
                    "private Claude setting '{key}' is unsupported in RESTRICTED mode"
                )))
            }
        }
    }
    Ok(())
}
fn passive_claude_state(bytes: &str) -> Result<(), String> {
    let value: Value = serde_json::from_str(bytes)
        .map_err(|_| reject("private Claude state JSON is malformed"))?;
    let object = value
        .as_object()
        .ok_or_else(|| reject("private Claude state must be an object"))?;
    for (key, value) in object {
        let accepted = match key.as_str() {
            "numStartups"
            | "installMethod"
            | "autoUpdates"
            | "firstStartTime"
            | "lastOnboardingVersion"
            | "hasCompletedOnboarding"
            | "userID"
            | "theme"
            | "preferredNotifChannel"
            | "hasTrustDialogAccepted"
            | "hasAcknowledgedCostThreshold" => {
                value.is_string() || value.is_number() || value.is_boolean()
            }
            "oauthAccount" => value.as_object().is_some_and(|account| {
                account.iter().all(|(key, value)| {
                    matches!(
                        key.as_str(),
                        "accountUuid"
                            | "emailAddress"
                            | "organizationUuid"
                            | "displayName"
                            | "organizationRole"
                            | "workspaceRole"
                            | "organizationName"
                            | "billingType"
                            | "hasExtraUsageEnabled"
                    ) && (value.is_string() || value.is_boolean() || value.is_null())
                })
            }),
            "projects" | "mcpServers" => {
                value.as_object().is_some_and(|entries| entries.is_empty())
                    || value.as_array().is_some_and(|entries| entries.is_empty())
            }
            _ => false,
        };
        if !accepted {
            return Err(reject(&format!(
                "private Claude state field '{key}' is unsupported in RESTRICTED mode"
            )));
        }
    }
    Ok(())
}
fn empty_configuration(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(v) => v.is_empty(),
        Value::Object(v) => v.values().all(empty_configuration),
        _ => false,
    }
}
impl ManagedConfiguration {
    pub(super) fn prepare(
        cli: TitleCli,
        version: &str,
        account_root: &Path,
        project: &Path,
        registration: Option<&Registration>,
    ) -> Result<Self, String> {
        native_policy::reviewed(cli, version)?;
        private(account_root)?;
        let root = account_root
            .canonicalize()
            .map_err(|_| reject("account root cannot be resolved"))?;
        if root != account_root {
            return Err(reject("account root must use its stable physical path"));
        }
        let project = project
            .canonicalize()
            .map_err(|_| reject("project root cannot be resolved"))?;
        codex_preferences(cli)?;
        let mut protected = Vec::new();
        let config = match cli {
            TitleCli::Codex => root.join("config.toml"),
            TitleCli::Claude => root.join("settings.json"),
            _ => unreachable!(),
        };
        let mut snapshots = vec![snapshot(&root)?];
        let state = snapshot(&config)?;
        if state.identity.is_some() {
            let m = fs::symlink_metadata(&config)
                .map_err(|_| reject("private configuration disappeared"))?;
            if !m.is_file() || m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 {
                return Err(reject("configuration must be an owned private file"));
            }
            let bytes = std::str::from_utf8(
                state
                    .contents
                    .as_deref()
                    .ok_or_else(|| reject("private configuration is not a file"))?,
            )
            .map_err(|_| reject("configuration is not UTF-8"))?;
            match cli {
                TitleCli::Codex => passive_codex(bytes)?,
                TitleCli::Claude => passive_claude(bytes)?,
                _ => unreachable!(),
            }
            if snapshot(&config)? != state {
                return Err(reject("configuration changed while parsing"));
            }
        }
        snapshots.push(state);
        protected.push(config);
        if cli == TitleCli::Claude {
            let state_path = root.join(".claude.json");
            let state = snapshot(&state_path)?;
            if state.identity.is_some() {
                let metadata = fs::symlink_metadata(&state_path)
                    .map_err(|_| reject("private Claude state disappeared"))?;
                if !metadata.is_file()
                    || metadata.uid() != unsafe { libc::geteuid() }
                    || metadata.mode() & 0o077 != 0
                {
                    return Err(reject("Claude state must be an owned private file"));
                }
                let bytes = std::str::from_utf8(
                    state
                        .contents
                        .as_deref()
                        .ok_or_else(|| reject("private Claude state is not a file"))?,
                )
                .map_err(|_| reject("private Claude state is not UTF-8"))?;
                passive_claude_state(bytes)?;
                if snapshot(&state_path)? != state {
                    return Err(reject("private Claude state changed while parsing"));
                }
            }
            snapshots.push(state);
            protected.push(state_path);
        }
        let unsupported: &[&str] = match cli {
            TitleCli::Codex => &[
                "requirements.toml",
                "hooks.json",
                "plugins",
                "managed_config.toml",
            ],
            TitleCli::Claude => &[
                "settings.local.json",
                "managed-settings.json",
                "managed-mcp.json",
                ".mcp.json",
                "plugins",
                "hooks",
            ],
            _ => unreachable!(),
        };
        for name in unsupported {
            let path = root.join(name);
            let state = snapshot(&path)?;
            if state.identity.is_some() {
                return Err(reject(&format!("private source '{name}' is unsupported")));
            }
            protected.push(path);
            snapshots.push(state);
        }
        let system: Vec<PathBuf> = match cli {
            TitleCli::Codex => vec![
                "/etc/codex/config.toml".into(),
                "/etc/codex/requirements.toml".into(),
            ],
            TitleCli::Claude => vec![
                "/Library/Application Support/ClaudeCode/managed-settings.json".into(),
                "/Library/Application Support/ClaudeCode/managed-mcp.json".into(),
                "/Library/Application Support/ClaudeCode/managed-settings.d".into(),
            ],
            _ => unreachable!(),
        };
        for path in &system {
            let state = snapshot(path)?;
            if state.identity.is_some() {
                return Err(reject(&format!(
                    "system managed source '{}' cannot be safely overridden",
                    path.display()
                )));
            }
            snapshots.push(state);
        }
        let mut blocked = system;
        for ancestor in project.ancestors() {
            for name in [".codex", ".claude", ".mcp.json"] {
                let path = ancestor.join(name);
                if !root.starts_with(&path) {
                    blocked.push(path);
                }
            }
        }
        if let Some(r) = registration {
            if !Path::new(&r.command).is_absolute()
                || r.command.chars().any(char::is_control)
                || r.args.iter().any(|v| v.chars().any(char::is_control))
            {
                return Err(reject("owned MCP registration is invalid"));
            }
        }
        Ok(Self {
            cli,
            version: version.into(),
            registration: registration.cloned(),
            snapshots,
            protected,
            blocked,
        })
    }
    pub(super) fn recheck(&self) -> Result<(), String> {
        codex_preferences(self.cli)?;
        for before in &self.snapshots {
            if snapshot(&before.path)? != *before {
                return Err(reject("configuration changed after admission"));
            }
        }
        Ok(())
    }
    pub(super) fn protected_paths(&self) -> &[PathBuf] {
        &self.protected
    }
    pub(super) fn blocked_paths(&self) -> &[PathBuf] {
        &self.blocked
    }
    pub(super) fn arguments(&self, mode: Mode) -> Result<Vec<String>, String> {
        self.recheck()?;
        let mut args = native_policy::arguments(self.cli, &self.version, mode)?;
        if self.cli == TitleCli::Codex {
            // The pinned CLI override selects file auth without reading or
            // rewriting the user's stored preference or invoking Keychain.
            args.extend(["-c".into(), "cli_auth_credentials_store=\"file\"".into()]);
        }
        if mode == Mode::Turn {
            if let Some(r) = &self.registration {
                match self.cli {
                    TitleCli::Claude => {
                        let index = args
                            .iter()
                            .position(|v| v == "--mcp-config")
                            .ok_or_else(|| reject("MCP policy prefix is missing"))?;
                        args[index + 1] =
                            json!({"mcpServers":{"lomi":{"command":r.command,"args":r.args}}})
                                .to_string();
                    }
                    TitleCli::Codex => {
                        let command = toml_edit::Value::from(r.command.clone()).to_string();
                        let array = r
                            .args
                            .iter()
                            .map(|v| toml_edit::Value::from(v.clone()))
                            .collect::<toml_edit::Array>();
                        args.extend([
                            "-c".into(),
                            format!("mcp_servers.lomi.command={command}"),
                            "-c".into(),
                            format!("mcp_servers.lomi.args={array}"),
                        ]);
                    }
                    _ => unreachable!(),
                }
            }
        }
        Ok(args)
    }
    pub(super) fn validate_codex_effective(&self, value: &Value) -> Result<(), String> {
        if self.cli != TitleCli::Codex {
            return Err(reject("effective Codex validation used for another client"));
        }
        self.recheck()?;
        let config = value
            .get("config")
            .unwrap_or(value)
            .as_object()
            .ok_or_else(|| reject("Codex effective configuration is missing"))?;
        for key in config.keys() {
            if !CODEX_EFFECTIVE_KEYS.contains(&key.as_str()) {
                return Err(reject(&format!(
                    "effective Codex field '{key}' is unrecognized"
                )));
            }
        }
        if config
            .get("cli_auth_credentials_store")
            .and_then(Value::as_str)
            != Some("file")
        {
            return Err(reject("Codex effective credential storage must be file"));
        }
        for key in [
            "apps",
            "browser_use",
            "computer_use",
            "desktop",
            "experimental_thread_store_endpoint",
            "js_repl_node_module_dirs",
            "orchestrator",
            "agents",
            "skills",
            "marketplaces",
            "cloud",
            "realtime",
            "audio",
            "mcp_enterprise_managed_auth",
            "model_instructions_file",
            "model_catalog_json",
            "js_repl_node_path",
            "experimental_compact_prompt_file",
            "experimental_realtime_ws_base_url",
            "experimental_realtime_webrtc_call_base_url",
            "openai_base_url",
        ] {
            if config.get(key).is_some_and(|v| !empty_configuration(v)) {
                return Err(reject(&format!("effective Codex '{key}' is unsupported")));
            }
        }
        if config
            .get("chatgpt_base_url")
            .is_some_and(|v| !v.is_null() && v != "https://chatgpt.com/backend-api/")
        {
            return Err(reject("effective ChatGPT provider route is unsupported"));
        }
        if let Some(features) = config.get("features").and_then(Value::as_object) {
            for key in [
                "hooks",
                "plugins",
                "remote_plugin",
                "apps",
                "remote_control",
                "network_proxy",
            ] {
                if features
                    .get(key)
                    .is_some_and(|v| !v.is_null() && v != false)
                {
                    return Err(reject(&format!(
                        "effective Codex feature '{key}' is enabled"
                    )));
                }
            }
        }
        if let Some(layers) = value.get("layers") {
            let layers = layers
                .as_array()
                .ok_or_else(|| reject("Codex configuration layers are malformed"))?;
            for layer in layers {
                let name = layer
                    .get("name")
                    .ok_or_else(|| reject("Codex configuration layer source is missing"))?;
                let accepted = match name.get("type").and_then(Value::as_str) {
                    Some("sessionFlags") => true,
                    Some("user") => {
                        name.get("file")
                            .and_then(Value::as_str)
                            .is_some_and(|p| self.protected.iter().any(|path| path == Path::new(p)))
                            && name.get("profile").is_none_or(Value::is_null)
                    }
                    Some("system") => layer.get("config").is_some_and(empty_configuration),
                    _ => false,
                };
                if !accepted {
                    return Err(reject(
                        "Codex loaded an unsupported inherited configuration layer",
                    ));
                }
            }
        }
        for key in [
            "hooks",
            "notify",
            "model_providers",
            "profiles",
            "plugins",
            "otel",
            "remote",
            "remote_exec",
            "exec_policy",
            "shell_environment_policy",
        ] {
            if let Some(v) = config.get(key) {
                if !v.is_null() && !empty_configuration(v) {
                    return Err(reject(&format!("effective Codex '{key}' is unsupported")));
                }
            }
        }
        if config
            .get("model_provider")
            .is_some_and(|v| !v.is_null() && v != "openai")
        {
            return Err(reject("effective provider route is unsupported"));
        }
        if config
            .get("analytics")
            .is_some_and(|v| v.get("enabled") == Some(&Value::Bool(true)))
        {
            return Err(reject("effective analytics is enabled"));
        }
        let expected = match &self.registration {
            Some(r) => json!({"lomi":{"command":r.command,"args":r.args}}),
            None => json!({}),
        };
        let mut actual = config.get("mcp_servers").cloned().unwrap_or(json!({}));
        if let Some(server) = actual.get_mut("lomi").and_then(Value::as_object_mut) {
            for (key, default) in [
                ("environment_id", json!("local")),
                ("enabled", json!(true)),
                ("tool_timeout_sec", Value::Null),
            ] {
                if server.get(key) == Some(&default) {
                    server.remove(key);
                }
            }
        }
        if actual != expected {
            return Err(reject(
                "effective MCP servers differ from the exact owned registration",
            ));
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn codex_preferences(cli: TitleCli) -> Result<(), String> {
    if cli != TitleCli::Codex {
        return Ok(());
    }
    use std::ffi::{c_void, CString};
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            value: *const std::ffi::c_char,
            encoding: u32,
        ) -> *const c_void;
        fn CFPreferencesAppSynchronize(app: *const c_void) -> u8;
        fn CFPreferencesAppValueIsForced(key: *const c_void, app: *const c_void) -> u8;
        fn CFPreferencesCopyAppValue(key: *const c_void, app: *const c_void) -> *const c_void;
        fn CFRelease(value: *const c_void);
    }
    unsafe {
        let string = |s: &str| {
            CFStringCreateWithCString(
                std::ptr::null(),
                CString::new(s).unwrap().as_ptr(),
                0x08000100,
            )
        };
        let app = string("com.openai.codex");
        if app.is_null() {
            return Err(reject("Codex preferences domain is unavailable"));
        }
        let mut unsupported = CFPreferencesAppSynchronize(app) == 0;
        for name in ["config_toml_base64", "requirements_toml_base64"] {
            let key = string(name);
            if key.is_null() {
                unsupported = true;
                continue;
            }
            let value = CFPreferencesCopyAppValue(key, app);
            unsupported |= CFPreferencesAppValueIsForced(key, app) != 0 || !value.is_null();
            if !value.is_null() {
                CFRelease(value);
            }
            CFRelease(key);
        }
        CFRelease(app);
        if unsupported {
            return Err(reject("Codex managed preferences are present or cannot be synchronized; CLI overrides cannot bypass managed requirements"));
        }
    }
    Ok(())
}
#[cfg(not(target_os = "macos"))]
fn codex_preferences(_: TitleCli) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn passive_configs_reject_execution_and_unknown_fields_without_rewriting() {
        passive_codex("cli_auth_credentials_store = 'file'\nmodel = 'gpt-5'\n").unwrap();
        for v in [
            "notify=['sh']",
            "[mcp_servers.x]\ncommand='sh'",
            "model_provider='custom'",
            "unknown=true",
        ] {
            assert!(passive_codex(v).is_err());
        }
        passive_claude(r#"{"disableAllHooks":true,"model":"opus"}"#).unwrap();
        for v in [
            r#"{"hooks":{}}"#,
            r#"{"env":{}}"#,
            r#"{"disableAllHooks":false}"#,
            r#"{"unknown":true}"#,
        ] {
            assert!(passive_claude(v).is_err());
        }
    }
    #[test]
    fn owned_mcp_is_injected_only_for_turns() {
        let configuration = ManagedConfiguration {
            cli: TitleCli::Claude,
            version: "2.1.287".into(),
            registration: Some(Registration {
                command: "/usr/bin/false".into(),
                args: vec!["--owned".into()],
            }),
            snapshots: Vec::new(),
            protected: Vec::new(),
            blocked: Vec::new(),
        };
        for mode in [
            Mode::Version,
            Mode::Collector,
            Mode::AccountTerminal,
            Mode::Turn,
        ] {
            let args = configuration.arguments(mode).unwrap();
            let position = args.iter().position(|v| v == "--mcp-config").unwrap();
            let mcp: Value = serde_json::from_str(&args[position + 1]).unwrap();
            assert_eq!(
                mcp,
                if mode == Mode::Turn {
                    json!({"mcpServers":{"lomi":{"command":"/usr/bin/false","args":["--owned"]}}})
                } else {
                    json!({"mcpServers":{}})
                }
            );
        }
    }
    #[test]
    fn codex_file_auth_override_preserves_keyring_and_auto_preferences() {
        for preference in ["keyring", "auto"] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("config.toml");
            let bytes = format!("cli_auth_credentials_store = '{preference}'\n");
            fs::write(&path, &bytes).unwrap();
            passive_codex(&bytes).unwrap();
            let configuration = ManagedConfiguration {
                cli: TitleCli::Codex,
                version: "0.160.0".into(),
                registration: None,
                snapshots: vec![snapshot(&path).unwrap()],
                protected: vec![path.clone()],
                blocked: vec![],
            };
            for mode in [
                Mode::Version,
                Mode::Collector,
                Mode::AccountTerminal,
                Mode::Turn,
            ] {
                let args = configuration.arguments(mode).unwrap();
                assert!(args
                    .windows(2)
                    .any(|pair| pair == ["-c", "cli_auth_credentials_store=\"file\""]));
                assert_eq!(fs::read_to_string(&path).unwrap(), bytes);
            }
            configuration
                .validate_codex_effective(
                    &json!({"config":{"cli_auth_credentials_store":"file","mcp_servers":{}}}),
                )
                .unwrap();
            assert!(configuration
                .validate_codex_effective(
                    &json!({"config":{"cli_auth_credentials_store":preference,"mcp_servers":{}}})
                )
                .is_err());
        }
    }
    #[test]
    fn private_claude_state_cannot_introduce_execution_sources() {
        passive_claude_state(r#"{"numStartups":1,"oauthAccount":{"emailAddress":"owned@example.invalid"},"projects":{}}"#).unwrap();
        for value in [
            r#"{"apiKeyHelper":"sh"}"#,
            r#"{"mcpServers":{"other":{"command":"sh"}}}"#,
            r#"{"projects":{"/tmp":{"mcpServers":{}}}}"#,
            r#"{"projects":{"/tmp":{}}}"#,
            r#"{"mcpServers":{"other":{}}}"#,
            r#"{"oauthAccount":{"command":"sh"}}"#,
        ] {
            assert!(passive_claude_state(value).is_err());
        }
    }
    #[test]
    fn bounded_snapshot_bytes_survive_growth_without_an_unbounded_parse_read() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        fs::write(&path, b"model = 'fixture'\n").unwrap();
        let captured = snapshot(&path).unwrap();
        fs::write(&path, vec![b'x'; MAX_CONFIG as usize + 1]).unwrap();
        passive_codex(std::str::from_utf8(captured.contents.as_deref().unwrap()).unwrap()).unwrap();
        assert!(snapshot(&path).is_err());
    }
    #[test]
    fn snapshots_detect_content_replacement_and_links() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config");
        let absent = snapshot(&path).unwrap();
        fs::write(&path, b"first").unwrap();
        assert!(snapshot(&path).unwrap() != absent);
        let first = snapshot(&path).unwrap();
        fs::write(&path, b"other").unwrap();
        assert!(snapshot(&path).unwrap() != first);
        fs::remove_file(&path).unwrap();
        symlink("missing", &path).unwrap();
        assert!(snapshot(&path).is_err());
    }
}
