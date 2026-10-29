pub use crate::cli_catalog::TitleCli;
use crate::cli_config::{read, revision};
use crate::{files::main_window, terminal::Terminals};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[cfg(target_os = "linux")]
use std::fs;
use std::{
    ffi::OsString,
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
        "kilo" | "kilocode" => TitleCli::Kilo,
        "qwen" => TitleCli::Qwen,
        "kiro-cli" => TitleCli::Kiro,
        "vibe" => TitleCli::Vibe,
        "kimi" => TitleCli::Kimi,
        "grok" => TitleCli::Grok,
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
        ("@mariozechner/pi-coding-agent/dist/cli.js", TitleCli::Pi),
        (
            "@earendil-works/pi-coding-agent/dist/bundle/cli.js",
            TitleCli::Pi,
        ),
        ("@qwen-code/qwen-code/dist/index.js", TitleCli::Qwen),
        ("@qwen-code/qwen-code/cli-entry.js", TitleCli::Qwen),
        ("qwen-code/lib/cli.js", TitleCli::Qwen),
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
                    b"hermes_cli" | b"hermes_cli.main" => TitleCli::Hermes,
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

#[cfg(target_os = "macos")]
const MAX_PROCESS_ARGS: usize = 1024 * 1024;

#[cfg(target_os = "macos")]
struct ParsedProcessArgs<'a> {
    argv: Vec<&'a [u8]>,
    environment: Vec<&'a [u8]>,
}

#[cfg(target_os = "macos")]
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
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

#[cfg(windows)]
fn environment_path(value: Option<&[u8]>) -> Option<PathBuf> {
    value.map(|value| PathBuf::from(String::from_utf8_lossy(value).into_owned()))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn usage_subprocess_environment(entries: &[&[u8]]) -> Vec<(OsString, OsString)> {
    const ALLOWED: &[&[u8]] = &[
        b"HOME",
        b"USER",
        b"PATH",
        b"TMPDIR",
        b"TMP",
        b"TEMP",
        b"LANG",
        b"LC_ALL",
        b"LC_CTYPE",
        b"LC_MESSAGES",
        b"LC_COLLATE",
        b"LC_MONETARY",
        b"LC_NUMERIC",
        b"LC_TIME",
        b"XDG_CONFIG_HOME",
        b"XDG_CACHE_HOME",
        b"XDG_DATA_HOME",
        b"XDG_STATE_HOME",
        b"XDG_RUNTIME_DIR",
    ];
    entries
        .iter()
        .filter_map(|entry| {
            let separator = entry.iter().position(|byte| *byte == b'=')?;
            let (name, value) = entry.split_at(separator);
            let value = value.get(1..)?;
            ALLOWED
                .contains(&name)
                .then(|| (os_string_from_bytes(name), os_string_from_bytes(value)))
        })
        .collect()
}

#[cfg(unix)]
fn os_string_from_bytes(bytes: &[u8]) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(bytes.to_vec())
}

#[cfg(not(unix))]
fn os_string_from_bytes(bytes: &[u8]) -> OsString {
    OsString::from(String::from_utf8_lossy(bytes).into_owned())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct UsageProcessPaths {
    pub has_auth_argument_override: bool,
    pub executable: Option<PathBuf>,
    pub subprocess_environment: Vec<(OsString, OsString)>,
    pub home: Option<PathBuf>,
    pub codex_home: Option<PathBuf>,
    pub claude_config_dir: Option<PathBuf>,
    pub claude_secure_storage_config_dir: Option<PathBuf>,
    pub claude_secure_storage_config_dir_set: bool,
    pub cursor_config_dir: Option<PathBuf>,
    pub kimi_code_home: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
    pub user: Option<String>,
    pub has_openai_api_key: bool,
    pub has_openai_custom_base_url: bool,
    pub has_claude_auth_override: bool,
    pub has_cursor_api_key: bool,
    pub has_cursor_custom_backend: bool,
    pub has_cursor_custom_config_dir: bool,
    pub cursor_credential_store: Option<String>,
    pub kimi_code_base_url: Option<String>,
    pub kimi_code_oauth_host: Option<String>,
    pub kimi_oauth_host: Option<String>,
    pub has_kimi_model_name_override: bool,
    pub is_legacy_kimi_cli: bool,
}

impl UsageProcessPaths {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn from_entries(entries: &[&[u8]]) -> Self {
        let path = |name| environment_path(environment_value(entries, name));
        let string = |name| {
            environment_value(entries, name)
                .and_then(|value| std::str::from_utf8(value).ok())
                .map(str::to_owned)
                .filter(|value| !value.is_empty())
        };
        let enabled = |name| {
            string(name).is_some_and(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
        };
        Self {
            subprocess_environment: usage_subprocess_environment(entries),
            home: path(b"HOME="),
            codex_home: path(b"CODEX_HOME="),
            claude_config_dir: path(b"CLAUDE_CONFIG_DIR="),
            claude_secure_storage_config_dir: path(b"CLAUDE_SECURESTORAGE_CONFIG_DIR="),
            claude_secure_storage_config_dir_set: entries
                .iter()
                .any(|entry| entry.starts_with(b"CLAUDE_SECURESTORAGE_CONFIG_DIR=")),
            cursor_config_dir: path(b"CURSOR_CONFIG_DIR="),
            kimi_code_home: path(b"KIMI_CODE_HOME="),
            xdg_config_home: path(b"XDG_CONFIG_HOME="),
            user: string(b"USER="),
            has_openai_api_key: environment_value(entries, b"OPENAI_API_KEY=").is_some(),
            has_openai_custom_base_url: [
                b"OPENAI_BASE_URL=".as_slice(),
                b"OPENAI_API_BASE=".as_slice(),
            ]
            .iter()
            .any(|name| environment_value(entries, name).is_some()),
            has_claude_auth_override: [
                b"ANTHROPIC_API_KEY=".as_slice(),
                b"ANTHROPIC_AUTH_TOKEN=".as_slice(),
                b"ANTHROPIC_BASE_URL=".as_slice(),
                b"CLAUDE_CODE_OAUTH_TOKEN=".as_slice(),
                b"CLAUDE_CODE_OAUTH_REFRESH_TOKEN=".as_slice(),
                b"CLAUDE_CODE_API_KEY_FILE_DESCRIPTOR=".as_slice(),
                b"CLAUDE_CODE_CUSTOM_OAUTH_URL=".as_slice(),
                b"CLAUDE_CODE_OAUTH_CLIENT_ID=".as_slice(),
                b"CLAUDE_CODE_API_BASE_URL=".as_slice(),
                b"CLAUDE_LOCAL_OAUTH_API_BASE=".as_slice(),
            ]
            .iter()
            .any(|name| environment_value(entries, name).is_some())
                || [
                    b"CLAUDE_CODE_USE_BEDROCK=".as_slice(),
                    b"CLAUDE_CODE_USE_VERTEX=".as_slice(),
                    b"CLAUDE_CODE_USE_FOUNDRY=".as_slice(),
                ]
                .iter()
                .any(|name| enabled(name)),
            has_cursor_custom_config_dir: environment_value(entries, b"CURSOR_CONFIG_DIR=")
                .is_some(),
            cursor_credential_store: string(b"AGENT_CLI_CREDENTIAL_STORE="),
            has_cursor_api_key: [
                b"CURSOR_API_KEY=".as_slice(),
                b"CURSOR_AUTH_TOKEN=".as_slice(),
            ]
            .iter()
            .any(|name| environment_value(entries, name).is_some()),
            has_cursor_custom_backend: [
                b"CURSOR_BASE_URL=".as_slice(),
                b"CURSOR_API_BASE_URL=".as_slice(),
                b"CURSOR_API_ENDPOINT=".as_slice(),
            ]
            .iter()
            .any(|name| environment_value(entries, name).is_some()),
            kimi_code_base_url: string(b"KIMI_CODE_BASE_URL="),
            kimi_code_oauth_host: string(b"KIMI_CODE_OAUTH_HOST="),
            kimi_oauth_host: string(b"KIMI_OAUTH_HOST="),
            // Current Kimi Code only synthesizes a temporary provider when
            // KIMI_MODEL_NAME is set. KIMI_API_KEY/KIMI_BASE_URL are names in
            // provider config, not process-level credential overrides.
            has_kimi_model_name_override: string(b"KIMI_MODEL_NAME=").is_some(),
            is_legacy_kimi_cli: false,
            ..Self::default()
        }
    }

    pub(crate) fn directory_for(&self, cli: TitleCli) -> Result<PathBuf, String> {
        let path = match cli {
            TitleCli::Codex => self
                .codex_home
                .clone()
                .or_else(|| self.home.as_ref().map(|path| path.join(".codex")))
                .ok_or("Cannot locate the running Codex configuration directory.")?,
            TitleCli::Claude => self
                .claude_config_dir
                .clone()
                .or_else(|| self.home.as_ref().map(|path| path.join(".claude")))
                .ok_or("Cannot locate the running Claude Code configuration directory.")?,
            TitleCli::Cursor => self
                .home
                .as_ref()
                .map(|home| {
                    #[cfg(target_os = "macos")]
                    {
                        home.join(".cursor")
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        self.xdg_config_home
                            .as_ref()
                            .map(|path| path.join("cursor"))
                            .unwrap_or_else(|| home.join(".config/cursor"))
                    }
                })
                .ok_or("Cannot locate the running Cursor configuration directory.")?,
            TitleCli::Kimi => self
                .kimi_code_home
                .clone()
                .or_else(|| self.home.as_ref().map(|path| path.join(".kimi-code")))
                .ok_or("Cannot locate the running Kimi Code configuration directory.")?,
            TitleCli::Agy => self
                .home
                .as_ref()
                .map(|path| path.join(".gemini/antigravity-cli"))
                .ok_or("Cannot locate the running Antigravity CLI configuration directory.")?,
            _ => return Err("Usage credentials are not available for this CLI.".into()),
        };
        if !path.is_absolute() {
            return Err("The running CLI configuration directory is not absolute.".into());
        }
        Ok(path)
    }

    pub(crate) fn cursor_cli_config_directory(&self) -> Result<PathBuf, String> {
        let path = self
            .cursor_config_dir
            .clone()
            .or_else(|| {
                self.xdg_config_home
                    .as_ref()
                    .map(|path| path.join("cursor"))
            })
            .or_else(|| self.home.as_ref().map(|path| path.join(".cursor")))
            .ok_or("Cannot locate the running Cursor configuration directory.")?;
        if !path.is_absolute() {
            return Err("The running Cursor configuration directory is not absolute.".into());
        }
        Ok(path)
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn usage_process_paths(process: TitleProcess) -> Result<UsageProcessPaths, String> {
    let bytes = process_environment(process.pid)?;
    let mut paths =
        UsageProcessPaths::from_entries(&bytes.split(|byte| *byte == 0).collect::<Vec<_>>());
    let mut args = Vec::new();
    fs::File::open(format!("/proc/{}/cmdline", process.pid))
        .and_then(|file| file.take(LIMIT + 1).read_to_end(&mut args))
        .map_err(|_| "Cannot read the running CLI arguments.")?;
    if args.len() as u64 > LIMIT {
        return Err("CLI arguments exceed 1 MiB.".into());
    }
    let executable = fs::read_link(format!("/proc/{}/exe", process.pid))
        .map_err(|_| "Cannot read the running CLI executable.")?;
    paths.executable = Some(executable.clone());
    let argv = args.split(|byte| *byte == 0).collect::<Vec<_>>();
    paths.has_auth_argument_override =
        usage_auth_argument_override(process.cli, cli_arguments(process.cli, &executable, &argv)?);
    paths.is_legacy_kimi_cli = is_legacy_kimi_python(&executable, &argv);
    Ok(paths)
}

#[cfg(target_os = "macos")]
pub(crate) fn usage_process_paths(process: TitleProcess) -> Result<UsageProcessPaths, String> {
    let bytes = macos_process::process_args(process.pid)
        .ok_or("Cannot read the running CLI environment.")?;
    let parsed = parse_process_args(&bytes).ok_or("Cannot parse the running CLI environment.")?;
    let executable = macos_process::process_path(process.pid)
        .ok_or("Cannot read the running CLI executable.")?;
    let mut paths = UsageProcessPaths::from_entries(&parsed.environment);
    paths.executable = Some(executable.clone());
    paths.has_auth_argument_override = usage_auth_argument_override(
        process.cli,
        cli_arguments(process.cli, &executable, &parsed.argv)?,
    );
    paths.is_legacy_kimi_cli = is_legacy_kimi_python(&executable, &parsed.argv);
    Ok(paths)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn is_legacy_kimi_python(executable: &Path, argv: &[&[u8]]) -> bool {
    executable
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(python_runtime)
        && identify_python(argv) == Some(TitleCli::Kimi)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn usage_auth_argument_override(cli: TitleCli, arguments: &[&[u8]]) -> bool {
    let arguments = arguments
        .iter()
        .copied()
        .take_while(|argument| *argument != b"--")
        .collect::<Vec<_>>();
    arguments.iter().enumerate().any(|(index, argument)| {
        let flag = argument
            .split(|byte| *byte == b'=')
            .next()
            .unwrap_or_default();
        match cli {
            TitleCli::Agy => matches!(
                flag,
                b"--api-key" | b"--auth" | b"--auth-token" | b"--provider" | b"--model-provider"
            ),
            TitleCli::Codex => {
                if matches!(flag, b"--profile" | b"-p" | b"--oss" | b"--local-provider")
                    || (argument.starts_with(b"-p") && argument.len() > 2)
                {
                    return true;
                }
                let config = if matches!(flag, b"--config" | b"-c") {
                    argument
                        .strip_prefix(b"--config=")
                        .or_else(|| argument.strip_prefix(b"-c="))
                        .or_else(|| arguments.get(index + 1).copied())
                } else {
                    argument.strip_prefix(b"-c")
                };
                config.is_some_and(|config| {
                    let key = config
                        .split(|byte| *byte == b'=')
                        .next()
                        .unwrap_or_default()
                        .trim_ascii();
                    matches!(
                        key,
                        b"cli_auth_credentials_store"
                            | b"model_provider"
                            | b"chatgpt_base_url"
                            | b"forced_login_method"
                            | b"forced_chatgpt_workspace_id"
                            | b"experimental_bearer_token"
                    ) || key.starts_with(b"model_providers.")
                })
            }
            TitleCli::Claude => matches!(
                flag,
                b"--settings" | b"--setting-sources" | b"--api-key" | b"--auth-token"
            ),
            TitleCli::Cursor => matches!(
                flag,
                b"--api-key" | b"--auth-token" | b"--endpoint" | b"-e" | b"--team-id"
            ),
            TitleCli::Kimi => matches!(flag, b"--model" | b"-m"),
            _ => false,
        }
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn usage_process_paths(_process: TitleProcess) -> Result<UsageProcessPaths, String> {
    Err("Native account usage is currently available on Linux and macOS.".into())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
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
