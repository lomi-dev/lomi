use crate::files::{directory, main_window};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
    process::{Command, Output},
    sync::{Mutex, OnceLock},
};
use tauri::Window;

mod diff;
pub mod history;
#[cfg(test)]
mod regression;

fn configured_command(root: &Path, args: &[&str]) -> Command {
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

fn command(root: &Path, args: &[&str]) -> Result<Output, String> {
    configured_command(root, args)
        .output()
        .map_err(|error| format!("Cannot run Git: {error}"))
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

pub fn status(path: &str) -> Result<Option<GitStatus>, String> {
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
    let branch = command(root_path, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let branch = if branch.status.success() {
        String::from_utf8_lossy(&branch.stdout).trim().to_owned()
    } else {
        String::from_utf8_lossy(&checked(root_path, &["rev-parse", "--short", "HEAD"])?)
            .trim()
            .to_owned()
    };
    let changes = parse_status(&checked(
        root_path,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?);
    Ok(Some(GitStatus {
        root,
        branch,
        changes,
    }))
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
    Ok(PathBuf::from(exact_status(root)?.root))
}

// Prevent Git's parent discovery even if .git disappears after validation.
pub(crate) fn checked_repository(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut command = configured_command(root, args);
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
pub(crate) struct MutationGuard(PathBuf);
impl Drop for MutationGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = MUTATIONS.get_or_init(Mutex::default).lock() {
            active.remove(&self.0);
        }
    }
}
pub(crate) fn mutation_guard(root: &str) -> Result<MutationGuard, String> {
    let path = directory(root)?;
    if !MUTATIONS
        .get_or_init(Mutex::default)
        .lock()
        .map_err(|_| "Git operations are unavailable.")?
        .insert(path.clone())
    {
        return Err("Wait for the current Git operation in this repository to finish.".into());
    }
    Ok(MutationGuard(path))
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
