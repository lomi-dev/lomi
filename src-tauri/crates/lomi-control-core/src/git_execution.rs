//! Native approval snapshots for code-executing Git. Nothing in this module is
//! exposed as a tool; the broker must first authorize git.execute and obtain an
//! exact main-window decision before a prepared plan can be dispatched.
pub mod commit;
pub mod discard;
pub mod fetch;
pub mod pull;
pub mod push;
use crate::{
    git_read::{self, Observation},
    project_files::{validate_relative, ProjectDirectory},
};
use lomi_control_protocol::ErrorCode;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap, ffi::OsString, fs::File, io::Read, os::unix::fs::MetadataExt, sync::Arc,
};

#[derive(Clone)]
pub struct Environment(BTreeMap<OsString, OsString>);
impl Environment {
    /// Caller options can never supply executable names, environment or -c flags.
    pub fn capture() -> Self {
        let mut values: BTreeMap<_, _> = std::env::vars_os()
            .filter(|(name, _)| {
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
            })
            .collect();
        for (key, value) in [
            ("LC_ALL", "C"),
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_NO_LAZY_FETCH", "1"),
            ("GIT_NO_REPLACE_OBJECTS", "1"),
        ] {
            values.insert(key.into(), value.into());
        }
        Self(values)
    }
    fn fingerprint(&self) -> String {
        let mut hash = Sha256::new();
        for (key, value) in &self.0 {
            for bytes in [key.as_encoded_bytes(), value.as_encoded_bytes()] {
                hash.update((bytes.len() as u64).to_le_bytes());
                hash.update(bytes);
            }
        }
        format!("{:x}", hash.finalize())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileIdentity {
    pub relative_path: String,
    pub sha256: Option<String>,
    pub byte_length: u64,
    pub mode: u32,
    device: u64,
    inode: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub head: Option<String>,
    pub branch: Option<String>,
    pub index_revision: String,
    pub configuration_revision: String,
    pub references_revision: String,
    pub status_revision: String,
    pub files: Vec<FileIdentity>,
    repository_identity: (u64, u64),
    environment_revision: String,
}
impl Snapshot {
    pub fn revision(&self) -> Result<String, ErrorCode> {
        Ok(digest(
            &serde_json::to_vec(self).map_err(|_| ErrorCode::ResourceExhausted)?,
        ))
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn metadata_bytes(
    directory: &File,
    name: &str,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<String, ErrorCode> {
    use rustix::fs::{openat, Mode, OFlags};
    let mut file = match openat(
        directory,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => File::from(fd),
        Err(rustix::io::Errno::NOENT) => return Ok(digest(b"missing")),
        Err(_) => return Err(ErrorCode::ScopeDenied),
    };
    let before = file.metadata().map_err(|_| ErrorCode::StorageUnavailable)?;
    if !before.is_file() || before.nlink() != 1 {
        return Err(ErrorCode::ScopeDenied);
    }
    if before.len() > 32 * 1024 * 1024 {
        return Err(ErrorCode::ResourceExhausted);
    }
    let mut hash = Sha256::new();
    hash.update(b"present");
    let mut total = 0;
    let mut bytes = [0; 65536];
    loop {
        check()?;
        let n = file
            .read(&mut bytes)
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        if n == 0 {
            break;
        }
        total += n;
        if total > 32 * 1024 * 1024 {
            return Err(ErrorCode::ResourceExhausted);
        }
        hash.update(&bytes[..n]);
    }
    let after = file.metadata().map_err(|_| ErrorCode::StorageUnavailable)?;
    if (
        before.dev(),
        before.ino(),
        before.len(),
        before.mtime(),
        before.mtime_nsec(),
        before.ctime(),
        before.ctime_nsec(),
    ) != (
        after.dev(),
        after.ino(),
        after.len(),
        after.mtime(),
        after.mtime_nsec(),
        after.ctime(),
        after.ctime_nsec(),
    ) {
        return Err(ErrorCode::RevisionConflict);
    }
    // Physical replacement also invalidates approval, even with identical bytes.
    for value in [
        after.dev(),
        after.ino(),
        after.len(),
        u64::from(after.mode()),
    ] {
        hash.update(value.to_le_bytes());
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn observe_file(
    project: &ProjectDirectory,
    repository: &str,
    relative: &str,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<FileIdentity, ErrorCode> {
    use rustix::fs::{statat, AtFlags, FileType};
    validate_relative(relative)?;
    let path = if repository.is_empty() {
        relative.to_string()
    } else {
        format!("{repository}/{relative}")
    };
    // Missing parent directories are a valid absent target. Existing components
    // are still opened individually without following links.
    let missing = || FileIdentity {
        relative_path: relative.into(),
        sha256: None,
        byte_length: 0,
        mode: 0,
        device: 0,
        inode: 0,
        modified: (0, 0),
        changed: (0, 0),
    };
    let (parent, name) = path.rsplit_once('/').unwrap_or(("", &path));
    let mut directory = project.open_directory("")?;
    if !parent.is_empty() {
        for component in parent.split('/') {
            use rustix::fs::{openat, Mode, OFlags};
            check()?;
            directory = match openat(
                &directory,
                component,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                Ok(fd) => File::from(fd),
                Err(rustix::io::Errno::NOENT) => return Ok(missing()),
                Err(_) => return Err(ErrorCode::ScopeDenied),
            };
        }
    }
    let parent = directory;
    let metadata = match statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(m) => m,
        Err(rustix::io::Errno::NOENT) => return Ok(missing()),
        Err(_) => return Err(ErrorCode::ScopeDenied),
    };
    if FileType::from_raw_mode(metadata.st_mode) != FileType::RegularFile || metadata.st_nlink != 1
    {
        return Err(ErrorCode::ScopeDenied);
    }
    let file = project.open_file(&path, 4 * 1024 * 1024)?;
    let meta = file
        .file
        .metadata()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let bytes = file.read_bytes(4 * 1024 * 1024, check)?;
    Ok(FileIdentity {
        relative_path: relative.into(),
        sha256: Some(digest(&bytes)),
        byte_length: meta.len(),
        mode: meta.mode(),
        device: meta.dev(),
        inode: meta.ino(),
        modified: (meta.mtime(), meta.mtime_nsec()),
        changed: (meta.ctime(), meta.ctime_nsec()),
    })
}
/// Snapshot effective configuration without executing configured helpers. Raw
/// values stay local and are replaced by a hash before returning the snapshot.
pub fn snapshot(
    project: &Arc<ProjectDirectory>,
    repository: &str,
    paths: &[String],
    environment: &Environment,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<Snapshot, ErrorCode> {
    use rustix::fs::{openat, Mode, OFlags};
    if !repository.is_empty() {
        validate_relative(repository)?;
    }
    if paths.len() > 64 {
        return Err(ErrorCode::ResourceExhausted);
    }
    let mut sorted = paths.to_vec();
    sorted.sort();
    sorted.dedup();
    if sorted.len() != paths.len() {
        return Err(ErrorCode::RevisionConflict);
    }
    let executable = git_read::trusted_executable()?;
    if executable.is_empty() {
        return Err(ErrorCode::UnsupportedCapability);
    }
    let directory = project.open_directory(repository)?;
    let metadata = directory
        .metadata()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let repo_id = (metadata.dev(), metadata.ino());
    let git_dir = File::from(
        openat(
            &directory,
            ".git",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| ErrorCode::UnsupportedCapability)?,
    );
    let index_revision = metadata_bytes(&git_dir, "index", check)?;
    let head_bytes = metadata_bytes(&git_dir, "HEAD", check)?;
    let observe =
        |query| git_read::execution_preview(project, repository, query, &environment.0, check);
    let head = observe(Observation::Head)?;
    let head = std::str::from_utf8(&head)
        .map_err(|_| ErrorCode::UnsupportedCapability)?
        .trim();
    if !head.is_empty()
        && (!matches!(head.len(), 40 | 64) || !head.bytes().all(|c| c.is_ascii_hexdigit()))
    {
        return Err(ErrorCode::UnsupportedCapability);
    }
    let branch = observe(Observation::SymbolicHead)?;
    let branch = std::str::from_utf8(&branch)
        .map_err(|_| ErrorCode::UnsupportedCapability)?
        .trim();
    if !branch.is_empty()
        && (!branch.starts_with("refs/heads/")
            || branch.len() > 1024
            || branch.chars().any(char::is_control))
    {
        return Err(ErrorCode::UnsupportedCapability);
    }
    let configuration_revision = digest(&observe(Observation::Configuration)?);
    let references_revision = digest(&observe(Observation::References)?);
    let status_revision = digest(&observe(Observation::Status)?);
    let mut files = Vec::new();
    let mut bytes = 0;
    for path in sorted {
        let file = observe_file(project, repository, &path, check)?;
        bytes += file.byte_length;
        if bytes > 32 * 1024 * 1024 {
            return Err(ErrorCode::ResourceExhausted);
        }
        files.push(file);
    }
    if metadata_bytes(&git_dir, "index", check)? != index_revision
        || metadata_bytes(&git_dir, "HEAD", check)? != head_bytes
    {
        return Err(ErrorCode::RevisionConflict);
    }
    project.check()?;
    check()?;
    let current = project
        .open_directory(repository)?
        .metadata()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    if (current.dev(), current.ino()) != repo_id {
        return Err(ErrorCode::RevisionConflict);
    }
    Ok(Snapshot {
        head: (!head.is_empty()).then(|| head.into()),
        branch: (!branch.is_empty()).then(|| branch.into()),
        index_revision,
        configuration_revision,
        references_revision,
        status_revision,
        files,
        repository_identity: repo_id,
        environment_revision: environment.fingerprint(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        process::Command,
        time::{Duration, Instant},
    };
    #[test]
    fn execution_snapshots_bind_user_config_head_index_and_exact_files_without_running_code() {
        let _serial = git_read::TEST_GATE.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        fs::create_dir(&project).unwrap();
        let global = root.path().join("global-config");
        fs::write(&global, "[user]\nname=Fixture Identity\nemail=fixture@example.invalid\n[credential]\nhelper=private-fixture-secret\n").unwrap();
        let environment = Environment(
            [
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("HOME".into(), root.path().as_os_str().into()),
                ("GIT_CONFIG_GLOBAL".into(), global.as_os_str().into()),
                ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
                ("GIT_TERMINAL_PROMPT".into(), "0".into()),
                ("LC_ALL".into(), "C".into()),
            ]
            .into(),
        );
        let git = |args: &[&str]| {
            let output = Command::new(git_read::trusted_executable().unwrap())
                .arg("-C")
                .arg(&project)
                .args(args)
                .env_clear()
                .envs(&environment.0)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            output.stdout
        };
        git(&["init", "-q", "--initial-branch=fixture-main"]);
        fs::write(project.join("a.txt"), "before\n").unwrap();
        let directory = Arc::new(ProjectDirectory::open(&project.canonicalize().unwrap()).unwrap());
        let paths = vec!["a.txt".into()];
        let initial = snapshot(&directory, "", &paths, &environment, &|| Ok(())).unwrap();
        assert_eq!(initial.head, None);
        assert_eq!(initial.branch.as_deref(), Some("refs/heads/fixture-main"));
        assert!(!project.join(".git/index").exists());
        assert_eq!(
            initial,
            snapshot(&directory, "", &paths, &environment, &|| Ok(())).unwrap()
        );
        fs::write(project.join("a.txt"), "after\n").unwrap();
        let content = snapshot(&directory, "", &paths, &environment, &|| Ok(())).unwrap();
        assert_eq!(initial.status_revision, content.status_revision);
        assert_ne!(initial.revision().unwrap(), content.revision().unwrap());
        git(&["add", "--", "a.txt"]);
        let index = snapshot(&directory, "", &paths, &environment, &|| Ok(())).unwrap();
        assert_ne!(content.index_revision, index.index_revision);
        git(&["commit", "-qm", "Fixture snapshot"]);
        let committed = snapshot(&directory, "", &paths, &environment, &|| Ok(())).unwrap();
        assert!(committed.head.is_some());
        assert_ne!(index.references_revision, committed.references_revision);
        fs::write(&global, "[user]\nname=Fixture Changed\nemail=changed@example.invalid\n[credential]\nhelper=private-fixture-secret\n").unwrap();
        let changed = snapshot(&directory, "", &paths, &environment, &|| Ok(())).unwrap();
        assert_ne!(
            committed.configuration_revision,
            changed.configuration_revision
        );
        let body = serde_json::to_string(&changed).unwrap();
        for secret in [
            "private-fixture-secret",
            "Fixture Changed",
            "changed@example.invalid",
            global.to_str().unwrap(),
        ] {
            assert!(!body.contains(secret), "{body}");
        }
        let helper = project.join("helper.sh");
        fs::write(
            &helper,
            format!("#!/bin/sh\ntouch '{}/HELPER_RAN'\n", project.display()),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
        git(&["config", "core.fsmonitor", helper.to_str().unwrap()]);
        snapshot(&directory, "", &paths, &environment, &|| Ok(())).unwrap();
        assert!(!project.join("HELPER_RAN").exists());
        fs::write(project.join(".env"), "secret").unwrap();
        std::os::unix::fs::symlink("a.txt", project.join("linked.txt")).unwrap();
        fs::hard_link(project.join("a.txt"), project.join("hard.txt")).unwrap();
        for path in [".env", "linked.txt", "hard.txt", "../outside"] {
            assert!(
                snapshot(&directory, "", &[path.into()], &environment, &|| Ok(())).is_err(),
                "{path}"
            );
        }
        fs::remove_file(project.join("hard.txt")).unwrap();
        fs::remove_file(&global).unwrap();
        let fifo = std::ffi::CString::new(global.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let start = Instant::now();
        assert_eq!(
            snapshot(&directory, "", &paths, &environment, &|| if start.elapsed()
                >= Duration::from_millis(100)
            {
                Err(ErrorCode::ControlRevoked)
            } else {
                Ok(())
            })
            .unwrap_err(),
            ErrorCode::ControlRevoked
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(!project.join("HELPER_RAN").exists());
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexOperation {
    Stage,
    Unstage,
}
/// Shared argument construction for human Source Control and the approved adapter.
/// This does not authorize or execute the returned arguments.
pub fn index_arguments(paths: &[String], stage: bool, has_head: bool) -> Vec<String> {
    let mut args: Vec<String> = if stage {
        vec!["--literal-pathspecs", "add", "--"]
    } else if has_head {
        vec!["--literal-pathspecs", "restore", "--staged", "--"]
    } else {
        vec!["--literal-pathspecs", "rm", "--cached", "--"]
    }
    .into_iter()
    .map(str::to_string)
    .collect();
    args.extend_from_slice(paths);
    args
}
pub struct IndexPlan {
    project: Arc<ProjectDirectory>,
    repository: String,
    operation: IndexOperation,
    paths: Vec<String>,
    environment: Environment,
    pub observation: Snapshot,
    pub revision: String,
}
impl IndexPlan {
    pub fn prepare(
        project: Arc<ProjectDirectory>,
        repository: String,
        operation: IndexOperation,
        paths: Vec<String>,
        environment: Environment,
        check: &dyn Fn() -> Result<(), ErrorCode>,
    ) -> Result<Self, ErrorCode> {
        if paths.is_empty() {
            return Err(ErrorCode::ResourceExhausted);
        }
        let observation = snapshot(&project, &repository, &paths, &environment, check)?;
        let repeated = snapshot(&project, &repository, &paths, &environment, check)?;
        if observation != repeated {
            return Err(ErrorCode::RevisionConflict);
        }
        let revision = digest(
            &serde_json::to_vec(&(&repository, operation, &observation))
                .map_err(|_| ErrorCode::ResourceExhausted)?,
        );
        Ok(Self {
            project,
            repository,
            operation,
            paths,
            environment,
            observation,
            revision,
        })
    }
}
#[derive(Debug)]
pub struct IndexOutcome {
    pub exit_code: Option<i32>,
    pub interrupted: Option<ErrorCode>,
    pub abandoned_descendants: bool,
    pub after: Option<Snapshot>,
}
/// Only call after the main view approves this exact revision, while retaining
/// the application's shared repository MutationGuard. Before spawn, failure has
/// no mutating effect. After spawn, even a nonzero exit may have changed files.
/// The synchronous owner deliberately retains its child and caller's guard until
/// the process group has a confirmed exit; an unconfirmed kill is never forgotten.
pub fn execute_index(
    plan: IndexPlan,
    approved_revision: &str,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<IndexOutcome, ErrorCode> {
    if plan.revision != approved_revision {
        return Err(ErrorCode::ControlRevoked);
    }
    let current = snapshot(
        &plan.project,
        &plan.repository,
        &plan.paths,
        &plan.environment,
        check,
    )?;
    if current != plan.observation {
        return Err(ErrorCode::RevisionConflict);
    }
    let args = index_arguments(
        &plan.paths,
        matches!(plan.operation, IndexOperation::Stage),
        current.head.is_some(),
    );
    let mut outcome = execute_process(
        &plan.project,
        &plan.repository,
        &plan.environment,
        &args,
        check,
    )?;
    if outcome.interrupted.is_none() {
        outcome.after = snapshot(
            &plan.project,
            &plan.repository,
            &plan.paths,
            &plan.environment,
            check,
        )
        .ok();
        if outcome.after.is_none() {
            outcome.interrupted = Some(ErrorCode::OutcomeUnknown);
        }
    }
    Ok(outcome)
}

// Sealed domain plans are the only callers; arguments never originate at IPC.
fn execute_process(
    project: &ProjectDirectory,
    repository: &str,
    environment: &Environment,
    args: &[String],
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<IndexOutcome, ErrorCode> {
    execute_process_captured(project, repository, environment, args, false, check)
        .map(|(outcome, _)| outcome)
}

// Captured bytes are bounded and remain native; only a domain-specific parser
// may convert them into a receipt. stderr is never returned.
fn execute_process_captured(
    project: &ProjectDirectory,
    repository: &str,
    environment: &Environment,
    args: &[String],
    capture: bool,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<(IndexOutcome, Vec<u8>), ErrorCode> {
    use std::{
        io,
        os::{fd::AsRawFd, unix::process::CommandExt},
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let program = git_read::trusted_executable()?;
    let directory = project.open_directory(repository)?;
    let mut command = Command::new(program);
    command
        .args(["--no-pager", "--git-dir=.git", "--work-tree=."])
        .args(args)
        .env_clear()
        .envs(&environment.0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(move || {
            if libc::fchdir(directory.as_raw_fd()) == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        });
    }
    check()?;
    project.check()?;
    let mut child = crate::process_tree::Tree::spawn(&mut command)
        .map_err(|_| ErrorCode::UnsupportedCapability)?;
    let (Some(mut stdout), Some(mut stderr)) =
        (child.child.stdout.take(), child.child.stderr.take())
    else {
        // A started process is never a before-effect error. Retain ownership
        // even if a future platform implementation fails to provide its pipes.
        loop {
            let _ = child.kill();
            if let Ok(Some(status)) = child.try_wait() {
                return Ok((
                    IndexOutcome {
                        exit_code: status.code(),
                        interrupted: Some(ErrorCode::OutcomeUnknown),
                        abandoned_descendants: child.abandoned_descendants,
                        after: None,
                    },
                    Vec::new(),
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut interrupted = None;
    for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0 {
            interrupted = Some(ErrorCode::OutcomeUnknown);
        }
    }
    let mut captured = Vec::new();
    let mut output = 0;
    let mut errors = 0;
    let mut out_done = false;
    let mut err_done = false;
    let mut exited = None;
    let mut exit_seen = None;
    loop {
        if interrupted.is_none() {
            interrupted = check().err();
        }
        if Instant::now() >= deadline {
            interrupted.get_or_insert(ErrorCode::DeadlineExceeded);
        }
        if interrupted.is_some() {
            let _ = child.kill();
        }
        if exited.is_none() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    exited = Some(status);
                    exit_seen = Some(Instant::now());
                }
                Ok(None) => {}
                Err(_) => {
                    interrupted.get_or_insert(ErrorCode::OutcomeUnknown);
                }
            }
        }
        if child.abandoned_descendants {
            interrupted.get_or_insert(ErrorCode::OutcomeUnknown);
        }
        if interrupted.is_some() {
            if exited.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        for (stream, total, done, limit, keep) in [
            (
                &mut stdout as &mut dyn Read,
                &mut output,
                &mut out_done,
                2 * 1024 * 1024,
                capture,
            ),
            (
                &mut stderr as &mut dyn Read,
                &mut errors,
                &mut err_done,
                16 * 1024,
                false,
            ),
        ] {
            if *done {
                continue;
            }
            let mut bytes = [0; 8192];
            // Yield between bounded batches even if a hook continuously writes.
            for _ in 0..8 {
                match stream.read(&mut bytes) {
                    Ok(0) => {
                        *done = true;
                        break;
                    }
                    Ok(n) => {
                        *total += n;
                        if *total > limit {
                            interrupted.get_or_insert(ErrorCode::ResourceExhausted);
                            break;
                        }
                        if keep {
                            captured.extend_from_slice(&bytes[..n]);
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        interrupted.get_or_insert(ErrorCode::OutcomeUnknown);
                        break;
                    }
                }
            }
        }
        if exited.is_some() && out_done && err_done {
            break;
        }
        if exit_seen.is_some_and(|at: Instant| at.elapsed() > Duration::from_millis(100)) {
            // Detached trusted code can escape a process group; never wait on
            // an inherited pipe forever or claim a fully observed completion.
            interrupted.get_or_insert(ErrorCode::OutcomeUnknown);
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok((
        IndexOutcome {
            exit_code: exited.and_then(|s| s.code()),
            interrupted,
            abandoned_descendants: child.abandoned_descendants,
            after: None,
        },
        captured,
    ))
}

#[cfg(test)]
mod index_tests {
    use super::*;
    use std::{
        fs,
        process::Command,
        time::{Duration, Instant},
    };
    #[test]
    fn approved_index_commands_recheck_exact_bytes_preserve_filters_and_confirm_cancellation() {
        let _serial = git_read::TEST_GATE.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        fs::create_dir(&project).unwrap();
        let environment = Environment(
            [
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("HOME".into(), root.path().as_os_str().into()),
                ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
                ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
                ("LC_ALL".into(), "C".into()),
            ]
            .into(),
        );
        let git = |args: &[&str]| {
            let out = Command::new(git_read::trusted_executable().unwrap())
                .arg("-C")
                .arg(&project)
                .args(args)
                .env_clear()
                .envs(&environment.0)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            out.stdout
        };
        git(&["init", "-q"]);
        fs::write(project.join("plain.txt"), "plain\n").unwrap();
        let directory = Arc::new(ProjectDirectory::open(&project.canonicalize().unwrap()).unwrap());
        let prepare = |name: &str, operation| {
            IndexPlan::prepare(
                directory.clone(),
                "".into(),
                operation,
                vec![name.into()],
                environment.clone(),
                &|| Ok(()),
            )
            .unwrap()
        };
        let stale = prepare("plain.txt", IndexOperation::Stage);
        let revision = stale.revision.clone();
        fs::write(project.join("plain.txt"), "different\n").unwrap();
        assert_eq!(
            execute_index(stale, &revision, &|| Ok(())).unwrap_err(),
            ErrorCode::RevisionConflict
        );
        assert!(!project.join(".git/index").exists());
        let stage = prepare("plain.txt", IndexOperation::Stage);
        let revision = stage.revision.clone();
        let result = execute_index(stage, &revision, &|| Ok(())).unwrap();
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.interrupted, None);
        assert!(result.after.is_some());
        assert_eq!(git(&["show", ":plain.txt"]), b"different\n");
        let unstage = prepare("plain.txt", IndexOperation::Unstage);
        let revision = unstage.revision.clone();
        let result = execute_index(unstage, &revision, &|| Ok(())).unwrap();
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.interrupted, None);
        assert!(git(&["ls-files", "-z"]).is_empty());
        assert_eq!(fs::read(project.join("plain.txt")).unwrap(), b"different\n");
        fs::write(project.join("filtered.txt"), "lowercase\n").unwrap();
        fs::write(
            project.join(".gitattributes"),
            "filtered.txt filter=fixture\n",
        )
        .unwrap();
        let helper = root.path().join("filter.sh");
        let marker = root.path().join("FILTER_RAN");
        fs::write(
            &helper,
            format!(
                "#!/bin/sh\nprintf run >> '{}'\ntr a-z A-Z\n",
                marker.display()
            ),
        )
        .unwrap();
        git(&[
            "config",
            "filter.fixture.clean",
            &format!("/bin/sh '{}'", helper.display()),
        ]);
        let filtered = prepare("filtered.txt", IndexOperation::Stage);
        let revision = filtered.revision.clone();
        assert!(!marker.exists());
        let result = execute_index(filtered, &revision, &|| Ok(())).unwrap();
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(git(&["show", ":filtered.txt"]), b"LOWERCASE\n");
        assert!(marker.exists());
        // A configured filter is preserved during execution, not silently disabled.
        // Snapshotting an untracked new file must never execute that filter.
        fs::write(project.join("wait.txt"), "waiting\n").unwrap();
        fs::write(project.join(".gitattributes"), "wait.txt filter=wait\n").unwrap();
        let wait_script = root.path().join("wait.sh");
        let pid_file = root.path().join("child.pid");
        let started = root.path().join("STARTED");
        fs::write(
            &wait_script,
            format!(
                "#!/bin/sh\nsleep 30 &\nprintf '%s' $! > '{}'\nprintf started > '{}'\nwait\ncat\n",
                pid_file.display(),
                started.display()
            ),
        )
        .unwrap();
        git(&[
            "config",
            "filter.wait.clean",
            &format!("/bin/sh '{}'", wait_script.display()),
        ]);
        let waiting = prepare("wait.txt", IndexOperation::Stage);
        let revision = waiting.revision.clone();
        assert!(!started.exists());
        let start = Instant::now();
        let result = execute_index(waiting, &revision, &|| {
            if started.exists() {
                Err(ErrorCode::ControlRevoked)
            } else {
                Ok(())
            }
        })
        .unwrap();
        assert_eq!(result.interrupted, Some(ErrorCode::ControlRevoked));
        assert!(start.elapsed() < Duration::from_secs(3));
        let pid: i32 = fs::read_to_string(pid_file).unwrap().parse().unwrap();
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let length = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                std::mem::size_of_val(&info) as i32,
            )
        };
        assert!(
            length == 0 || info.pbi_status == 5,
            "Cancelled filter left its owned child running"
        );
    }
}
