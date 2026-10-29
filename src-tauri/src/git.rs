use crate::files::{directory, main_window};
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use std::thread;
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Webview, Window};

mod diff;
pub mod history;
pub(crate) mod local;
mod observation;

const MUTATION_STDOUT_LIMIT: usize = 2 * 1024 * 1024;
const MUTATION_STDERR_LIMIT: usize = 64 * 1024;
const MUTATION_DEADLINE: Duration = Duration::from_secs(120);

// Explicit desktop mutations use the user's Git and credential setup. Read-only
// observations keep using the separately sandboxed `configured_command` path.
fn mutation_command(root: &Path, args: &[&str]) -> Command {
    mutation_command_with_environment(root, args, std::env::vars_os())
}

fn mutation_command_with_environment(
    root: &Path,
    args: &[&str],
    environment: impl Iterator<Item = (OsString, OsString)>,
) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .env_clear()
        .envs(environment.filter(|(name, _)| safe_mutation_environment_name(name)))
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}

fn safe_mutation_environment_name(name: &std::ffi::OsStr) -> bool {
    #[cfg(windows)]
    let key = name.to_string_lossy().to_ascii_uppercase();
    #[cfg(not(windows))]
    let key = name.to_string_lossy();
    (!key.starts_with("GIT_")
        || matches!(
            key.as_ref(),
            "GIT_AUTHOR_NAME"
                | "GIT_AUTHOR_EMAIL"
                | "GIT_AUTHOR_DATE"
                | "GIT_COMMITTER_NAME"
                | "GIT_COMMITTER_EMAIL"
                | "GIT_COMMITTER_DATE"
                | "GIT_CONFIG_GLOBAL"
                | "GIT_CONFIG_SYSTEM"
                | "GIT_CONFIG_NOSYSTEM"
                | "GIT_SSH"
                | "GIT_SSH_COMMAND"
                | "GIT_SSH_VARIANT"
                | "GIT_ASKPASS"
        ))
        && !key.starts_with("DYLD_")
        && key != "LD_PRELOAD"
}

#[cfg(windows)]
fn read_limited<R: Read>(reader: R, limit: usize) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn run_mutation_command(command: &mut Command, deadline: Duration) -> Result<Output, String> {
    let _ = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut tree = lomi_control_core::process_tree::Tree::spawn(command)
        .map_err(|error| format!("Cannot run Git: {error}"))?;
    let (Some(stdout), Some(stderr)) = (tree.child.stdout.take(), tree.child.stderr.take()) else {
        terminate_mutation_tree(&mut tree);
        return Err(unknown_mutation_outcome(
            "Git did not provide bounded output streams",
        ));
    };
    #[cfg(unix)]
    return run_mutation_command_unix(tree, stdout, stderr, deadline);
    #[cfg(windows)]
    return run_mutation_command_windows(tree, stdout, stderr, deadline);
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (stdout, stderr, deadline);
        terminate_mutation_tree(&mut tree);
        Err(unknown_mutation_outcome(
            "bounded process ownership is unavailable",
        ))
    }
}

fn unknown_mutation_outcome(reason: &str) -> String {
    format!(
        "Git operation outcome is unknown because {reason}. Refresh Git status before trying again."
    )
}

fn terminate_mutation_tree(tree: &mut lomi_control_core::process_tree::Tree) {
    let _ = tree.kill();
    loop {
        match tree.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) | Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

#[cfg(unix)]
fn set_nonblocking<R: std::os::fd::AsRawFd>(stream: &R) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let fd = stream.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn drain_nonblocking<R: Read>(
    stream: &mut R,
    output: &mut Vec<u8>,
    limit: usize,
    retain: bool,
) -> Result<bool, &'static str> {
    use std::io::ErrorKind;
    let mut bytes = [0; 8192];
    for _ in 0..8 {
        match stream.read(&mut bytes) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                if retain {
                    if output.len() + count > limit {
                        return Err("Git output exceeded its safety limit");
                    }
                    output.extend_from_slice(&bytes[..count]);
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return Err("Git output could not be read completely"),
        }
    }
    Ok(false)
}

#[cfg(unix)]
fn run_mutation_command_unix(
    mut tree: lomi_control_core::process_tree::Tree,
    mut stdout: std::process::ChildStdout,
    mut stderr: std::process::ChildStderr,
    deadline: Duration,
) -> Result<Output, String> {
    if set_nonblocking(&stdout).is_err() || set_nonblocking(&stderr).is_err() {
        terminate_mutation_tree(&mut tree);
        return Err(unknown_mutation_outcome(
            "Git output streams could not be made nonblocking",
        ));
    }
    let started = Instant::now();
    let mut exited_at = None;
    let mut exit = None;
    let mut stdout_done = false;
    let mut stderr_done = false;
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let mut interrupted = None;

    loop {
        if started.elapsed() >= deadline {
            interrupted.get_or_insert("the deadline expired");
        }
        if tree.abandoned_descendants {
            interrupted.get_or_insert("Git left an unowned descendant");
        }
        let retain = interrupted.is_none();
        if !stdout_done {
            match drain_nonblocking(
                &mut stdout,
                &mut stdout_bytes,
                MUTATION_STDOUT_LIMIT,
                retain,
            ) {
                Ok(done) => stdout_done = done,
                Err(reason) => {
                    interrupted.get_or_insert(reason);
                }
            }
        }
        if !stderr_done {
            match drain_nonblocking(
                &mut stderr,
                &mut stderr_bytes,
                MUTATION_STDERR_LIMIT,
                retain,
            ) {
                Ok(done) => stderr_done = done,
                Err(reason) => {
                    interrupted.get_or_insert(reason);
                }
            }
        }
        if interrupted.is_some() {
            let _ = tree.kill();
        }
        if exit.is_none() {
            match tree.try_wait() {
                Ok(Some(status)) => {
                    exited_at = Some(Instant::now());
                    exit = Some(status);
                }
                Ok(None) => {}
                Err(_) => {
                    interrupted.get_or_insert("Git process ownership became uncertain");
                    let _ = tree.kill();
                }
            }
        }
        if exit.is_some() && stdout_done && stderr_done {
            break;
        }
        if exited_at.is_some_and(|at| at.elapsed() >= Duration::from_millis(100)) {
            interrupted.get_or_insert("Git left output open after its process tree exited");
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    if let Some(reason) = interrupted {
        return Err(unknown_mutation_outcome(reason));
    }
    let status =
        exit.ok_or_else(|| unknown_mutation_outcome("Git process status was unavailable"))?;
    Ok(Output {
        status,
        stdout: stdout_bytes,
        stderr: stderr_bytes,
    })
}

#[cfg(windows)]
fn run_mutation_command_windows(
    mut tree: lomi_control_core::process_tree::Tree,
    stdout: std::process::ChildStdout,
    stderr: std::process::ChildStderr,
    deadline: Duration,
) -> Result<Output, String> {
    let mut stdout_reader = Some(thread::spawn(move || {
        read_limited(stdout, MUTATION_STDOUT_LIMIT)
    }));
    let mut stderr_reader = Some(thread::spawn(move || {
        read_limited(stderr, MUTATION_STDERR_LIMIT)
    }));
    let started = Instant::now();
    let mut exited_at = None;
    let mut exit = None;
    let mut stdout_bytes = None;
    let mut stderr_bytes = None;
    let mut interrupted = None;

    loop {
        if started.elapsed() >= deadline {
            interrupted.get_or_insert("the deadline expired");
        }
        if stdout_bytes.is_none()
            && stdout_reader
                .as_ref()
                .is_some_and(|reader| reader.is_finished())
        {
            match stdout_reader.take().unwrap().join() {
                Ok(Ok(bytes)) if bytes.len() <= MUTATION_STDOUT_LIMIT => stdout_bytes = Some(bytes),
                _ => {
                    interrupted.get_or_insert("stdout could not be bounded");
                }
            }
        }
        if stderr_bytes.is_none()
            && stderr_reader
                .as_ref()
                .is_some_and(|reader| reader.is_finished())
        {
            match stderr_reader.take().unwrap().join() {
                Ok(Ok(bytes)) if bytes.len() <= MUTATION_STDERR_LIMIT => stderr_bytes = Some(bytes),
                _ => {
                    interrupted.get_or_insert("stderr could not be bounded");
                }
            }
        }
        if tree.abandoned_descendants {
            interrupted.get_or_insert("Git left an unowned descendant");
        }
        if interrupted.is_some() {
            let _ = tree.kill();
        }
        if exit.is_none() {
            match tree.try_wait() {
                Ok(Some(status)) => {
                    exited_at = Some(Instant::now());
                    exit = Some(status);
                }
                Ok(None) => {}
                Err(_) => {
                    interrupted.get_or_insert("Git process ownership became uncertain");
                    let _ = tree.kill();
                }
            }
        }
        if exit.is_some() && stdout_bytes.is_some() && stderr_bytes.is_some() {
            break;
        }
        if exited_at.is_some_and(|at| at.elapsed() >= Duration::from_millis(100)) {
            interrupted.get_or_insert("Git left output open after its process tree exited");
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    if let Some(reason) = interrupted {
        return Err(unknown_mutation_outcome(reason));
    }
    let status =
        exit.ok_or_else(|| unknown_mutation_outcome("Git process status was unavailable"))?;
    let stdout =
        stdout_bytes.ok_or_else(|| unknown_mutation_outcome("Git stdout was unavailable"))?;
    let stderr =
        stderr_bytes.ok_or_else(|| unknown_mutation_outcome("Git stderr was unavailable"))?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

// Configuration overrides are defense in depth. The platform process policy
// enforces helper isolation without enumerating mutable repository config.
fn configured_command(root: &Path, args: &[&str]) -> Result<Command, String> {
    let mut command = observation::configured()?;
    command
        .args([
            "--no-pager",
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
            "-c",
            "core.preloadIndex=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "gc.auto=0",
            "-c",
            "index.threads=1",
            "-c",
            "pack.threads=1",
            "-c",
            "diff.submodule=short",
        ])
        .arg("-C")
        .arg(root)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", root)
        .env("LC_ALL", "C")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_SYSTEM", observation::null_device())
        .env("GIT_CONFIG_GLOBAL", observation::null_device())
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Git for Windows needs its native loader and executable search environment.
    #[cfg(windows)]
    for key in ["SystemRoot", "WINDIR", "PATH"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    Ok(command)
}

fn spawn_observation(root: &Path, args: &[&str]) -> Result<(Child, observation::Guard), String> {
    observation::spawn(&mut configured_command(root, args)?)
        .map_err(|error| format!("Cannot safely observe Git: {error}"))
}

fn spawn_observation_with_stdin(
    root: &Path,
    args: &[&str],
) -> Result<(Child, observation::Guard), String> {
    let mut command = configured_command(root, args)?;
    command.stdin(Stdio::piped());
    observation::spawn(&mut command).map_err(|error| format!("Cannot safely observe Git: {error}"))
}

fn command(root: &Path, args: &[&str]) -> Result<Output, String> {
    let (child, _guard) = spawn_observation(root, args)?;
    child
        .wait_with_output()
        .map_err(|error| format!("Cannot read Git: {error}"))
}

pub(crate) fn checked(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = command(root, args)?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(output.stdout)
}

#[derive(Deserialize, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    path: String,
    original_path: Option<String>,
    index: char,
    worktree: char,
}

impl From<Change> for lomi_files::desktop::DesktopGitChange {
    fn from(value: Change) -> Self {
        Self {
            path: value.path,
            original_path: value.original_path,
            index: value.index,
            worktree: value.worktree,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub(crate) root: String,
    branch: String,
    changes: Vec<Change>,
}

fn parse_status(bytes: &[u8]) -> Vec<Change> {
    let mut entries = bytes.split(|byte| *byte == 0);
    let mut changes = Vec::new();
    while let Some(entry) = entries.next() {
        if entry.len() < 4 {
            continue;
        }
        let index = entry[0] as char;
        let worktree = entry[1] as char;
        let original_path = if matches!(index, 'R' | 'C') || matches!(worktree, 'R' | 'C') {
            entries
                .next()
                .map(|path| String::from_utf8_lossy(path).into_owned())
        } else {
            None
        };
        changes.push(Change {
            path: String::from_utf8_lossy(&entry[3..]).into_owned(),
            original_path,
            index,
            worktree,
        });
    }
    changes
}

#[derive(Default)]
struct StatusBudget {
    visited: HashSet<PathBuf>,
    entries: usize,
}

pub fn status(path: &str) -> Result<Option<GitStatus>, String> {
    observed_status(path, &mut StatusBudget::default(), 0, true)
}

fn observed_status(
    path: &str,
    budget: &mut StatusBudget,
    depth: usize,
    include_untracked: bool,
) -> Result<Option<GitStatus>, String> {
    if depth > 8 || budget.visited.len() >= 64 {
        return Err(
            "Safe Git submodule observation exceeded its depth or repository limit.".into(),
        );
    }
    let directory = directory(path)?;
    let probe = command(&directory, &["rev-parse", "--show-toplevel"])?;
    if !probe.status.success() {
        let error = String::from_utf8_lossy(&probe.stderr);
        if error.contains("not a git repository") {
            return Ok(None);
        }
        return Err(error.trim().to_owned());
    }
    let root = String::from_utf8_lossy(&probe.stdout).trim().to_string();
    let root_path = Path::new(&root);
    let canonical = fs::canonicalize(root_path).map_err(|error| error.to_string())?;
    if !budget.visited.insert(canonical.clone()) {
        return Err("A Git submodule points to an already observed repository.".into());
    }
    let branch = command(root_path, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let branch = if branch.status.success() {
        String::from_utf8_lossy(&branch.stdout).trim().to_owned()
    } else {
        String::from_utf8_lossy(&checked(root_path, &["rev-parse", "--short", "HEAD"])?)
            .trim()
            .to_owned()
    };
    let mut changes = parse_status(&checked(
        root_path,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            if include_untracked {
                "--untracked-files=all"
            } else {
                "--untracked-files=no"
            },
            "--ignore-submodules=dirty",
        ],
    )?);
    changes.retain(|change| {
        !local::private_staging_path(Path::new(&change.path))
            && !change
                .original_path
                .as_deref()
                .is_some_and(|path| local::private_staging_path(Path::new(path)))
    });
    observe_submodules(&canonical, &mut changes, budget, depth, include_untracked)?;
    Ok(Some(GitStatus {
        root,
        branch,
        changes,
    }))
}

fn submodule_configuration_file(path: &Path) -> Result<Option<Vec<u8>>, String> {
    const LIMIT: u64 = 1024 * 1024;
    let file = match crate::files::resolved::open_resolved_file(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Cannot safely read .gitmodules: {error}")),
    };
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > LIMIT {
        return Err(
            "The .gitmodules configuration must be a regular file no larger than 1 MiB.".into(),
        );
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("The .gitmodules configuration exceeds its 1 MiB input limit.".into());
    }
    Ok(Some(bytes))
}

fn submodule_ignore_modes(root: &Path) -> Result<HashMap<String, String>, String> {
    let Some(bytes) = submodule_configuration_file(&root.join(".gitmodules"))? else {
        return Ok(HashMap::new());
    };
    submodule_ignore_modes_from_bytes(root, bytes)
}

fn submodule_ignore_modes_from_bytes(
    root: &Path,
    module_bytes: Vec<u8>,
) -> Result<HashMap<String, String>, String> {
    const CONFIG_LIMIT: usize = 1024 * 1024;
    let read = |args: &[&str], input: Option<Vec<u8>>| -> Result<HashMap<String, String>, String> {
        let bytes = match input {
            Some(input) => history::bounded_bytes_with_input(root, args, CONFIG_LIMIT, input)?,
            None => history::bounded_bytes(root, args, CONFIG_LIMIT)?,
        };
        if bytes.len() > CONFIG_LIMIT {
            return Err("Safe Git submodule configuration exceeded its output limit.".into());
        }
        let mut values = HashMap::new();
        for entry in bytes
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
        {
            let separator = entry
                .iter()
                .position(|byte| *byte == b'\n')
                .unwrap_or(entry.len());
            if !entry[..separator].starts_with(b"submodule.") {
                continue;
            }
            let key = std::str::from_utf8(&entry[..separator])
                .map_err(|_| "Invalid Git submodule configuration key.")?;
            if !key.starts_with("submodule.")
                || !(key.ends_with(".path") || key.ends_with(".ignore"))
            {
                continue;
            }
            let value = std::str::from_utf8(entry.get(separator + 1..).unwrap_or(b"true"))
                .map_err(|_| "Invalid Git submodule configuration value.")?;
            values.insert(key.to_owned(), value.to_owned());
        }
        Ok(values)
    };
    // Git parses only the validated descriptor snapshot, never reopens the
    // .gitmodules pathname after the no-follow/nonblocking regular-file check.
    let modules = read(
        &["config", "--null", "--file", "-", "--no-includes", "--list"],
        Some(module_bytes),
    )?;
    // The isolated command sees effective repository/worktree configuration;
    // local ignore settings take precedence over the .gitmodules defaults.
    let local = read(&["config", "--null", "--list"], None)?;
    let mut modes = HashMap::new();
    for (key, path) in &modules {
        let Some(name) = key
            .strip_prefix("submodule.")
            .and_then(|key| key.strip_suffix(".path"))
        else {
            continue;
        };
        relative(path)?;
        let key = format!("submodule.{name}.ignore");
        let mode = local
            .get(&key)
            .or_else(|| modules.get(&key))
            .map(String::as_str)
            .unwrap_or("none");
        if !matches!(mode, "none" | "untracked" | "dirty" | "all") {
            return Err("Invalid Git submodule ignore setting.".into());
        }
        modes.insert(path.clone(), mode.to_owned());
    }
    Ok(modes)
}

fn observe_submodules(
    root: &Path,
    changes: &mut Vec<Change>,
    budget: &mut StatusBudget,
    depth: usize,
    include_untracked: bool,
) -> Result<(), String> {
    // Let Git report gitlink HEAD/index changes without spawning its own nested
    // status processes. Observe initialized submodules through the same guarded
    // launcher and aggregate their dirty/untracked state into porcelain-v1 M.
    const INDEX_LIMIT: usize = 64 * 1024 * 1024;
    let index = history::bounded_bytes(root, &["ls-files", "--stage", "-z"], INDEX_LIMIT)?;
    if index.len() > INDEX_LIMIT {
        return Err("Safe Git submodule observation exceeded its index output limit.".into());
    }
    if !index
        .split(|byte| *byte == 0)
        .any(|entry| entry.starts_with(b"160000 "))
    {
        return Ok(());
    }
    let ignore_modes = submodule_ignore_modes(root)?;
    let mut seen = HashSet::new();
    for entry in index
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        if !entry.starts_with(b"160000 ") {
            continue;
        }
        budget.entries += 1;
        if budget.entries > 100_000 {
            return Err("Safe Git submodule observation exceeded its gitlink entry limit.".into());
        }
        let separator = entry
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or("Invalid Git submodule index entry.")?;
        let path = &entry[separator + 1..];
        let path =
            std::str::from_utf8(path).map_err(|_| "This Git submodule path is not UTF-8.")?;
        relative(path)?;
        if !seen.insert(path) {
            continue;
        }
        let ignore = ignore_modes.get(path).map(String::as_str).unwrap_or("none");
        if ignore == "all" {
            // Configured ignore=all hides worktree/gitlink HEAD changes, but
            // explicitly staged changes and merge conflicts remain visible.
            changes.retain_mut(|change| {
                if change.path != path {
                    return true;
                }
                if change.index == ' ' {
                    return false;
                }
                let conflict = change.index == 'U'
                    || change.worktree == 'U'
                    || matches!((change.index, change.worktree), ('A', 'A') | ('D', 'D'));
                if !conflict {
                    change.worktree = ' ';
                }
                true
            });
            continue;
        }
        if ignore == "dirty" {
            continue;
        }
        let candidate = root.join(path);
        // Missing .git means an uninitialized/deleted submodule. The root
        // porcelain result already reports a deleted gitlink where appropriate.
        match fs::symlink_metadata(candidate.join(".git")) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        }
        let canonical = fs::canonicalize(&candidate).map_err(|error| error.to_string())?;
        if canonical == root || !canonical.starts_with(root) {
            return Err("This Git submodule points outside its containing repository.".into());
        }
        let child = observed_status(
            canonical
                .to_str()
                .ok_or("This Git submodule path is not UTF-8.")?,
            budget,
            depth + 1,
            include_untracked && ignore != "untracked",
        )?
        .ok_or("An initialized Git submodule is no longer a repository.")?;
        if fs::canonicalize(&child.root).map_err(|error| error.to_string())? != canonical {
            return Err("The Git submodule changed while observing its status.".into());
        }
        if child.changes.is_empty() {
            continue;
        }
        if let Some(change) = changes.iter_mut().find(|change| change.path == path) {
            if change.worktree == ' ' {
                change.worktree = 'M';
            }
        } else {
            changes.push(Change {
                path: path.into(),
                original_path: None,
                index: ' ',
                worktree: 'M',
            });
        }
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(())
}

#[tauri::command]
pub async fn git_status(window: Window, root: String) -> Result<Option<GitStatus>, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || status(&root))
        .await
        .map_err(|error| error.to_string())?
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryScan {
    repositories: Vec<GitStatus>,
    errors: Vec<RepositoryError>,
    limited: bool,
}

#[derive(Serialize)]
pub struct RepositoryError {
    root: String,
    message: String,
}

impl RepositoryScan {
    fn read(&mut self, path: &Path, exact: bool) {
        let root = path.to_string_lossy().into_owned();
        let result = if exact {
            exact_status(&root).map(Some)
        } else {
            status(&root)
        };
        match result {
            Ok(Some(repository)) => {
                if !self
                    .repositories
                    .iter()
                    .any(|item| item.root == repository.root)
                {
                    self.repositories.push(repository);
                }
            }
            Ok(None) => {}
            Err(message) => self.errors.push(RepositoryError { root, message }),
        }
    }
}

fn repositories(project: &str, known_roots: Option<&[String]>) -> Result<RepositoryScan, String> {
    let project = directory(project)?;
    let mut result = RepositoryScan::default();
    if let Some(roots) = known_roots {
        for root in roots.iter().take(64) {
            result.read(Path::new(root), true);
        }
        result.limited = roots.len() > 64;
        return Ok(result);
    }
    result.read(&project, false);
    let mut pending = vec![(project, 0usize)];
    let mut visited = 0usize;
    'scan: while let Some((folder, depth)) = pending.pop() {
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) => {
                result.errors.push(RepositoryError {
                    root: folder.to_string_lossy().into_owned(),
                    message: error.to_string(),
                });
                continue;
            }
        };
        for entry in entries {
            if visited >= 10_000 || result.repositories.len() >= 64 {
                result.limited = true;
                break 'scan;
            }
            visited += 1;
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    result.errors.push(RepositoryError {
                        root: folder.to_string_lossy().into_owned(),
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                ".git" | "node_modules" | "target" | "dist" | "build" | ".next" | ".venv"
            ) {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if !kind.is_dir() || kind.is_symlink() {
                continue;
            }
            let path = entry.path();
            if path.join(".git").exists() {
                result.read(&path, true);
            }
            if depth < 4 {
                pending.push((path, depth + 1));
            } else {
                result.limited = true;
            }
        }
    }
    result.repositories.sort_by(|a, b| a.root.cmp(&b.root));
    Ok(result)
}

#[tauri::command]
pub async fn git_repositories(
    window: Window,
    root: String,
    known_roots: Option<Vec<String>>,
) -> Result<RepositoryScan, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || repositories(&root, known_roots.as_deref()))
        .await
        .map_err(|error| error.to_string())?
}

fn remotes(root: &Path) -> Result<Vec<String>, String> {
    Ok(String::from_utf8_lossy(&checked(root, &["remote"])?)
        .lines()
        .map(str::to_owned)
        .collect())
}

fn validate_remote(root: &Path, remote: &str) -> Result<(), String> {
    if !remotes(root)?.iter().any(|name| name == remote) {
        return Err("This Git remote is no longer configured. Choose a remote again.".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn git_remotes(window: Window, root: String) -> Result<Vec<String>, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || remotes(&repository(&root)?))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn git_fetch(
    window: Window,
    webview: Webview,
    app: AppHandle,
    host: tauri::State<'_, crate::shared_host::SharedHost>,
    root: String,
    remote: Option<String>,
) -> Result<(), String> {
    main_window(&window)?;
    crate::shared_host::trusted_full_window(&window, &webview)?;
    if crate::desktop_runtime::shared_host_enabled() {
        let host = host.inner().clone();
        tauri::async_runtime::spawn_blocking(move || {
            crate::native_git_effects::fetch(&app, &host, &root, remote.as_deref())
        })
        .await
        .map_err(|error| error.to_string())?
    } else {
        tauri::async_runtime::spawn_blocking(move || {
            crate::desktop_runtime::ensure_local_git_write_safe(&app, &root)?;
            local::fetch(&root, remote.as_deref())
        })
        .await
        .map_err(|error| error.to_string())?
    }
}

#[tauri::command]
pub async fn git_push(
    window: Window,
    webview: Webview,
    app: AppHandle,
    host: tauri::State<'_, crate::shared_host::SharedHost>,
    root: String,
    remote: Option<String>,
    force: bool,
) -> Result<(), String> {
    main_window(&window)?;
    crate::shared_host::trusted_full_window(&window, &webview)?;
    if crate::desktop_runtime::shared_host_enabled() {
        let host = host.inner().clone();
        tauri::async_runtime::spawn_blocking(move || {
            crate::native_git_effects::push(&app, &host, &root, remote.as_deref(), force)
        })
        .await
        .map_err(|error| error.to_string())?
    } else {
        tauri::async_runtime::spawn_blocking(move || {
            crate::desktop_runtime::ensure_local_git_write_safe(&app, &root)?;
            local::push(&root, remote.as_deref(), force)
        })
        .await
        .map_err(|error| error.to_string())?
    }
}

fn exact_status(root: &str) -> Result<GitStatus, String> {
    let expected = directory(root)?;
    let status = status(root)?.ok_or("This directory is not a Git repository.")?;
    if directory(&status.root)? != expected {
        return Err("The Git repository changed. Refresh Source Control and try again.".into());
    }
    Ok(status)
}

pub(crate) fn repository(root: &str) -> Result<PathBuf, String> {
    let expected = directory(root)?;
    let found = checked(&expected, &["rev-parse", "--show-toplevel"])?;
    let found = String::from_utf8_lossy(&found);
    if directory(found.trim())? != expected {
        return Err("The Git repository changed. Refresh Source Control and try again.".into());
    }
    Ok(expected)
}

// Prevent Git's parent discovery even if .git disappears after validation.
pub(crate) fn checked_repository(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut command = mutation_command(root, args);
    if let Some(parent) = root.parent() {
        command.env(
            "GIT_CEILING_DIRECTORIES",
            std::env::join_paths([parent]).map_err(|error| error.to_string())?,
        );
    }
    let output = run_mutation_command(&mut command, MUTATION_DEADLINE)?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(output.stdout)
}

static MUTATIONS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
pub(crate) struct MutationGuard {
    path: PathBuf,
    _admission: crate::project_write_guard::WriteAdmission,
}
impl Drop for MutationGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = MUTATIONS.get_or_init(Mutex::default).lock() {
            active.remove(&self.path);
        }
    }
}
pub(crate) fn mutation_guard(root: &str) -> Result<MutationGuard, String> {
    let path = directory(root)?;
    let admission = crate::project_write_guard::admit(&[&path])?;
    if !MUTATIONS
        .get_or_init(Mutex::default)
        .lock()
        .map_err(|_| "Git operations are unavailable.")?
        .insert(path.clone())
    {
        return Err("Wait for the current Git operation in this repository to finish.".into());
    }
    Ok(MutationGuard {
        path,
        _admission: admission,
    })
}

fn relative(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.contains('\0')
        || Path::new(path).components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("Git paths must be relative to the repository.".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn git_stage(
    window: Window,
    webview: Webview,
    app: AppHandle,
    host: tauri::State<'_, crate::shared_host::SharedHost>,
    root: String,
    paths: Vec<String>,
    stage: bool,
) -> Result<(), String> {
    main_window(&window)?;
    crate::shared_host::trusted_full_window(&window, &webview)?;
    if crate::desktop_runtime::shared_host_enabled() {
        let host = host.inner().clone();
        tauri::async_runtime::spawn_blocking(move || {
            crate::native_git_effects::stage(&app, &host, &root, &paths, stage)
        })
        .await
        .map_err(|error| error.to_string())?
    } else {
        tauri::async_runtime::spawn_blocking(move || {
            crate::desktop_runtime::ensure_local_git_write_safe(&app, &root)?;
            local::stage(&root, &paths, stage)
        })
        .await
        .map_err(|error| error.to_string())?
    }
}

#[tauri::command]
pub async fn git_diff(
    window: Window,
    root: String,
    path: String,
    staged: bool,
) -> Result<diff::FileDiff, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || diff::read(&root, &path, staged))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn git_commit(
    window: Window,
    webview: Webview,
    app: AppHandle,
    host: tauri::State<'_, crate::shared_host::SharedHost>,
    root: String,
    message: String,
) -> Result<(), String> {
    main_window(&window)?;
    crate::shared_host::trusted_full_window(&window, &webview)?;
    if crate::desktop_runtime::shared_host_enabled() {
        let host = host.inner().clone();
        tauri::async_runtime::spawn_blocking(move || {
            crate::native_git_effects::commit(&app, &host, &root, &message)
        })
        .await
        .map_err(|error| error.to_string())?
    } else {
        tauri::async_runtime::spawn_blocking(move || {
            crate::desktop_runtime::ensure_local_git_write_safe(&app, &root)?;
            local::commit(&root, &message)
        })
        .await
        .map_err(|error| error.to_string())?
    }
}
