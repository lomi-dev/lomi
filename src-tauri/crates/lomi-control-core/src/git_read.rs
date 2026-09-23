//! Closed, bounded Git observations on the qualified macOS host. This guard is
//! independent of ordinary user Git commands, which retain their configuration.
use crate::project_files::{validate_relative, ProjectDirectory};
use lomi_control_protocol::ErrorCode;
use std::{
    fs,
    io::{self, Read},
    os::{
        fd::AsRawFd,
        unix::{fs::MetadataExt, process::CommandExt},
    },
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

const GIT: &str = "/Library/Developer/CommandLineTools/usr/bin/git";
const PROFILE: &str = r#"(version 1)
(allow default)
(deny network*)
(deny process-fork)
(deny process-exec)
(allow process-exec (literal (param "GIT_EXEC")))
(deny file-write*)
(allow file-write* (literal "/dev/null"))
(deny file-read-data)
(allow file-read-data (subpath (param "PROJECT")) (subpath "/System")
  (subpath "/usr/lib") (subpath "/usr/share") (literal (param "GIT_EXEC"))
  (literal "/dev/null") (literal "/"))
"#;
const EXECUTION_PREVIEW_PROFILE: &str = r#"(version 1)
(allow default)
(deny network*)
(deny process-fork)
(deny process-exec)
(allow process-exec (literal (param "GIT_EXEC")))
(deny file-write*)
(allow file-write* (literal "/dev/null"))
"#;
const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
const ERROR_LIMIT: usize = 16 * 1024;
static ACTIVE: Mutex<()> = Mutex::new(());
#[cfg(test)]
pub(crate) static TEST_GATE: Mutex<()> = Mutex::new(());
static PROGRAM: OnceLock<Result<Identity, ErrorCode>> = OnceLock::new();
type Identity = (u64, u64, u64, i64, i64, i64, i64);
fn identity(file: &fs::Metadata) -> Identity {
    (
        file.dev(),
        file.ino(),
        file.len(),
        file.mtime(),
        file.mtime_nsec(),
        file.ctime(),
        file.ctime_nsec(),
    )
}
fn program() -> Result<Identity, ErrorCode> {
    let approved = *PROGRAM.get_or_init(|| {
        // Never resolve an executable through PATH or an agent-provided argument.
        for path in Path::new(GIT).ancestors() {
            let m = fs::symlink_metadata(path).map_err(|_| ErrorCode::UnsupportedCapability)?;
            if m.is_symlink() || m.uid() != 0 || m.mode() & 0o022 != 0 {
                return Err(ErrorCode::UnsupportedCapability);
            }
        }
        let m = fs::symlink_metadata(GIT).map_err(|_| ErrorCode::UnsupportedCapability)?;
        if !m.is_file() || m.mode() & 0o111 == 0 {
            return Err(ErrorCode::UnsupportedCapability);
        }
        Ok(identity(&m))
    });
    let approved = approved?;
    if identity(&fs::symlink_metadata(GIT).map_err(|_| ErrorCode::UnsupportedCapability)?)
        != approved
    {
        return Err(ErrorCode::RevisionConflict);
    }
    Ok(approved)
}

/// No arbitrary executable, environment, config option, shell or Git argument.
/// The broker must authorize git.read and the pinned project before calling.
pub enum Observation<'a> {
    Version,
    /// Native execution approval only; raw configuration must never enter MCP replies.
    Configuration,
    Head,
    SymbolicHead,
    References,
    /// Native commit approval only, with effective user configuration.
    StagedChanges,
    IndexedFiles,
    TrackedStatus,
    IndexFlags {
        relative: &'a str,
    },
    TreeChanges {
        from: &'a str,
        to: &'a str,
    },
    ReplayCommits {
        head: &'a str,
        upstream: &'a str,
    },
    ObjectSize {
        object: &'a str,
    },
    IndexEntry {
        relative: &'a str,
    },
    CommittedChanges {
        commit: &'a str,
    },
    CommitObject {
        commit: &'a str,
    },
    RemoteUrls {
        name: &'a str,
        push: bool,
    },
    MergeBase {
        ancestor: &'a str,
        descendant: &'a str,
    },
    RemoteFetchSpecs {
        name: &'a str,
    },
    Identity {
        author: bool,
    },
    Status,
    Diff {
        relative: &'a str,
        staged: bool,
    },
    CommitDiff {
        relative: &'a str,
        commit: &'a str,
    },
    CommitFiles {
        commit: &'a str,
    },
    PathStatus {
        relative: &'a str,
    },
    History {
        commit: Option<&'a str>,
        skip: u32,
        limit: u16,
    },
    Commit {
        commit: &'a str,
    },
    Remotes,
}
fn commit_id(value: &str) -> Result<(), ErrorCode> {
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ErrorCode::ScopeDenied);
    }
    Ok(())
}
pub(crate) fn valid_remote_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
        && !name.contains("..")
        && !name.ends_with('.')
        && !name.ends_with(".lock")
}
fn diff_arguments(
    relative: &str,
    staged: bool,
    commit: Option<&str>,
) -> Result<Vec<String>, ErrorCode> {
    validate_relative(relative)?;
    let mut args: Vec<_> = [
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--ignore-submodules=all",
        "--no-renames",
        "--unified=3",
        "--raw",
        "--patch",
        "-z",
        "--no-relative",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        "--output-indicator-new=+",
        "--output-indicator-old=-",
        "--output-indicator-context= ",
    ]
    .map(str::to_string)
    .into();
    if staged {
        args.push("--cached".into());
    }
    if let Some(commit) = commit {
        commit_id(commit)?;
        args[0] = "show".into();
        args.extend(
            [
                "--format=",
                "--no-show-signature",
                "--no-notes",
                "--root",
                "--diff-merges=first-parent",
            ]
            .map(str::to_string),
        );
        args.push(format!("{commit}^{{commit}}"));
    }
    args.extend(["--".into(), relative.to_string()]);
    Ok(args)
}
impl Observation<'_> {
    fn arguments(&self) -> Result<Vec<String>, ErrorCode> {
        let mut args: Vec<String> = match self {
            Self::Version => vec!["--version".into()],
            Self::Configuration => [
                "config",
                "--null",
                "--includes",
                "--show-origin",
                "--show-scope",
                "--list",
            ]
            .map(str::to_string)
            .into(),
            Self::Head => ["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]
                .map(str::to_string)
                .into(),
            Self::SymbolicHead => ["symbolic-ref", "--quiet", "HEAD"]
                .map(str::to_string)
                .into(),
            Self::References => ["for-each-ref", "--format=%(refname)%00%(objectname)"]
                .map(str::to_string)
                .into(),
            Self::StagedChanges => [
                "diff",
                "--cached",
                "--raw",
                "-z",
                "--no-renames",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--abbrev=64",
                "--ignore-submodules=none",
                "--",
            ]
            .map(str::to_string)
            .into(),
            Self::IndexedFiles => ["ls-files", "--stage", "-z"].map(str::to_string).into(),
            Self::TrackedStatus => [
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=no",
                "--ignore-submodules=none",
            ]
            .map(str::to_string)
            .into(),
            Self::IndexFlags { relative } => {
                validate_relative(relative)?;
                ["ls-files", "-v", "-z", "--", relative]
                    .map(str::to_string)
                    .into()
            }
            Self::TreeChanges { from, to } => {
                commit_id(from)?;
                commit_id(to)?;
                [
                    "diff-tree",
                    "--no-commit-id",
                    "-r",
                    "--raw",
                    "-z",
                    "--no-renames",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-color",
                    "--abbrev=64",
                    "--ignore-submodules=none",
                    from,
                    to,
                    "--",
                ]
                .map(str::to_string)
                .into()
            }
            Self::ReplayCommits { head, upstream } => {
                commit_id(head)?;
                commit_id(upstream)?;
                [
                    "rev-list",
                    "--reverse",
                    "--topo-order",
                    "--parents",
                    "--max-count=65",
                    head,
                    "--not",
                    upstream,
                    "--",
                ]
                .map(str::to_string)
                .into()
            }
            Self::ObjectSize { object } => {
                commit_id(object)?;
                ["cat-file", "-s", object].map(str::to_string).into()
            }
            Self::IndexEntry { relative } => {
                validate_relative(relative)?;
                ["ls-files", "--stage", "-z", "--", relative]
                    .map(str::to_string)
                    .into()
            }
            Self::Identity { author } => [
                "var",
                if *author {
                    "GIT_AUTHOR_IDENT"
                } else {
                    "GIT_COMMITTER_IDENT"
                },
            ]
            .map(str::to_string)
            .into(),
            Self::CommittedChanges { commit } => {
                commit_id(commit)?;
                [
                    "diff-tree",
                    "--root",
                    "--no-commit-id",
                    "-r",
                    "--raw",
                    "-z",
                    "--no-renames",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-color",
                    "--abbrev=64",
                    "--ignore-submodules=none",
                    commit,
                    "--",
                ]
                .map(str::to_string)
                .into()
            }
            Self::CommitObject { commit } => {
                commit_id(commit)?;
                ["cat-file", "commit", commit].map(str::to_string).into()
            }
            Self::RemoteUrls { name, push } => {
                if !valid_remote_name(name) {
                    return Err(ErrorCode::ScopeDenied);
                }
                let mut args: Vec<String> =
                    ["remote", "get-url", "--all"].map(str::to_string).into();
                if *push {
                    args.push("--push".into());
                }
                args.extend(["--".into(), name.to_string()]);
                args
            }
            Self::MergeBase {
                ancestor,
                descendant,
            } => {
                commit_id(ancestor)?;
                commit_id(descendant)?;
                ["merge-base", ancestor, descendant]
                    .map(str::to_string)
                    .into()
            }
            Self::RemoteFetchSpecs { name } => {
                if !valid_remote_name(name) {
                    return Err(ErrorCode::ScopeDenied);
                }
                vec![
                    "config".into(),
                    "--get-all".into(),
                    format!("remote.{name}.fetch"),
                ]
            }
            Self::Status => [
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=normal",
                "--ignore-submodules=all",
            ]
            .map(str::to_string)
            .into(),
            Self::Diff { relative, staged } => diff_arguments(relative, *staged, None)?,
            Self::CommitDiff { relative, commit } => diff_arguments(relative, false, Some(commit))?,
            Self::CommitFiles { commit } => {
                commit_id(commit)?;
                let mut args: Vec<String> = [
                    "diff-tree",
                    "--root",
                    "--no-commit-id",
                    "--diff-merges=first-parent",
                    "-r",
                    "--raw",
                    "-z",
                    "--no-renames",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-color",
                    "--ignore-submodules=all",
                ]
                .map(str::to_string)
                .into();
                args.extend([format!("{commit}^{{commit}}"), "--".into()]);
                args
            }
            Self::PathStatus { relative } => {
                validate_relative(relative)?;
                [
                    "status",
                    "--porcelain=v1",
                    "-z",
                    "--untracked-files=all",
                    "--ignore-submodules=all",
                    "--",
                    relative,
                ]
                .map(str::to_string)
                .into()
            }
            Self::History {
                commit,
                skip,
                limit,
            } => {
                if *skip > 10000 || !(1..=100).contains(limit) {
                    return Err(ErrorCode::ResourceExhausted);
                }
                if let Some(commit) = commit {
                    commit_id(commit)?;
                }
                let mut args: Vec<_> = [
                    "log",
                    "--no-show-signature",
                    "--no-color",
                    "--no-decorate",
                    "--no-notes",
                    "--no-use-mailmap",
                    "--encoding=UTF-8",
                    "--no-patch",
                    "--format=%H%x00%P%x00%an%x00%aI%x00%s%x00",
                    "-z",
                ]
                .map(str::to_string)
                .into();
                args.extend([
                    format!("--skip={skip}"),
                    format!("--max-count={limit}"),
                    format!("{}^{{commit}}", commit.unwrap_or("HEAD")),
                    "--".into(),
                ]);
                args
            }
            Self::Commit { commit } => {
                commit_id(commit)?;
                let mut args: Vec<_> = [
                    "show",
                    "--no-show-signature",
                    "--no-color",
                    "--no-decorate",
                    "--no-notes",
                    "--no-use-mailmap",
                    "--encoding=UTF-8",
                    "--no-patch",
                    "--format=%H%x00%P%x00%an%x00%ae%x00%aI%x00%cn%x00%ce%x00%cI%x00%B",
                    "-z",
                ]
                .map(str::to_string)
                .into();
                args.extend([format!("{commit}^{{commit}}"), "--".into()]);
                args
            }
            // Values can contain embedded credentials. These raw native bytes
            // must be sanitized by the broker before any MCP/UI disclosure.
            Self::Remotes => [
                "config",
                "--null",
                "--get-regexp",
                r"^remote\..*\.(url|pushurl)$",
            ]
            .map(str::to_string)
            .into(),
        };
        args.shrink_to_fit();
        Ok(args)
    }
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn drain(mut pipe: impl Read, limit: usize, overflow: &AtomicBool) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let size = pipe.read(&mut buffer)?;
        if size == 0 {
            return Ok(bytes);
        }
        if size > limit.saturating_sub(bytes.len()) {
            overflow.store(true, Ordering::SeqCst);
            return Ok(bytes);
        }
        bytes.extend_from_slice(&buffer[..size]);
    }
}

/// Returned bytes are not a public DTO. In particular remotes and status paths
/// still need domain filtering, parsing and pagination before disclosure.
pub fn read(
    project: &ProjectDirectory,
    repository_relative: &str,
    observation: Observation<'_>,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<Vec<u8>, ErrorCode> {
    read_with_environment(project, repository_relative, observation, None, check)
}
/// Requires a separate git.execute grant before calling. This preview may read
/// effective user configuration outside the project, but cannot run helpers,
/// contact the network or write. Only hashes/sanitized facts may leave native RAM.
pub(crate) fn execution_preview(
    project: &ProjectDirectory,
    repository_relative: &str,
    observation: Observation<'_>,
    environment: &std::collections::BTreeMap<std::ffi::OsString, std::ffi::OsString>,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<Vec<u8>, ErrorCode> {
    read_with_environment(
        project,
        repository_relative,
        observation,
        Some(environment),
        check,
    )
}
fn read_with_environment(
    project: &ProjectDirectory,
    repository_relative: &str,
    observation: Observation<'_>,
    environment: Option<&std::collections::BTreeMap<std::ffi::OsString, std::ffi::OsString>>,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<Vec<u8>, ErrorCode> {
    let _active = ACTIVE.try_lock().map_err(|_| ErrorCode::TargetBusy)?;
    let arguments = observation.arguments()?;
    check()?;
    project.check()?;
    program()?;
    let repository = project.open_directory(repository_relative)?;
    let repository_identity = identity(
        &repository
            .metadata()
            .map_err(|_| ErrorCode::StorageUnavailable)?,
    );
    let metadata = rustix::fs::statat(&repository, ".git", rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|_| ErrorCode::TargetNotFound)?;
    if rustix::fs::FileType::from_raw_mode(metadata.st_mode) != rustix::fs::FileType::Directory {
        // Linked worktrees require a separately qualified metadata-directory grant.
        return Err(ErrorCode::UnsupportedCapability);
    }
    let root = project
        .canonical_path()
        .to_str()
        .ok_or(ErrorCode::ScopeDenied)?;
    let mut command = Command::new("/usr/bin/sandbox-exec");
    command
        .args([
            "-p",
            if environment.is_some() {
                EXECUTION_PREVIEW_PROFILE
            } else {
                PROFILE
            },
        ])
        .arg(format!("-DPROJECT={root}"))
        .arg(format!("-DGIT_EXEC={GIT}"))
        .arg(GIT)
        .args([
            "--no-pager",
            "--no-optional-locks",
            "--literal-pathspecs",
            "--git-dir=.git",
            "--work-tree=.",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.attributesFile=/dev/null",
            "-c",
            "gc.auto=0",
        ])
        .args(arguments)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", root)
        .env("LC_ALL", "C")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(environment) = environment {
        // The captured native environment is part of the exact approval hash.
        // The command's explicit --git-dir/--work-tree always fence discovery.
        command
            .env_clear()
            .envs(environment)
            .env("GIT_OPTIONAL_LOCKS", "0");
    }
    let cwd = repository
        .try_clone()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    // Only async-signal-safe operations run between fork and exec. fchdir pins
    // the actual directory instead of reopening its pathname after authorization.
    unsafe {
        command.pre_exec(move || {
            if libc::fchdir(cwd.as_raw_fd()) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    check()?;
    let mut child = OwnedChild(
        command
            .spawn()
            .map_err(|_| ErrorCode::UnsupportedCapability)?,
    );
    let out = child.0.stdout.take().ok_or(ErrorCode::StorageUnavailable)?;
    let err = child.0.stderr.take().ok_or(ErrorCode::StorageUnavailable)?;
    let overflow = Arc::new(AtomicBool::new(false));
    let deadline = Instant::now() + Duration::from_secs(5);
    let result = thread::scope(|threads| {
        let out = threads.spawn(|| drain(out, OUTPUT_LIMIT, &overflow));
        let err = threads.spawn(|| drain(err, ERROR_LIMIT, &overflow));
        let wait = loop {
            if let Err(code) = check() {
                break Err(code);
            }
            if overflow.load(Ordering::SeqCst) {
                break Err(ErrorCode::ResourceExhausted);
            }
            if Instant::now() >= deadline {
                break Err(ErrorCode::DeadlineExceeded);
            }
            match child.0.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Err(_) => break Err(ErrorCode::StorageUnavailable),
                Ok(None) => thread::sleep(Duration::from_millis(10)),
            }
        };
        if wait.is_err() {
            let _ = child.0.kill();
        }
        let _ = child.0.wait();
        let out = out
            .join()
            .map_err(|_| ErrorCode::StorageUnavailable)?
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        let err = err
            .join()
            .map_err(|_| ErrorCode::StorageUnavailable)?
            .map_err(|_| ErrorCode::StorageUnavailable)?;
        let status = wait?;
        if overflow.load(Ordering::SeqCst) {
            return Err(ErrorCode::ResourceExhausted);
        }
        // A blocked filter may produce exit0 and a raw diff. Never call that a
        // successful Git comparison or return stderr/configuration to clients.
        if !err.is_empty()
            || (!status.success()
                && !(matches!(
                    observation,
                    Observation::Remotes | Observation::Head | Observation::SymbolicHead
                ) && status.code() == Some(1)))
        {
            return Err(ErrorCode::UnsupportedCapability);
        }
        Ok(out)
    });
    check()?;
    project.check()?;
    program()?;
    let current = project.open_directory(repository_relative)?;
    let m = current
        .metadata()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    if (m.dev(), m.ino()) != (repository_identity.0, repository_identity.1) {
        return Err(ErrorCode::RevisionConflict);
    }
    result
}

/// Recheck the root-owned executable immediately before a mutating dispatch.
pub(crate) fn trusted_executable() -> Result<&'static str, ErrorCode> {
    program()?;
    Ok(GIT)
}

pub(crate) fn guarded_patch(raw: &[u8]) -> Result<&str, ErrorCode> {
    if raw.is_empty() {
        return Ok("");
    }
    let split = raw
        .windows(2)
        .position(|b| b == [0, 0])
        .ok_or(ErrorCode::UnsupportedCapability)?;
    let records: Vec<_> = raw[..split].split(|b| *b == 0).collect();
    if records.len() != 2 {
        return Err(ErrorCode::UnsupportedCapability);
    }
    let fields: Vec<_> = std::str::from_utf8(records[0])
        .map_err(|_| ErrorCode::UnsupportedCapability)?
        .split(' ')
        .collect();
    if fields.len() != 5 || !fields[0].starts_with(':') {
        return Err(ErrorCode::UnsupportedCapability);
    }
    for mode in [&fields[0][1..], fields[1]] {
        if !matches!(mode, "000000" | "100644" | "100755") {
            return Err(ErrorCode::ScopeDenied);
        }
    }
    // Raw mode records prevent historical symlink/submodule bodies from being
    // exposed even when their current filesystem entry has already disappeared.
    std::str::from_utf8(&raw[split + 2..]).map_err(|_| ErrorCode::UnsupportedCapability)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn git(root: &Path, args: &[&str]) -> Vec<u8> {
        let output = Command::new(GIT)
            .arg("-C")
            .arg(root)
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_ATTR_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }
    #[test]
    fn guarded_reads_reject_helpers_outside_data_and_blocked_io_without_side_effects() {
        let _serial = TEST_GATE.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        fs::create_dir(&project).unwrap();
        let canonical = project.canonicalize().unwrap();
        git(&canonical, &["init", "-q"]);
        fs::write(project.join("Zażółć.txt"), "original\n").unwrap();
        fs::write(project.join("large.txt"), "base\n").unwrap();
        git(&canonical, &["add", "--", "Zażółć.txt", "large.txt"]);
        git(
            &canonical,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "Fixture history",
            ],
        );
        fs::write(project.join("Zażółć.txt"), "new text\n").unwrap();
        let directory = Arc::new(ProjectDirectory::open(&canonical).unwrap());
        let status = read(&directory, "", Observation::Status, &|| Ok(())).unwrap();
        assert_eq!(String::from_utf8(status).unwrap(), " M Zażółć.txt\0");
        let patch = read(
            &directory,
            "",
            Observation::Diff {
                relative: "Zażółć.txt",
                staged: false,
            },
            &|| Ok(()),
        )
        .unwrap();
        assert!(String::from_utf8(patch).unwrap().contains("+new text"));
        let history = read(
            &directory,
            "",
            Observation::History {
                commit: None,
                skip: 0,
                limit: 10,
            },
            &|| Ok(()),
        )
        .unwrap();
        assert!(String::from_utf8(history)
            .unwrap()
            .contains("Fixture history"));
        assert!(read(
            &directory,
            "",
            Observation::Diff {
                relative: "../outside",
                staged: false
            },
            &|| Ok(())
        )
        .is_err());
        assert!(read(
            &directory,
            "",
            Observation::History {
                commit: Some("--all"),
                skip: 0,
                limit: 1
            },
            &|| Ok(())
        )
        .is_err());
        let blob = String::from_utf8(git(&canonical, &["rev-parse", "HEAD:Zażółć.txt"])).unwrap();
        assert_eq!(
            read(
                &directory,
                "",
                Observation::Commit {
                    commit: blob.trim()
                },
                &|| Ok(())
            )
            .unwrap_err(),
            ErrorCode::UnsupportedCapability
        );
        fs::write(project.join("large.txt"), vec![b'x'; OUTPUT_LIMIT + 65536]).unwrap();
        assert_eq!(
            read(
                &directory,
                "",
                Observation::Diff {
                    relative: "large.txt",
                    staged: false
                },
                &|| Ok(())
            )
            .unwrap_err(),
            ErrorCode::ResourceExhausted
        );
        fs::write(project.join("large.txt"), "base\n").unwrap();
        let helper = project.join("helper.sh");
        fs::write(
            &helper,
            format!(
                "#!/bin/sh\nprintf ran > '{}/HELPER_RAN'\ncat\n",
                canonical.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
        git(
            &canonical,
            &["config", "core.fsmonitor", helper.to_str().unwrap()],
        );
        git(
            &canonical,
            &["config", "diff.external", helper.to_str().unwrap()],
        );
        git(
            &canonical,
            &["config", "diff.fixture.textconv", helper.to_str().unwrap()],
        );
        git(
            &canonical,
            &["config", "filter.fixture.clean", helper.to_str().unwrap()],
        );
        fs::write(
            project.join(".gitattributes"),
            "*.txt filter=fixture diff=fixture\n",
        )
        .unwrap();
        assert_eq!(
            read(
                &directory,
                "",
                Observation::Diff {
                    relative: "Zażółć.txt",
                    staged: false
                },
                &|| Ok(())
            )
            .unwrap_err(),
            ErrorCode::UnsupportedCapability
        );
        assert!(!project.join("HELPER_RAN").exists());
        // Outside config is inaccessible even when referenced from an approved repository.
        let outside = root.path().join("outside-config");
        fs::write(
            &outside,
            "[remote \"fixture\"]\nurl = https://private-fixture.invalid/secret\n",
        )
        .unwrap();
        git(
            &canonical,
            &["config", "include.path", outside.to_str().unwrap()],
        );
        assert_eq!(
            read(&directory, "", Observation::Remotes, &|| Ok(())).unwrap_err(),
            ErrorCode::UnsupportedCapability
        );
        assert!(!project.join("HELPER_RAN").exists());
        git(
            &canonical,
            &["config", "--no-includes", "--unset", "include.path"],
        );
        let original_config = fs::read(project.join(".git/config")).unwrap();
        fs::remove_file(project.join(".git/config")).unwrap();
        let fifo =
            std::ffi::CString::new(project.join(".git/config").as_os_str().as_encoded_bytes())
                .unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let start = Instant::now();
        let cancelled = read(&directory, "", Observation::Status, &|| {
            if start.elapsed() > Duration::from_millis(100) {
                Err(ErrorCode::ControlRevoked)
            } else {
                Ok(())
            }
        });
        assert_eq!(cancelled.unwrap_err(), ErrorCode::ControlRevoked);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "Blocked config read was not interrupted"
        );
        fs::remove_file(project.join(".git/config")).unwrap();
        fs::write(project.join(".git/config"), original_config).unwrap();
        fs::remove_file(project.join(".gitattributes")).unwrap();
        assert!(read(&directory, "", Observation::Status, &|| Ok(())).is_ok());
        assert!(!project.join("HELPER_RAN").exists());
    }
}
