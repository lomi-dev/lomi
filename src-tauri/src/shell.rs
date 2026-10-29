use portable_pty::CommandBuilder;

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
    build_with_startup(profile, cwd, integration, None)
}

pub fn build_with_cli(
    profile: &Profile,
    cwd: &str,
    integration: &Path,
    program: &Path,
    argument: Option<&str>,
) -> Result<(CommandBuilder, String), String> {
    if profile.distro.is_some() {
        return Err("Agent launch is not supported in WSL terminals yet.".into());
    }
    if !matches!(profile.kind.as_str(), "bash" | "zsh" | "fish" | "sh") {
        return Err(format!(
            "Agent launch is not supported in {} terminals yet.",
            profile.name
        ));
    }
    let (mut command, cwd) = build_with_command(
        profile,
        cwd,
        integration,
        cli_startup(profile.kind.as_str())?,
    )?;
    if profile.kind == "fish" {
        command.arg(program);
        if let Some(argument) = argument {
            command.arg(argument);
        }
    } else {
        command.arg("lomi-cli-launch");
        command.arg(program);
        if let Some(argument) = argument {
            command.arg(argument);
        }
    }
    Ok((command, cwd))
}

pub fn build_with_command(
    profile: &Profile,
    cwd: &str,
    integration: &Path,
    startup: &str,
) -> Result<(CommandBuilder, String), String> {
    if profile.distro.is_some() {
        return Err("This startup command is not supported in WSL terminals.".into());
    }
    if !matches!(profile.kind.as_str(), "bash" | "zsh" | "fish" | "sh") {
        return Err(format!(
            "This startup command is not supported in {} terminals.",
            profile.name
        ));
    }
    build_with_startup(profile, cwd, integration, Some(startup))
}

const POSIX_CLI_STARTUP: &str =
    "if [ -n \"${2-}\" ]; then exec \"$1\" \"$2\"; else exec \"$1\"; fi";
const FISH_CLI_STARTUP: &str =
    "if test -n \"$argv[2]\"\n    exec \"$argv[1]\" \"$argv[2]\"\nelse\n    exec \"$argv[1]\"\nend";

fn cli_startup(kind: &str) -> Result<&'static str, String> {
    match kind {
        "bash" | "zsh" | "sh" => Ok(POSIX_CLI_STARTUP),
        "fish" => Ok(FISH_CLI_STARTUP),
        _ => Err("Agent launch is not supported by this shell.".into()),
    }
}

fn build_with_startup(
    profile: &Profile,
    cwd: &str,
    integration: &Path,
    startup: Option<&str>,
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
    match profile.kind.as_str() {
        "bash" => {
            command.args(["--rcfile", &join("bash.sh"), "-i"]);
            if let Some(startup) = startup {
                command.args(["-c", startup]);
            }
        }
        "zsh" => {
            command.env(
                "LOMI_ZDOTDIR",
                env::var("ZDOTDIR").unwrap_or_else(|_| profile.home.clone()),
            );
            command.env("ZDOTDIR", join("zsh"));
            if cfg!(target_os = "macos") && profile.distro.is_none() {
                command.arg("-l");
            }
            command.arg("-i");
            if let Some(startup) = startup {
                command.args(["-c", startup]);
            }
        }
        "fish" => {
            let integration_script = format!("source {}", quote(&join("fish.fish"), "fish")?);
            command.args(["-i", "-C", &integration_script]);
            if let Some(startup) = startup {
                command.args(["-c", startup]);
            }
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
            if startup.is_some() {
                return Err("Agent launch is not supported in cmd terminals yet.".into());
            }
            command.args(["/Q", "/D", "/V:OFF"]);
            command.env(
                "PROMPT",
                "$E]133;D$E\\$E]7;file://localhost/$P$E\\$E]133;A$E\\$P$G$E]133;B$E\\",
            );
        }
        _ => {
            command.arg("-i");
            if let Some(startup) = startup {
                command.args(["-c", startup]);
            }
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
