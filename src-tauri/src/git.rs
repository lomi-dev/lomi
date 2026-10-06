use crate::files::{directory, main_window};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::{Mutex, OnceLock},
};
use tauri::Window;

mod diff;
pub mod history;
mod observation;
#[cfg(test)]
mod regression;

fn mutation_command(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
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

#[cfg(test)]
pub(crate) fn test_checked(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = mutation_command(root, args)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(output.stdout)
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

fn fetch(root: &str, remote: Option<&str>) -> Result<(), String> {
    let _operation = mutation_guard(root)?;
    let root = repository(root)?;
    if let Some(remote) = remote {
        validate_remote(&root, remote)?;
        checked_repository(&root, &["fetch", "--", remote])?;
    } else if remotes(&root)?.is_empty() {
        return Err(
            "No Git remote is configured. Add a remote in a terminal, then try again.".into(),
        );
    } else {
        checked_repository(&root, &["fetch", "--all"])?;
    }
    Ok(())
}

#[tauri::command]
pub async fn git_fetch(window: Window, root: String, remote: Option<String>) -> Result<(), String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || fetch(&root, remote.as_deref()))
        .await
        .map_err(|error| error.to_string())?
}

pub(crate) fn pull(root: &str, rebase: bool) -> Result<(), String> {
    let _operation = mutation_guard(root)?;
    let root = repository(root)?;
    checked_repository(
        &root,
        if rebase {
            &["pull", "--rebase", "--ff", "--no-autostash"]
        } else {
            &["pull", "--ff-only", "--no-rebase", "--no-autostash"]
        },
    )?;
    Ok(())
}

fn push(root: &str, remote: Option<&str>, force: bool) -> Result<(), String> {
    let _operation = mutation_guard(root)?;
    let root = repository(root)?;
    checked(&root, &["symbolic-ref", "--quiet", "HEAD"])
        .map_err(|_| "Check out a branch before pushing.".to_string())?;
    let mut args = vec!["push"];
    if force {
        args.push("--force-with-lease");
    }
    if let Some(remote) = remote {
        validate_remote(&root, remote)?;
        args.extend(["--", remote, "HEAD"]);
    }
    checked_repository(&root, &args)?;
    Ok(())
}

#[tauri::command]
pub async fn git_push(
    window: Window,
    root: String,
    remote: Option<String>,
    force: bool,
) -> Result<(), String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || push(&root, remote.as_deref(), force))
        .await
        .map_err(|error| error.to_string())?
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
    let output = command
        .output()
        .map_err(|error| format!("Cannot run Git: {error}"))?;
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

pub fn change_index(root: &str, paths: &[String], stage: bool) -> Result<(), String> {
    let _operation = mutation_guard(root)?;
    let root = repository(root)?;
    if paths.is_empty() {
        return Ok(());
    }
    for path in paths {
        relative(path)?;
    }
    let has_head = command(&root, &["rev-parse", "--verify", "HEAD"])?
        .status
        .success();
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    let owned = lomi_control_core::git_execution::index_arguments(paths, stage, has_head);
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    let args: Vec<&str> = owned.iter().map(String::as_str).collect();
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    let args = {
        let mut args = if stage {
            vec!["--literal-pathspecs", "add", "--"]
        } else if has_head {
            vec!["--literal-pathspecs", "restore", "--staged", "--"]
        } else {
            vec!["--literal-pathspecs", "rm", "--cached", "--"]
        };
        args.extend(paths.iter().map(String::as_str));
        args
    };
    checked_repository(&root, &args)?;
    Ok(())
}

pub(crate) fn discard(root: &str, expected: &Change) -> Result<(), String> {
    let _operation = mutation_guard(root)?;
    relative(&expected.path)?;
    let status = exact_status(root)?;
    let change = status
        .changes
        .iter()
        .find(|change| change.path == expected.path);
    if change != Some(expected) {
        return Err("The file status changed. Refresh Source Control and try again.".into());
    }
    let root = Path::new(&status.root);
    let path = root.join(&expected.path);
    for parent in path
        .ancestors()
        .skip(1)
        .take_while(|parent| *parent != root)
    {
        match std::fs::symlink_metadata(parent) {
            Ok(metadata) if metadata.is_symlink() => {
                return Err("Cannot discard changes through a symbolic link.".into())
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(error.to_string())
            }
            _ => {}
        }
    }
    if expected.index == '?' && expected.worktree == '?' {
        return trash::delete(&path).map_err(|error| error.to_string());
    }
    if !matches!(expected.worktree, 'M' | 'D' | 'T')
        || expected.index == 'U'
        || std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_dir())
    {
        return Err(
            "Only unconflicted working tree files can be discarded. Unstage staged changes first."
                .into(),
        );
    }
    checked_repository(
        root,
        &[
            "--literal-pathspecs",
            "restore",
            "--worktree",
            "--",
            &expected.path,
        ],
    )?;
    Ok(())
}

#[tauri::command]
pub async fn git_stage(
    window: Window,
    root: String,
    paths: Vec<String>,
    stage: bool,
) -> Result<(), String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || change_index(&root, &paths, stage))
        .await
        .map_err(|error| error.to_string())?
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

fn commit(root: &str, message: &str) -> Result<(), String> {
    if message.trim().is_empty() {
        return Err("Enter a commit message.".into());
    }
    let _operation = mutation_guard(root)?;
    let root = repository(root)?;
    checked_repository(&root, &["commit", "--cleanup=verbatim", "-m", message])?;
    Ok(())
}

#[tauri::command]
pub async fn git_commit(window: Window, root: String, message: String) -> Result<(), String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || commit(&root, &message))
        .await
        .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::test_checked as checked;
    use super::*;

    fn test_repository() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        checked(root.path(), &["init", "-b", "main"]).unwrap();
        for (key, value) in [
            ("user.name", "Git Test"),
            ("user.email", "git@example.test"),
            ("commit.gpgsign", "false"),
            ("core.hooksPath", ".git/disabled-hooks"),
            ("core.autocrlf", "false"),
        ] {
            checked(root.path(), &["config", key, value]).unwrap();
        }
        root
    }

    #[test]
    fn fetch_and_pull_update_remotes_and_preserve_local_work() {
        let remote = test_repository();
        let local = test_repository();
        let path = local.path().to_str().unwrap();
        let commit_remote = |content: &str| {
            std::fs::write(remote.path().join("file.txt"), content).unwrap();
            checked(remote.path(), &["add", "file.txt"]).unwrap();
            checked(remote.path(), &["commit", "-m", content]).unwrap();
            checked(remote.path(), &["rev-parse", "HEAD"]).unwrap()
        };
        let initial = commit_remote("initial");
        assert!(fetch(path, None).unwrap_err().contains("No Git remote"));
        checked(
            local.path(),
            &["remote", "add", "origin", remote.path().to_str().unwrap()],
        )
        .unwrap();
        fetch(path, None).unwrap();
        checked(
            local.path(),
            &["checkout", "-b", "main", "--track", "origin/main"],
        )
        .unwrap();
        for (key, value) in [
            ("pull.rebase", "true"),
            ("pull.ff", "false"),
            ("rebase.autoStash", "true"),
            ("merge.autoStash", "true"),
        ] {
            checked(local.path(), &["config", key, value]).unwrap();
        }
        let updated = commit_remote("remote update");
        fetch(path, None).unwrap();
        assert_eq!(
            checked(local.path(), &["rev-parse", "HEAD"]).unwrap(),
            initial
        );
        assert_eq!(
            checked(local.path(), &["rev-parse", "origin/main"]).unwrap(),
            updated
        );
        assert_eq!(
            std::fs::read_to_string(local.path().join("file.txt")).unwrap(),
            "initial"
        );
        pull(path, false).unwrap();
        assert_eq!(
            checked(local.path(), &["rev-parse", "HEAD"]).unwrap(),
            updated
        );
        assert_eq!(
            std::fs::read_to_string(local.path().join("file.txt")).unwrap(),
            "remote update"
        );

        std::fs::write(local.path().join("file.txt"), "staged local edits").unwrap();
        checked(local.path(), &["add", "file.txt"]).unwrap();
        std::fs::write(local.path().join("file.txt"), "unstaged local edits").unwrap();
        commit_remote("another remote update");
        assert!(pull(path, false).is_err());
        assert_eq!(
            checked(local.path(), &["rev-parse", "HEAD"]).unwrap(),
            updated
        );
        assert_eq!(
            checked(local.path(), &["show", ":file.txt"]).unwrap(),
            b"staged local edits"
        );
        assert_eq!(
            std::fs::read_to_string(local.path().join("file.txt")).unwrap(),
            "unstaged local edits"
        );
        assert!(checked(local.path(), &["stash", "list"])
            .unwrap()
            .is_empty());

        checked(local.path(), &["add", "file.txt"]).unwrap();
        checked(local.path(), &["commit", "-m", "local commit"]).unwrap();
        let diverged = checked(local.path(), &["rev-parse", "HEAD"]).unwrap();
        assert!(pull(path, false).unwrap_err().contains("fast-forward"));
        assert_eq!(
            checked(local.path(), &["rev-parse", "HEAD"]).unwrap(),
            diverged
        );
        assert!(!local.path().join(".git/MERGE_HEAD").exists());
        assert!(!local.path().join(".git/rebase-merge").exists());

        checked(local.path(), &["checkout", "-b", "without-upstream"]).unwrap();
        assert!(pull(path, false)
            .unwrap_err()
            .contains("tracking information"));
        checked(local.path(), &["checkout", "--detach"]).unwrap();
        assert!(pull(path, false).is_err());
        checked(
            local.path(),
            &[
                "remote",
                "set-url",
                "origin",
                remote.path().join("missing.git").to_str().unwrap(),
            ],
        )
        .unwrap();
        assert!(fetch(path, None).is_err());
    }

    #[test]
    fn selected_remotes_rebase_and_force_push_preserve_unseen_remote_commits() {
        let origin = tempfile::tempdir().unwrap();
        let backup = tempfile::tempdir().unwrap();
        let local = test_repository();
        let other = test_repository();
        let path = local.path().to_str().unwrap();
        let other_path = other.path().to_str().unwrap();
        for (name, remote) in [("origin", &origin), ("backup", &backup)] {
            checked(remote.path(), &["init", "--bare", "-b", "main"]).unwrap();
            checked(
                local.path(),
                &["remote", "add", name, remote.path().to_str().unwrap()],
            )
            .unwrap();
        }
        let commit = |root: &Path, name: &str| {
            std::fs::write(root.join(name), name).unwrap();
            checked(root, &["add", name]).unwrap();
            checked(root, &["commit", "-m", name]).unwrap();
        };
        commit(local.path(), "initial.txt");
        assert_eq!(remotes(local.path()).unwrap(), ["backup", "origin"]);
        push(path, Some("origin"), false).unwrap();
        push(path, Some("backup"), false).unwrap();
        checked(local.path(), &["branch", "--set-upstream-to=origin/main"]).unwrap();
        checked(
            other.path(),
            &["remote", "add", "origin", origin.path().to_str().unwrap()],
        )
        .unwrap();
        fetch(other_path, Some("origin")).unwrap();
        checked(
            other.path(),
            &["checkout", "-b", "main", "--track", "origin/main"],
        )
        .unwrap();
        commit(other.path(), "remote.txt");
        push(other_path, None, false).unwrap();
        let remote_head = checked(origin.path(), &["rev-parse", "main"]).unwrap();
        commit(local.path(), "local.txt");
        checked(local.path(), &["config", "pull.ff", "only"]).unwrap();
        checked(local.path(), &["config", "pull.rebase", "false"]).unwrap();
        pull(path, true).unwrap();
        assert_eq!(
            checked(local.path(), &["rev-parse", "HEAD^"]).unwrap(),
            remote_head
        );
        assert!(local.path().join("local.txt").exists());
        assert!(local.path().join("remote.txt").exists());
        push(path, None, false).unwrap();

        checked(
            local.path(),
            &["commit", "--amend", "-m", "rewrite local commit"],
        )
        .unwrap();
        assert!(push(path, None, false).is_err());
        push(path, None, true).unwrap();
        let pushed = checked(local.path(), &["rev-parse", "HEAD"]).unwrap();
        assert_eq!(
            checked(origin.path(), &["rev-parse", "main"]).unwrap(),
            pushed
        );

        fetch(other_path, Some("origin")).unwrap();
        checked(other.path(), &["reset", "--hard", "origin/main"]).unwrap();
        commit(other.path(), "unseen.txt");
        push(other_path, None, false).unwrap();
        let unseen = checked(origin.path(), &["rev-parse", "main"]).unwrap();
        checked(local.path(), &["commit", "--amend", "-m", "rewrite again"]).unwrap();
        assert!(push(path, None, true).is_err());
        assert_eq!(
            checked(origin.path(), &["rev-parse", "main"]).unwrap(),
            unseen
        );
        fetch(path, Some("backup")).unwrap();
        assert_eq!(
            checked(local.path(), &["rev-parse", "origin/main"]).unwrap(),
            pushed
        );
        fetch(path, Some("origin")).unwrap();
        assert_eq!(
            checked(local.path(), &["rev-parse", "origin/main"]).unwrap(),
            unseen
        );
        for remote in ["missing", "--upload-pack=unexpected"] {
            assert!(fetch(path, Some(remote)).is_err());
            assert!(push(path, Some(remote), false).is_err());
        }
        checked(local.path(), &["checkout", "--detach"]).unwrap();
        assert!(push(path, Some("origin"), false).is_err());
    }

    #[test]
    fn discard_restores_only_the_selected_worktree_file_and_rejects_stale_status() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().to_str().unwrap();
        checked(root.path(), &["init", "-b", "main"]).unwrap();
        let name = "literal[1].txt";
        std::fs::write(root.path().join(name), "staged text").unwrap();
        std::fs::write(root.path().join("literal1.txt"), "other staged text").unwrap();
        change_index(path, &[name.into(), "literal1.txt".into()], true).unwrap();
        std::fs::write(root.path().join(name), "unstaged text").unwrap();
        std::fs::write(root.path().join("literal1.txt"), "keep other changes").unwrap();
        let change = status(path)
            .unwrap()
            .unwrap()
            .changes
            .into_iter()
            .find(|change| change.path == name)
            .unwrap();
        discard(path, &change).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join(name)).unwrap(),
            "staged text"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("literal1.txt")).unwrap(),
            "keep other changes"
        );
        assert_eq!(
            checked(root.path(), &["show", &format!(":{name}")]).unwrap(),
            b"staged text"
        );
        assert!(discard(path, &change).is_err());
        std::fs::remove_file(root.path().join(name)).unwrap();
        let change = status(path)
            .unwrap()
            .unwrap()
            .changes
            .into_iter()
            .find(|change| change.path == name)
            .unwrap();
        discard(path, &change).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join(name)).unwrap(),
            "staged text"
        );
        let mut change = change;
        change.path = "../outside".into();
        assert!(discard(path, &change).is_err());
        change.path = ".".into();
        assert!(discard(path, &change).is_err());
        std::fs::write(root.path().join("new.txt"), "untracked").unwrap();
        let new = status(path)
            .unwrap()
            .unwrap()
            .changes
            .into_iter()
            .find(|change| change.path == "new.txt")
            .unwrap();
        change_index(path, &["new.txt".into()], true).unwrap();
        assert!(discard(path, &new).is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("new.txt")).unwrap(),
            "untracked"
        );
    }

    #[cfg(unix)]
    #[test]
    fn discard_does_not_follow_a_replaced_parent_symlink() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let path = root.path().to_str().unwrap();
        checked(root.path(), &["init", "-b", "main"]).unwrap();
        std::fs::create_dir(root.path().join("nested")).unwrap();
        std::fs::write(root.path().join("nested/file"), "staged").unwrap();
        change_index(path, &["nested/file".into()], true).unwrap();
        std::fs::remove_dir_all(root.path().join("nested")).unwrap();
        std::fs::write(outside.path().join("file"), "outside data").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("nested")).unwrap();
        let change = status(path)
            .unwrap()
            .unwrap()
            .changes
            .into_iter()
            .find(|change| change.path == "nested/file")
            .unwrap();
        assert!(discard(path, &change).is_err());
        assert_eq!(
            std::fs::read_to_string(outside.path().join("file")).unwrap(),
            "outside data"
        );
    }

    #[test]
    fn parses_renames_and_unusual_names() {
        let changes = parse_status(b"R  new name\0old name\0?? a\nfile\0 M space name\0");
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].original_path.as_deref(), Some("old name"));
        assert_eq!(changes[1].path, "a\nfile");
        assert_eq!(changes[2].worktree, 'M');
    }
    #[test]
    fn lists_individual_untracked_files_and_excludes_ignored_files() {
        let root = tempfile::tempdir().unwrap();
        checked(root.path(), &["init", "-b", "main"]).unwrap();
        std::fs::create_dir_all(root.path().join("new/nested")).unwrap();
        std::fs::write(root.path().join(".git/info/exclude"), "*.log\n").unwrap();
        std::fs::write(root.path().join("new/nested/file.ts"), "new").unwrap();
        std::fs::write(root.path().join("new/nested/ignored.log"), "ignored").unwrap();
        let changes = status(root.path().join("new").to_str().unwrap())
            .unwrap()
            .unwrap()
            .changes;
        assert_eq!(
            changes,
            vec![Change {
                path: "new/nested/file.ts".into(),
                original_path: None,
                index: '?',
                worktree: '?',
            }]
        );
    }

    #[test]
    fn detects_repositories_and_unstages_unborn_commits_without_deleting_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().to_str().unwrap();
        assert!(status(path).unwrap().is_none());
        checked(root.path(), &["init", "-b", "main"]).unwrap();
        std::fs::write(root.path().join("literal[1].txt"), "hello").unwrap();
        let paths = vec!["literal[1].txt".into()];
        change_index(path, &paths, true).unwrap();
        assert_eq!(status(path).unwrap().unwrap().changes[0].index, 'A');
        change_index(path, &paths, false).unwrap();
        assert!(root.path().join("literal[1].txt").exists());
        assert_eq!(status(path).unwrap().unwrap().changes[0].index, '?');
    }

    #[test]
    fn discovers_sibling_and_nested_repositories() {
        let project = tempfile::tempdir().unwrap();
        let first = project.path().join("first");
        let second = project.path().join("group").join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        checked(&first, &["init", "-b", "main"]).unwrap();
        checked(&second, &["init", "-b", "main"]).unwrap();
        fs::write(first.join("one.txt"), "one").unwrap();
        fs::write(second.join("two.txt"), "two").unwrap();
        let found = repositories(project.path().to_str().unwrap(), None)
            .unwrap()
            .repositories;
        assert_eq!(found.len(), 2);
        assert_eq!(
            found[0].root,
            fs::canonicalize(&first).unwrap().to_string_lossy()
        );
        assert_eq!(found[0].changes[0].path, "one.txt");
        assert_eq!(
            found[1].root,
            fs::canonicalize(&second).unwrap().to_string_lossy()
        );
        assert_eq!(found[1].changes[0].path, "two.txt");
    }
}
