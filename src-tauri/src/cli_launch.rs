use crate::{
    cli_catalog::TitleCli,
    files::main_window,
    shell::{self, Profile},
    terminal::Shells,
};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tauri::{State, Window};

const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const PROBE_OUTPUT_LIMIT: u64 = 32 * 1024;
const PROBE_MARKER: &[u8] = b"LOMI_AGENT_CLI_PROBE:";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledAgentCli {
    cli: TitleCli,
    name: String,
    command: String,
}

#[derive(Clone, Copy)]
struct CliSpec {
    cli: TitleCli,
    aliases: &'static [&'static str],
    argument: Option<&'static str>,
}

const SPECS: [CliSpec; 16] = [
    CliSpec {
        cli: TitleCli::Claude,
        aliases: &["claude"],
        argument: None,
    },
    CliSpec {
        cli: TitleCli::Codex,
        aliases: &["codex"],
        argument: None,
    },
    CliSpec {
        cli: TitleCli::Gemini,
        aliases: &["gemini"],
        argument: None,
    },
    CliSpec {
        cli: TitleCli::Copilot,
        aliases: &["copilot"],
        argument: None,
    },
    CliSpec {
        cli: TitleCli::Cursor,
        aliases: &["cursor-agent"],
        argument: None,
    },
    CliSpec {
        cli: TitleCli::Opencode,
        aliases: &["opencode"],
        argument: None,
    },
    CliSpec {
        cli: TitleCli::Openclaw,
        aliases: &["openclaw"],
        argument: Some("tui"),
    },
    CliSpec {
        cli: TitleCli::Hermes,
        aliases: &["hermes"],
        argument: None,
    },
    CliSpec {
        cli: TitleCli::Kilo,
        aliases: &["kilo", "kilocode"],
        argument: None,
    },
    CliSpec {
        cli: TitleCli::Qwen,
        aliases: &["qwen"],
        argument: None,
    },
    CliSpec {
        cli: TitleCli::Kiro,
        aliases: &["kiro-cli"],
        argument: Some("chat"),
    },
    CliSpec {
        cli: TitleCli::Vibe,
        aliases: &["vibe"],
        argument: None,
    },
    CliSpec {
        cli: TitleCli::Kimi,
        aliases: &["kimi"],
        argument: None,
    },
    CliSpec {
        cli: TitleCli::Grok,
        aliases: &["grok"],
        argument: None,
    },
    CliSpec {
        cli: TitleCli::Pi,
        aliases: &["pi"],
        argument: None,
    },
    CliSpec {
        cli: TitleCli::Agy,
        aliases: &["agy"],
        argument: None,
    },
];

#[derive(Clone, Debug)]
pub(crate) struct ResolvedCli {
    pub program: PathBuf,
    pub argument: Option<&'static str>,
}

#[tauri::command]
pub async fn installed_agent_clis(
    window: Window,
    shells: State<'_, Shells>,
    profile_id: String,
    cwd: String,
) -> Result<Vec<InstalledAgentCli>, String> {
    main_window(&window)?;
    let shells = shells.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let profile = shells
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .ok_or("The selected shell is no longer installed. Choose another shell.")?;
        let installed = resolve_all(profile, &cwd, &shells.integration, None)?;
        Ok(TitleCli::MCP_CLIENTS
            .iter()
            .filter_map(|cli| {
                let resolved = installed.get(cli)?;
                let spec = spec(*cli)?;
                let alias = spec
                    .aliases
                    .iter()
                    .find(|alias| resolved.alias == **alias)?;
                let command = match spec.argument {
                    Some(argument) => format!("{alias} {argument}"),
                    None => (*alias).to_owned(),
                };
                Some(InstalledAgentCli {
                    cli: *cli,
                    name: cli.name().to_owned(),
                    command,
                })
            })
            .collect())
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(crate) fn resolve_cli(
    profile: &Profile,
    cwd: &str,
    integration: &Path,
    cli: TitleCli,
) -> Result<ResolvedCli, String> {
    let spec = spec(cli).ok_or("This CLI cannot be launched from a terminal.")?;
    resolve_all(profile, cwd, integration, None)?
        .remove(&cli)
        .map(|resolved| ResolvedCli {
            program: resolved.program,
            argument: spec.argument,
        })
        .ok_or_else(|| {
            format!(
                "{} is no longer installed in this shell environment.",
                cli.name()
            )
        })
}

pub(crate) fn installed_local_clis(shells: &Shells) -> Result<HashSet<TitleCli>, String> {
    let profile = shells
        .profiles
        .iter()
        .find(|profile| {
            profile.distro.is_none()
                && matches!(profile.kind.as_str(), "bash" | "zsh" | "fish" | "sh")
        })
        .ok_or("No supported local shell is available to detect installed CLI clients.")?;
    Ok(
        resolve_all(profile, &profile.home, &shells.integration, None)?
            .into_keys()
            .collect(),
    )
}

fn spec(cli: TitleCli) -> Option<CliSpec> {
    SPECS.iter().copied().find(|spec| spec.cli == cli)
}

#[derive(Clone, Debug)]
struct FoundCli {
    alias: &'static str,
    program: PathBuf,
}

fn resolve_all(
    profile: &Profile,
    cwd: &str,
    integration: &Path,
    home_override: Option<&Path>,
) -> Result<HashMap<TitleCli, FoundCli>, String> {
    if !cfg!(any(target_os = "macos", target_os = "linux")) {
        return Err("Agent CLI launch is not supported on this host yet.".into());
    }
    let mut names = Vec::new();
    for spec in SPECS {
        names.extend(spec.aliases.iter().copied());
    }
    let script = probe_script(profile.kind.as_str(), &names)?;
    let (mut builder, resolved_cwd) =
        shell::build_with_command(profile, cwd, integration, &script)?;
    if let Some(home) = home_override {
        builder.env("HOME", home);
    }
    let argv = builder.get_argv();
    let program = argv
        .first()
        .ok_or("The selected shell has no executable path.")?;
    let mut command = shell::quiet_command(program);
    command
        .args(&argv[1..])
        .current_dir(&resolved_cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (key, value) in builder.iter_extra_env_as_str() {
        command.env(key, value);
    }
    let output = run_bounded_probe(command)?;
    parse_probe_output(&output, &names, &resolved_cwd)
}

fn probe_script(kind: &str, names: &[&str]) -> Result<String, String> {
    let list = names.join(" ");
    let body = match kind {
        "bash" => format!(
            "for lomi_name in {list}; do lomi_path=\"$(type -P -- \"$lomi_name\" 2>/dev/null)\"; printf '%s\\0%s\\0' \"$lomi_name\" \"$lomi_path\"; done"
        ),
        "zsh" => format!(
            "for lomi_name in {list}; do lomi_path=\"$(whence -p -- \"$lomi_name\" 2>/dev/null)\"; printf '%s\\0%s\\0' \"$lomi_name\" \"$lomi_path\"; done"
        ),
        "sh" => format!(
            "for lomi_name in {list}; do lomi_path=; lomi_old_ifs=$IFS; IFS=:; case $- in *f*) lomi_old_noglob=1;; *) lomi_old_noglob=0; set -f;; esac; set -- $PATH; for lomi_dir do [ -n \"$lomi_dir\" ] || lomi_dir=.; if [ -f \"$lomi_dir/$lomi_name\" ] && [ -x \"$lomi_dir/$lomi_name\" ]; then lomi_path=\"$lomi_dir/$lomi_name\"; break; fi; done; IFS=$lomi_old_ifs; [ \"$lomi_old_noglob\" = 1 ] || set +f; printf '%s\\0%s\\0' \"$lomi_name\" \"$lomi_path\"; done"
        ),
        "fish" => format!(
            "for lomi_name in {list}\n    set -l lomi_path (type -p -- \"$lomi_name\" 2>/dev/null)\n    printf '%s\\0%s\\0' \"$lomi_name\" \"$lomi_path\"\nend"
        ),
        _ => return Err(format!("Agent launch is not supported in {kind} terminals yet.")),
    };
    Ok(format!(
        "printf '%s' '{marker}'; {body}",
        marker = String::from_utf8_lossy(PROBE_MARKER)
    ))
}

#[cfg(unix)]
fn run_bounded_probe(mut command: Command) -> Result<Vec<u8>, String> {
    use std::os::{fd::AsRawFd, unix::process::CommandExt};

    // Interactive shells can stop with SIGTTIN in a background process group
    // when the app was launched from a terminal. Detach the probe from that
    // controlling terminal while retaining its own group for bounded cleanup.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("Cannot inspect the selected shell environment: {error}"))?;
    let pid = child.id();
    let mut stdout = child
        .stdout
        .take()
        .ok_or("Cannot read the selected shell environment.")?;
    let fd = stdout.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        terminate_probe(&mut child, pid);
        return Err("Cannot read the selected shell environment.".into());
    }
    let deadline = Instant::now() + PROBE_TIMEOUT;
    let mut output = Vec::new();
    loop {
        if let Err(error) = read_available_probe_output(&mut stdout, &mut output) {
            terminate_probe(&mut child, pid);
            return Err(error);
        }

        match child.try_wait() {
            Ok(Some(_)) => {
                if let Err(error) = read_available_probe_output(&mut stdout, &mut output) {
                    terminate_process_group(pid);
                    return Err(error);
                }
                // Do not wait for EOF: an rcfile may have detached a process that
                // inherited stdout after the probe shell exited.
                terminate_process_group(pid);
                return Ok(output);
            }
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                terminate_probe(&mut child, pid);
                return Err("The selected shell took too long to inspect its environment.".into());
            }
            Err(error) => {
                terminate_probe(&mut child, pid);
                return Err(format!(
                    "Cannot inspect the selected shell environment: {error}"
                ));
            }
        }
    }
}

#[cfg(unix)]
fn read_available_probe_output(stdout: &mut impl Read, output: &mut Vec<u8>) -> Result<(), String> {
    use std::io::ErrorKind;

    let mut buffer = [0u8; 4096];
    loop {
        match stdout.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => {
                output.extend_from_slice(&buffer[..count]);
                if output.len() as u64 > PROBE_OUTPUT_LIMIT {
                    return Err(
                        "The selected shell produced too much output while starting.".into(),
                    );
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(()),
            Err(error) => {
                return Err(format!(
                    "Cannot read the selected shell environment: {error}"
                ))
            }
        }
    }
}

#[cfg(not(unix))]
fn run_bounded_probe(_command: Command) -> Result<Vec<u8>, String> {
    Err("Agent CLI launch is not supported on this host yet.".into())
}

#[cfg(unix)]
fn terminate_probe(child: &mut Child, pid: u32) {
    terminate_process_group(pid);
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(not(unix))]
fn terminate_probe(child: &mut Child, _pid: u32) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
fn terminate_process_group(pid: u32) {
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

fn parse_probe_output(
    output: &[u8],
    names: &[&'static str],
    cwd: &str,
) -> Result<HashMap<TitleCli, FoundCli>, String> {
    let marker = output
        .windows(PROBE_MARKER.len())
        .rposition(|window| window == PROBE_MARKER)
        .ok_or("The selected shell did not return its command search paths.")?;
    let fields = output[marker + PROBE_MARKER.len()..]
        .split(|byte| *byte == 0)
        .collect::<Vec<_>>();
    let mut found = HashMap::new();
    for pair in fields.as_chunks::<2>().0 {
        let Ok(alias) = std::str::from_utf8(pair[0]) else {
            continue;
        };
        if pair[1].is_empty() || !names.contains(&alias) {
            continue;
        }
        let program = os_path(pair[1]);
        let path = PathBuf::from(&program);
        if !path.is_absolute() && path.components().count() < 2 {
            continue;
        }
        let path = if path.is_absolute() {
            path
        } else {
            Path::new(cwd).join(path)
        };
        let Ok(path) = path.canonicalize() else {
            continue;
        };
        if !is_executable_file(&path) {
            continue;
        }
        if let Some(spec) = SPECS.iter().find(|spec| spec.aliases.contains(&alias)) {
            found.entry(spec.cli).or_insert_with(|| FoundCli {
                alias: spec
                    .aliases
                    .iter()
                    .copied()
                    .find(|candidate| *candidate == alias)
                    .unwrap(),
                program: path,
            });
        }
    }
    Ok(found)
}

#[cfg(unix)]
fn os_path(bytes: &[u8]) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(bytes.to_vec())
}

#[cfg(not(unix))]
fn os_path(bytes: &[u8]) -> OsString {
    OsString::from(String::from_utf8_lossy(bytes).into_owned())
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
            return false;
        };
        unsafe { libc::access(path.as_ptr(), libc::X_OK) == 0 }
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        sync::atomic::{AtomicUsize, Ordering},
    };

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    fn bash_profile(home: &Path) -> Profile {
        let bash = ["/bin/bash", "/usr/bin/bash"]
            .into_iter()
            .find(|path| Path::new(path).is_file())
            .expect("bash is available");
        Profile {
            id: "test:bash".into(),
            name: "bash".into(),
            kind: "bash".into(),
            program: bash.into(),
            distro: None,
            home: home.to_string_lossy().into_owned(),
        }
    }

    #[test]
    fn discovers_only_executable_clients_from_interactive_rc_path() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("agent home");
        let bin = root.path().join("a path with 'quote");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&bin).unwrap();
        let codex = bin.join("codex");
        fs::write(&codex, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&codex, fs::Permissions::from_mode(0o755)).unwrap();
        let claude = bin.join("claude");
        fs::write(&claude, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&claude, fs::Permissions::from_mode(0o644)).unwrap();
        fs::write(
            home.join(".bashrc"),
            format!(
                "export PATH={}:/usr/bin:/bin\n",
                shell::quote(&bin.to_string_lossy(), "bash").unwrap()
            ),
        )
        .unwrap();
        let profile = bash_profile(&home);
        let integration = root.path().join("integration");
        shell::prepare(&integration).unwrap();
        let cwd = root.path().to_string_lossy();
        let found = resolve_all(&profile, &cwd, &integration, Some(&home)).unwrap();
        assert_eq!(
            found.get(&TitleCli::Codex).unwrap().program,
            codex.canonicalize().unwrap()
        );
        assert!(!found.contains_key(&TitleCli::Claude));
        assert!(!found.is_empty());
    }

    #[test]
    fn settings_detects_installed_clients_without_launching_and_refreshes_after_removal() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("agent home");
        let bin = root.path().join("agent bin");
        fs::create_dir_all(home.join(".gemini")).unwrap();
        fs::create_dir_all(&bin).unwrap();
        fs::write(home.join(".gemini/settings.json"), "{}").unwrap();
        let launched = root.path().join("launched");
        for alias in ["codex", "kilocode", "claude"] {
            let executable = bin.join(alias);
            fs::write(
                &executable,
                format!(
                    "#!/bin/sh\nprintf launched > {}\n",
                    shell::quote(launched.to_str().unwrap(), "bash").unwrap()
                ),
            )
            .unwrap();
            let mode = if alias == "claude" { 0o644 } else { 0o755 };
            fs::set_permissions(&executable, fs::Permissions::from_mode(mode)).unwrap();
        }
        fs::create_dir(bin.join("gemini")).unwrap();
        std::os::unix::fs::symlink(bin.join("missing"), bin.join("cursor-agent")).unwrap();
        let shell_rc = format!(
            "export PATH={}\nclaude() {{ :; }}\n",
            shell::quote(bin.to_str().unwrap(), "bash").unwrap()
        );
        fs::write(home.join(".bashrc"), format!("/bin/sleep 5\n{shell_rc}")).unwrap();
        let mut profile = bash_profile(&home);
        let wrapper = root.path().join("fixture-bash");
        fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nexport HOME={}\nexec {} \"$@\"\n",
                shell::quote(home.to_str().unwrap(), "bash").unwrap(),
                shell::quote(&profile.program, "bash").unwrap()
            ),
        )
        .unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
        profile.program = wrapper.to_string_lossy().into_owned();
        let mut unsupported = profile.clone();
        unsupported.kind = "pwsh".into();
        let mut remote = profile.clone();
        remote.distro = Some("test-wsl".into());
        let integration = root.path().join("integration");
        shell::prepare(&integration).unwrap();
        let shells = Shells {
            profiles: vec![unsupported, remote, profile],
            integration,
        };

        assert_eq!(
            installed_local_clis(&shells).unwrap(),
            HashSet::from([TitleCli::Codex, TitleCli::Kilo])
        );
        assert!(!launched.exists());
        fs::write(home.join(".bashrc"), shell_rc).unwrap();
        fs::remove_file(bin.join("codex")).unwrap();
        assert_eq!(
            installed_local_clis(&shells).unwrap(),
            HashSet::from([TitleCli::Kilo])
        );
        fs::remove_file(bin.join("kilocode")).unwrap();
        assert!(installed_local_clis(&shells).unwrap().is_empty());
        assert!(home.join(".gemini/settings.json").is_file());
        assert!(!launched.exists());
    }

    #[test]
    fn non_executable_path_is_ignored() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("codex");
        fs::write(&file, "not executable").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        let output = format!(
            "{}codex\0{}\0",
            String::from_utf8_lossy(PROBE_MARKER),
            file.display()
        );
        let parsed =
            parse_probe_output(output.as_bytes(), &["codex"], root.path().to_str().unwrap())
                .unwrap();
        assert!(!parsed.contains_key(&TitleCli::Codex));
    }

    #[test]
    fn executable_path_parser_rejects_function_names_shadowed_in_project_cwd() {
        let root = tempfile::tempdir().unwrap();
        let shadow = root.path().join("codex");
        fs::write(&shadow, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&shadow, fs::Permissions::from_mode(0o755)).unwrap();
        let output = format!("{}codex\0codex\0", String::from_utf8_lossy(PROBE_MARKER));
        let parsed =
            parse_probe_output(output.as_bytes(), &["codex"], root.path().to_str().unwrap())
                .unwrap();
        assert!(!parsed.contains_key(&TitleCli::Codex));
    }

    #[test]
    fn launch_specs_cover_mcp_clients_and_native_gateway_clients() {
        for cli in TitleCli::MCP_CLIENTS {
            assert!(SPECS.iter().any(|spec| spec.cli == cli));
        }
        assert_eq!(
            SPECS
                .iter()
                .map(|spec| spec.cli)
                .collect::<HashSet<_>>()
                .len(),
            SPECS.len()
        );
        assert!(spec(TitleCli::Pi).is_some());
        assert_eq!(spec(TitleCli::Kiro).unwrap().argument, Some("chat"));
        assert_eq!(spec(TitleCli::Openclaw).unwrap().argument, Some("tui"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn discovery_from_a_controlling_terminal_does_not_stop_the_probe() {
        const CHILD_ENV: &str = "LOMI_CLI_DISCOVERY_TTY_TEST_CHILD";
        if std::env::var_os(CHILD_ENV).is_some() {
            assert!(fs::File::open("/dev/tty").is_ok());
            let root = tempfile::tempdir().unwrap();
            let home = root.path().join("shell home");
            let bin = root.path().join("agent bin");
            let integration = root.path().join("integration");
            fs::create_dir_all(&home).unwrap();
            fs::create_dir_all(&bin).unwrap();
            shell::prepare(&integration).unwrap();
            let launched = root.path().join("launched");
            let codex = bin.join("codex");
            fs::write(
                &codex,
                format!(
                    "#!/bin/sh\nprintf launched > {}\n",
                    shell::quote(launched.to_str().unwrap(), "sh").unwrap()
                ),
            )
            .unwrap();
            fs::set_permissions(&codex, fs::Permissions::from_mode(0o755)).unwrap();
            fs::write(
                home.join(".zshrc"),
                format!(
                    "export PATH={}:/usr/bin:/bin\n",
                    shell::quote(bin.to_str().unwrap(), "zsh").unwrap()
                ),
            )
            .unwrap();
            let wrapper = root.path().join("selected-zsh");
            fs::write(
                &wrapper,
                format!(
                    "#!/bin/sh\nexport LOMI_ZDOTDIR={}\nexec /bin/zsh \"$@\"\n",
                    shell::quote(home.to_str().unwrap(), "sh").unwrap()
                ),
            )
            .unwrap();
            fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
            let profile = Profile {
                id: "test:zsh".into(),
                name: "zsh".into(),
                kind: "zsh".into(),
                program: wrapper.to_string_lossy().into_owned(),
                distro: None,
                home: home.to_string_lossy().into_owned(),
            };
            let start = Instant::now();
            let found = resolve_all(&profile, &profile.home, &integration, Some(&home)).unwrap();
            assert_eq!(
                found[&TitleCli::Codex].program,
                codex.canonicalize().unwrap()
            );
            let shells = Shells {
                profiles: vec![profile],
                integration,
            };
            assert_eq!(
                installed_local_clis(&shells).unwrap(),
                HashSet::from([TitleCli::Codex])
            );
            assert!(!launched.exists());
            assert!(start.elapsed() < Duration::from_secs(5));
            return;
        }

        // A separate test process gives the caller a real controlling terminal
        // without changing the session or environment of other test threads.
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = portable_pty::CommandBuilder::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            "cli_launch::tests::discovery_from_a_controlling_terminal_does_not_stop_the_probe",
            "--nocapture",
            "--test-threads=1",
        ]);
        command.env(CHILD_ENV, "1");
        let mut reader = pair.master.try_clone_reader().unwrap();
        let reading = thread::spawn(move || {
            let mut output = String::new();
            let _ = reader.read_to_string(&mut output);
            output
        });
        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let status = child.wait().unwrap();
        let output = reading.join().unwrap();
        assert!(status.success(), "{output}");
    }

    #[test]
    fn bounded_probe_kills_timed_out_process_group() {
        let root = tempfile::tempdir().unwrap();
        let pid_file = root
            .path()
            .join(format!("child-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed)));
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(format!(
                "sleep 30 & echo $! > {}; wait",
                shell::quote(&pid_file.display().to_string(), "bash").unwrap()
            ))
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let start = Instant::now();
        let result = run_bounded_probe(command);
        assert!(start.elapsed() < PROBE_TIMEOUT + Duration::from_secs(2));
        assert!(result.is_err());
        if let Ok(pid) = fs::read_to_string(pid_file) {
            let pid: i32 = pid.trim().parse().unwrap();
            // The group signal precedes the kernel scheduling and reaping its
            // descendants; kill(0) can still observe them immediately after it.
            let deadline = Instant::now() + Duration::from_secs(1);
            while unsafe { libc::kill(pid, 0) == 0 } && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            let alive = unsafe { libc::kill(pid, 0) == 0 };
            assert!(!alive, "probe left child process {pid} alive");
        }
    }
}
