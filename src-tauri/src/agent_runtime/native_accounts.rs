//! Persistent namespaces for unmodified native account login. The caller must
//! clear the child environment before applying this Launch, version-qualify the
//! resolved CLI, and repeat executable admission under its final owner fence.
//! Native OAuth/browser flows stay inside the native client; no gateway token,
//! shared HOME credentials, API-key override or subscription proxy is injected.
use super::environment::Launch;
use crate::cli_catalog::TitleCli;
use std::{
    ffi::OsString,
    fs,
    io::{Read, Write},
    path::Path,
};

const AGY_SHA256: &str = "7dca095cfc1df2c057a385ed88a76c7ba98dc103258a80be87a8f42e484cb3aa";
fn isolation() -> String {
    "The native account requires an owned private profile directory.".into()
}
pub(crate) fn available(cli: TitleCli) -> bool {
    cfg!(unix)
        && match cli {
            TitleCli::Claude | TitleCli::Codex => {
                cfg!(all(target_os = "macos", target_arch = "aarch64"))
            }
            TitleCli::Kimi | TitleCli::Kilo | TitleCli::Opencode => true,
            TitleCli::Pi => cfg!(all(target_os = "macos", target_arch = "aarch64")),
            TitleCli::Grok => super::grok_artifact::compiled_available(),
            TitleCli::Agy => cfg!(all(target_os = "macos", target_arch = "aarch64")),
            _ => false,
        }
}
pub(crate) fn check_private_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                fs::DirBuilder::new()
                    .mode(0o700)
                    .create(path)
                    .map_err(|_| isolation())?;
            }
            #[cfg(not(unix))]
            return Err(isolation());
        }
        Err(_) => return Err(isolation()),
        Ok(_) => {}
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| isolation())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(isolation());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(isolation());
        }
    }
    Ok(())
}
fn environment(launch: &mut Launch, name: &str, value: impl Into<OsString>) {
    launch.environment.push((name.into(), value.into()));
}
const AGY_ASK: &[&str] = &[
    "write_file(*)",
    "command(*)",
    "unsandboxed(*)",
    "mcp(*)",
    "read_url(*)",
    "execute_url(*)",
];

// These rules retain Antigravity's native PTY confirmations. They do not route
// native tool effects through Lomi's structured approval supervisor.
fn agy_policy(value: &mut serde_json::Value) -> Result<(), String> {
    use serde_json::{json, Value};
    let settings = value.as_object_mut().ok_or_else(isolation)?;
    if settings
        .get("agentMode")
        .is_some_and(|mode| !mode.is_string())
    {
        return Err(isolation());
    }
    let permissions = settings.entry("permissions").or_insert_with(|| json!({}));
    let permissions = permissions.as_object_mut().ok_or_else(isolation)?;
    for name in ["deny", "ask", "allow"] {
        if let Some(rules) = permissions.get(name) {
            let rules = rules.as_array().ok_or_else(isolation)?;
            if rules.iter().any(|rule| !rule.is_string()) {
                return Err(isolation());
            }
        }
    }
    let ask = permissions.entry("ask").or_insert_with(|| json!([]));
    let ask = ask.as_array_mut().ok_or_else(isolation)?;
    for rule in AGY_ASK {
        if !ask.iter().any(|existing| existing.as_str() == Some(*rule)) {
            ask.push(Value::String((*rule).into()));
        }
    }
    // Deny > Ask > Allow in the pinned client. Preserve stricter Deny rules,
    // existing Ask rules, Allow rules and unrelated account/UI settings.
    settings.insert("agentMode".into(), Value::String("default".into()));
    Ok(())
}

#[cfg(unix)]
fn agy_settings(root: &Path) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    const MAX_SETTINGS: u64 = 1024 * 1024;
    let parent = root.join(".gemini");
    check_private_directory(&parent)?;
    let parent = parent.join("antigravity-cli");
    check_private_directory(&parent)?;
    let path = parent.join("settings.json");
    let same = |a: &fs::Metadata, b: &fs::Metadata| {
        (
            a.dev(),
            a.ino(),
            a.len(),
            a.uid(),
            a.mode(),
            a.nlink(),
            a.mtime(),
            a.mtime_nsec(),
            a.ctime(),
            a.ctime_nsec(),
        ) == (
            b.dev(),
            b.ino(),
            b.len(),
            b.uid(),
            b.mode(),
            b.nlink(),
            b.mtime(),
            b.mtime_nsec(),
            b.ctime(),
            b.ctime_nsec(),
        )
    };
    let before = match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o077 != 0
                || metadata.nlink() != 1
                || metadata.len() > MAX_SETTINGS
            {
                return Err(isolation());
            }
            Some(metadata)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err(isolation()),
    };
    let mut value = if let Some(before) = &before {
        let mut file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .map_err(|_| isolation())?;
        if !same(before, &file.metadata().map_err(|_| isolation())?) {
            return Err(isolation());
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(MAX_SETTINGS + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| isolation())?;
        if bytes.len() as u64 > MAX_SETTINGS
            || !same(before, &file.metadata().map_err(|_| isolation())?)
            || !same(
                before,
                &fs::symlink_metadata(&path).map_err(|_| isolation())?,
            )
        {
            return Err(isolation());
        }
        serde_json::from_slice(&bytes).map_err(|_| isolation())?
    } else {
        serde_json::json!({})
    };
    let original = value.clone();
    agy_policy(&mut value)?;
    if before.is_some() && original == value {
        return Ok(());
    }
    let bytes = serde_json::to_vec_pretty(&value).map_err(|_| isolation())?;
    if bytes.len() as u64 > MAX_SETTINGS {
        return Err(isolation());
    }
    let temporary = parent.join(format!(".lomi-settings-{}", super::new_id()?));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&temporary)
        .map_err(|_| isolation())?;
    let result = (|| {
        file.write_all(&bytes).map_err(|_| isolation())?;
        file.write_all(b"\n").map_err(|_| isolation())?;
        file.sync_all().map_err(|_| isolation())?;
        check_private_directory(&root.join(".gemini"))?;
        check_private_directory(&parent)?;
        match (&before, fs::symlink_metadata(&path)) {
            (Some(before), Ok(current)) if same(before, &current) => {}
            (None, Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(isolation()),
        }
        fs::rename(&temporary, &path).map_err(|_| isolation())?;
        fs::File::open(&parent)
            .and_then(|parent| parent.sync_all())
            .map_err(|_| isolation())
    })();
    let _ = fs::remove_file(temporary);
    result
}
#[cfg(not(unix))]
fn agy_settings(_: &Path) -> Result<(), String> {
    Err(isolation())
}
pub(crate) fn prepare(profile_directory: &Path, cli: TitleCli) -> Result<Launch, String> {
    if !available(cli) {
        return Err("Native account login is unavailable for this CLI artifact/platform.".into());
    }
    // Require the private profiles parent created by the router's owner.
    // Retain this root for existing Claude/Codex credential namespace compatibility.
    let parent = profile_directory.parent().ok_or_else(isolation)?;
    let metadata = fs::symlink_metadata(parent).map_err(|_| isolation())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(isolation());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(isolation());
        }
    }
    check_private_directory(profile_directory)?;
    let root = profile_directory.canonicalize().map_err(|_| isolation())?;
    check_private_directory(&root)?;
    let mut launch = Launch {
        arguments: Vec::new(),
        environment: Vec::new(),
        private_home: root.clone(),
    };
    for name in ["HOME", "USERPROFILE", "CFFIXED_USER_HOME"] {
        environment(&mut launch, name, root.as_os_str());
    }
    for (variable, directory) in [
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_STATE_HOME", "state"),
        ("XDG_CACHE_HOME", "cache"),
        ("TMPDIR", "tmp"),
        ("TMP", "tmp"),
        ("TEMP", "tmp"),
    ] {
        let path = root.join(directory);
        check_private_directory(&path)?;
        environment(&mut launch, variable, path.into_os_string());
    }
    match cli {
        TitleCli::Claude => {
            environment(&mut launch, "CLAUDE_CONFIG_DIR", root.as_os_str());
            environment(
                &mut launch,
                "CLAUDE_CODE_TMPDIR",
                root.join("tmp").as_os_str(),
            );
            environment(&mut launch, "DISABLE_AUTOUPDATER", "1");
        }
        TitleCli::Codex => {
            environment(&mut launch, "CODEX_HOME", root.as_os_str());
            // The reviewed policy prefix owns --no-daemon at each launch.
            // New accounts use file credentials. Existing configuration is
            // preserved, including legacy accounts already stored in keyring.
            let config = root.join("config.toml");
            match fs::symlink_metadata(&config) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let mut options = fs::OpenOptions::new();
                    options.write(true).create_new(true);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::OpenOptionsExt;
                        options
                            .mode(0o600)
                            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
                    }
                    let mut file = options.open(&config).map_err(|_| isolation())?;
                    file.write_all(b"cli_auth_credentials_store = \"file\"\n")
                        .map_err(|_| isolation())?;
                    file.sync_all().map_err(|_| isolation())?;
                }
                Ok(metadata) => {
                    if !metadata.is_file() || metadata.file_type().is_symlink() {
                        return Err(isolation());
                    }
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        if metadata.uid() != unsafe { libc::geteuid() }
                            || metadata.mode() & 0o077 != 0
                        {
                            return Err(isolation());
                        }
                    }
                }
                Err(_) => return Err(isolation()),
            }
        }
        TitleCli::Grok => {
            environment(&mut launch, "GROK_HOME", root.as_os_str());
            launch.arguments.push("--no-auto-update".into());
        }
        TitleCli::Kimi => {
            environment(&mut launch, "KIMI_CODE_HOME", root.as_os_str());
            environment(&mut launch, "KIMI_CODE_NO_AUTO_UPDATE", "1");
            environment(&mut launch, "KIMI_CLI_NO_AUTO_UPDATE", "1");
        }
        TitleCli::Pi => environment(&mut launch, "PI_CODING_AGENT_DIR", root.as_os_str()),
        TitleCli::Kilo => {
            environment(&mut launch, "KILO_NO_DAEMON", "1");
            environment(&mut launch, "KILO_DISABLE_AUTOUPDATE", "1");
        }
        TitleCli::Opencode => {
            environment(&mut launch, "OPENCODE_TEST_HOME", root.as_os_str());
            environment(&mut launch, "OPENCODE_DISABLE_AUTOUPDATE", "1");
        }
        TitleCli::Agy => {
            // Official 1.2.16 darwin_arm64 artifact inspection proves that a
            // nonempty SSH_TTY bypasses its fixed shared keyring and selects
            // private HOME-backed OAuth plus the native URL/code login flow.
            // This artifact-specific branch is not a general environment API.
            environment(&mut launch, "SSH_TTY", "lomi-private-account");
            environment(&mut launch, "AGY_CLI_DISABLE_AUTO_UPDATE", "1");
            agy_settings(&root)?;
            launch.arguments.push("--mode=default".into());
        }
        _ => return Err(isolation()),
    }
    Ok(launch)
}

pub(crate) fn admit_executable(cli: TitleCli, path: &Path) -> Result<(), String> {
    if !available(cli) {
        return Err("Native account artifact is unavailable.".into());
    }
    if cli == TitleCli::Kimi {
        // Native Kimi can re-exec a manually staged update before --version,
        // even when automatic updates are disabled. npm JS does not use this
        // branch; the absent-directory gate also admits that family safely.
        let staged = path
            .canonicalize()
            .map_err(|_| isolation())?
            .parent()
            .ok_or_else(isolation)?
            .join(".staging");
        match fs::symlink_metadata(staged) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            _ => return Err("Kimi has a staged native update. Finish the update outside this routed profile before selecting a reviewed client version.".into()),
        }
    }
    if cli != TitleCli::Agy {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use sha2::{Digest, Sha256};
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        const MAX_BINARY: u64 = 512 * 1024 * 1024;
        let reject = || {
            "Antigravity account login requires the verified private-storage artifact.".to_string()
        };
        let before = fs::symlink_metadata(path).map_err(|_| reject())?;
        if !before.is_file()
            || before.file_type().is_symlink()
            || before.len() == 0
            || before.len() > MAX_BINARY
            || before.mode() & 0o022 != 0
            || before.mode() & 0o111 == 0
            || ![0, unsafe { libc::geteuid() }].contains(&before.uid())
        {
            return Err(reject());
        }
        let same = |a: &fs::Metadata, b: &fs::Metadata| {
            (
                a.dev(),
                a.ino(),
                a.len(),
                a.uid(),
                a.mode(),
                a.mtime(),
                a.mtime_nsec(),
                a.ctime(),
                a.ctime_nsec(),
            ) == (
                b.dev(),
                b.ino(),
                b.len(),
                b.uid(),
                b.mode(),
                b.mtime(),
                b.mtime_nsec(),
                b.ctime(),
                b.ctime_nsec(),
            )
        };
        let mut file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|_| reject())?;
        let opened = file.metadata().map_err(|_| reject())?;
        if !same(&before, &opened) {
            return Err(reject());
        }
        let mut hash = Sha256::new();
        let mut total = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            let n = file.read(&mut buffer).map_err(|_| reject())?;
            if n == 0 {
                break;
            }
            total = total.checked_add(n as u64).ok_or_else(reject)?;
            if total > MAX_BINARY {
                return Err(reject());
            }
            hash.update(&buffer[..n]);
        }
        let after = file.metadata().map_err(|_| reject())?;
        let named = fs::symlink_metadata(path).map_err(|_| reject())?;
        if total != opened.len()
            || !same(&opened, &after)
            || !same(&opened, &named)
            || named.file_type().is_symlink()
            || format!("{:x}", hash.finalize()) != AGY_SHA256
        {
            return Err(reject());
        }
        Ok(())
    }
    #[cfg(not(unix))]
    Err("Native account artifact is unavailable.".into())
}
