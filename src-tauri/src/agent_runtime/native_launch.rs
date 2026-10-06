//! Common managed-native launch admission. macOS never falls back to direct spawn.
#[cfg(target_os = "macos")]
pub(crate) use super::host_boundary::Purpose;
#[cfg(target_os = "macos")]
#[path = "claude_temp_alias.rs"]
mod claude_temp_alias;
use crate::cli_catalog::TitleCli;
use std::{
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    sync::atomic::AtomicBool,
};
#[cfg(target_os = "macos")]
pub(crate) type LaunchContext = super::host_child::Context;
#[cfg(not(target_os = "macos"))]
pub(crate) type LaunchContext = ();
#[cfg(not(target_os = "macos"))]
#[derive(Clone, Copy)]
pub(crate) enum Purpose {
    Attempt,
    VersionProbe,
    Collector,
    AccountTerminal,
}
#[cfg(target_os = "macos")]
pub(crate) type Stdin = std::fs::File;
#[cfg(target_os = "macos")]
pub(crate) type Stdout = std::fs::File;
#[cfg(target_os = "macos")]
pub(crate) type Stderr = std::fs::File;
#[cfg(not(target_os = "macos"))]
pub(crate) type Stdin = std::process::ChildStdin;
#[cfg(not(target_os = "macos"))]
pub(crate) type Stdout = std::process::ChildStdout;
#[cfg(not(target_os = "macos"))]
pub(crate) type Stderr = std::process::ChildStderr;

pub(crate) struct Child {
    outcome: Option<ExitStatus>,
    retired: bool,
    #[cfg(target_os = "macos")]
    host: super::host_child::HostChild,
    #[cfg(target_os = "macos")]
    _proxy: Option<std::sync::Arc<super::native_network::Proxy>>,
    #[cfg(not(target_os = "macos"))]
    direct: super::process_owner::OwnedChild,
}
impl std::ops::Deref for Child {
    #[cfg(target_os = "macos")]
    type Target = super::host_child::HostChild;
    #[cfg(not(target_os = "macos"))]
    type Target = super::process_owner::OwnedChild;
    fn deref(&self) -> &Self::Target {
        #[cfg(target_os = "macos")]
        {
            &self.host
        }
        #[cfg(not(target_os = "macos"))]
        {
            &self.direct
        }
    }
}
impl std::ops::DerefMut for Child {
    fn deref_mut(&mut self) -> &mut Self::Target {
        #[cfg(target_os = "macos")]
        {
            &mut self.host
        }
        #[cfg(not(target_os = "macos"))]
        {
            &mut self.direct
        }
    }
}
impl Child {
    pub(crate) fn exit_pending(&mut self) -> Result<bool, String> {
        #[cfg(target_os = "macos")]
        {
            self.host.exit_pending()
        }
        #[cfg(not(target_os = "macos"))]
        {
            super::process_owner::exit_pending(&self.direct)
        }
    }
    pub(crate) fn stop_and_wait(&mut self) -> Result<Option<ExitStatus>, String> {
        #[cfg(target_os = "macos")]
        {
            if !self.retired {
                if self.outcome.is_none() {
                    self.outcome = self.host.try_wait().ok().flatten();
                }
                self.host.stop_and_wait()?;
                self.retired = true;
            }
            Ok(self.outcome)
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.direct.stop_and_wait().map(Some)
        }
    }
    pub(crate) fn signal(&self, signal: i32) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            self.host.signal(signal)
        }
        #[cfg(not(target_os = "macos"))]
        {
            if unsafe { libc::kill(-(self.direct.id() as i32), signal) } != 0 {
                return Err("Cannot signal the owned native group.".into());
            }
            Ok(())
        }
    }
}

pub(crate) fn resolve(cli: TitleCli) -> Result<PathBuf, String> {
    let paths = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    resolve_from(cli, paths, home.as_deref())
}

fn resolve_from(
    cli: TitleCli,
    mut paths: Vec<PathBuf>,
    home: Option<&Path>,
) -> Result<PathBuf, String> {
    let aliases: &[&str] = match cli {
        TitleCli::Codex => &["codex"],
        TitleCli::Claude => &["claude"],
        TitleCli::Agy => &["agy"],
        TitleCli::Cursor => &["cursor-agent"],
        TitleCli::Gemini => &["gemini"],
        TitleCli::Copilot => &["copilot"],
        TitleCli::Opencode => &["opencode"],
        TitleCli::Openclaw => &["openclaw"],
        TitleCli::Hermes => &["hermes"],
        TitleCli::Pi => &["pi"],
        TitleCli::Kilo => &["kilo", "kilocode"],
        TitleCli::Qwen => &["qwen"],
        TitleCli::Kiro => &["kiro-cli"],
        TitleCli::Vibe => &["vibe"],
        TitleCli::Kimi => &["kimi"],
        TitleCli::Grok => return Err("Grok requires its owned artifact resolver.".into()),
    };
    if let Some(home) = home.filter(|home| trusted_search_directory(home)) {
        paths.push(home.join(".local/bin"));
    }
    paths.extend(
        [
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/usr/bin",
            "/bin",
            "/usr/sbin",
            "/sbin",
        ]
        .map(PathBuf::from),
    );
    if paths.len() > 256 {
        return Err("The native executable search exceeds its bound.".into());
    }
    for root in paths {
        if !root.is_absolute()
            || root.components().any(|c| {
                !matches!(
                    c,
                    std::path::Component::RootDir | std::path::Component::Normal(_)
                )
            })
        {
            continue;
        }
        if !trusted_search_directory(&root) {
            continue;
        }
        for alias in aliases {
            let path = root.join(alias);
            if let Ok(identity) = super::process_owner::executable(&path) {
                use std::os::unix::fs::PermissionsExt;
                if std::fs::metadata(&identity.0)
                    .map(|m| m.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false)
                {
                    return Ok(identity.0);
                }
            }
        }
    }
    if let Some(home) = home.filter(|home| trusted_search_directory(home)) {
        let base = home.join(".local/share/lomi-router-clis");
        if trusted_search_directory(&base) {
            let manifest: serde_json::Value = serde_json::from_str(include_str!(
                "../../../scripts/agent-cli-installations.json"
            ))
            .map_err(|_| "The embedded installer manifest is invalid.")?;
            if let Some(tool) = manifest["tools"].as_array().and_then(|tools| {
                tools.iter().find(|tool| {
                    tool["cli"]
                        .as_str()
                        .is_some_and(|name| aliases.contains(&name))
                })
            }) {
                let name = tool["cli"].as_str().ok_or("Invalid installer tool.")?;
                let version = tool["version"]
                    .as_str()
                    .ok_or("Invalid installer version.")?;
                let entry = Path::new(tool["entry"].as_str().ok_or("Invalid installer entry.")?);
                if entry
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
                {
                    let root = base.join(format!("{name}-{version}"));
                    if trusted_search_directory(&root) {
                        if let Ok(identity) = super::process_owner::executable(&root.join(entry)) {
                            use std::os::unix::fs::PermissionsExt;
                            if std::fs::metadata(&identity.0)
                                .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
                            {
                                return Ok(identity.0);
                            }
                        }
                    }
                }
            }
        }
    }
    Err("The reviewed native client is absent from the application's bounded PATH. Interactive shell startup is not executed.".into())
}

pub(super) fn trusted_search_directory(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    path.is_absolute()
        && std::fs::symlink_metadata(path).is_ok_and(|m| {
            m.is_dir()
                && !m.file_type().is_symlink()
                && m.mode() & 0o022 == 0
                && [0, unsafe { libc::geteuid() }].contains(&m.uid())
        })
}

#[cfg(test)]
mod resolver_tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};
    #[test]
    fn finder_path_resolves_an_owned_installer_alias_without_shell_startup() {
        let home = tempfile::tempdir().unwrap();
        std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let bin = home.path().join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        let target = home.path().join("owned-kimi-entry");
        std::fs::write(&target, b"owned resolver fixture; never executed").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
        symlink(&target, bin.join("kimi")).unwrap();
        assert_eq!(
            resolve_from(
                TitleCli::Kimi,
                vec!["/usr/bin".into(), "/bin".into()],
                Some(home.path())
            )
            .unwrap(),
            target.canonicalize().unwrap()
        );
        std::fs::remove_file(bin.join("kimi")).unwrap();
        let installed = home.path().join(".local/share/lomi-router-clis/kimi-2.1.1/node_modules/@moonshot-ai/kimi-code/dist/main.mjs");
        std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
        std::fs::write(
            &installed,
            b"owned direct installer fixture; never executed",
        )
        .unwrap();
        std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            resolve_from(
                TitleCli::Kimi,
                vec!["/usr/bin".into(), "/bin".into()],
                Some(home.path())
            )
            .unwrap(),
            installed.canonicalize().unwrap()
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "Owned public pinned Kimi artifact version; no accounts or inference"]
    fn pinned_kimi_version_uses_the_managed_host_boundary() {
        use std::io::Read;
        let artifact = std::env::var_os("LOMI_KIMI_PUBLIC_ROOT")
            .expect("set the owned pinned Kimi artifact root");
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let dirs: Vec<_> = ["project", "account", "storage"]
            .map(|name| root.join(name))
            .into();
        for path in &dirs {
            std::fs::create_dir(path).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let context = LaunchContext {
            parent_operation_id: "owned-kimi-version".into(),
            account_id: "owned-kimi-fixture".into(),
            auth_revision: 1,
            task_id: None,
            attempt_id: None,
            generation: None,
            project_root: dirs[0].clone(),
            physical_account_root: dirs[1].clone(),
            storage_root: dirs[2].clone(),
        };
        let mut command = Command::new(
            Path::new(&artifact).join("node_modules/@moonshot-ai/kimi-code/dist/main.mjs"),
        );
        command
            .arg("--version")
            .env_clear()
            .env("HOME", &dirs[1])
            .env("PATH", "/usr/bin:/bin")
            .current_dir(&dirs[0]);
        let mut child = spawn(
            command,
            Some(&context),
            TitleCli::Kimi,
            Purpose::VersionProbe,
            None,
            &[],
            &[],
            None,
            &AtomicBool::new(false),
            || Ok(()),
        )
        .unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let (mut output, mut errors) = (Vec::new(), Vec::new());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            for (file, collected) in [(&mut stdout, &mut output), (&mut stderr, &mut errors)] {
                let mut bytes = [0; 4096];
                match file.read(&mut bytes) {
                    Ok(count) => collected.extend_from_slice(&bytes[..count]),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) => panic!("owned Kimi output failed: {error}"),
                }
            }
            if child.exit_pending().unwrap() {
                assert!(
                    child.stop_and_wait().unwrap().unwrap().success(),
                    "{}",
                    String::from_utf8_lossy(&errors)
                );
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{}",
                String::from_utf8_lossy(&errors)
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            String::from_utf8_lossy(&output).contains("2.1.1"),
            "{} {}",
            String::from_utf8_lossy(&output),
            String::from_utf8_lossy(&errors)
        );
        assert!(super::super::host_boundary::parent_completed(
            &dirs[2],
            &context.parent_operation_id
        )
        .unwrap());
    }
    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "Owned host effect drain timeout and audited retirement retry"]
    fn delayed_effect_stop_can_be_retried_without_a_native_exit_outcome() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        for name in ["project", "account", "storage"] {
            let path = root.join(name);
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let context = LaunchContext {
            parent_operation_id: "owned-stop-retry".into(),
            account_id: "owned-stop-fixture".into(),
            auth_revision: 1,
            task_id: None,
            attempt_id: None,
            generation: None,
            project_root: root.join("project"),
            physical_account_root: root.join("account"),
            storage_root: root.join("storage"),
        };
        let mut command = Command::new("/bin/sleep");
        command
            .arg("60")
            .env_clear()
            .env("HOME", &context.physical_account_root)
            .current_dir(&context.project_root);
        let mut child = spawn(
            command,
            Some(&context),
            TitleCli::Pi,
            Purpose::VersionProbe,
            None,
            &[],
            &[],
            None,
            &AtomicBool::new(false),
            || Ok(()),
        )
        .unwrap();
        let effect = child.effect_scope().enter().unwrap();
        assert!(child.stop_and_wait().is_err());
        assert!(!super::super::host_boundary::parent_completed(
            &context.storage_root,
            &context.parent_operation_id
        )
        .unwrap());
        drop(effect);
        child.stop_and_wait().unwrap();
        assert!(super::super::host_boundary::parent_completed(
            &context.storage_root,
            &context.parent_operation_id
        )
        .unwrap());
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn(
    command: Command,
    context: Option<&LaunchContext>,
    cli: TitleCli,
    purpose: Purpose,
    configuration: Option<&super::managed_config::ManagedConfiguration>,
    runtime_reads: &[PathBuf],
    runtime_roots: &[PathBuf],
    registration: Option<&crate::cli_mcp::Registration>,
    cancel: &AtomicBool,
    before_release: impl FnOnce() -> Result<(), String>,
) -> Result<Child, String> {
    #[cfg(target_os = "macos")]
    {
        spawn_io(
            command,
            context.ok_or("A durable native host context is required.")?,
            cli,
            purpose,
            configuration,
            runtime_reads,
            runtime_roots,
            registration,
            cancel,
            super::host_boundary::NativeIo::Pipes,
            before_release,
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            context,
            cli,
            purpose,
            configuration,
            runtime_reads,
            runtime_roots,
            registration,
            cancel,
        );
        before_release()?;
        let mut command = command;
        Ok(Child {
            outcome: None,
            retired: false,
            direct: super::process_owner::OwnedChild::new(
                command
                    .spawn()
                    .map_err(|_| "Cannot spawn the native client.")?,
            ),
        })
    }
}

#[cfg(target_os = "macos")]
#[allow(clippy::too_many_arguments)]
fn spawn_io(
    mut command: Command,
    context: &LaunchContext,
    cli: TitleCli,
    purpose: Purpose,
    configuration: Option<&super::managed_config::ManagedConfiguration>,
    runtime_reads: &[PathBuf],
    runtime_roots: &[PathBuf],
    registration: Option<&crate::cli_mcp::Registration>,
    cancel: &AtomicBool,
    io: super::host_boundary::NativeIo,
    before_release: impl FnOnce() -> Result<(), String>,
) -> Result<Child, String> {
    use super::host_boundary::{NativeSpec, Policy};
    use std::os::unix::fs::DirBuilderExt;
    let kimi = if cli == TitleCli::Kimi {
        let proof = kimi_artifact::admit(Path::new(command.get_program()))?;
        let mut interpreted = Command::new(&proof.node);
        interpreted
            .arg(&proof.entry)
            .args(command.get_args())
            .env_clear();
        for (key, value) in command.get_envs() {
            if let Some(value) = value {
                interpreted.env(key, value);
            }
        }
        if let Some(cwd) = command.get_current_dir() {
            interpreted.current_dir(cwd);
        }
        interpreted.env("NODE_DISABLE_COMPILE_CACHE", "1");
        command = interpreted;
        Some(proof)
    } else {
        None
    };
    let proxy = if matches!(purpose, Purpose::Attempt | Purpose::AccountTerminal) {
        Some(super::native_network::Proxy::start(cli)?)
    } else {
        None
    };
    if let Some(proxy) = &proxy {
        command.envs(proxy.environment());
    }
    let file_credentials = if cli == TitleCli::Claude && configuration.is_some() {
        let selector = super::file_credentials::FileCredentials::prepare(&context.storage_root)?;
        let path = command
            .get_envs()
            .find_map(|(name, value)| (name == "PATH").then_some(value).flatten())
            .map(std::ffi::OsStr::to_owned)
            .unwrap_or_else(|| std::ffi::OsString::from("/usr/bin:/bin:/usr/sbin:/sbin"));
        let paths = std::iter::once(selector.directory.clone()).chain(std::env::split_paths(&path));
        command.env(
            "PATH",
            std::env::join_paths(paths)
                .map_err(|_| "Cannot select private native file credentials.")?,
        );
        Some(selector)
    } else {
        None
    };
    let temp = if matches!(purpose, Purpose::VersionProbe) {
        command
            .get_current_dir()
            .ok_or("Version probe directory is missing.")?
            .to_owned()
    } else {
        context.physical_account_root.join(".native-tmp")
    };
    match std::fs::DirBuilder::new().mode(0o700).create(&temp) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err("Cannot create private native temporary storage.".into()),
    }
    super::native_accounts::check_private_directory(&temp)?;
    command.env("TMPDIR", &temp);
    let temp_alias = if cli == TitleCli::Claude && !matches!(purpose, Purpose::VersionProbe) {
        Some(claude_temp_alias::Alias::prepare(
            &context.storage_root,
            &temp,
        )?)
    } else {
        None
    };
    if cli == TitleCli::Claude {
        // Keep both the pinned client's base and per-uid helper paths below its
        // 44-byte fallback threshold, with data in the original account temp.
        command.env(
            "CLAUDE_CODE_TMPDIR",
            temp_alias
                .as_ref()
                .map(|alias| alias.path.as_path())
                .unwrap_or(temp.as_path()),
        );
    }
    let program = PathBuf::from(command.get_program())
        .canonicalize()
        .map_err(|_| "Cannot resolve the admitted native executable.")?;
    let mut reads = vec![program.clone()];
    reads.extend_from_slice(runtime_reads);
    if let Some(alias) = &temp_alias {
        reads.push(alias.wrapper.clone());
    }
    if let Some(selector) = &file_credentials {
        reads.extend(selector.reads.clone());
    }

    if let Some(proof) = &kimi {
        reads.push(proof.entry.clone());
        for path in ["node_modules/@moonshot-ai/kimi-code/native/darwin/prebuilds/darwin-arm64/darwin-platform.node", "node_modules/node-pty/prebuilds/darwin-arm64/pty.node", "node_modules/node-pty/prebuilds/darwin-arm64/spawn-helper"] {
            reads.push(proof.root.join(path));
        }
    }
    reads.sort();
    reads.dedup();
    let mut roots = runtime_roots.to_vec();
    if let Some(proof) = &kimi {
        roots.push(proof.root.join("node_modules"));
    }
    roots.sort();
    roots.dedup();
    let mut protected_reads = configuration
        .map(|c| c.protected_paths().to_vec())
        .unwrap_or_default();
    if let Some(proof) = &kimi {
        protected_reads.extend([
            proof.root.join("package.json"),
            proof.root.join("package-lock.json"),
        ]);
    }
    if matches!(purpose, Purpose::AccountTerminal) && matches!(cli, TitleCli::Claude) {
        let login_state = context.physical_account_root.join(".claude.json");
        protected_reads.retain(|path| path != &login_state);
    }
    if proxy.is_some() {
        let certificates = public_root_certificates()?;
        for name in ["SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE"] {
            command.env(name, &certificates);
        }
        protected_reads.push(certificates);
    }
    let mut unix_sockets = Vec::new();
    if let Some(registration) = registration {
        protected_reads.extend(super::managed_mcp::discovery_read_paths(registration)?);
        let discovery = lomi_control_core::discovery::read(
            Path::new(&registration.args[2]),
            &registration.args[4],
        )
        .map_err(|_| "Cannot verify the authenticated MCP broker endpoint.")?;
        unix_sockets.push(discovery.endpoint);
        reads.push(PathBuf::from(&registration.command));
    }
    let spec = NativeSpec {
        program,
        arguments: command
            .get_args()
            .map(|a| {
                a.to_str()
                    .map(str::to_owned)
                    .ok_or("Native arguments must be UTF-8.")
            })
            .collect::<Result<_, _>>()?,
        environment: command
            .get_envs()
            .filter_map(|(k, v)| v.map(|v| (k, v)))
            .map(|(k, v)| {
                Ok((
                    k.to_str()
                        .ok_or("Native environment must be UTF-8.")?
                        .to_owned(),
                    v.to_str()
                        .ok_or("Native environment must be UTF-8.")?
                        .to_owned(),
                ))
            })
            .collect::<Result<_, String>>()?,
        cwd: command
            .get_current_dir()
            .ok_or("Native working directory is missing.")?
            .canonicalize()
            .map_err(|_| "Cannot verify native working directory.")?,
        policy: Policy {
            project_root: context.project_root.clone(),
            account_root: context.physical_account_root.clone(),
            temp_root: temp
                .canonicalize()
                .map_err(|_| "Cannot resolve private temporary storage.")?,
            runtime_reads: reads,
            runtime_read_roots: roots,
            protected_reads,
            blocked_reads: configuration
                .map(|c| c.blocked_paths().to_vec())
                .unwrap_or_default(),
            codex_preferences: cli == TitleCli::Codex,
            loopback_tcp_ports: proxy.as_ref().map(|p| vec![p.port()]).unwrap_or_default(),
            loopback_listener: matches!(purpose, Purpose::AccountTerminal)
                || matches!(cli, TitleCli::Kimi | TitleCli::Kilo | TitleCli::Opencode)
                    && matches!(purpose, Purpose::Attempt),
            unix_sockets,
            allow_native_tools: matches!(purpose, Purpose::Attempt | Purpose::AccountTerminal),
        },
        io,
    };
    let mut host = context.spawn_with_boundary(purpose.clone(), spec, cancel, |boundary| {
        if let Some(proxy) = &proxy {
            proxy.attach(boundary.effects())?;
            boundary.retain_retirement_resource(proxy.clone())?;
        }
        before_release()?;
        if let Some(proof) = &kimi {
            kimi_artifact::recheck(proof)?;
        }
        if let Some(configuration) = configuration {
            configuration.recheck()?;
        }
        if let Some(selector) = &file_credentials {
            selector.recheck()?;
        }
        if let Some(alias) = &temp_alias {
            alias.recheck()?;
        }
        Ok(())
    })?;
    if matches!(purpose, Purpose::VersionProbe | Purpose::Collector) {
        host.close_stdin();
    }
    Ok(Child {
        host,
        _proxy: proxy,
        outcome: None,
        retired: false,
    })
}

#[cfg(target_os = "macos")]
fn public_root_certificates() -> Result<PathBuf, String> {
    use std::os::unix::fs::MetadataExt;
    let path = PathBuf::from("/private/etc/ssl/cert.pem");
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|_| "The public system certificate bundle is unavailable.")?;
    if !metadata.is_file()
        || metadata.uid() != 0
        || metadata.mode() & 0o022 != 0
        || metadata.len() == 0
        || metadata.len() > 4 * 1024 * 1024
        || path
            .canonicalize()
            .map_err(|_| "Cannot verify the public certificate bundle.")?
            != path
    {
        return Err("The public system certificate bundle is not trusted.".into());
    }
    let bytes =
        std::fs::read(&path).map_err(|_| "Cannot read the public system certificate bundle.")?;
    if !bytes
        .windows(b"-----BEGIN CERTIFICATE-----".len())
        .any(|window| window == b"-----BEGIN CERTIFICATE-----")
        || bytes
            .windows(b"PRIVATE KEY".len())
            .any(|window| window == b"PRIVATE KEY")
    {
        return Err("The public system certificate bundle has invalid contents.".into());
    }
    Ok(path)
}

#[cfg(target_os = "macos")]
pub(crate) struct OwnedPty {
    pub master: Box<dyn portable_pty::MasterPty + Send>,
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
    pub read_fd: std::os::fd::RawFd,
    #[cfg(test)]
    pub effects: std::sync::Arc<super::host_boundary::EffectScope>,
}
#[cfg(target_os = "macos")]
pub(crate) type PtyFactory =
    Box<dyn FnOnce(portable_pty::PtySize) -> Result<OwnedPty, String> + Send>;
#[cfg(target_os = "macos")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_pty(
    command: Command,
    context: &LaunchContext,
    cli: TitleCli,
    configuration: Option<&super::managed_config::ManagedConfiguration>,
    runtime_reads: &[PathBuf],
    runtime_roots: &[PathBuf],
    cancel: &AtomicBool,
    size: portable_pty::PtySize,
    before_release: impl FnOnce() -> Result<(), String>,
) -> Result<OwnedPty, String> {
    use std::os::fd::AsRawFd;
    let mut child = spawn_io(
        command,
        context,
        cli,
        Purpose::AccountTerminal,
        configuration,
        runtime_reads,
        runtime_roots,
        None,
        cancel,
        super::host_boundary::NativeIo::Pty {
            rows: size.rows,
            cols: size.cols,
        },
        before_release,
    )?;
    let reader = child
        .stdout
        .take()
        .ok_or("Native PTY output is unavailable.")?;
    let writer = child
        .stdin
        .take()
        .ok_or("Native PTY input is unavailable.")?;
    let write_fd = writer.as_raw_fd();
    let read_fd = reader.as_raw_fd();
    #[cfg(test)]
    let effects = child.effect_scope();
    let child = std::sync::Arc::new(std::sync::Mutex::new(child));
    Ok(OwnedPty {
        master: Box::new(HostMaster {
            child: child.clone(),
            reader,
            writer: std::sync::Mutex::new(Some(writer)),
            write_fd,
            size: std::sync::Mutex::new(size),
        }),
        child: Box::new(HostPtyChild { child }),
        read_fd,
        #[cfg(test)]
        effects,
    })
}
#[cfg(target_os = "macos")]
struct HostMaster {
    child: std::sync::Arc<std::sync::Mutex<Child>>,
    reader: std::fs::File,
    writer: std::sync::Mutex<Option<std::fs::File>>,
    write_fd: std::os::fd::RawFd,
    size: std::sync::Mutex<portable_pty::PtySize>,
}
#[cfg(target_os = "macos")]
impl portable_pty::MasterPty for HostMaster {
    fn resize(&self, size: portable_pty::PtySize) -> Result<(), anyhow::Error> {
        self.child
            .lock()
            .map_err(|_| anyhow::anyhow!("Native PTY ownership is unavailable."))?
            .host
            .resize(size.rows, size.cols)
            .map_err(anyhow::Error::msg)?;
        *self
            .size
            .lock()
            .map_err(|_| anyhow::anyhow!("Native PTY size is unavailable."))? = size;
        Ok(())
    }
    fn get_size(&self) -> Result<portable_pty::PtySize, anyhow::Error> {
        self.size
            .lock()
            .map(|s| *s)
            .map_err(|_| anyhow::anyhow!("Native PTY size is unavailable."))
    }
    fn try_clone_reader(&self) -> Result<Box<dyn std::io::Read + Send>, anyhow::Error> {
        Ok(Box::new(self.reader.try_clone()?))
    }
    fn take_writer(&self) -> Result<Box<dyn std::io::Write + Send>, anyhow::Error> {
        Ok(Box::new(
            self.writer
                .lock()
                .map_err(|_| anyhow::anyhow!("Native PTY input is unavailable."))?
                .take()
                .ok_or_else(|| anyhow::anyhow!("Native PTY input was already taken."))?,
        ))
    }
    fn process_group_leader(&self) -> Option<libc::pid_t> {
        None
    }
    fn as_raw_fd(&self) -> Option<std::os::fd::RawFd> {
        Some(self.write_fd)
    }
    fn tty_name(&self) -> Option<PathBuf> {
        None
    }
}
#[cfg(target_os = "macos")]
#[derive(Clone)]
struct HostPtyChild {
    child: std::sync::Arc<std::sync::Mutex<Child>>,
}
#[cfg(target_os = "macos")]
impl std::fmt::Debug for HostPtyChild {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HostPtyChild")
    }
}
#[cfg(target_os = "macos")]
impl portable_pty::ChildKiller for HostPtyChild {
    fn kill(&mut self) -> std::io::Result<()> {
        self.child
            .lock()
            .map_err(|_| std::io::Error::other("Native PTY ownership is unavailable."))?
            .stop_and_wait()
            .map(|_| ())
            .map_err(std::io::Error::other)
    }
    fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
        Box::new(self.clone())
    }
}
#[cfg(target_os = "macos")]
impl portable_pty::Child for HostPtyChild {
    fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> {
        let mut child = self
            .child
            .lock()
            .map_err(|_| std::io::Error::other("Native PTY ownership is unavailable."))?;
        if child.retired {
            return Ok(child.outcome.map(Into::into));
        }
        child
            .host
            .try_wait()
            .map(|s| s.map(Into::into))
            .map_err(std::io::Error::other)
    }
    fn wait(&mut self) -> std::io::Result<portable_pty::ExitStatus> {
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            if self
                .child
                .lock()
                .map_err(|_| std::io::Error::other("Native PTY ownership is unavailable."))?
                .retired
            {
                return Err(std::io::Error::other(
                    "The native PTY retired without a confirmed exit outcome.",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    fn process_id(&self) -> Option<u32> {
        self.child.lock().ok().map(|child| child.host.id())
    }
}

#[cfg(target_os = "macos")]
mod kimi_artifact {
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
        path::{Path, PathBuf},
    };
    const ENTRY: &str = "node_modules/@moonshot-ai/kimi-code/dist/main.mjs";
    const NODE: &str = "5d9d3872911e2340a43b707962e68143de8a4e8d54628845c0c4f2de1fb7cd5c";
    // Derived from the embedded npm lock, isolated npm ci --ignore-scripts,
    // empty npm configs and verified required assets; never a live receipt.
    const CLOSURE: &str = "9be70a266c95028af34221c5b364d96f4bf8b01086be73137a2232199a37a3d7";
    const COUNT: usize = 1193;
    fn failure() -> String {
        "Managed Kimi requires the reviewed Kimi 2.1.1 closure and Node 22.22.3 standard installer entry.".into()
    }
    pub(super) struct Proof {
        pub(super) node: PathBuf,
        pub(super) root: PathBuf,
        pub(super) entry: PathBuf,
        node_identity: (PathBuf, u64, u64, u64, i64, i64),
    }
    fn safe(m: &fs::Metadata) -> Result<(), String> {
        if m.uid() != unsafe { libc::geteuid() }
            || (!m.file_type().is_symlink() && m.mode() & 0o022 != 0)
        {
            return Err(failure());
        }
        Ok(())
    }
    fn hash(path: &Path, budget: &mut u64) -> Result<String, String> {
        hash_with_hook(path, budget, |_, _| {})
    }
    fn hash_with_hook(
        path: &Path,
        budget: &mut u64,
        opened: impl FnOnce(&Path, &fs::File),
    ) -> Result<String, String> {
        hash_file(path, budget, false, opened)
    }
    fn hash_node(path: &Path, budget: &mut u64) -> Result<String, String> {
        hash_file(path, budget, true, |_, _| {})
    }
    fn hash_file(
        path: &Path,
        budget: &mut u64,
        allow_root: bool,
        opened: impl FnOnce(&Path, &fs::File),
    ) -> Result<String, String> {
        let mut f = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(|_| failure())?;
        let before = f.metadata().map_err(|_| failure())?;
        if allow_root && before.uid() == 0 {
            if before.mode() & 0o022 != 0 {
                return Err(failure());
            }
        } else {
            safe(&before)?;
        }
        if !before.is_file()
            || before.nlink() != 1
            || before.len() > 128 * 1024 * 1024
            || before.mode() & 0o6000 != 0
            || (allow_root && before.mode() & 0o111 == 0)
        {
            return Err(failure());
        }
        let prior_budget = *budget;
        *budget = budget
            .checked_add(before.len())
            .filter(|n| *n <= 1024 * 1024 * 1024)
            .ok_or_else(failure)?;
        opened(path, &f);
        let mut h = Sha256::new();
        let mut b = [0u8; 65536];
        let mut total = 0u64;
        loop {
            let n = f.read(&mut b).map_err(|_| failure())?;
            if n == 0 {
                break;
            }
            total = total
                .checked_add(n as u64)
                .filter(|total| {
                    *total <= before.len()
                        && *total <= 128 * 1024 * 1024
                        && prior_budget
                            .checked_add(*total)
                            .is_some_and(|bytes| bytes <= 1024 * 1024 * 1024)
                })
                .ok_or_else(failure)?;
            h.update(&b[..n]);
        }
        if total != before.len() {
            return Err(failure());
        }
        let after = f.metadata().map_err(|_| failure())?;
        let named = fs::symlink_metadata(path).map_err(|_| failure())?;
        if (
            before.dev(),
            before.ino(),
            before.len(),
            before.mtime(),
            before.mtime_nsec(),
            before.ctime(),
            before.ctime_nsec(),
            before.uid(),
            before.mode(),
            before.nlink(),
        ) != (
            after.dev(),
            after.ino(),
            after.len(),
            after.mtime(),
            after.mtime_nsec(),
            after.ctime(),
            after.ctime_nsec(),
            after.uid(),
            after.mode(),
            after.nlink(),
        ) || named.ino() != before.ino()
            || named.dev() != before.dev()
            || !named.is_file()
        {
            return Err(failure());
        }
        Ok(format!("{:x}", h.finalize()))
    }
    fn closure(root: &Path) -> Result<(usize, String), String> {
        fn walk(
            root: &Path,
            path: &Path,
            rows: &mut Vec<(String, &'static str, String)>,
            budget: &mut u64,
            depth: usize,
            visited: &mut usize,
        ) -> Result<(), String> {
            *visited += 1;
            if depth > 32 || rows.len() > 20000 || *visited > 30000 {
                return Err(failure());
            }
            let m = fs::symlink_metadata(path).map_err(|_| failure())?;
            safe(&m)?;
            if m.is_dir() {
                for e in fs::read_dir(path).map_err(|_| failure())? {
                    walk(
                        root,
                        &e.map_err(|_| failure())?.path(),
                        rows,
                        budget,
                        depth + 1,
                        visited,
                    )?;
                }
                let after = fs::symlink_metadata(path).map_err(|_| failure())?;
                if !after.is_dir()
                    || (m.dev(), m.ino(), m.mtime(), m.mtime_nsec())
                        != (after.dev(), after.ino(), after.mtime(), after.mtime_nsec())
                {
                    return Err(failure());
                }
            } else {
                let name = path
                    .strip_prefix(root)
                    .map_err(|_| failure())?
                    .to_str()
                    .ok_or_else(failure)?
                    .to_owned();
                if m.file_type().is_symlink() {
                    // Unix link mode is conventionally 0777; only ownership matters.
                    let target = fs::read_link(path).map_err(|_| failure())?;
                    let canonical = path.canonicalize().map_err(|_| failure())?;
                    if target.is_absolute() || !canonical.starts_with(root) || !canonical.is_file()
                    {
                        return Err(failure());
                    }
                    rows.push((name, "l", target.to_str().ok_or_else(failure)?.into()));
                } else if m.is_file() {
                    rows.push((name, "f", hash(path, budget)?));
                } else {
                    return Err(failure());
                }
            }
            Ok(())
        }
        let m = fs::symlink_metadata(root).map_err(|_| failure())?;
        safe(&m)?;
        if !m.is_dir() {
            return Err(failure());
        }
        let mut rows = Vec::new();
        let mut budget = 0;
        let mut visited = 0;
        for name in ["package.json", "package-lock.json", "node_modules"] {
            walk(
                root,
                &root.join(name),
                &mut rows,
                &mut budget,
                0,
                &mut visited,
            )?;
        }
        rows.sort();
        let mut h = Sha256::new();
        for (p, k, v) in &rows {
            for x in [p.as_str(), *k, v.as_str()] {
                h.update(x.as_bytes());
                h.update([0]);
            }
        }
        Ok((rows.len(), format!("{:x}", h.finalize())))
    }

    pub(super) fn admit(entry: &Path) -> Result<Proof, String> {
        let entry = entry.canonicalize().map_err(|_| failure())?;
        let root = entry.ancestors().nth(5).ok_or_else(failure)?.to_owned();
        if root.join(ENTRY) != entry || closure(&root)? != (COUNT, CLOSURE.into()) {
            return Err(failure());
        }
        let mut paths: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        if let Some(home) = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|p| super::trusted_search_directory(p))
        {
            paths.push(home.join(".local/bin"));
        }
        paths
            .extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
        if paths.len() > 256 {
            return Err(failure());
        }
        let node = paths
            .into_iter()
            .filter(|p| super::trusted_search_directory(p))
            .find_map(|p| {
                let node = p.join("node").canonicalize().ok()?;
                let mut budget = 0;
                (hash_node(&node, &mut budget).ok()?.as_str() == NODE).then_some(node)
            })
            .ok_or_else(failure)?;
        let node_identity = super::super::process_owner::executable(&node)?;
        Ok(Proof {
            node,
            root,
            entry,
            node_identity,
        })
    }
    pub(super) fn recheck(proof: &Proof) -> Result<(), String> {
        let mut budget = 0;
        if super::super::process_owner::executable(&proof.node)? != proof.node_identity
            || hash_node(&proof.node, &mut budget)? != NODE
            || closure(&proof.root)? != (COUNT, CLOSURE.into())
            || proof.root.join(ENTRY) != proof.entry
        {
            return Err(failure());
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::Write;
        #[test]
        fn a_file_growing_after_open_is_rejected_within_its_initial_byte_budget() {
            let fixture = tempfile::tempdir().unwrap();
            let path = fixture.path().join("owned-growing-artifact");
            fs::write(&path, b"original").unwrap();
            let mut budget = 0;
            assert!(hash_with_hook(&path, &mut budget, |path, _| {
                fs::OpenOptions::new()
                    .append(true)
                    .open(path)
                    .unwrap()
                    .write_all(&vec![0; 2 * 65536])
                    .unwrap();
            })
            .is_err());
            assert_eq!(budget, b"original".len() as u64);
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod retirement_resource_tests {
    use super::*;
    use std::{net::TcpListener, os::unix::fs::PermissionsExt};

    #[test]
    #[ignore = "Owned dropped child, delayed effect and proxy reservation retirement"]
    fn dropped_child_keeps_both_proxy_ports_until_restored_retirement() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        for name in ["project", "account", "storage"] {
            let path = root.join(name);
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let context = LaunchContext {
            parent_operation_id: "owned-proxy-retirement".into(),
            account_id: "owned-proxy-fixture".into(),
            auth_revision: 1,
            task_id: None,
            attempt_id: None,
            generation: None,
            project_root: root.join("project"),
            physical_account_root: root.join("account"),
            storage_root: root.join("storage"),
        };
        let mut command = Command::new("/bin/sleep");
        command
            .arg("1")
            .env_clear()
            .env("HOME", &context.physical_account_root)
            .current_dir(&context.project_root);
        let mut child = spawn(
            command,
            Some(&context),
            TitleCli::Codex,
            Purpose::Attempt,
            None,
            &[],
            &[],
            None,
            &AtomicBool::new(false),
            || Ok(()),
        )
        .unwrap();
        let port = child._proxy.as_ref().unwrap().port();
        let effects = child.effect_scope();
        let pending = effects.enter().unwrap();
        // The live effect outlives the trusted native result. Unit builds never
        // enumerate unrelated PIDs; wait for this owned command before Stop.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !child.exit_pending().unwrap() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        effects.cancel();
        assert!(effects.enter().is_err());
        drop(child); // Stop physically retires, then times out on the retained effect.
        for address in ["127.0.0.1", "::1"] {
            assert_eq!(
                TcpListener::bind((address, port)).unwrap_err().kind(),
                std::io::ErrorKind::AddrInUse,
                "reservation lost after failed Drop Stop: {address}"
            );
        }
        assert!(!super::super::host_boundary::parent_completed(
            &context.storage_root,
            &context.parent_operation_id
        )
        .unwrap());
        drop(pending);
        let receipts = super::super::host_boundary::settle_parent(
            &context.storage_root,
            &context.parent_operation_id,
        )
        .unwrap();
        assert_eq!(receipts.len(), 1);
        assert!(super::super::host_boundary::parent_completed(
            &context.storage_root,
            &context.parent_operation_id
        )
        .unwrap());
        for address in ["127.0.0.1", "::1"] {
            let _released = TcpListener::bind((address, port)).unwrap();
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod claude_alias_kernel_tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{io::Read, os::unix::fs::PermissionsExt};

    #[test]
    #[ignore = "Owned pinned original Claude temp helpers under the real held sandbox"]
    fn original_temp_helpers_write_through_short_alias_only_to_account_temp() {
        let node = PathBuf::from(
            std::env::var_os("LOMI_CLAUDE_TEMP_NODE").expect("owned public Node 22.22.3 fixture"),
        )
        .canonicalize()
        .unwrap();
        let mut hash = Sha256::new();
        let mut file = std::fs::File::open(&node).unwrap();
        let mut chunk = [0; 65536];
        loop {
            let count = file.read(&mut chunk).unwrap();
            if count == 0 {
                break;
            }
            hash.update(&chunk[..count]);
        }
        assert_eq!(
            format!("{:x}", hash.finalize()),
            "5d9d3872911e2340a43b707962e68143de8a4e8d54628845c0c4f2de1fb7cd5c"
        );
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        for name in ["project", "account", "storage"] {
            let directory = root.join(name);
            std::fs::create_dir(&directory).unwrap();
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        std::fs::write(
            root.join("outside-owned-canary"),
            b"owned outside authority canary",
        )
        .unwrap();
        let script = root.join("original-temp-helpers.cjs");
        let bytes = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/native/claude-temp-directory.cjs"),
        )
        .unwrap();
        std::fs::write(&script, &bytes).unwrap();
        let artifact = PathBuf::from("/private/tmp/lomi-public-cli-help-c2hyiv50/claude");
        let proof = super::super::native_artifact::admit(TitleCli::Claude, &artifact)
            .unwrap()
            .unwrap();
        let context = LaunchContext {
            parent_operation_id: "owned-claude-short-temp".into(),
            account_id: "owned-alias-fixture".into(),
            auth_revision: 1,
            task_id: None,
            attempt_id: None,
            generation: None,
            project_root: root.join("project"),
            physical_account_root: root.join("account"),
            storage_root: root.join("storage"),
        };
        let physical = context.physical_account_root.join(".native-tmp");
        assert!(physical.as_os_str().len() > 44);
        let mut command = Command::new(&node);
        command
            .arg(&script)
            .arg(&artifact)
            .arg("--held")
            .arg(&physical)
            .env_clear()
            .env("HOME", &context.physical_account_root)
            .current_dir(&context.project_root);
        let mut child = spawn(
            command,
            Some(&context),
            TitleCli::Claude,
            Purpose::Collector,
            None,
            &[script.clone(), artifact],
            &[],
            None,
            &AtomicBool::new(false),
            || {
                super::super::native_artifact::recheck(&proof)?;
                if std::fs::read(&script).map_err(|e| e.to_string())? != bytes {
                    return Err("owned helper fixture changed".into());
                }
                Ok(())
            },
        )
        .unwrap();
        let alias = claude_temp_alias::Alias::prepare(&context.storage_root, &physical).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !child.exit_pending().unwrap() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let outcome = child.stop_and_wait().unwrap().unwrap();
        let mut output = String::new();
        let mut errors = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut errors)
            .unwrap();
        assert!(outcome.success(), "{output} {errors}");
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["passed"], true, "{output} {errors}");
        assert_eq!(value["checks"], 8);
        assert_eq!(
            std::fs::read(root.join("outside-owned-canary")).unwrap(),
            b"owned outside authority canary"
        );
        alias.recheck().unwrap();
        assert!(super::super::host_boundary::parent_completed(
            &context.storage_root,
            &context.parent_operation_id
        )
        .unwrap());
        std::fs::remove_file(&alias.path).unwrap();
        std::fs::remove_dir(&alias.wrapper).unwrap(); // Only this retired test account.
    }
}
