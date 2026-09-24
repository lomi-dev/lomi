pub use crate::cli_catalog::TitleCli;
use crate::cli_config::{read, revision};
use crate::{files::main_window, terminal::Terminals};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[cfg(any(target_os = "linux", test))]
use std::fs;
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{State, Window};
use toml_edit::{Array, DocumentMut, Item, Table};

const LIMIT: u64 = 1024 * 1024;

#[derive(Default)]
pub struct CliTitleConfig(pub(crate) Mutex<()>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleSetup {
    cli: TitleCli,
    path: String,
    revision: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TitleProcess {
    pub cli: TitleCli,
    pub pid: u32,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn identify(executable: &Path, argv: &[&[u8]]) -> Option<TitleCli> {
    use std::os::unix::ffi::OsStrExt;
    match executable.file_name()?.to_str()? {
        "codex" => Some(TitleCli::Codex),
        "agy" => Some(TitleCli::Agy),
        "claude" | "claude.exe" => Some(TitleCli::Claude),
        "cursor-agent" | "cursor-agent-sea" => Some(TitleCli::Cursor),
        "node" | "nodejs" | "bun" => {
            // Only match the executable name or the first script passed to Node/Bun.
            // Never search prompt text, later arguments, or shell command contents.
            let invoked = Path::new(std::ffi::OsStr::from_bytes(argv.first()?));
            if invoked.file_name().is_some_and(|name| name == "claude") {
                return Some(TitleCli::Claude);
            }
            let script = node_script(argv.get(1..)?, executable.file_name()?)?;
            identify_node_script(
                executable,
                Path::new(std::ffi::OsStr::from_bytes(script.script)),
            )
        }
        name if python_runtime(name) => identify_python(argv),
        // Native Claude installations use the version number as the binary filename.
        _ if executable.parent()?.ends_with("claude/versions") => Some(TitleCli::Claude),
        name => identify_name(name),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn identify_name(name: &str) -> Option<TitleCli> {
    Some(match name {
        "gemini" => TitleCli::Gemini,
        "copilot" => TitleCli::Copilot,
        "opencode" => TitleCli::Opencode,
        "openclaw" => TitleCli::Openclaw,
        "hermes" => TitleCli::Hermes,
        "pi" => TitleCli::Pi,
        "aider" => TitleCli::Aider,
        "goose" => TitleCli::Goose,
        "cline" => TitleCli::Cline,
        "kilo" | "kilocode" => TitleCli::Kilo,
        "qwen" => TitleCli::Qwen,
        "kiro-cli" => TitleCli::Kiro,
        "droid" => TitleCli::Droid,
        "openhands" => TitleCli::Openhands,
        "cn" => TitleCli::Continue,
        "amp" => TitleCli::Amp,
        "auggie" => TitleCli::Auggie,
        "crush" => TitleCli::Crush,
        "vibe" => TitleCli::Vibe,
        "kimi" => TitleCli::Kimi,
        "interpreter" => TitleCli::Interpreter,
        "grok" => TitleCli::Grok,
        "junie" => TitleCli::Junie,
        "deepagents" | "deepagents-code" | "dcode" => TitleCli::Deepagents,
        "freebuff" => TitleCli::Freebuff,
        "trae-cli" => TitleCli::Trae,
        "sweagent" => TitleCli::Sweagent,
        _ => return None,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn identify_script(script: &Path) -> Option<TitleCli> {
    // Installed console entrypoints may be passed through a runtime unchanged.
    if script
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "bin" || name == ".bin")
    {
        if let Some(cli) = script
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(identify_name)
        {
            return Some(cli);
        }
    }
    for (suffix, cli) in [
        ("@google/gemini-cli/dist/index.js", TitleCli::Gemini),
        ("@google/gemini-cli/bundle/gemini.js", TitleCli::Gemini),
        ("@github/copilot/index.js", TitleCli::Copilot),
        ("opencode-ai/bin/opencode", TitleCli::Opencode),
        ("opencode/bin/opencode", TitleCli::Opencode),
        ("openclaw/openclaw.mjs", TitleCli::Openclaw),
        ("freebuff/index.js", TitleCli::Freebuff),
        ("@mariozechner/pi-coding-agent/dist/cli.js", TitleCli::Pi),
        (
            "@earendil-works/pi-coding-agent/dist/bundle/cli.js",
            TitleCli::Pi,
        ),
        ("@qwen-code/qwen-code/dist/index.js", TitleCli::Qwen),
        ("@qwen-code/qwen-code/cli-entry.js", TitleCli::Qwen),
        ("qwen-code/lib/cli.js", TitleCli::Qwen),
        ("@continuedev/cli/dist/cn.js", TitleCli::Continue),
        ("@augmentcode/auggie/augment.mjs", TitleCli::Auggie),
        ("@kilocode/cli/bin/kilo", TitleCli::Kilo),
    ] {
        if script.ends_with(suffix) {
            return Some(cli);
        }
    }
    None
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn identify_node_script(executable: &Path, script: &Path) -> Option<TitleCli> {
    if script.ends_with("@anthropic-ai/claude-code/cli.js") {
        return Some(TitleCli::Claude);
    }
    if script.ends_with("@openai/codex/bin/codex.js") {
        return Some(TitleCli::Codex);
    }
    if script.file_name().is_some_and(|name| name == "index.js")
        && script.parent() == executable.parent()
        && executable.with_file_name("cursor-agent").is_file()
    {
        return Some(TitleCli::Cursor);
    }
    identify_script(script)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn python_runtime(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    let name = normalized.as_str();
    name == "python"
        || name.strip_prefix("python").is_some_and(|version| {
            !version.is_empty()
                && version
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'.')
        })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn needs_arguments(executable: &Path) -> bool {
    executable
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, "node" | "nodejs" | "bun") || python_runtime(name))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn identify_python(argv: &[&[u8]]) -> Option<TitleCli> {
    python_invocation(argv).map(|invocation| invocation.cli)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct PythonInvocation<'a> {
    cli: TitleCli,
    arguments: &'a [&'a [u8]],
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn python_invocation<'a>(argv: &'a [&'a [u8]]) -> Option<PythonInvocation<'a>> {
    use std::os::unix::ffi::OsStrExt;
    let mut index = 1;
    while let Some(arg) = argv.get(index).copied() {
        match arg {
            b"-m" => {
                let module = argv.get(index + 1).copied()?;
                let cli = match module {
                    b"aider" | b"aider.main" => TitleCli::Aider,
                    b"hermes_cli" | b"hermes_cli.main" => TitleCli::Hermes,
                    b"openhands_cli" => TitleCli::Openhands,
                    b"interpreter" => TitleCli::Interpreter,
                    b"sweagent" | b"sweagent.run.run" => TitleCli::Sweagent,
                    b"trae_agent" | b"trae_agent.cli" => TitleCli::Trae,
                    b"deepagents_cli" | b"deepagents_code" => TitleCli::Deepagents,
                    b"kimi_cli" | b"kimi_code" => TitleCli::Kimi,
                    _ => return None,
                };
                return Some(PythonInvocation {
                    cli,
                    arguments: &argv[index + 2..],
                });
            }
            b"-c" | b"-" => return None,
            b"-W" | b"-X" => {
                argv.get(index + 1)?;
                index += 2;
            }
            b"-u" | b"-B" | b"-E" | b"-I" | b"-s" | b"-S" | b"-O" | b"-OO" => {
                index += 1;
            }
            b"--" => {
                let script = argv.get(index + 1).copied()?;
                let cli = identify_script(Path::new(std::ffi::OsStr::from_bytes(script)))?;
                return Some(PythonInvocation {
                    cli,
                    arguments: &argv[index + 2..],
                });
            }
            _ if arg.starts_with(b"-") => return None,
            _ => {
                let cli = identify_script(Path::new(std::ffi::OsStr::from_bytes(arg)))?;
                return Some(PythonInvocation {
                    cli,
                    arguments: &argv[index + 1..],
                });
            }
        }
    }
    None
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct NodeInvocation<'a> {
    script: &'a [u8],
    arguments: &'a [&'a [u8]],
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn node_script<'a>(args: &'a [&'a [u8]], runtime: &std::ffi::OsStr) -> Option<NodeInvocation<'a>> {
    let is_bun = runtime == "bun";
    let mut index = usize::from(is_bun && args.first().is_some_and(|arg| *arg == b"run"));
    while let Some(arg) = args.get(index).copied() {
        if arg == b"--" {
            let script_index = index + 1;
            return Some(NodeInvocation {
                script: args.get(script_index)?,
                arguments: args.get(script_index + 1..).unwrap_or_default(),
            });
        }
        if !arg.starts_with(b"-") {
            return Some(NodeInvocation {
                script: arg,
                arguments: args.get(index + 1..).unwrap_or_default(),
            });
        }
        // Eval/print modes execute inline text, which must never identify a CLI.
        if matches!(arg, b"-e" | b"--eval" | b"-p" | b"--print") {
            return None;
        }
        if matches!(
            arg,
            b"-r"
                | b"--require"
                | b"--import"
                | b"--loader"
                | b"--experimental-loader"
                | b"--inspect-port"
                | b"--watch-path"
                | b"--conditions"
                | b"-C"
        ) {
            args.get(index + 1)?;
            index += 2;
        } else if arg.starts_with(b"--") {
            // Node accepts both --option value and --option=value. Unknown options
            // make the script position ambiguous, so fail closed.
            let option = arg.split(|byte| *byte == b'=').next()?;
            if !matches!(
                option,
                b"--no-warnings"
                    | b"--use-system-ca"
                    | b"--trace-warnings"
                    | b"--enable-source-maps"
                    | b"--experimental-strip-types"
                    | b"--experimental-transform-types"
                    | b"--watch"
                    | b"--watch-path"
            ) {
                return None;
            }
            index += 1;
        } else {
            return None;
        }
    }
    None
}

#[cfg(target_os = "linux")]
pub fn process_in_group(group: u32) -> Option<TitleProcess> {
    let mut pending = vec![group];
    // Bound /proc traversal to 64 wrapper descendants per terminal group.
    for _ in 0..64 {
        let pid = pending.pop()?;
        let root = PathBuf::from(format!("/proc/{pid}"));
        let Ok(stat) = fs::read_to_string(root.join("stat")) else {
            continue;
        };
        let process_group = stat
            .rsplit_once(") ")
            .and_then(|(_, fields)| fields.split_whitespace().nth(2))
            .and_then(|value| value.parse::<u32>().ok());
        if process_group != Some(group) {
            continue;
        }
        if let Ok(executable) = fs::read_link(root.join("exe")) {
            let mut argv = Vec::new();
            if needs_arguments(&executable) {
                if let Ok(file) = fs::File::open(root.join("cmdline")) {
                    let _ = file.take(4096).read_to_end(&mut argv);
                }
            }
            let argv = argv.split(|byte| *byte == 0).collect::<Vec<_>>();
            if let Some(cli) = identify(&executable, &argv) {
                return Some(TitleProcess { cli, pid });
            }
        }
        if let Ok(children) = fs::read_to_string(root.join(format!("task/{pid}/children"))) {
            pending.extend(
                children
                    .split_whitespace()
                    .filter_map(|pid| pid.parse::<u32>().ok())
                    .take(64 - pending.len().min(64)),
            );
        }
    }
    None
}

#[cfg(target_os = "macos")]
pub(crate) fn process_in_group(group: u32) -> Option<TitleProcess> {
    macos_process::process_in_group(group)
}

#[cfg(target_os = "macos")]
mod macos_process {
    use super::{identify, needs_arguments, parse_process_args, TitleProcess, MAX_PROCESS_ARGS};
    use std::{
        collections::HashSet, ffi::OsString, mem::MaybeUninit, os::unix::ffi::OsStringExt,
        path::PathBuf,
    };

    #[link(name = "proc")]
    unsafe extern "C" {
        fn proc_listchildpids(
            ppid: libc::pid_t,
            buffer: *mut libc::c_void,
            buffersize: libc::c_int,
        ) -> libc::c_int;
        fn proc_pidinfo(
            pid: libc::c_int,
            flavor: libc::c_int,
            arg: u64,
            buffer: *mut libc::c_void,
            buffersize: libc::c_int,
        ) -> libc::c_int;
        fn proc_pidpath(
            pid: libc::c_int,
            buffer: *mut libc::c_void,
            buffersize: u32,
        ) -> libc::c_int;
    }

    fn process_info(pid: u32) -> Option<libc::proc_bsdinfo> {
        let mut info = MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let count = unsafe {
            proc_pidinfo(
                pid as libc::c_int,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                size,
            )
        };
        (count == size).then(|| unsafe { info.assume_init() })
    }

    pub(super) fn process_path(pid: u32) -> Option<PathBuf> {
        let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let count = unsafe {
            proc_pidpath(
                pid as libc::c_int,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
            )
        };
        if count <= 0 {
            return None;
        }
        buffer.truncate(buffer.iter().position(|byte| *byte == 0)?);
        (!buffer.is_empty()).then(|| PathBuf::from(OsString::from_vec(buffer)))
    }

    fn children(pid: u32, limit: usize) -> Vec<u32> {
        if limit == 0 {
            return Vec::new();
        }
        let mut buffer = [0i32; 64];
        let count = unsafe {
            proc_listchildpids(
                pid as libc::pid_t,
                buffer.as_mut_ptr().cast(),
                std::mem::size_of_val(&buffer) as libc::c_int,
            )
        };
        if count <= 0 {
            return Vec::new();
        }
        buffer
            .iter()
            .take((count as usize).min(limit))
            .filter_map(|pid| (*pid > 0).then_some(*pid as u32))
            .collect()
    }

    pub(super) fn process_in_group(group: u32) -> Option<TitleProcess> {
        let mut pending = vec![group];
        let mut visited = HashSet::new();
        let mut examined = 0usize;
        while examined < 64 {
            let pid = pending.pop()?;
            if pid <= 1 || !visited.insert(pid) {
                continue;
            }
            examined += 1;
            let Some(info) = process_info(pid) else {
                continue;
            };
            if info.pbi_pid != pid || info.pbi_pgid != group {
                continue;
            }
            if let Some(executable) = process_path(pid) {
                let cli = if needs_arguments(&executable) {
                    process_args(pid).and_then(|bytes| {
                        parse_process_args(&bytes)
                            .and_then(|parsed| identify(&executable, &parsed.argv))
                    })
                } else {
                    identify(&executable, &[])
                };
                if let Some(cli) = cli {
                    return Some(TitleProcess { cli, pid });
                }
            }
            let remaining = 64usize.saturating_sub(examined + pending.len());
            pending.extend(children(pid, remaining));
        }
        None
    }

    pub(super) fn process_args(pid: u32) -> Option<Vec<u8>> {
        let mut name = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
        let mut length = 0usize;
        let result = unsafe {
            libc::sysctl(
                name.as_mut_ptr(),
                name.len() as libc::c_uint,
                std::ptr::null_mut(),
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        };
        if result != 0 || length == 0 || length > MAX_PROCESS_ARGS {
            return None;
        }
        let mut bytes = vec![0u8; length];
        let mut actual = bytes.len();
        let result = unsafe {
            libc::sysctl(
                name.as_mut_ptr(),
                name.len() as libc::c_uint,
                bytes.as_mut_ptr().cast(),
                &mut actual,
                std::ptr::null_mut(),
                0,
            )
        };
        if result != 0 || actual > bytes.len() {
            return None;
        }
        bytes.truncate(actual);
        Some(bytes)
    }
}

#[cfg(any(target_os = "macos", test))]
const MAX_PROCESS_ARGS: usize = 1024 * 1024;

#[cfg(any(target_os = "macos", test))]
struct ParsedProcessArgs<'a> {
    argv: Vec<&'a [u8]>,
    environment: Vec<&'a [u8]>,
}

#[cfg(any(target_os = "macos", test))]
fn parse_process_args(bytes: &[u8]) -> Option<ParsedProcessArgs<'_>> {
    const MAX_ITEMS: usize = 4096;

    if bytes.len() < std::mem::size_of::<i32>() || bytes.len() > MAX_PROCESS_ARGS {
        return None;
    }
    let argc = i32::from_ne_bytes(bytes[..4].try_into().ok()?);
    if !(1..=MAX_ITEMS as i32).contains(&argc) {
        return None;
    }
    let mut index = 4usize;
    let executable_end = bytes[index..].iter().position(|byte| *byte == 0)? + index;
    if executable_end == index {
        return None;
    }
    index = executable_end + 1;
    while bytes.get(index) == Some(&0) {
        index += 1;
    }

    let mut argv = Vec::with_capacity(argc as usize);
    for _ in 0..argc {
        let end = bytes.get(index..)?.iter().position(|byte| *byte == 0)? + index;
        argv.push(&bytes[index..end]);
        index = end + 1;
    }
    while bytes.get(index) == Some(&0) {
        index += 1;
    }

    let mut environment = Vec::new();
    while index < bytes.len() {
        let end = bytes.get(index..)?.iter().position(|byte| *byte == 0)? + index;
        if end == index {
            break;
        }
        environment.push(&bytes[index..end]);
        if environment.len() > MAX_ITEMS {
            return None;
        }
        index = end + 1;
    }
    Some(ParsedProcessArgs { argv, environment })
}

#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn environment_value<'a>(entries: &[&'a [u8]], name: &[u8]) -> Option<&'a [u8]> {
    entries
        .iter()
        .find_map(|entry| entry.strip_prefix(name).filter(|value| !value.is_empty()))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn environment_path(value: Option<&[u8]>) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    value.map(|value| PathBuf::from(std::ffi::OsString::from_vec(value.to_vec())))
}

#[cfg(all(windows, test))]
fn environment_path(value: Option<&[u8]>) -> Option<PathBuf> {
    value.map(|value| PathBuf::from(String::from_utf8_lossy(value).into_owned()))
}

#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn resolved_config_path(directory: &Path, filename: &str) -> Result<PathBuf, String> {
    if !directory.is_absolute() {
        return Err("CLI configuration setup requires an absolute configuration directory.".into());
    }
    let mut ancestor = directory;
    let mut missing = Vec::new();
    while !ancestor.try_exists().map_err(|error| error.to_string())? {
        missing.push(
            ancestor
                .file_name()
                .ok_or("Invalid CLI configuration directory.")?,
        );
        ancestor = ancestor
            .parent()
            .ok_or("Invalid CLI configuration directory.")?;
    }
    let mut path = ancestor.canonicalize().map_err(|error| error.to_string())?;
    for component in missing.iter().rev() {
        path.push(component);
    }
    path.push(filename);
    match path.symlink_metadata() {
        Ok(_) => path.canonicalize().map_err(|error| error.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn title_configuration_from_environment(
    cli: TitleCli,
    entries: &[&[u8]],
) -> Result<(PathBuf, bool), String> {
    let home = environment_path(environment_value(entries, b"HOME="));
    let path_variable = |name| environment_path(environment_value(entries, name));
    let (directory, filename) = match cli {
        TitleCli::Codex => (
            path_variable(b"CODEX_HOME=").or_else(|| home.map(|home| home.join(".codex"))),
            "config.toml",
        ),
        TitleCli::Agy => (
            home.map(|home| home.join(".gemini/antigravity-cli")),
            "settings.json",
        ),
        TitleCli::Cursor => (
            path_variable(b"CURSOR_CONFIG_DIR=")
                .or_else(|| path_variable(b"XDG_CONFIG_HOME=").map(|home| home.join("cursor")))
                .or_else(|| home.map(|home| home.join(".cursor"))),
            "cli-config.json",
        ),
        TitleCli::Claude => (
            path_variable(b"CLAUDE_CONFIG_DIR=").or_else(|| home.map(|home| home.join(".claude"))),
            "settings.json",
        ),
        TitleCli::Gemini => (
            path_variable(b"GEMINI_CLI_HOME=")
                .or(home)
                .map(|home| home.join(".gemini")),
            "settings.json",
        ),
        TitleCli::Qwen => {
            return crate::cli_mcp::configuration_path(cli, entries).map(|path| (path, false))
        }
        _ => return Err("Automatic title configuration is not available for this CLI.".into()),
    };
    let directory = directory.ok_or("Cannot locate the running CLI configuration directory.")?;
    let path = resolved_config_path(&directory, filename)?;
    let disabled = environment_value(entries, b"CLAUDE_CODE_DISABLE_TERMINAL_TITLE=")
        .is_some_and(|value| truthy(&String::from_utf8_lossy(value)));
    Ok((path, disabled))
}

#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn mcp_configuration_from_environment(cli: TitleCli, entries: &[&[u8]]) -> Result<PathBuf, String> {
    let home = environment_path(environment_value(entries, b"HOME="))
        .ok_or("Cannot locate the running CLI home directory.")?;
    let (directory, filename) = match cli {
        TitleCli::Codex => (
            environment_path(environment_value(entries, b"CODEX_HOME="))
                .unwrap_or_else(|| home.join(".codex")),
            "config.toml",
        ),
        TitleCli::Agy => (home.join(".gemini/config"), "mcp_config.json"),
        TitleCli::Cursor => (home.join(".cursor"), "mcp.json"),
        TitleCli::Claude => {
            if environment_value(entries, b"CLAUDE_CONFIG_DIR=").is_some() {
                return Err("Cannot safely resolve Claude MCP configuration while CLAUDE_CONFIG_DIR is set.".into());
            }
            (home, ".claude.json")
        }
        _ => return crate::cli_mcp::configuration_path(cli, entries),
    };
    resolved_config_path(&directory, filename)
}

#[cfg(target_os = "linux")]
fn process_environment(pid: u32) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    fs::File::open(format!("/proc/{pid}/environ"))
        .and_then(|file| file.take(LIMIT + 1).read_to_end(&mut bytes))
        .map_err(|_| "Cannot read the running CLI configuration location.")?;
    if bytes.len() as u64 > LIMIT {
        return Err("The CLI process environment exceeds 1 MiB.".into());
    }
    Ok(bytes)
}

#[cfg(target_os = "linux")]
pub(crate) fn configuration(process: TitleProcess) -> Result<(PathBuf, bool), String> {
    if matches!(process.cli, TitleCli::Gemini | TitleCli::Qwen) {
        let mut args = Vec::new();
        fs::File::open(format!("/proc/{}/cmdline", process.pid))
            .and_then(|file| file.take(LIMIT + 1).read_to_end(&mut args))
            .map_err(|_| "Cannot read CLI configuration arguments.")?;
        if args.len() as u64 > LIMIT {
            return Err("CLI arguments exceed 1 MiB.".into());
        }
        let executable = fs::read_link(format!("/proc/{}/exe", process.pid))
            .map_err(|_| "Cannot read the running CLI executable.")?;
        check_configuration_arguments(
            process.cli,
            &executable,
            &args.split(|byte| *byte == 0).collect::<Vec<_>>(),
        )?;
    }
    let bytes = process_environment(process.pid)?;
    let entries = bytes.split(|byte| *byte == 0).collect::<Vec<_>>();
    title_configuration_from_environment(process.cli, &entries)
}

#[cfg(target_os = "macos")]
pub(crate) fn configuration(process: TitleProcess) -> Result<(PathBuf, bool), String> {
    let bytes = macos_process::process_args(process.pid)
        .ok_or("Cannot read the running CLI configuration location.")?;
    let parsed =
        parse_process_args(&bytes).ok_or("Cannot parse the running CLI configuration location.")?;
    let executable = macos_process::process_path(process.pid)
        .ok_or("Cannot read the running CLI executable.")?;
    check_configuration_arguments(process.cli, &executable, &parsed.argv)?;
    title_configuration_from_environment(process.cli, &parsed.environment)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cli_arguments<'a>(
    cli: TitleCli,
    executable: &Path,
    argv: &'a [&'a [u8]],
) -> Result<&'a [&'a [u8]], String> {
    let executable_name = executable.file_name().and_then(|name| name.to_str());
    match executable_name {
        Some("node" | "nodejs" | "bun") => {
            if identify(executable, argv) != Some(cli) {
                return Err("Cannot safely determine the running CLI argument boundary.".into());
            }
            use std::os::unix::ffi::OsStrExt;
            let runtime = executable
                .file_name()
                .ok_or("Cannot safely determine the running CLI argument boundary.")?;
            let invocation = node_script(
                argv.get(1..)
                    .ok_or("Cannot safely determine the running CLI argument boundary.")?,
                runtime,
            )
            .ok_or("Cannot safely determine the running CLI argument boundary.")?;
            let script = Path::new(std::ffi::OsStr::from_bytes(invocation.script));
            if identify_node_script(executable, script) != Some(cli) {
                return Err("Cannot safely determine the running CLI argument boundary.".into());
            }
            Ok(invocation.arguments)
        }
        Some(name) if python_runtime(name) => {
            let invocation = python_invocation(argv)
                .filter(|invocation| invocation.cli == cli)
                .ok_or("Cannot safely determine the running CLI argument boundary.")?;
            Ok(invocation.arguments)
        }
        _ => Ok(argv.get(1..).unwrap_or_default()),
    }
}

#[cfg(all(test, not(any(target_os = "linux", target_os = "macos"))))]
fn cli_arguments<'a>(
    _cli: TitleCli,
    _executable: &Path,
    argv: &'a [&'a [u8]],
) -> Result<&'a [&'a [u8]], String> {
    Ok(argv.get(1..).unwrap_or_default())
}

#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn check_configuration_arguments(
    cli: TitleCli,
    executable: &Path,
    argv: &[&[u8]],
) -> Result<(), String> {
    if matches!(
        cli,
        TitleCli::Codex | TitleCli::Claude | TitleCli::Cursor | TitleCli::Agy
    ) {
        return Ok(());
    }
    let arguments = cli_arguments(cli, executable, argv)?;
    for arg in arguments.iter().take_while(|arg| **arg != b"--") {
        let flag = arg.split(|byte| *byte == b'=').next().unwrap_or_default();
        if matches!(
            flag,
            b"--config"
                | b"--config-file"
                | b"--config-location"
                | b"--settings-file"
                | b"--data-dir"
                | b"--mcp-config"
                | b"--additional-mcp-config"
                | b"--profile"
                | b"--dev"
        ) {
            return Err("This CLI was started with a configuration override. Register Lomi in that configuration, or use Settings for the default user configuration.".into());
        }
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn check_mcp_configuration_arguments(
    cli: TitleCli,
    executable: &Path,
    argv: &[&[u8]],
) -> Result<(), String> {
    let arguments = cli_arguments(cli, executable, argv)?;
    for arg in arguments.iter().take_while(|arg| **arg != b"--") {
        let flag = arg.split(|byte| *byte == b'=').next().unwrap_or_default();
        if matches!(
            flag,
            b"--mcp-config" | b"--strict-mcp-config" | b"--additional-mcp-config"
        ) {
            return Err("This CLI was started with a custom MCP configuration. Register Lomi there or use Settings for its default user configuration.".into());
        }
    }
    check_configuration_arguments(cli, executable, argv)
}

#[cfg(target_os = "linux")]
pub(crate) fn mcp_configuration(process: TitleProcess) -> Result<PathBuf, String> {
    let mut args = Vec::new();
    fs::File::open(format!("/proc/{}/cmdline", process.pid))
        .and_then(|file| file.take(LIMIT + 1).read_to_end(&mut args))
        .map_err(|_| "Cannot read the running CLI configuration arguments.")?;
    if args.len() as u64 > LIMIT {
        return Err("CLI arguments exceed 1 MiB.".into());
    }
    let executable = fs::read_link(format!("/proc/{}/exe", process.pid))
        .map_err(|_| "Cannot read the running CLI executable.")?;
    check_mcp_configuration_arguments(
        process.cli,
        &executable,
        &args.split(|byte| *byte == 0).collect::<Vec<_>>(),
    )?;
    let bytes = process_environment(process.pid)?;
    let entries = bytes.split(|byte| *byte == 0).collect::<Vec<_>>();
    mcp_configuration_from_environment(process.cli, &entries)
}

#[cfg(target_os = "macos")]
pub(crate) fn mcp_configuration(process: TitleProcess) -> Result<PathBuf, String> {
    let bytes = macos_process::process_args(process.pid)
        .ok_or("Cannot read the running CLI configuration location.")?;
    let parsed =
        parse_process_args(&bytes).ok_or("Cannot parse the running CLI configuration location.")?;
    let executable = macos_process::process_path(process.pid)
        .ok_or("Cannot read the running CLI executable.")?;
    check_mcp_configuration_arguments(process.cli, &executable, &parsed.argv)?;
    mcp_configuration_from_environment(process.cli, &parsed.environment)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn configuration(_process: TitleProcess) -> Result<(PathBuf, bool), String> {
    Err("Automatic CLI title setup is currently available on Linux and macOS.".into())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn mcp_configuration(_process: TitleProcess) -> Result<PathBuf, String> {
    Err("Automatic MCP configuration setup is currently available on Linux and macOS.".into())
}

fn truthy(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn document(source: Option<&str>) -> Result<DocumentMut, String> {
    let doc = source
        .unwrap_or_default()
        .parse::<DocumentMut>()
        .map_err(|_| "CLI configuration is not valid TOML. The file was left intact.")?;
    if let Some(tui) = doc.get("tui") {
        let table = tui
            .as_table_like()
            .ok_or("Codex tui settings must be a TOML table.")?;
        if let Some(titles) = table.get("terminal_title") {
            if !titles
                .as_array()
                .is_some_and(|items| items.iter().all(|item| item.is_str()))
            {
                return Err(
                    "Codex terminal_title must be an array of strings. The file was left intact."
                        .into(),
                );
            }
        }
    }
    Ok(doc)
}

fn agy_command() -> Result<String, String> {
    let executable = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_exe().map_err(|error| error.to_string())?)
        .canonicalize()
        .map_err(|_| "Cannot locate Lomi's title formatter. Restart Lomi and try again.")?;
    let path = executable
        .to_str()
        .ok_or("The Lomi executable path is not UTF-8.")?;
    Ok(format!(
        "{} --agy-terminal-title",
        crate::shell::quote(path, "bash")?
    ))
}

/// Resolves the active conversation's title without opening a window or reading transcripts.
pub fn print_agy_title() -> Result<(), String> {
    let mut input = String::new();
    std::io::stdin()
        .take(LIMIT + 1)
        .read_to_string(&mut input)
        .map_err(|_| "Cannot read agy title data.")?;
    if input.len() as u64 > LIMIT {
        return Err("agy title data exceeds 1 MiB.".into());
    }
    let data: Value = serde_json::from_str(&input).map_err(|_| "Invalid agy title data.")?;
    if !data.is_object() {
        return Err("agy title data must be a JSON object.".into());
    }
    let annotations = crate::shell::home().join(".gemini/antigravity-cli/annotations");
    println!("{}", agy_title(&data, &annotations));
    Ok(())
}

fn agy_annotation_title(source: &str) -> Option<String> {
    // agy writes title as the first protobuf text field; never match text inside tags.
    let field = regex::Regex::new(r#"^\s*title\s*:\s*("(?:\\.|[^"\\])*")"#).ok()?;
    let quoted = field.captures(source)?.get(1)?.as_str();
    let escapes = regex::Regex::new(r#"\\\\|\\x([0-9a-fA-F]{2})|\\U([0-9a-fA-F]{8})"#).ok()?;
    let quoted = escapes.replace_all(quoted, |captures: &regex::Captures<'_>| {
        if let Some(hex) = captures.get(1) {
            format!("\\u00{}", hex.as_str())
        } else if let Some(hex) = captures.get(2) {
            u32::from_str_radix(hex.as_str(), 16)
                .ok()
                .and_then(char::from_u32)
                .and_then(|character| serde_json::to_string(&character.to_string()).ok())
                .map(|quoted| quoted[1..quoted.len() - 1].to_owned())
                .unwrap_or_else(|| captures[0].to_owned())
        } else {
            captures[0].to_owned()
        }
    });
    serde_json::from_str::<String>(&quoted)
        .ok()
        .filter(|title| !title.trim().is_empty())
}

fn agy_title(data: &Value, annotations: &Path) -> String {
    let id = data["conversation_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .or_else(|| data["session_id"].as_str())
        .filter(|id| !id.is_empty());
    let title = id
        .and_then(|id| {
            if id.len() > 128
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            {
                return None;
            }
            let path = annotations.join(format!("{id}.pbtxt"));
            if !path.symlink_metadata().ok()?.is_file() {
                return None;
            }
            // The summaries database may lag behind active conversations and /resume renames.
            agy_annotation_title(&read(&path).ok()??)
        })
        .or_else(|| {
            data["conversation_title"]
                .as_str()
                .filter(|title| !title.trim().is_empty())
                .map(str::to_owned)
        });
    title
        .as_deref()
        .unwrap_or("agy")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect()
}

fn json_document(cli: TitleCli, source: Option<&str>) -> Result<Value, String> {
    let doc: Value = if matches!(cli, TitleCli::Gemini | TitleCli::Qwen) {
        crate::cli_mcp::json_document(cli, source)?
    } else {
        serde_json::from_str(source.unwrap_or("{}"))
            .map_err(|_| "CLI configuration is not valid JSON. The file was left intact.")?
    };
    if !doc.is_object() {
        return Err("CLI configuration must be a JSON object.".into());
    }
    let (section, key) = match cli {
        TitleCli::Agy => ("title", "enabled"),
        TitleCli::Cursor => ("display", "showStatusIndicators"),
        TitleCli::Claude => ("env", "CLAUDE_CODE_DISABLE_TERMINAL_TITLE"),
        TitleCli::Gemini | TitleCli::Qwen => ("ui", "hideWindowTitle"),
        _ => return Err("Automatic title configuration is not available for this CLI.".into()),
    };
    if let Some(settings) = doc.get(section) {
        if !settings.is_object() {
            return Err(format!("CLI {section} settings must be a JSON object."));
        }
        if let Some(value) = settings.get(key) {
            if !(if cli == TitleCli::Claude {
                value.is_string()
            } else {
                value.is_boolean()
            }) {
                return Err(format!(
                    "CLI {section}.{key} has an invalid type. The file was left intact."
                ));
            }
        }
    }
    if matches!(cli, TitleCli::Gemini | TitleCli::Qwen) {
        for key in ["dynamicWindowTitle", "showStatusInTitle"] {
            if doc
                .get("ui")
                .and_then(|ui| ui.get(key))
                .is_some_and(|value| !value.is_boolean())
            {
                return Err(format!(
                    "CLI ui.{key} must be a boolean. The file was left intact."
                ));
            }
        }
    }
    if cli == TitleCli::Agy {
        for key in ["type", "command"] {
            if doc
                .get("title")
                .and_then(|title| title.get(key))
                .is_some_and(|value| !value.is_string())
            {
                return Err(format!("agy title.{key} must be a string."));
            }
        }
    }
    if cli == TitleCli::Claude
        && doc
            .get("terminalTitleFromRename")
            .is_some_and(|value| !value.is_boolean())
    {
        return Err("Claude Code terminalTitleFromRename must be a boolean.".into());
    }
    Ok(doc)
}

pub(crate) fn configured(
    cli: TitleCli,
    source: Option<&str>,
    disabled: bool,
) -> Result<bool, String> {
    if cli == TitleCli::Codex {
        let doc = document(source)?;
        return Ok(doc
            .get("tui")
            .and_then(|tui| tui.get("terminal_title"))
            .and_then(Item::as_array)
            .is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item.as_str() == Some("thread-title"))
            }));
    }
    let doc = json_document(cli, source)?;
    Ok(match cli {
        TitleCli::Agy => {
            doc["title"]["command"]
                .as_str()
                .is_some_and(|command| !command.trim().is_empty())
                && doc["title"]["enabled"].as_bool() != Some(false)
        }
        TitleCli::Cursor => doc["display"]["showStatusIndicators"] == true,
        TitleCli::Gemini => {
            doc["ui"]["hideWindowTitle"] != true && doc["ui"]["dynamicWindowTitle"] != false
        }
        TitleCli::Qwen => {
            doc["ui"]["hideWindowTitle"] != true && doc["ui"]["showStatusInTitle"] != false
        }
        TitleCli::Claude => {
            !doc["env"]["CLAUDE_CODE_DISABLE_TERMINAL_TITLE"]
                .as_str()
                .map(truthy)
                .unwrap_or(disabled)
                && doc["terminalTitleFromRename"].as_bool() != Some(false)
        }
        _ => return Err("Automatic title configuration is not available for this CLI.".into()),
    })
}

fn inspect(cli: TitleCli, path: &Path, disabled: bool) -> Result<Option<TitleSetup>, String> {
    let source = read(path)?;
    Ok(
        (!configured(cli, source.as_deref(), disabled)?).then(|| TitleSetup {
            cli,
            path: path.to_string_lossy().into_owned(),
            revision: revision(source.as_deref()),
        }),
    )
}

pub(crate) fn enable(cli: TitleCli, path: &Path, expected: Option<&str>) -> Result<(), String> {
    let source = read(path)?;
    let conflict =
        "CLI configuration changed. Check the settings again before allowing the update.";
    if revision(source.as_deref()).as_deref() != expected {
        return Err(conflict.into());
    }
    let output = if cli == TitleCli::Codex {
        let mut doc = document(source.as_deref())?;
        let tui = doc
            .entry("tui")
            .or_insert(Item::Table(Table::new()))
            .as_table_like_mut()
            .ok_or("Codex tui settings must be a TOML table.")?;
        let titles = tui.entry("terminal_title").or_insert(Item::None);
        let mut value =
            toml_edit::Value::Array(["activity", "thread-title"].into_iter().collect::<Array>());
        if let Some(previous) = titles.as_value() {
            *value.decor_mut() = previous.decor().clone();
        }
        *titles = Item::Value(value);
        doc.to_string()
    } else if matches!(cli, TitleCli::Gemini | TitleCli::Qwen) {
        json_document(cli, source.as_deref())?;
        let output = crate::cli_mcp::set_json(
            cli,
            source.as_deref().unwrap_or("{}\n"),
            &["ui", "hideWindowTitle"],
            &json!(false),
        )?;
        let key = if cli == TitleCli::Gemini {
            "dynamicWindowTitle"
        } else {
            "showStatusInTitle"
        };
        crate::cli_mcp::set_json(cli, &output, &["ui", key], &json!(true))?
    } else {
        let mut doc = json_document(cli, source.as_deref())?;
        match cli {
            TitleCli::Agy => {
                if doc["title"]["command"]
                    .as_str()
                    .is_none_or(|command| command.trim().is_empty())
                {
                    doc["title"]["type"] = json!("command");
                    doc["title"]["command"] = json!(agy_command()?);
                }
                doc["title"]["enabled"] = json!(true);
            }
            TitleCli::Cursor => {
                doc["display"]["showStatusIndicators"] = json!(true);
            }
            TitleCli::Claude => {
                doc["env"]["CLAUDE_CODE_DISABLE_TERMINAL_TITLE"] = json!("0");
                if doc["terminalTitleFromRename"] == false {
                    doc["terminalTitleFromRename"] = json!(true);
                }
            }
            _ => return Err("Automatic title configuration is not available for this CLI.".into()),
        }
        format!(
            "{}\n",
            serde_json::to_string_pretty(&doc).map_err(|error| error.to_string())?
        )
    };
    crate::cli_config::write(path, source.as_deref(), output)
}

#[tauri::command]
pub async fn inspect_cli_titles(
    window: Window,
    terminals: State<'_, Terminals>,
    state: State<'_, CliTitleConfig>,
    id: String,
    process: TitleProcess,
) -> Result<Option<TitleSetup>, String> {
    main_window(&window)?;
    let _guard = state.0.lock().map_err(|error| error.to_string())?;
    terminals.check_title_process(&id, process)?;
    let (path, disabled) = configuration(process)?;
    inspect(process.cli, &path, disabled)
}

#[tauri::command]
pub async fn enable_cli_titles(
    window: Window,
    terminals: State<'_, Terminals>,
    state: State<'_, CliTitleConfig>,
    id: String,
    process: TitleProcess,
    path: String,
    revision: Option<String>,
) -> Result<(), String> {
    main_window(&window)?;
    let _guard = state.0.lock().map_err(|error| error.to_string())?;
    terminals.check_title_process(&id, process)?;
    let (current, _) = configuration(process)?;
    if current != Path::new(&path) {
        return Err(
            "The running CLI configuration location changed. Check the settings again.".into(),
        );
    }
    enable(process.cli, &current, revision.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_cli_process() {
        if std::env::var_os("LOMI_CLI_PROCESS_FIXTURE").is_some() {
            loop {
                std::thread::park_timeout(std::time::Duration::from_secs(60));
            }
        }
    }

    #[cfg(unix)]
    fn process_args_fixture(argv: &[&[u8]], environment: &[&[u8]]) -> Vec<u8> {
        let mut bytes = (argv.len() as i32).to_ne_bytes().to_vec();
        bytes.extend_from_slice(b"/usr/bin/node\0\0");
        for argument in argv {
            bytes.extend_from_slice(argument);
            bytes.push(0);
        }
        for value in environment {
            bytes.extend_from_slice(value);
            bytes.push(0);
        }
        bytes.push(0);
        bytes
    }

    #[cfg(unix)]
    #[test]
    fn detects_extended_cli_entrypoints_without_matching_prompt_arguments() {
        for (binary, cli) in [
            ("gemini", TitleCli::Gemini),
            ("copilot", TitleCli::Copilot),
            ("opencode", TitleCli::Opencode),
            ("openclaw", TitleCli::Openclaw),
            ("hermes", TitleCli::Hermes),
            ("pi", TitleCli::Pi),
            ("aider", TitleCli::Aider),
            ("goose", TitleCli::Goose),
            ("cline", TitleCli::Cline),
            ("kilo", TitleCli::Kilo),
            ("qwen", TitleCli::Qwen),
            ("kiro-cli", TitleCli::Kiro),
            ("droid", TitleCli::Droid),
            ("openhands", TitleCli::Openhands),
            ("cn", TitleCli::Continue),
            ("amp", TitleCli::Amp),
            ("auggie", TitleCli::Auggie),
            ("crush", TitleCli::Crush),
            ("vibe", TitleCli::Vibe),
            ("kimi", TitleCli::Kimi),
            ("interpreter", TitleCli::Interpreter),
            ("grok", TitleCli::Grok),
            ("junie", TitleCli::Junie),
            ("dcode", TitleCli::Deepagents),
            ("deepagents-code", TitleCli::Deepagents),
            ("freebuff", TitleCli::Freebuff),
            ("trae-cli", TitleCli::Trae),
            ("sweagent", TitleCli::Sweagent),
        ] {
            assert_eq!(
                super::identify(&Path::new("/opt/bin").join(binary), &[]),
                Some(cli),
                "{binary}"
            );
            let script = format!("/opt/bin/{binary}");
            assert_eq!(
                super::identify(
                    Path::new("/usr/bin/python3.12"),
                    &[b"python3", script.as_bytes()]
                ),
                Some(cli),
                "{binary}"
            );
            assert_eq!(
                super::identify(
                    Path::new("/usr/bin/node"),
                    &[b"node", b"/tmp/other.js", script.as_bytes()]
                ),
                None
            );
        }
        for (script, cli) in [
            ("@google/gemini-cli/bundle/gemini.js", TitleCli::Gemini),
            ("@github/copilot/index.js", TitleCli::Copilot),
            (
                "@earendil-works/pi-coding-agent/dist/bundle/cli.js",
                TitleCli::Pi,
            ),
            ("openclaw/openclaw.mjs", TitleCli::Openclaw),
            ("freebuff/index.js", TitleCli::Freebuff),
        ] {
            let script = format!("/usr/lib/node_modules/{script}");
            assert_eq!(
                super::identify(Path::new("/usr/bin/node"), &[b"node", script.as_bytes()]),
                Some(cli)
            );
            assert_eq!(
                super::identify(
                    Path::new("/usr/bin/bun"),
                    &[b"bun", b"run", script.as_bytes()]
                ),
                Some(cli)
            );
            assert_eq!(
                super::identify(
                    Path::new("/usr/bin/node"),
                    &[b"node", b"--eval", script.as_bytes()]
                ),
                None
            );
        }
        assert_eq!(
            super::identify(
                Path::new("/usr/bin/python3"),
                &[b"python3", b"-m", b"aider", b"--message", b"qwen"]
            ),
            Some(TitleCli::Aider)
        );
        assert_eq!(
            super::identify(
                Path::new("/usr/bin/python3"),
                &[b"python3", b"-c", b"/opt/bin/aider"]
            ),
            None
        );
        assert_eq!(
            super::identify(
                Path::new("/usr/bin/python3"),
                &[b"python3", b"-m", b"other", b"aider"]
            ),
            None
        );
        assert_eq!(
            super::identify(Path::new("/usr/bin/ssh"), &[b"ssh", b"host", b"gemini"]),
            None
        );
        assert_eq!(
            super::identify(Path::new("/usr/bin/agent"), &[b"agent"]),
            None
        );
    }

    #[test]
    fn explicit_profile_overrides_do_not_silently_edit_default_configuration() {
        for flag in [
            "--config",
            "--config=/tmp/private.json",
            "--settings-file",
            "--mcp-config",
            "--profile",
        ] {
            assert!(check_configuration_arguments(
                TitleCli::Amp,
                Path::new("/opt/bin/amp"),
                &[b"amp", flag.as_bytes()]
            )
            .is_err());
        }
        assert!(check_configuration_arguments(
            TitleCli::Amp,
            Path::new("/opt/bin/amp"),
            &[b"amp", b"--", b"--settings-file"]
        )
        .is_ok());
        assert!(check_configuration_arguments(
            TitleCli::Gemini,
            Path::new("/opt/bin/gemini"),
            &[b"gemini", b"--model", b"test"]
        )
        .is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn configuration_argument_checks_start_after_runtime_entrypoints() {
        let node = Path::new("/usr/bin/node");
        let node_args = [
            b"node".as_slice(),
            b"--",
            b"/opt/bin/cline",
            b"--data-dir=/custom",
        ];
        assert_eq!(identify(node, &node_args), Some(TitleCli::Cline));
        assert!(check_mcp_configuration_arguments(TitleCli::Cline, node, &node_args).is_err());

        let python = Path::new("/usr/bin/python3");
        let python_script_args = [
            b"python3".as_slice(),
            b"--",
            b"/opt/bin/hermes",
            b"--config=/custom",
        ];
        assert_eq!(
            identify(python, &python_script_args),
            Some(TitleCli::Hermes)
        );
        assert!(
            check_mcp_configuration_arguments(TitleCli::Hermes, python, &python_script_args)
                .is_err()
        );

        let python_module_args = [
            b"python3".as_slice(),
            b"-m",
            b"hermes_cli",
            b"--strict-mcp-config",
        ];
        assert_eq!(
            identify(python, &python_module_args),
            Some(TitleCli::Hermes)
        );
        assert!(
            check_mcp_configuration_arguments(TitleCli::Hermes, python, &python_module_args)
                .is_err()
        );

        let node_payload = [
            b"node".as_slice(),
            b"--",
            b"/opt/bin/cline",
            b"--",
            b"--data-dir=/payload",
        ];
        assert!(check_mcp_configuration_arguments(TitleCli::Cline, node, &node_payload).is_ok());

        let python_payload = [
            b"python3".as_slice(),
            b"-m",
            b"hermes_cli",
            b"--",
            b"--config=/payload",
        ];
        assert!(
            check_mcp_configuration_arguments(TitleCli::Hermes, python, &python_payload).is_ok()
        );

        let claude_alias = [
            b"claude".as_slice(),
            b"--",
            b"/opt/node_modules/@anthropic-ai/claude-code/cli.js",
            b"--strict-mcp-config",
        ];
        assert_eq!(identify(node, &claude_alias), Some(TitleCli::Claude));
        assert!(check_mcp_configuration_arguments(TitleCli::Claude, node, &claude_alias).is_err());

        let claude_alias_payload = [
            b"claude".as_slice(),
            b"--",
            b"/opt/node_modules/@anthropic-ai/claude-code/cli.js",
            b"--",
            b"--strict-mcp-config",
        ];
        assert!(
            check_mcp_configuration_arguments(TitleCli::Claude, node, &claude_alias_payload)
                .is_ok()
        );
    }

    #[cfg(all(test, unix))]
    #[test]
    fn parses_bounded_macos_process_arguments_and_environment() {
        let bytes = process_args_fixture(
            &[
                b"node",
                b"/opt/home/node_modules/@openai/codex/bin/codex.js",
            ],
            &[
                b"HOME=/tmp/home",
                b"CODEX_HOME=/tmp/codex",
                b"TOKEN=private",
            ],
        );
        let parsed = parse_process_args(&bytes).unwrap();
        assert_eq!(parsed.argv.len(), 2);
        assert_eq!(
            parsed.argv[1],
            b"/opt/home/node_modules/@openai/codex/bin/codex.js"
        );
        assert_eq!(
            environment_value(&parsed.environment, b"HOME="),
            Some(&b"/tmp/home"[..])
        );
        assert_eq!(
            environment_value(&parsed.environment, b"CODEX_HOME="),
            Some(&b"/tmp/codex"[..])
        );
        assert_eq!(
            environment_value(&parsed.environment, b"TOKEN="),
            Some(&b"private"[..])
        );
    }

    #[cfg(all(test, unix))]
    #[test]
    fn rejects_malformed_and_oversized_macos_process_arguments() {
        for bytes in [
            Vec::new(),
            0i32.to_ne_bytes().to_vec(),
            (-1i32).to_ne_bytes().to_vec(),
            2i32.to_ne_bytes()
                .into_iter()
                .chain(b"/bin/node\0node\0".iter().copied())
                .collect(),
            1i32.to_ne_bytes()
                .into_iter()
                .chain(b"/bin/node".iter().copied())
                .collect(),
        ] {
            assert!(parse_process_args(&bytes).is_none());
        }
        let mut too_many = (4097i32).to_ne_bytes().to_vec();
        too_many.extend_from_slice(b"/bin/node\0node\0\0");
        assert!(parse_process_args(&too_many).is_none());
        let mut oversized = (1i32).to_ne_bytes().to_vec();
        oversized.extend_from_slice(b"/bin/node\0node\0");
        oversized.resize(MAX_PROCESS_ARGS + 1, 0);
        assert!(parse_process_args(&oversized).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn identifies_only_known_node_launchers_not_prompt_text() {
        use std::os::unix::ffi::OsStrExt;

        let runtime = Path::new("/usr/local/bin/node");
        assert_eq!(
            identify(
                runtime,
                &[
                    b"node",
                    b"/opt/home/node_modules/@openai/codex/bin/codex.js",
                    b"claude is a word in this prompt",
                ],
            ),
            Some(TitleCli::Codex)
        );
        assert_eq!(
            identify(
                runtime,
                &[
                    b"node",
                    b"/opt/home/node_modules/@anthropic-ai/claude-code/cli.js",
                    b"hello",
                ],
            ),
            Some(TitleCli::Claude)
        );
        assert_eq!(
            identify(
                runtime,
                &[
                    b"node",
                    b"-e",
                    b"require('/node_modules/@anthropic-ai/claude-code/cli.js')",
                ],
            ),
            None
        );
        assert_eq!(
            identify(
                Path::new(std::ffi::OsStr::from_bytes(b"/tmp/not-a-cli")),
                &[]
            ),
            None
        );
    }

    #[test]
    fn resolves_verified_mcp_paths_from_only_process_environment_overrides() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let home = root.to_string_lossy();
        let codex_home = root.join("codex-home");
        let codex_home_text = codex_home.to_string_lossy();
        let other_cursor = root.join("ignored-cursor-config");
        let other_cursor_text = other_cursor.to_string_lossy();
        let entries = [
            format!("HOME={home}").into_bytes(),
            format!("CODEX_HOME={codex_home_text}").into_bytes(),
            format!("CURSOR_CONFIG_DIR={other_cursor_text}").into_bytes(),
            b"UNRELATED_SECRET=must-not-appear-in-paths".to_vec(),
        ];
        let entries = entries.iter().map(Vec::as_slice).collect::<Vec<_>>();
        assert_eq!(
            mcp_configuration_from_environment(TitleCli::Codex, &entries).unwrap(),
            codex_home.join("config.toml")
        );
        assert_eq!(
            mcp_configuration_from_environment(TitleCli::Cursor, &entries).unwrap(),
            root.join(".cursor/mcp.json")
        );
        assert_eq!(
            mcp_configuration_from_environment(TitleCli::Agy, &entries).unwrap(),
            root.join(".gemini/config/mcp_config.json")
        );
        assert_eq!(
            mcp_configuration_from_environment(TitleCli::Claude, &entries).unwrap(),
            root.join(".claude.json")
        );
        let (title_path, disabled) =
            title_configuration_from_environment(TitleCli::Codex, &entries).unwrap();
        assert_eq!(title_path, codex_home.join("config.toml"));
        assert!(!disabled);
        assert!(!directory.path().join(".cursor").exists());
        let claude_override = [
            format!("HOME={home}").into_bytes(),
            b"CLAUDE_CONFIG_DIR=/tmp/other".to_vec(),
        ];
        let claude_override = claude_override
            .iter()
            .map(Vec::as_slice)
            .collect::<Vec<_>>();
        assert!(mcp_configuration_from_environment(TitleCli::Claude, &claude_override).is_err());
    }

    #[test]
    fn updates_json_title_settings_without_replacing_customizations() {
        for (cli, source) in [
            (
                TitleCli::Agy,
                r#"{"title":{"type":"command","command":"my-title --custom","enabled":false},"notifications":true}"#,
            ),
            (
                TitleCli::Cursor,
                r#"{"display":{"showStatusIndicators":false,"showLineNumbers":true},"permissions":{"allow":["Shell(ls)"],"deny":["Shell(rm)"]}}"#,
            ),
            (
                TitleCli::Claude,
                r#"{"env":{"CLAUDE_CODE_DISABLE_TERMINAL_TITLE":"1","CUSTOM":"keep"},"terminalTitleFromRename":false,"hooks":{"Stop":[]}}"#,
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("settings.json");
            fs::write(&path, source).unwrap();
            let setup = inspect(cli, &path, false).unwrap().unwrap();
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
            enable(cli, &path, setup.revision.as_deref()).unwrap();
            assert!(inspect(cli, &path, false).unwrap().is_none());
            let mut before: Value = serde_json::from_str(source).unwrap();
            let after: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            match cli {
                TitleCli::Agy => before["title"]["enabled"] = json!(true),
                TitleCli::Cursor => before["display"]["showStatusIndicators"] = json!(true),
                TitleCli::Claude => {
                    before["env"]["CLAUDE_CODE_DISABLE_TERMINAL_TITLE"] = json!("0");
                    before["terminalTitleFromRename"] = json!(true);
                }
                _ => unreachable!(),
            }
            assert_eq!(after, before);
            let backup = fs::read_dir(dir.path())
                .unwrap()
                .filter_map(Result::ok)
                .find(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .contains(".lomi-backup-")
                })
                .unwrap();
            assert_eq!(fs::read_to_string(backup.path()).unwrap(), source);
            fs::write(&path, "{\"external\":true}").unwrap();
            assert!(enable(cli, &path, setup.revision.as_deref()).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), "{\"external\":true}");
        }
        for cli in [TitleCli::Agy, TitleCli::Cursor, TitleCli::Claude] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("new/settings.json");
            assert_eq!(
                inspect(cli, &path, false).unwrap().is_none(),
                cli == TitleCli::Claude
            );
            assert!(!path.parent().unwrap().exists());
            enable(cli, &path, None).unwrap();
            assert!(inspect(cli, &path, true).unwrap().is_none());
            if cli == TitleCli::Agy {
                let doc = json_document(cli, read(&path).unwrap().as_deref()).unwrap();
                assert_eq!(doc["title"]["command"], agy_command().unwrap());
            }
        }
        assert!(!configured(TitleCli::Claude, None, true).unwrap());
        assert!(configured(
            TitleCli::Claude,
            Some(r#"{"env":{"CLAUDE_CODE_DISABLE_TERMINAL_TITLE":"0"}}"#),
            true
        )
        .unwrap());
        assert!(configured(
            TitleCli::Agy,
            Some(r#"{"title":{"type":"command","command":"custom"}}"#),
            false
        )
        .unwrap());
    }

    #[test]
    fn rejects_invalid_json_without_losing_the_original_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        for cli in [TitleCli::Agy, TitleCli::Cursor, TitleCli::Claude] {
            for source in [
                "secret invalid",
                "[]",
                "null",
                r#"{"title":[],"display":[],"env":[]}"#,
                r#"{"title":{"enabled":"no"},"display":{"showStatusIndicators":0},"env":{"CLAUDE_CODE_DISABLE_TERMINAL_TITLE":true}}"#,
            ] {
                fs::write(&path, source).unwrap();
                assert!(inspect(cli, &path, false).is_err());
                assert!(enable(cli, &path, revision(Some(source)).as_deref()).is_err());
                assert_eq!(fs::read_to_string(&path).unwrap(), source);
            }
        }
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn formats_agy_conversation_changes_and_bounds_untrusted_titles() {
        let dir = tempfile::tempdir().unwrap();
        let annotations = dir.path().join("annotations");
        let mut data =
            json!({"conversation_title":"Ulepsz system zakładek", "agent_state":"working"});
        assert_eq!(agy_title(&data, &annotations), "Ulepsz system zakładek");
        data["conversation_title"] = json!("Inna rozmowa");
        data["tool_confirmation_pending"] = json!(true);
        assert_eq!(agy_title(&data, &annotations), "Inna rozmowa");
        assert_eq!(
            agy_title(
                &json!({"cwd":"/tmp/project", "conversation_id":"unknown"}),
                &annotations
            ),
            "agy"
        );
        assert!(!annotations.exists());
        data["conversation_title"] = json!("\u{1b}\u{7}\n".to_owned() + &"ą".repeat(500));
        let title = agy_title(&data, &annotations);
        assert_eq!(title.chars().count(), 256);
        assert!(!title.chars().any(char::is_control));
    }

    #[test]
    fn resolves_live_agy_names_and_resume_renames_without_a_summaries_database() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first-conversation.pbtxt");
        let second = dir.path().join("second-conversation.pbtxt");
        fs::write(&first, r#"title:"Data Wydania Dipsick V4""#).unwrap();
        fs::write(&second, r#"title: "Druga rozmowa""#).unwrap();
        let mut data = json!({
            "cwd": "/tmp/lomi",
            "conversation_id": "first-conversation",
            "agent_state": "idle",
            "transcript_path": "/unreadable/transcript.jsonl",
        });
        assert_eq!(agy_title(&data, dir.path()), "Data Wydania Dipsick V4");
        assert_eq!(
            fs::read_to_string(&first).unwrap(),
            r#"title:"Data Wydania Dipsick V4""#
        );
        data["conversation_id"] = json!("second-conversation");
        assert_eq!(agy_title(&data, dir.path()), "Druga rozmowa");

        fs::write(
            &second,
            r#"title:"Zażółć \"gęślą\" \\x41 \u015b \U0001f980""#,
        )
        .unwrap();
        data["conversation_title"] = json!("Stale CLI title");
        assert_eq!(agy_title(&data, dir.path()), "Zażółć \"gęślą\" \\x41 ś 🦀");
        data["conversation_id"] = json!("");
        data["session_id"] = json!("second-conversation");
        assert_eq!(agy_title(&data, dir.path()), "Zażółć \"gęślą\" \\x41 ś 🦀");
        fs::write(&second, r#"title:"Safe\x1b\x07\nname""#).unwrap();
        assert_eq!(agy_title(&data, dir.path()), "Safe name");
        assert!(agy_annotation_title(r#"tags: "a title: " tags: "Not a title""#).is_none());
        fs::write(&second, "invalid annotation").unwrap();
        assert_eq!(agy_title(&data, dir.path()), "Stale CLI title");
        assert_eq!(fs::read_to_string(&second).unwrap(), "invalid annotation");
        data.as_object_mut().unwrap().remove("conversation_title");
        fs::write(&second, "x".repeat(LIMIT as usize + 1)).unwrap();
        assert_eq!(agy_title(&data, dir.path()), "agy");
    }

    #[test]
    fn agy_title_ids_cannot_escape_the_annotations_directory() {
        let dir = tempfile::tempdir().unwrap();
        let annotations = dir.path().join("annotations");
        fs::create_dir(&annotations).unwrap();
        let outside = dir.path().join("outside.pbtxt");
        fs::write(&outside, r#"title:"Must not read""#).unwrap();
        for id in [
            "../outside".to_owned(),
            outside.with_extension("").to_string_lossy().into_owned(),
        ] {
            assert_eq!(
                agy_title(&json!({"conversation_id":id}), &annotations),
                "agy"
            );
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, annotations.join("link.pbtxt")).unwrap();
            assert_eq!(
                agy_title(&json!({"conversation_id":"link"}), &annotations),
                "agy"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn identifies_native_clis_and_node_launchers_without_matching_prompt_arguments() {
        for (path, cli) in [
            ("/bin/agy", TitleCli::Agy),
            ("/bin/codex", TitleCli::Codex),
            ("/bin/claude", TitleCli::Claude),
            (
                "/home/user/.local/share/claude/versions/2.1.263",
                TitleCli::Claude,
            ),
            ("/cursor/cursor-agent-sea", TitleCli::Cursor),
        ] {
            assert_eq!(identify(Path::new(path), b""), Some(cli));
        }
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("cursor-agent"), "launcher").unwrap();
        let argv = format!(
            "/home/user/.local/bin/agent\0--use-system-ca\0{}/index.js\0",
            dir.path().display()
        );
        assert_eq!(
            identify(&dir.path().join("node"), argv.as_bytes()),
            Some(TitleCli::Cursor)
        );
        assert_eq!(
            identify(
                Path::new("/usr/bin/node"),
                b"node\0/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js\0"
            ),
            Some(TitleCli::Claude)
        );
        for path in [
            "/usr/bin/node",
            "/usr/bin/bash",
            "/bin/agent",
            "/usr/bin/cursor",
            "/bin/ssh",
        ] {
            assert_eq!(
                identify(
                    Path::new(path),
                    b"node\0other.js\0codex\0agy\0claude\0cursor-agent\0"
                ),
                None
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn resolves_each_cli_environment_without_creating_settings_before_consent() {
        use std::process::{Command, Stdio};
        let dir = tempfile::tempdir().unwrap();
        for (cli, variables, relative) in [
            (
                TitleCli::Agy,
                vec![],
                ".gemini/antigravity-cli/settings.json",
            ),
            (TitleCli::Cursor, vec![], ".cursor/cli-config.json"),
            (
                TitleCli::Cursor,
                vec![("XDG_CONFIG_HOME", "xdg")],
                "xdg/cursor/cli-config.json",
            ),
            (
                TitleCli::Cursor,
                vec![
                    ("XDG_CONFIG_HOME", "xdg"),
                    ("CURSOR_CONFIG_DIR", "cursor-custom"),
                ],
                "cursor-custom/cli-config.json",
            ),
            (TitleCli::Claude, vec![], ".claude/settings.json"),
            (
                TitleCli::Claude,
                vec![("CLAUDE_CONFIG_DIR", "claude-custom")],
                "claude-custom/settings.json",
            ),
        ] {
            let mut command = Command::new("sh");
            command
                .env_clear()
                .env("HOME", dir.path())
                .env("CLAUDE_CODE_DISABLE_TERMINAL_TITLE", "1");
            for (key, value) in variables {
                command.env(key, dir.path().join(value));
            }
            let mut child = command
                .args(["-c", "printf ready; read -r _"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdout
                .as_mut()
                .unwrap()
                .read_exact(&mut [0; 5])
                .unwrap();
            let result = configuration(TitleProcess {
                cli,
                pid: child.id(),
            });
            child.kill().unwrap();
            child.wait().unwrap();
            assert_eq!(result.unwrap(), (dir.path().join(relative), true));
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn updates_only_titles_and_keeps_a_backup_for_toml_layouts() {
        for (source, preserved) in [
            (
                "# Preferences\nmodel = 'custom-model'\n",
                "# Preferences\nmodel = 'custom-model'\n",
            ),
            (
                "[tui]\nterminal_title = ['project'] # custom title\nnotifications = false\n",
                "# custom title\nnotifications = false",
            ),
            (
                "tui = { terminal_title = [], notifications = false }\n",
                "notifications = false",
            ),
            (
                "tui.terminal_title = ['project']\ntui.notifications = false\n",
                "tui.notifications = false",
            ),
            (
                "[tui.model_availability_nux]\ncustom = 4\n",
                "[tui.model_availability_nux]\ncustom = 4",
            ),
            (
                "# Title settings\r\n[tui]\r\nterminal_title = []\r\n",
                "# Title settings",
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            fs::write(&path, source).unwrap();
            let setup = inspect(TitleCli::Codex, &path, false).unwrap().unwrap();
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
            enable(TitleCli::Codex, &path, setup.revision.as_deref()).unwrap();
            assert!(inspect(TitleCli::Codex, &path, false).unwrap().is_none());
            let updated = fs::read_to_string(&path).unwrap();
            assert!(updated.contains(preserved), "{updated}");
            if source.contains("\r\n") {
                assert!(!updated.replace("\r\n", "").contains('\n'));
            }
            let backups = fs::read_dir(dir.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("config.toml.lomi-backup-")
                })
                .collect::<Vec<_>>();
            assert_eq!(backups.len(), 1);
            assert_eq!(fs::read_to_string(backups[0].path()).unwrap(), source);
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert!(inspect(TitleCli::Codex, &path, false)
            .unwrap()
            .unwrap()
            .revision
            .is_none());
        assert!(!path.exists());
        enable(TitleCli::Codex, &path, None).unwrap();
        assert!(inspect(TitleCli::Codex, &path, false).unwrap().is_none());
        fs::write(
            &path,
            "[tui]\nterminal_title = ['thread-title', 'project']\n",
        )
        .unwrap();
        assert!(inspect(TitleCli::Codex, &path, false).unwrap().is_none());
    }

    #[test]
    fn leaves_invalid_and_concurrently_changed_files_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for source in [
            "broken = [secret".to_string(),
            "tui = 1".into(),
            "[tui]\nterminal_title = 'project'".into(),
            "[tui]\nterminal_title = [1]".into(),
            " ".repeat(LIMIT as usize + 1),
        ] {
            fs::write(&path, &source).unwrap();
            assert!(inspect(TitleCli::Codex, &path, false).is_err());
            assert!(enable(TitleCli::Codex, &path, revision(Some(&source)).as_deref()).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
        }
        fs::write(&path, "model = 'before'\n").unwrap();
        let setup = inspect(TitleCli::Codex, &path, false).unwrap().unwrap();
        let external = "model = 'external edit'\n";
        fs::write(&path, external).unwrap();
        assert!(enable(TitleCli::Codex, &path, setup.revision.as_deref())
            .unwrap_err()
            .contains("changed"));
        assert_eq!(fs::read_to_string(&path).unwrap(), external);
        fs::write(&path, "").unwrap();
        assert!(enable(TitleCli::Codex, &path, None).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
        let full = format!("#{}\n", " ".repeat(LIMIT as usize - 2));
        fs::write(&path, &full).unwrap();
        let setup = inspect(TitleCli::Codex, &path, false).unwrap().unwrap();
        assert!(enable(TitleCli::Codex, &path, setup.revision.as_deref()).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), full);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn resolves_process_home_and_preserves_symlinks_and_permissions() {
        use std::{
            os::unix::fs::{symlink, PermissionsExt},
            process::{Command, Stdio},
        };
        let dir = tempfile::tempdir().unwrap();
        let running_path = |command: &mut Command| {
            let mut child = command
                .args(["-c", "printf ready; read -r _"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdout
                .as_mut()
                .unwrap()
                .read_exact(&mut [0; 5])
                .unwrap();
            let path = configuration(TitleProcess {
                cli: TitleCli::Codex,
                pid: child.id(),
            })
            .map(|(path, _)| path);
            child.kill().unwrap();
            child.wait().unwrap();
            path.unwrap()
        };
        let running_mcp_path = |command: &mut Command| {
            let mut child = command
                .args(["-c", "printf ready; read -r _"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdout
                .as_mut()
                .unwrap()
                .read_exact(&mut [0; 5])
                .unwrap();
            let path = mcp_configuration(TitleProcess {
                cli: TitleCli::Codex,
                pid: child.id(),
            });
            child.kill().unwrap();
            child.wait().unwrap();
            path.unwrap()
        };
        let target = dir.path().join("dotfiles.toml");
        fs::write(&target, "model = 'test'\n").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        symlink(&target, dir.path().join("config.toml")).unwrap();
        let path = running_path(Command::new("sh").env("CODEX_HOME", dir.path()));
        assert_eq!(path, target);
        let setup = inspect(TitleCli::Codex, &path, false).unwrap().unwrap();
        enable(TitleCli::Codex, &path, setup.revision.as_deref()).unwrap();
        assert!(dir.path().join("config.toml").is_symlink());
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        fs::set_permissions(&target, fs::Permissions::from_mode(0o440)).unwrap();
        let before = fs::read_to_string(&target).unwrap();
        assert!(enable(TitleCli::Codex, &target, revision(Some(&before)).as_deref()).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), before);

        let default = dir.path().join(".codex");
        fs::create_dir(&default).unwrap();
        let path = running_path(
            Command::new("sh")
                .env_remove("CODEX_HOME")
                .env("HOME", dir.path()),
        );
        assert_eq!(path, default.join("config.toml"));
        let mcp_home = dir.path().join("mcp-home");
        let process_codex_home = dir.path().join("process-codex-home");
        assert_eq!(
            running_mcp_path(
                Command::new("sh")
                    .env("HOME", &mcp_home)
                    .env("CODEX_HOME", &process_codex_home),
            ),
            process_codex_home.join("config.toml")
        );
    }
    #[test]
    fn gemini_and_qwen_title_setup_keeps_other_ui_preferences() {
        for cli in [TitleCli::Gemini, TitleCli::Qwen] {
            assert!(configured(cli, None, false).unwrap());
            let source = r#"{"ui":{"hideWindowTitle":true,"theme":"custom","showStatusInTitle":false,"dynamicWindowTitle":false},"mcpServers":{"other":{"command":"keep"}}}"#;
            assert!(!configured(cli, Some(source), false).unwrap());
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("settings.json");
            fs::write(&path, source).unwrap();
            enable(cli, &path, revision(Some(source)).as_deref()).unwrap();
            let output = fs::read_to_string(path).unwrap();
            assert!(configured(cli, Some(&output), false).unwrap());
            let doc: Value = serde_json::from_str(&output).unwrap();
            assert_eq!(doc["ui"]["theme"], "custom");
            assert_eq!(doc["mcpServers"]["other"]["command"], "keep");
        }
    }

    #[cfg(unix)]
    #[test]
    fn detects_a_live_python_console_script_in_its_process_group() {
        use std::{
            io::{BufRead, BufReader},
            os::unix::process::CommandExt,
            process::{Command, Stdio},
        };
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("bin")).unwrap();
        let script = dir.path().join("bin/aider");
        fs::write(
            &script,
            "import time\nprint('ready', flush=True)\ntime.sleep(30)\n",
        )
        .unwrap();
        let mut command = Command::new("python3");
        command
            .arg(&script)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            });
        }
        let mut child = command
            .spawn()
            .expect("Python 3 is required for the native console-script fixture");
        let mut ready = String::new();
        let result = BufReader::new(child.stdout.take().unwrap()).read_line(&mut ready);
        let detected = process_in_group(child.id());
        let _ = child.kill();
        let _ = child.wait();
        result.unwrap();
        assert_eq!(ready.trim(), "ready");
        assert_eq!(detected.map(|process| process.cli), Some(TitleCli::Aider));
    }
    #[test]
    fn commented_settings_support_mcp_and_title_setup_in_custom_qwen_home() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let home = format!("HOME={}", root.display());
        let custom = format!("QWEN_HOME={}/profile", root.display());
        let resolved = title_configuration_from_environment(
            TitleCli::Qwen,
            &[home.as_bytes(), custom.as_bytes()],
        )
        .unwrap();
        assert_eq!(resolved.0, root.join("profile/settings.json"));
        for cli in [TitleCli::Gemini, TitleCli::Qwen] {
            let source = "{ // keep title comment\n\"ui\":{\"hideWindowTitle\":true,\"theme\":\"custom\"},\n/* keep MCP comment */ \"mcpServers\":{}}\n";
            let path = root.join(format!("{cli:?}.json"));
            fs::write(&path, source).unwrap();
            enable(cli, &path, revision(Some(source)).as_deref()).unwrap();
            let titled = fs::read_to_string(&path).unwrap();
            assert!(titled.contains("// keep title comment"));
            assert!(titled.contains("/* keep MCP comment */"));
            assert!(configured(cli, Some(&titled), false).unwrap());
            let registration = crate::cli_mcp::Registration {
                command: "/app/lomi".into(),
                args: vec!["--mcp".into()],
            };
            crate::cli_mcp::enable(
                cli,
                &path,
                revision(Some(&titled)).as_deref(),
                &registration,
            )
            .unwrap();
            let output = fs::read_to_string(&path).unwrap();
            assert!(output.contains("// keep title comment"));
            assert!(output.contains("/* keep MCP comment */"));
            assert!(crate::cli_mcp::configured(cli, Some(&output), Some(&registration)).unwrap());
        }
        assert!(check_mcp_configuration_arguments(
            TitleCli::Claude,
            Path::new("/opt/bin/claude"),
            &[b"claude", b"--strict-mcp-config"]
        )
        .is_err());
        assert!(check_mcp_configuration_arguments(
            TitleCli::Cline,
            Path::new("/opt/bin/cline"),
            &[b"cline", b"--data-dir=/custom"]
        )
        .is_err());
        assert!(check_mcp_configuration_arguments(
            TitleCli::Claude,
            Path::new("/opt/bin/claude"),
            &[b"claude", b"--", b"--strict-mcp-config"]
        )
        .is_ok());
    }
}
