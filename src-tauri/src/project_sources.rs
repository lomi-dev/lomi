//! Explicit "new project" sources: create an empty Git repository or clone one.
//! Never starts a login flow; GitHub state is read through the installed `gh`.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::Window;

const MAX_REPOS: usize = 1000;

/// The single running clone. Kept in the slot (not taken) while polled so
/// `project_source_cancel` can kill it without waiting on the clone thread.
static RUNNING: Mutex<Option<Child>> = Mutex::new(None);
static CANCELLED: AtomicBool = AtomicBool::new(false);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubStatus {
    available: bool,
    login: Option<String>,
    reason: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubRepo {
    #[serde(alias = "full_name")]
    full_name: String,
    description: Option<String>,
    private: bool,
    #[serde(alias = "pushed_at")]
    pushed_at: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CloneSource {
    Url { url: String },
    Github { repo: String },
}

fn unavailable(reason: &str) -> GithubStatus {
    GithubStatus {
        available: false,
        login: None,
        reason: Some(reason.into()),
    }
}

fn github_status() -> GithubStatus {
    let Some(gh) = crate::cli_bin::resolve("gh") else {
        return unavailable("missing");
    };
    let mut command = gh_command(&gh);
    command.args(["api", "user", "--jq", ".login"]);
    match run_capture(command, Duration::from_secs(15)) {
        Ok((status, out, _)) if status.success() && !out.trim().is_empty() => GithubStatus {
            available: true,
            login: Some(out.trim().to_owned()),
            reason: None,
        },
        _ => unavailable("signed-out"),
    }
}

fn github_repos() -> Result<Vec<GithubRepo>, String> {
    let gh = crate::cli_bin::resolve("gh").ok_or("GitHub CLI (gh) is not installed.")?;
    let mut command = gh_command(&gh);
    command.args([
        "api",
        "--paginate",
        "user/repos?per_page=100&sort=pushed&affiliation=owner,collaborator,organization_member",
        "--jq",
        ".[] | {full_name, description, private, pushed_at}",
    ]);
    let (status, out, err) = run_capture(command, Duration::from_secs(45))?;
    if !status.success() {
        return Err(failure_message(&err, "Could not list GitHub repositories."));
    }
    out.lines()
        .filter(|line| !line.trim().is_empty())
        .take(MAX_REPOS)
        .map(|line| serde_json::from_str(line).map_err(|error| error.to_string()))
        .collect()
}

fn gh_command(gh: &Path) -> Command {
    let mut command = Command::new(gh);
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null());
    hide_console(&mut command);
    command
}

fn git_command() -> Command {
    let mut command = Command::new("git");
    command
        .env_clear()
        .envs(
            std::env::vars_os()
                .filter(|(name, _)| crate::git::safe_mutation_environment_name(name)),
        )
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PROTOCOL_FROM_USER", "0")
        .stdin(Stdio::null());
    hide_console(&mut command);
    command
}

fn hide_console(_command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        _command.creation_flags(0x08000000);
    }
}

fn tail_lines(reader: impl Read) -> String {
    let mut tail = VecDeque::new();
    for line in BufReader::new(reader).split(b'\n').map_while(Result::ok) {
        let line = String::from_utf8_lossy(&line).trim().to_owned();
        // git's own progress header is noise in an error message.
        if !line.is_empty() && !line.starts_with("Cloning into") {
            if tail.len() == 8 {
                tail.pop_front();
            }
            tail.push_back(line);
        }
    }
    Vec::from(tail).join("\n")
}

fn failure_message(stderr: &str, fallback: &str) -> String {
    let text = stderr.trim();
    if text.is_empty() {
        fallback.into()
    } else {
        text.into()
    }
}

/// Runs a command to completion, killing it after `timeout`.
fn run_capture(
    mut command: Command,
    timeout: Duration,
) -> Result<(ExitStatus, String, String), String> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let mut stdout = child.stdout.take().ok_or("missing stdout")?;
    let stderr = child.stderr.take().ok_or("missing stderr")?;
    let out = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let err = std::thread::spawn(move || tail_lines(stderr));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("The command timed out.".into());
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    Ok((
        status,
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default(),
    ))
}

fn valid_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    let bad = name.is_empty()
        || name.len() > 255
        || name == "."
        || name == ".."
        || name.starts_with('-')
        || name.chars().any(|c| {
            c.is_control()
                || matches!(c, '/' | '\\')
                || (cfg!(windows) && matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        });
    if bad {
        Err("Enter a valid folder name.".into())
    } else {
        Ok(name)
    }
}

fn valid_url(url: &str) -> Result<&str, String> {
    let url = url.trim();
    let invalid = || Err("Enter a valid repository URL.".to_string());
    if url.is_empty()
        || url.starts_with('-')
        || url.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return invalid();
    }
    if ["https://", "http://", "ssh://", "git://"]
        .iter()
        .any(|scheme| url.len() > scheme.len() && url.starts_with(scheme))
    {
        return Ok(url);
    }
    // scp-like `user@host:path`: no scheme separator and no slash before the colon.
    match url.split_once(':') {
        Some((host, path))
            if !path.is_empty()
                && !path.starts_with("//")
                && !host.contains('/')
                && host
                    .split_once('@')
                    .is_some_and(|(u, h)| !u.is_empty() && !h.is_empty()) =>
        {
            Ok(url)
        }
        _ => invalid(),
    }
}

fn valid_repo(repo: &str) -> Result<&str, String> {
    let ok = repo.split_once('/').is_some_and(|(owner, name)| {
        !owner.is_empty()
            && !name.is_empty()
            && !owner.starts_with('-')
            && name != "."
            && name != ".."
            && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    });
    if ok {
        Ok(repo)
    } else {
        Err("Enter a repository as owner/name.".into())
    }
}

/// Validates the inputs and returns the not-yet-existing target path.
fn target_path(parent: &str, name: &str) -> Result<PathBuf, String> {
    let name = valid_name(name)?;
    let parent = std::fs::canonicalize(parent)
        .ok()
        .filter(|path| path.is_dir())
        .ok_or("The parent folder does not exist.")?;
    let target = parent.join(name);
    if target.symlink_metadata().is_ok() {
        return Err(format!("A folder named {name} already exists here."));
    }
    Ok(target)
}

fn canonical(target: &Path) -> Result<String, String> {
    std::fs::canonicalize(target)
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|error| error.to_string())
}

fn create(parent: &str, name: &str) -> Result<String, String> {
    let target = target_path(parent, name)?;
    std::fs::create_dir(&target).map_err(|error| error.to_string())?;
    let result = (|| {
        let mut command = git_command();
        command.arg("-C").arg(&target).arg("init");
        let (status, _, err) = run_capture(command, Duration::from_secs(30))?;
        if status.success() {
            Ok(())
        } else {
            Err(failure_message(&err, "git init failed."))
        }
    })();
    if let Err(error) = result {
        // We created the folder just now and nothing else wrote to it.
        let _ = std::fs::remove_dir_all(&target);
        return Err(error);
    }
    canonical(&target)
}

fn clone(parent: &str, name: &str, source: CloneSource) -> Result<String, String> {
    let target = target_path(parent, name)?;
    let mut command = match &source {
        CloneSource::Url { url } => {
            let url = valid_url(url)?;
            let mut command = git_command();
            command.args(["clone", "--", url]).arg(&target);
            command
        }
        CloneSource::Github { repo } => {
            let repo = valid_repo(repo)?;
            let gh = crate::cli_bin::resolve("gh").ok_or("GitHub CLI (gh) is not installed.")?;
            let mut command = gh_command(&gh);
            command
                .env("GIT_PROTOCOL_FROM_USER", "0")
                .args(["repo", "clone", repo])
                .arg(&target);
            command
        }
    };
    command.stdout(Stdio::null()).stderr(Stdio::piped());
    // Own process group, so cancel also stops git/ssh children spawned by gh.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let stderr = {
        let mut slot = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return Err("Another clone is already running.".into());
        }
        CANCELLED.store(false, Ordering::SeqCst);
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        let stderr = child.stderr.take();
        *slot = Some(child);
        stderr
    };
    let reader = std::thread::spawn(move || stderr.map(tail_lines).unwrap_or_default());
    // Poll with the lock released between checks so cancel is never blocked.
    let status = loop {
        let polled = RUNNING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
            .map(Child::try_wait);
        match polled {
            Some(Ok(None)) => std::thread::sleep(Duration::from_millis(100)),
            Some(Ok(Some(status))) => break Ok(status),
            Some(Err(error)) => break Err(error.to_string()),
            None => break Err("Clone cancelled.".to_string()),
        }
    };
    *RUNNING.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let err = reader.join().unwrap_or_default();
    let outcome = match status {
        _ if CANCELLED.load(Ordering::SeqCst) => Err("Clone cancelled.".into()),
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Err(failure_message(&err, "Clone failed.")),
        Err(error) => Err(error),
    };
    match outcome {
        Ok(()) => canonical(&target),
        Err(error) => {
            // The target did not exist before this clone started.
            if target.symlink_metadata().is_ok() {
                let _ = std::fs::remove_dir_all(&target);
            }
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn project_source_github_status(window: Window) -> Result<GithubStatus, String> {
    crate::files::main_window(&window)?;
    tauri::async_runtime::spawn_blocking(github_status)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn project_source_github_repos(window: Window) -> Result<Vec<GithubRepo>, String> {
    crate::files::main_window(&window)?;
    tauri::async_runtime::spawn_blocking(github_repos)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn project_source_create(
    window: Window,
    parent: String,
    name: String,
) -> Result<String, String> {
    crate::files::main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || create(&parent, &name))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn project_source_clone(
    window: Window,
    parent: String,
    name: String,
    source: CloneSource,
) -> Result<String, String> {
    crate::files::main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || clone(&parent, &name, source))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn project_source_cancel(window: Window) -> Result<(), String> {
    crate::files::main_window(&window)?;
    tauri::async_runtime::spawn_blocking(|| {
        if let Some(child) = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            CANCELLED.store(true, Ordering::SeqCst);
            // SAFETY: killpg only signals the group this clone created with process_group(0).
            #[cfg(unix)]
            unsafe {
                libc::killpg(child.id() as libc::pid_t, libc::SIGKILL)
            };
            let _ = child.kill();
        }
    })
    .await
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        for ok in ["app", " app ", "a b", "my.repo", "é"] {
            assert!(valid_name(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "  ",
            ".",
            "..",
            "a/b",
            "a\\b",
            "a\0b",
            "-x",
            "a\nb",
            &"x".repeat(256),
        ] {
            assert!(valid_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn urls() {
        for ok in [
            "https://github.com/a/b.git",
            "ssh://git@host/a/b",
            "git@github.com:a/b.git",
        ] {
            assert!(valid_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "-oProxyCommand=x",
            "ext::sh -c x",
            "file:///etc",
            "",
            "https://a b",
            "ext::sh",
            "host:path",
        ] {
            assert!(valid_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn repos() {
        for ok in ["a/b", "My-Org/repo.name_1"] {
            assert!(valid_repo(ok).is_ok(), "{ok}");
        }
        for bad in [
            "a", "a/", "/b", "a/b/c", "a b/c", "-x/../y", "a/b c", "a_b/c", "-x/y", "a/..",
        ] {
            assert!(valid_repo(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn create_makes_repository_and_refuses_existing() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().to_str().unwrap();
        let path = create(parent, "proj").unwrap();
        assert!(Path::new(&path).join(".git").is_dir());
        let error = create(parent, "proj").unwrap_err();
        assert_eq!(error, "A folder named proj already exists here.");
    }
}
