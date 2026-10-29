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
