use portable_pty::CommandBuilder;
#[cfg(all(test, target_os = "macos"))]
#[path = "../../tests/native/mcp-terminal-support.rs"]
mod mcp_qualification;
use serde::Serialize;
use std::{
    env,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub program: String,
    pub distro: Option<String>,
    pub home: String,
}

pub fn home() -> PathBuf {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn executable(program: &str) -> Option<PathBuf> {
    let program_path = Path::new(program);
    if program_path.is_absolute() && is_local_shell(program_path) {
        return Some(program_path.to_owned());
    }
    let directories = env::var_os("PATH").unwrap_or_default();
    let directories = env::split_paths(&directories);
    // Finder-launched applications do not inherit a terminal's Homebrew PATH.
    #[cfg(target_os = "macos")]
    let directories = directories.chain([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    for directory in directories {
        let path = directory.join(program);
        if is_local_shell(&path) {
            return Some(path);
        }
        #[cfg(windows)]
        if is_local_shell(&path.with_extension("exe")) {
            return Some(path.with_extension("exe"));
        }
    }
    #[cfg(windows)]
    {
        // GUI launches may omit the built-in shells from PATH.
        let path = match program {
            "cmd" => Some(windows_system_program("cmd.exe")),
            "powershell" => Some(windows_system_program(
                r"WindowsPowerShell\v1.0\powershell.exe",
            )),
            _ => None,
        };
        if let Some(path) = path.filter(|path| is_local_shell(path)) {
            return Some(path);
        }
    }
    None
}

fn is_local_shell(path: &Path) -> bool {
    #[cfg(windows)]
    if path
        .file_stem()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("bash"))
    {
        // These bash.exe files launch WSL and cannot load local Bash rcfiles.
        let normalized = path
            .to_string_lossy()
            .replace('/', "\\")
            .to_ascii_lowercase();
        if normalized
            == windows_system_program("bash.exe")
                .to_string_lossy()
                .to_ascii_lowercase()
            || normalized.ends_with(r"\microsoft\windowsapps\bash.exe")
            || normalized.contains(r"\windowsapps\microsoftcorporationii.windowssubsystemforlinux_")
        {
            return false;
        }
    }
    path.is_file()
}

#[cfg(windows)]
fn windows_system_program(program: &str) -> PathBuf {
    PathBuf::from(env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into()))
        .join("System32")
        .join(program)
}

fn wsl_program() -> PathBuf {
    // The system launcher works with Store WSL without using an app execution alias.
    #[cfg(windows)]
    return windows_system_program("wsl.exe");
    #[cfg(not(windows))]
    PathBuf::from("wsl.exe")
}

fn kind(program: &str) -> String {
    Path::new(program)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
}

pub fn discover() -> Vec<Profile> {
    let default = if cfg!(windows) {
        "pwsh".into()
    } else {
        env::var("SHELL").unwrap_or_else(|_| {
            if cfg!(target_os = "macos") {
                "/bin/zsh"
            } else {
                "bash"
            }
            .into()
        })
    };
    let mut profiles = Vec::new();
    let candidates = if cfg!(windows) {
        [
            &default,
            "pwsh",
            "powershell",
            "cmd",
            "zsh",
            "bash",
            "fish",
            "sh",
        ]
    } else {
        [
            &default,
            "zsh",
            "bash",
            "fish",
            "pwsh",
            "powershell",
            "cmd",
            "sh",
        ]
    };
    for candidate in candidates {
        if let Some(path) = executable(candidate) {
            let program = path.to_string_lossy().into_owned();
            let kind = kind(&program);
            if profiles
                .iter()
                .any(|profile: &Profile| profile.id == format!("local:{kind}"))
            {
                continue;
            }
            profiles.push(Profile {
                id: format!("local:{kind}"),
                name: kind.clone(),
                kind,
                program,
                distro: None,
                home: home().to_string_lossy().into_owned(),
            });
        }
    }
    #[cfg(windows)]
    if let Ok(output) = quiet_command(wsl_program())
        .args(["--list", "--quiet"])
        .output()
    {
        if output.status.success() {
            let text = wsl_text(&output.stdout);
            for distro in text
                .trim_start_matches('\u{feff}')
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
            {
                let result = quiet_command(wsl_program())
                    .args([
                        "-d",
                        distro,
                        "--exec",
                        "sh",
                        "-c",
                        "printf '%s\\n%s\\n' \"${SHELL:-/bin/sh}\" \"$HOME\"",
                    ])
                    .output();
                let (program, home) = result
                    .ok()
                    .filter(|output| output.status.success())
                    .map(|output| {
                        let text = String::from_utf8_lossy(&output.stdout);
                        let mut lines = text.lines();
                        (
                            lines.next().unwrap_or("/bin/sh").to_string(),
                            lines.next().unwrap_or("/").to_string(),
                        )
                    })
                    .unwrap_or(("/bin/sh".into(), "/".into()));
                profiles.push(Profile {
                    id: format!("wsl:{distro}"),
                    name: format!("WSL · {distro}"),
                    kind: kind(&program),
                    program,
                    distro: Some(distro.into()),
                    home,
                });
            }
        }
    }
    profiles
}

pub fn quiet_command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}

fn wsl_text(bytes: &[u8]) -> String {
    // WSL's Windows launcher emits UTF-16LE; Linux commands emit UTF-8.
    let text = if bytes.starts_with(&[0xff, 0xfe]) || bytes.contains(&0) {
        String::from_utf16_lossy(
            &bytes
                .as_chunks::<2>()
                .0
                .iter()
                .copied()
                .map(u16::from_le_bytes)
                .collect::<Vec<_>>(),
        )
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    text.trim_start_matches('\u{feff}').to_owned()
}

pub fn wsl_path(distro: &str, path: &str) -> Result<String, String> {
    if path.starts_with('/') {
        return Ok(path.into());
    }
    // Windows canonicalization adds a verbatim prefix that wslpath does not accept.
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    let path = path
        .strip_prefix(r"UNC\")
        .map_or_else(|| path.to_owned(), |tail| format!(r"\\{tail}"));
    let output = quiet_command(wsl_program())
        .args(["-d", distro, "--exec", "wslpath", "-a", "-u", &path])
        .output()
        .map_err(|error| format!("Cannot run WSL ({distro}): {error}"))?;
    wsl_path_result(
        distro,
        output.status.success(),
        &output.stdout,
        &output.stderr,
    )
}

fn wsl_path_result(
    distro: &str,
    success: bool,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<String, String> {
    let path = wsl_text(stdout);
    let path = path.trim_end_matches(['\r', '\n']);
    if !success || !path.starts_with('/') || path.contains(['\0', '\r', '\n']) {
        let detail = format!("{}\n{}", wsl_text(stderr).trim(), path.trim());
        let detail = detail.trim();
        return Err(format!(
            "Cannot translate a path for WSL ({distro}): {}",
            if detail.is_empty() {
                "wslpath returned no Linux path."
            } else {
                detail
            }
        ));
    }
    Ok(path.to_owned())
}

pub fn prepare(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path.join("zsh")).map_err(|error| error.to_string())?;
    for (name, content) in [
        ("bash.sh", include_str!("../shell/bash.sh")),
        ("zsh/.zshrc", include_str!("../shell/zsh.sh")),
        ("fish.fish", include_str!("../shell/fish.fish")),
    ] {
        fs::write(path.join(name), content).map_err(|error| error.to_string())?;
    }
    // Zsh reads .zshenv before .zshrc; forward the user's original file as well.
    fs::write(path.join("zsh/.zshenv"), "[[ -f \"${LOMI_ZDOTDIR:-$HOME}/.zshenv\" ]] && source \"${LOMI_ZDOTDIR:-$HOME}/.zshenv\"\n").map_err(|error| error.to_string())?;
    // Login zsh reads this before .zshrc restores the user's ZDOTDIR.
    fs::write(path.join("zsh/.zprofile"), "[[ -f \"${LOMI_ZDOTDIR:-$HOME}/.zprofile\" ]] && source \"${LOMI_ZDOTDIR:-$HOME}/.zprofile\"\n").map_err(|error| error.to_string())?;
    Ok(())
}

pub fn build(
    profile: &Profile,
    cwd: &str,
    integration: &Path,
) -> Result<(CommandBuilder, String), String> {
    let cwd = if let Some(distro) = &profile.distro {
        wsl_path(distro, cwd)?
    } else {
        dunce::simplified(&crate::files::directory(cwd)?)
            .to_string_lossy()
            .into_owned()
    };
    let integration = if let Some(distro) = &profile.distro {
        wsl_path(distro, &integration.to_string_lossy())?
    } else {
        integration.to_string_lossy().into_owned()
    };
    let join = |name: &str| {
        if profile.distro.is_some() {
            format!("{integration}/{name}")
        } else {
            Path::new(&integration)
                .join(name)
                .to_string_lossy()
                .into_owned()
        }
    };
    let mut command = if let Some(distro) = &profile.distro {
        let mut command = CommandBuilder::new(wsl_program());
        command.args([
            "-d",
            distro,
            "--cd",
            &cwd,
            "--exec",
            "env",
            "TERM=xterm-256color",
            "COLORTERM=truecolor",
            "TERM_PROGRAM=Lomi",
        ]);
        if profile.kind == "zsh" {
            command.arg(format!("ZDOTDIR={}", join("zsh")));
            command.arg(format!("LOMI_ZDOTDIR={}", profile.home));
        }
        command.arg(&profile.program);
        command
    } else {
        let mut command = CommandBuilder::new(&profile.program);
        command.cwd(&cwd);
        command
    };
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("TERM_PROGRAM", "Lomi");
    #[cfg(all(feature = "mcp-probe", target_os = "macos"))]
    if env::var_os("LOMI_MCP_TERMINAL_ONLY").is_some() {
        let directory = env::var_os("LOMI_MCP_CONTROL_PROBE_DIRECTORY")
            .ok_or("Terminal probe directory is missing")?;
        let config = PathBuf::from(directory).join("shell-home");
        command.env("HOME", &config);
        command.env("INPUTRC", "/dev/null");
        command.env("HISTFILE", "/dev/null");
        command.env("NODE_REPL_HISTORY", "/dev/null");
        command.env_remove("PROMPT_COMMAND");
    }
    match profile.kind.as_str() {
        "bash" => {
            command.args(["--rcfile", &join("bash.sh"), "-i"]);
        }
        "zsh" => {
            command.env(
                "LOMI_ZDOTDIR",
                env::var("ZDOTDIR").unwrap_or_else(|_| profile.home.clone()),
            );
            command.env("ZDOTDIR", join("zsh"));
            #[cfg(all(feature = "mcp-probe", target_os = "macos"))]
            if env::var_os("LOMI_MCP_TERMINAL_ONLY").is_some() {
                let directory = env::var_os("LOMI_MCP_CONTROL_PROBE_DIRECTORY")
                    .ok_or("Terminal probe directory is missing")?;
                command.env("LOMI_ZDOTDIR", PathBuf::from(directory).join("shell-home"));
            }
            if cfg!(target_os = "macos") && profile.distro.is_none() {
                command.arg("-l");
            }
            command.arg("-i");
        }
        "fish" => {
            command.args([
                "-i",
                "-C",
                &format!("source {}", quote(&join("fish.fish"), "fish")?),
            ]);
        }
        "pwsh" | "powershell" => {
            // Inline the bundled hook so Restricted execution policy does not block
            // terminal integration. Leave the user's policy and profile unchanged.
            command.args([
                "-NoLogo",
                "-NoExit",
                "-Command",
                include_str!("../shell/powershell.ps1"),
            ]);
        }
        "cmd" => {
            command.args(["/Q", "/D", "/V:OFF"]);
            command.env(
                "PROMPT",
                "$E]133;D$E\\$E]7;file://localhost/$P$E\\$E]133;A$E\\$P$G$E]133;B$E\\",
            );
        }
        _ => {
            command.arg("-i");
        }
    }
    Ok((command, cwd))
}

pub fn quote(path: &str, shell: &str) -> Result<String, String> {
    if path
        .chars()
        .any(|character| matches!(character, '\0' | '\n' | '\r' | '\u{1b}'))
    {
        return Err("Paths with control characters cannot be pasted into a terminal.".into());
    }
    match shell {
        "pwsh" | "powershell" => Ok(format!("'{}'", path.replace('\'', "''"))),
        "cmd" => {
            if path.contains(['%', '!', '"']) {
                return Err("cmd cannot safely paste paths containing %, !, or quotes. Use PowerShell for this path.".into());
            }
            Ok(format!("\"{path}\""))
        }
        _ => Ok(format!("'{}'", path.replace('\'', "'\\''"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl_paths_preserve_spaces_and_report_launcher_errors() {
        assert_eq!(
            wsl_path_result("Ubuntu", true, b"/mnt/c/project with spaces /\n", b"").unwrap(),
            "/mnt/c/project with spaces /"
        );
        assert_eq!(
            wsl_path_result("Ubuntu", true, b"/mnt/c/trailing space \n", b"").unwrap(),
            "/mnt/c/trailing space "
        );
        let message = "\u{feff}WSL is not installed.\r\n";
        let utf16 = message
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        let error = wsl_path_result("Ubuntu", false, &utf16, b"").unwrap_err();
        assert!(error.contains("Ubuntu"), "{error}");
        assert!(error.contains("WSL is not installed."), "{error}");
        assert!(!error.contains('\0'), "{error}");
        for output in [
            b"".as_slice(),
            br"C:\Program Files\WindowsApps\wsl.exe",
            b"/mnt/c/a\n/mnt/c/b",
        ] {
            assert!(wsl_path_result("Ubuntu", true, output, b"").is_err());
        }
        let error = wsl_path_result("Ubuntu", false, b"details", b"wslpath: denied").unwrap_err();
        assert!(
            error.contains("wslpath: denied") && error.contains("details"),
            "{error}"
        );
        assert_eq!(wsl_text(&utf16), "WSL is not installed.\r\n");
        assert_eq!(wsl_text("Ubuntu-żółć\r\n".as_bytes()), "Ubuntu-żółć\r\n");
    }

    #[cfg(windows)]
    #[test]
    fn windows_discovery_excludes_wsl_bash_launchers() {
        let directory = tempfile::tempdir().unwrap();
        for (name, expected) in [
            (r"Microsoft\WindowsApps\bash.exe", false),
            (r"microsoft\windowsapps\BASH.EXE", false),
            (
                r"WindowsApps\MicrosoftCorporationII.WindowsSubsystemForLinux_2.7.13.0_x64__8wekyb3d8bbwe\bash.exe",
                false,
            ),
            (r"Git\bin\bash.exe", true),
        ] {
            let path = directory.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "").unwrap();
            assert_eq!(is_local_shell(&path), expected, "{}", path.display());
        }
        assert!(!is_local_shell(&windows_system_program("bash.exe")));
        assert_eq!(wsl_program(), windows_system_program("wsl.exe"));
        let profiles = discover();
        assert!(matches!(profiles[0].kind.as_str(), "pwsh" | "powershell"));
        assert!(profiles.iter().any(|profile| profile.kind == "cmd"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_shells_start_in_project_and_keep_prompt_hooks() {
        use std::io::{Read, Write};
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project żółć & it's a folder");
        fs::create_dir(&project).unwrap();
        let integration = directory.path().join("integration");
        prepare(&integration).unwrap();
        let mut tested = 0;
        for name in ["powershell", "pwsh", "cmd"] {
            let Some(program) = executable(name) else {
                continue;
            };
            tested += 1;
            let profile = Profile {
                id: format!("local:{name}"),
                name: name.into(),
                kind: name.into(),
                program: program.to_string_lossy().into_owned(),
                distro: None,
                home: project.to_string_lossy().into_owned(),
            };
            let canonical = fs::canonicalize(&project).unwrap();
            let (mut command, cwd) =
                build(&profile, &canonical.to_string_lossy(), &integration).unwrap();
            assert!(!cwd.starts_with(r"\\?\"), "{cwd}");
            assert_eq!(fs::canonicalize(&cwd).unwrap(), canonical);
            // Isolate the test from user profiles without changing any registry policy.
            if name != "cmd" {
                command.get_argv_mut().insert(1, "-NoProfile".into());
                command.env("PSExecutionPolicyPreference", "Restricted");
            }
            let pair = portable_pty::native_pty_system()
                .openpty(portable_pty::PtySize {
                    rows: 24,
                    cols: 240,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .unwrap();
            let mut child = pair.slave.spawn_command(command).unwrap();
            drop(pair.slave);
            let mut reader = pair.master.try_clone_reader().unwrap();
            let mut writer = pair.master.take_writer().unwrap();
            let (send, receive) = mpsc::channel();
            let reading = std::thread::spawn(move || {
                let mut buffer = [0; 4096];
                while let Ok(count) = reader.read(&mut buffer) {
                    if count == 0 || send.send(buffer[..count].to_vec()).is_err() {
                        break;
                    }
                }
            });
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut bytes = Vec::new();
            let mut sent = false;
            let mut answered_cursor = 0;
            while Instant::now() < deadline {
                let chunk = match receive.recv_timeout(Duration::from_millis(100)) {
                    Ok(chunk) => chunk,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                bytes.extend(chunk);
                let output = String::from_utf8_lossy(&bytes);
                let cursor_requests = output.matches("\x1b[6n").count();
                for _ in answered_cursor..cursor_requests {
                    writer.write_all(b"\x1b[1;1R").unwrap();
                }
                answered_cursor = cursor_requests;
                if !sent && output.contains("\x1b]133;B") {
                    if name == "cmd" {
                        writer
                            .write_all(b"echo DIRECTORY=\"%CD%\"\r\necho SMOKE_^OK\r\nexit\r\n")
                            .unwrap();
                    } else {
                        // Console.WriteLine in Windows PowerShell uses the system code
                        // page; use the shell's Unicode output to check the actual cwd.
                        writer.write_all(b"Write-Output ('POLICY=' + (Get-ExecutionPolicy)); Write-Output ('DIRECTORY=' + $PWD.ProviderPath); Write-Output ('SMOKE_' + 'OK'); exit\r").unwrap();
                    }
                    sent = true;
                }
                if output.contains("SMOKE_OK") {
                    break;
                }
            }
            let _ = child.kill();
            let _ = child.wait();
            drop(writer);
            drop(pair.master);
            reading.join().unwrap();
            let output = String::from_utf8_lossy(&bytes);
            assert!(output.contains("SMOKE_OK"), "{name}: {output:?}");
            let reported_directory = if name == "cmd" {
                format!("DIRECTORY=\"{cwd}\"")
            } else {
                assert!(output.contains("POLICY=Restricted"), "{name}: {output:?}");
                format!("DIRECTORY={cwd}")
            };
            assert!(output.contains(&reported_directory), "{name}: {output:?}");
            assert!(output.contains("\x1b]7;file://"), "{name}: {output:?}");
            assert!(
                !output.contains("PSSecurityException"),
                "{name}: {output:?}"
            );
        }
        assert!(
            tested >= 2,
            "Windows PowerShell and cmd must be available for the PTY smoke test"
        );
    }

    #[cfg(unix)]
    #[test]
    fn bash_prompt_keeps_input_after_prompt_when_widened() {
        use std::io::{Read, Write};
        use std::sync::mpsc;
        use std::time::Duration;

        let directory = tempfile::tempdir().unwrap();
        let integration = directory.path().join("integration");
        fs::write(
            directory.path().join(".bashrc"),
            "PS1='[woro@woro-home lomi]$ '\n",
        )
        .unwrap();
        prepare(&integration).unwrap();
        let profile = Profile {
            id: "local:bash".into(),
            name: "bash".into(),
            kind: "bash".into(),
            program: "bash".into(),
            distro: None,
            home: directory.path().to_string_lossy().into_owned(),
        };
        let (mut command, _) = build(&profile, &profile.home, &integration).unwrap();
        command.env("HOME", directory.path());
        command.env("INPUTRC", "/dev/null");
        command.env("HISTFILE", "/dev/null");
        command.env_remove("PROMPT_COMMAND");
        let size = |cols| portable_pty::PtySize {
            rows: 24,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = portable_pty::native_pty_system().openpty(size(80)).unwrap();
        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut writer = pair.master.take_writer().unwrap();
        let (send, receive) = mpsc::channel();
        let reading = std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(count) = reader.read(&mut buffer) {
                if count == 0 || send.send(buffer[..count].to_vec()).is_err() {
                    break;
                }
            }
        });
        let output = || {
            let mut bytes = receive.recv_timeout(Duration::from_secs(5)).unwrap();
            // A redraw can span reads; include cursor movement after the prompt marker.
            while let Ok(chunk) = receive.recv_timeout(Duration::from_millis(100)) {
                bytes.extend(chunk);
            }
            String::from_utf8_lossy(&bytes).into_owned()
        };
        let initial = output();
        let mut redraws = Vec::new();
        for cols in [24, 80, 18, 100, 29, 80] {
            pair.master.resize(size(cols)).unwrap();
            let redraw = output();
            if cols >= 80 {
                redraws.push(redraw);
            }
        }
        writer
            .write_all(b"printf 'INPUT_OK:%s\\n' 'hello'\r")
            .unwrap();
        let executed = output();
        let _ = child.kill();
        let _ = child.wait();
        reading.join().unwrap();

        assert!(initial.contains("\x1b]133;A\x07"), "{initial:?}");
        for redraw in redraws {
            // With no input to restore, the cursor must stay at the end marker.
            assert!(redraw.ends_with("\x1b]133;B\x07"), "{redraw:?}");
        }
        assert!(executed.contains("INPUT_OK:hello\r\n"), "{executed:?}");
        assert!(executed.contains("\x1b]133;D;0\x07"), "{executed:?}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_zsh_loads_login_files_and_keeps_terminal_integration() {
        use std::io::{Read, Write};
        use std::sync::mpsc;
        use std::time::Duration;

        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("shell config #50%? żółć");
        let integration = directory.path().join("integration");
        fs::create_dir(&config).unwrap();
        fs::write(config.join(".zshenv"), "export LOMI_TEST_ORDER=env\n").unwrap();
        fs::write(
            config.join(".zprofile"),
            "export LOMI_TEST_ORDER=$LOMI_TEST_ORDER,profile\n",
        )
        .unwrap();
        fs::write(
            config.join(".zshrc"),
            "export LOMI_TEST_ORDER=$LOMI_TEST_ORDER,rc\nPROMPT='ready> '\n",
        )
        .unwrap();
        fs::write(
            config.join(".zlogin"),
            "export LOMI_TEST_ORDER=$LOMI_TEST_ORDER,login\n",
        )
        .unwrap();
        prepare(&integration).unwrap();
        let profile = Profile {
            id: "local:zsh".into(),
            name: "zsh".into(),
            kind: "zsh".into(),
            program: "/bin/zsh".into(),
            distro: None,
            home: config.to_string_lossy().into_owned(),
        };
        let (mut command, _) = build(&profile, &config.to_string_lossy(), &integration).unwrap();
        // Isolate the spawned shell without changing the test process's environment.
        command.env("LOMI_ZDOTDIR", &config);
        command.env_remove("HISTFILE");
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut writer = pair.master.take_writer().unwrap();
        let (send, receive) = mpsc::channel();
        let reading = std::thread::spawn(move || {
            let mut output = Vec::new();
            let _ = reader.read_to_end(&mut output);
            let _ = send.send(String::from_utf8_lossy(&output).into_owned());
        });
        writer
            .write_all(b"printf '\\nORDER=%s\\n' \"$LOMI_TEST_ORDER\"; cd ..\rexit\r")
            .unwrap();
        let output = receive.recv_timeout(Duration::from_secs(10));
        let _ = child.kill();
        let _ = child.wait();
        reading.join().unwrap();
        let output = output.expect("login shell did not exit");
        assert!(output.contains("ORDER=env,profile,rc,login"), "{output:?}");
        assert!(output.contains("\u{1b}]133;A"), "{output:?}");
        assert!(output.contains("\u{1b}]133;C"), "{output:?}");
        let reported: Vec<_> = output
            .split("\x1b]7;")
            .skip(1)
            .map(|part| {
                let url = part.split('\x07').next().unwrap();
                tauri::Url::parse(url).unwrap().to_file_path().unwrap()
            })
            .collect();
        assert_eq!(
            reported,
            [
                fs::canonicalize(config).unwrap(),
                fs::canonicalize(directory.path()).unwrap()
            ],
            "{output:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn shell_hooks_preserve_cli_arguments_status_and_custom_commands() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let bin = directory.path().join("installed cli");
        fs::create_dir(&bin).unwrap();
        let codex = bin.join("codex");
        fs::write(&codex, "#!/bin/sh\nprintf '%s\\0' \"$@\"\nexit 7\n").unwrap();
        fs::set_permissions(&codex, fs::Permissions::from_mode(0o755)).unwrap();
        let path = env::join_paths(
            std::iter::once(bin).chain(env::split_paths(&env::var_os("PATH").unwrap())),
        )
        .unwrap();
        let run = |setup: &str, args: &[&str]| {
            Command::new("bash")
                .args(["--noprofile", "--norc", "-c"])
                // Source the integration without loading the user's startup files.
                .arg(format!(
                    "source() {{ :; }}\n{setup}\n{}\ncodex \"$@\"",
                    include_str!("../shell/bash.sh")
                ))
                .arg("bash")
                .args(args)
                .env("PATH", &path)
                .current_dir(directory.path())
                .output()
                .unwrap()
        };
        let args = [
            "resume",
            "Dodaj edytor kodu w aplikacji 🦀",
            "",
            "quotes '\"; $(touch should-not-exist)",
            "-c",
            "tui.terminal_title=['project']",
        ];
        let output = run("", &args);
        assert_eq!(output.status.code(), Some(7));
        let expected = args
            .into_iter()
            .flat_map(|arg| arg.bytes().chain(std::iter::once(0)))
            .collect::<Vec<_>>();
        assert_eq!(output.stdout, expected);
        assert!(!directory.path().join("should-not-exist").exists());
        for setup in [
            "function codex { printf custom; return 9; }",
            "shopt -s expand_aliases\nalias codex='printf custom'",
        ] {
            let output = run(setup, &[]);
            assert_eq!(output.stdout, b"custom");
        }
    }

    #[test]
    fn quotes_metacharacters_without_execution() {
        assert_eq!(
            quote("a'b $(touch /tmp/nope)", "bash").unwrap(),
            "'a'\\''b $(touch /tmp/nope)'"
        );
        assert_eq!(
            quote("C:\\it's a file", "pwsh").unwrap(),
            "'C:\\it''s a file'"
        );
        assert_eq!(quote("C:\\a & b", "cmd").unwrap(), "\"C:\\a & b\"");
        assert!(quote("%PATH%", "cmd").is_err());
        assert!(quote("file\ncommand", "bash").is_err());
    }
}
