//! Approved fetch followed by integration of the exact observed commit. A
//! changed remote or a real rebase conflict is retained as a partial outcome.
use super::*;
pub use lomi_control_protocol::git::{
    GitPullApproval as Preview, GitPullMode as Mode, GitPullOutcome as Status,
    GitPullTarget as Target,
};
use std::collections::BTreeSet;

pub struct Plan {
    project: Arc<ProjectDirectory>,
    repository: String,
    environment: Environment,
    fetch: fetch::Plan,
    references: BTreeMap<String, String>,
    pub observation: Snapshot,
    pub preview: Preview,
    pub revision: String,
}
impl Plan {
    pub fn prepare(
        project: Arc<ProjectDirectory>,
        repository: String,
        target: Target,
        environment: Environment,
        check: &dyn Fn() -> Result<(), ErrorCode>,
    ) -> Result<Self, ErrorCode> {
        if !push::oid(&target.source_commit) || !push::oid(&target.expected_remote_commit) {
            return Err(ErrorCode::ScopeDenied);
        }
        commit::idle(&project, &repository)?;
        push::full_history(&project, &repository)?;
        let fetch = fetch::Plan::prepare(
            project.clone(),
            repository.clone(),
            &target.remote,
            &target.reference,
            environment.clone(),
            check,
        )?;
        let read = |query: Observation<'_>| {
            git_read::execution_preview(&project, &repository, query, &environment.0, check)
        };
        if fetch.observation.head.as_ref() != Some(&target.source_commit)
            || fetch.observation.branch.is_none()
            || !read(Observation::TrackedStatus)?.is_empty()
        {
            return Err(ErrorCode::RevisionConflict);
        }
        let refs = fetch::references(&read(Observation::References)?)?;
        let base = read(Observation::MergeBase {
            ancestor: &target.source_commit,
            descendant: &target.expected_remote_commit,
        })?;
        let base = std::str::from_utf8(&base)
            .map_err(|_| ErrorCode::UnsupportedCapability)?
            .trim();
        if !push::oid(base) || (target.mode == Mode::FfOnly && base != target.source_commit) {
            return Err(ErrorCode::RevisionConflict);
        }
        let mut paths = BTreeSet::new();
        let mut objects = BTreeSet::new();
        let mut add_changes = |from: &str, to: &str| -> Result<(), ErrorCode> {
            let raw = read(Observation::TreeChanges { from, to })?;
            if raw.is_empty() {
                return Ok(());
            }
            for change in commit::parse_changes(&raw)? {
                paths.insert(change.relative_path);
                for object in [change.old_object, change.new_object] {
                    if !object.bytes().all(|b| b == b'0') {
                        objects.insert(object);
                    }
                }
            }
            if paths.len() > 64 || objects.len() > 256 {
                return Err(ErrorCode::ResourceExhausted);
            }
            Ok(())
        };
        // The initial checkout in rebase can touch every difference between
        // current HEAD and upstream, including locally added/deleted paths.
        add_changes(&target.source_commit, &target.expected_remote_commit)?;
        let mut replay_commits = Vec::new();
        if target.mode == Mode::Rebase {
            let raw = read(Observation::ReplayCommits {
                head: &target.source_commit,
                upstream: &target.expected_remote_commit,
            })?;
            for line in std::str::from_utf8(&raw)
                .map_err(|_| ErrorCode::UnsupportedCapability)?
                .lines()
            {
                let parts: Vec<_> = line.split(' ').collect();
                let [head, parent] = parts.as_slice() else {
                    // A linear rebase is qualified here; merge-preserving and
                    // root/unrelated history require their own explicit flow.
                    return Err(ErrorCode::UnsupportedCapability);
                };
                if !push::oid(head) || !push::oid(parent) {
                    return Err(ErrorCode::UnsupportedCapability);
                }
                replay_commits.push((*head).to_string());
                if replay_commits.len() > 64 {
                    return Err(ErrorCode::ResourceExhausted);
                }
                add_changes(parent, head)?;
            }
        }
        let mut bytes = 0_u64;
        for object in objects {
            let raw = read(Observation::ObjectSize { object: &object })?;
            let size = std::str::from_utf8(&raw)
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .ok_or(ErrorCode::UnsupportedCapability)?;
            bytes = bytes
                .checked_add(size)
                .ok_or(ErrorCode::ResourceExhausted)?;
            if size > 4 * 1024 * 1024 || bytes > 32 * 1024 * 1024 {
                return Err(ErrorCode::ResourceExhausted);
            }
        }
        let paths: Vec<_> = paths.into_iter().collect();
        let observation = snapshot(&project, &repository, &paths, &environment, check)?;
        let mut without_files = observation.clone();
        without_files.files.clear();
        if without_files != fetch.observation {
            return Err(ErrorCode::RevisionConflict);
        }
        for file in &observation.files {
            let raw = read(Observation::IndexFlags {
                relative: &file.relative_path,
            })?;
            if raw.is_empty() {
                if file.sha256.is_some() {
                    return Err(ErrorCode::RevisionConflict);
                }
            } else if raw != format!("H {}\0", file.relative_path).as_bytes()
                || file.sha256.is_none()
            {
                // No assume-unchanged/sparse entries, hidden dirty files or
                // ignored/untracked collisions may be overwritten by rebase.
                return Err(ErrorCode::ScopeDenied);
            }
        }
        if snapshot(&project, &repository, &paths, &environment, check)? != observation
            || !read(Observation::TrackedStatus)?.is_empty()
        {
            return Err(ErrorCode::RevisionConflict);
        }
        commit::idle(&project, &repository)?;
        push::full_history(&project, &repository)?;
        let preview = Preview {
            target,
            network: fetch.preview.clone(),
            replay_commits,
            affected_paths: paths,
        };
        let revision = digest(
            &serde_json::to_vec(&(&observation, &preview, &fetch.revision))
                .map_err(|_| ErrorCode::ResourceExhausted)?,
        );
        Ok(Self {
            project,
            repository,
            environment,
            fetch,
            references: refs,
            observation,
            preview,
            revision,
        })
    }
}

pub struct Outcome {
    pub execution: IndexOutcome,
    pub fetched_commit: Option<String>,
    pub status: Option<Status>,
}
fn rebase_conflict(
    project: &ProjectDirectory,
    repository: &str,
    preview: &Preview,
    branch: &str,
    index: &[u8],
) -> bool {
    use rustix::fs::{openat, Mode as FileMode, OFlags};
    let result = || -> Result<bool, ErrorCode> {
        let directory = project.open_directory(repository)?;
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let git = openat(&directory, ".git", flags, FileMode::empty())
            .map_err(|_| ErrorCode::ScopeDenied)?;
        let state = openat(&git, "rebase-merge", flags, FileMode::empty())
            .map_err(|_| ErrorCode::ScopeDenied)?;
        for (name, expected) in [
            ("orig-head", preview.target.source_commit.as_str()),
            ("onto", preview.target.expected_remote_commit.as_str()),
            ("head-name", branch),
        ] {
            let mut file = File::from(
                openat(
                    &state,
                    name,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                    FileMode::empty(),
                )
                .map_err(|_| ErrorCode::ScopeDenied)?,
            );
            let meta = file.metadata().map_err(|_| ErrorCode::ScopeDenied)?;
            if !meta.is_file() || meta.nlink() != 1 || meta.len() > 2048 {
                return Ok(false);
            }
            let mut value = String::new();
            file.by_ref()
                .take(2049)
                .read_to_string(&mut value)
                .map_err(|_| ErrorCode::ScopeDenied)?;
            if value.trim() != expected {
                return Ok(false);
            }
        }
        Ok(index.split(|b| *b == 0).any(|entry| {
            entry.split(|b| *b == b'\t').next().is_some_and(|header| {
                header.ends_with(b" 1") || header.ends_with(b" 2") || header.ends_with(b" 3")
            })
        }))
    };
    result().unwrap_or(false)
}

pub fn execute(
    plan: Plan,
    approved_revision: &str,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<Outcome, ErrorCode> {
    if plan.revision != approved_revision {
        return Err(ErrorCode::ControlRevoked);
    }
    let fresh = Plan::prepare(
        plan.project.clone(),
        plan.repository.clone(),
        plan.preview.target.clone(),
        plan.environment.clone(),
        check,
    )?;
    if fresh.revision != plan.revision {
        return Err(ErrorCode::RevisionConflict);
    }
    let fetch_revision = plan.fetch.revision.clone();
    let fetched = fetch::execute(plan.fetch, &fetch_revision, check)?;
    let mut outcome = Outcome {
        execution: fetched.execution,
        fetched_commit: fetched.fetched_commit,
        status: None,
    };
    let integrate = || -> Result<(IndexOutcome, Status), ErrorCode> {
        let commit = outcome
            .fetched_commit
            .as_deref()
            .ok_or(ErrorCode::OutcomeUnknown)?;
        let read = |query: Observation<'_>| {
            git_read::execution_preview(
                &plan.project,
                &plan.repository,
                query,
                &plan.environment.0,
                check,
            )
        };
        let paths = &plan.preview.affected_paths;
        let after_fetch = snapshot(
            &plan.project,
            &plan.repository,
            paths,
            &plan.environment,
            check,
        )?;
        let mut before = plan.observation.clone();
        before
            .references_revision
            .clone_from(&after_fetch.references_revision);
        if before != after_fetch {
            return Err(ErrorCode::OutcomeUnknown);
        }
        if commit != plan.preview.target.expected_remote_commit {
            return Ok((
                IndexOutcome {
                    exit_code: Some(0),
                    interrupted: None,
                    abandoned_descendants: false,
                    after: Some(after_fetch),
                },
                Status::RemoteChanged,
            ));
        }
        commit::idle(&plan.project, &plan.repository)?;
        push::full_history(&plan.project, &plan.repository)?;
        let args: Vec<String> = match plan.preview.target.mode {
            Mode::FfOnly => [
                "merge",
                "--ff-only",
                "--no-autostash",
                "--no-overwrite-ignore",
                "--no-edit",
                "--no-stat",
                "--",
                commit,
            ]
            .map(str::to_string)
            .into(),
            Mode::Rebase => [
                "rebase",
                "--merge",
                "--no-autostash",
                "--no-update-refs",
                "--no-autosquash",
                "--no-rebase-merges",
                "--no-fork-point",
                "--no-rerere-autoupdate",
                "--empty=drop",
                "-Xno-renames",
                "--onto",
                commit,
                commit,
            ]
            .map(str::to_string)
            .into(),
        };
        let mut executed = execute_process(
            &plan.project,
            &plan.repository,
            &plan.environment,
            &args,
            check,
        )?;
        if executed.interrupted.is_some() || executed.abandoned_descendants {
            return Err(ErrorCode::OutcomeUnknown);
        }
        let after = snapshot(
            &plan.project,
            &plan.repository,
            paths,
            &plan.environment,
            check,
        )?;
        if after.configuration_revision != before.configuration_revision {
            return Err(ErrorCode::OutcomeUnknown);
        }
        let branch = before.branch.as_deref().ok_or(ErrorCode::OutcomeUnknown)?;
        let refs = fetch::references(&read(Observation::References)?)?;
        let mut expected_refs = plan.references.clone();
        expected_refs.insert(plan.preview.network.destination.clone(), commit.into());
        let status = if executed.exit_code == Some(0) {
            commit::idle(&plan.project, &plan.repository)?;
            let head = after.head.as_deref().ok_or(ErrorCode::OutcomeUnknown)?;
            if after.branch != before.branch || !read(Observation::TrackedStatus)?.is_empty() {
                return Err(ErrorCode::OutcomeUnknown);
            }
            if plan.preview.target.mode == Mode::FfOnly {
                if head != commit {
                    return Err(ErrorCode::OutcomeUnknown);
                }
            } else {
                let base = read(Observation::MergeBase {
                    ancestor: commit,
                    descendant: head,
                })?;
                if std::str::from_utf8(&base).ok().map(str::trim) != Some(commit) {
                    return Err(ErrorCode::OutcomeUnknown);
                }
            }
            expected_refs.insert(branch.into(), head.into());
            Status::Applied
        } else if plan.preview.target.mode == Mode::Rebase
            && rebase_conflict(
                &plan.project,
                &plan.repository,
                &plan.preview,
                branch,
                &read(Observation::IndexedFiles)?,
            )
        {
            Status::Conflicted
        } else {
            return Err(ErrorCode::OutcomeUnknown);
        };
        if refs != expected_refs
            || snapshot(
                &plan.project,
                &plan.repository,
                paths,
                &plan.environment,
                check,
            )? != after
        {
            return Err(ErrorCode::OutcomeUnknown);
        }
        executed.after = Some(after);
        Ok((executed, status))
    };
    if outcome.execution.exit_code == Some(0)
        && outcome.execution.interrupted.is_none()
        && !outcome.execution.abandoned_descendants
    {
        match integrate() {
            Ok((execution, status)) => {
                outcome.execution = execution;
                outcome.status = Some(status);
            }
            Err(_) => {
                outcome.execution.interrupted = Some(ErrorCode::OutcomeUnknown);
                outcome.execution.after = None;
            }
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path, process::Command};
    #[test]
    fn exact_pull_preserves_remote_races_dirty_files_and_real_rebase_conflicts() {
        let _serial = git_read::TEST_GATE.lock().unwrap();
        for case in [
            "ff",
            "rebase",
            "conflict",
            "remote-changed",
            "ignored-collision",
            "stale-file",
            "dirty",
            "cancel-hook",
        ] {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source");
            let local = root.path().join("local");
            let remote = root.path().join("remote.git");
            fs::create_dir(&source).unwrap();
            let environment = Environment(
                [
                    ("PATH".into(), "/usr/bin:/bin".into()),
                    ("HOME".into(), root.path().as_os_str().into()),
                    ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
                    ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
                    ("GIT_NO_REPLACE_OBJECTS".into(), "1".into()),
                    ("GIT_NO_LAZY_FETCH".into(), "1".into()),
                    ("LC_ALL".into(), "C".into()),
                ]
                .into(),
            );
            let git = |directory: &Path, args: &[&str]| {
                let out = Command::new(git_read::trusted_executable().unwrap())
                    .arg("-C")
                    .arg(directory)
                    .args(args)
                    .env_clear()
                    .envs(&environment.0)
                    .output()
                    .unwrap();
                assert!(
                    out.status.success(),
                    "{case} {args:?}: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
                String::from_utf8(out.stdout).unwrap()
            };
            git(&source, &["init", "-q", "--initial-branch=main"]);
            git(&source, &["config", "user.name", "Fixture"]);
            git(
                &source,
                &["config", "user.email", "fixture@example.invalid"],
            );
            fs::write(source.join("a.txt"), "base\n").unwrap();
            fs::write(source.join(".gitignore"), "new/\n").unwrap();
            git(&source, &["add", "."]);
            git(&source, &["commit", "-qm", "base"]);
            git(
                root.path(),
                &["init", "-q", "--bare", remote.to_str().unwrap()],
            );
            git(
                &source,
                &["remote", "add", "origin", remote.to_str().unwrap()],
            );
            git(&source, &["push", "-q", "origin", "HEAD:refs/heads/main"]);
            git(
                root.path(),
                &[
                    "clone",
                    "-q",
                    "--branch=main",
                    remote.to_str().unwrap(),
                    local.to_str().unwrap(),
                ],
            );
            git(&local, &["config", "user.name", "Fixture"]);
            git(&local, &["config", "user.email", "fixture@example.invalid"]);
            let rebase = matches!(case, "rebase" | "conflict" | "cancel-hook");
            if rebase {
                let name = if case == "conflict" {
                    "a.txt"
                } else {
                    "local.txt"
                };
                fs::write(local.join(name), "local committed work\n").unwrap();
                git(&local, &["add", "--", name]);
                git(&local, &["commit", "-qm", "local change"]);
            }
            fs::write(source.join("a.txt"), "remote committed work\n").unwrap();
            fs::create_dir(source.join("new")).unwrap();
            fs::write(source.join("new/incoming.txt"), "new remote bytes\n").unwrap();
            git(&source, &["add", "-f", "--", "a.txt", "new/incoming.txt"]);
            git(&source, &["commit", "-qm", "remote change"]);
            git(&source, &["push", "-q", "origin", "HEAD:refs/heads/main"]);
            git(&local, &["fetch", "-q", "origin"]);
            if case == "ignored-collision" {
                fs::create_dir(local.join("new")).unwrap();
                fs::write(local.join("new/incoming.txt"), "ignored human content\n").unwrap();
            }
            if case == "dirty" {
                fs::write(local.join("a.txt"), "human work\n").unwrap();
            }
            if case == "cancel-hook" {
                use std::os::unix::fs::PermissionsExt;
                let hook = local.join(".git/hooks/pre-rebase");
                fs::write(
                    &hook,
                    "#!/bin/sh\ntouch HOOK_RAN\nsleep 20\ntouch LATE_EFFECT\n",
                )
                .unwrap();
                fs::set_permissions(hook, fs::Permissions::from_mode(0o700)).unwrap();
            }
            let before = git(&local, &["rev-parse", "HEAD"]).trim().to_string();
            let expected_remote = git(&source, &["rev-parse", "HEAD"]).trim().to_string();
            let project = Arc::new(ProjectDirectory::open(&local.canonicalize().unwrap()).unwrap());
            let target = Target {
                remote: "origin".into(),
                reference: "refs/heads/main".into(),
                source_commit: before.clone(),
                expected_remote_commit: expected_remote.clone(),
                mode: if rebase { Mode::Rebase } else { Mode::FfOnly },
            };
            let plan = Plan::prepare(project, "".into(), target, environment.clone(), &|| Ok(()));
            if matches!(case, "dirty" | "ignored-collision") {
                assert!(matches!(plan, Err(ErrorCode::RevisionConflict)), "{case}");
                assert_eq!(git(&local, &["rev-parse", "HEAD"]).trim(), before);
                continue;
            }
            let plan = plan.unwrap_or_else(|code| panic!("{case}: {code:?}"));
            let revision = plan.revision.clone();
            assert!(!local.join("HOOK_RAN").exists());
            if case == "stale-file" {
                fs::write(local.join("a.txt"), "human change after approval\n").unwrap();
            }
            if case == "remote-changed" {
                fs::write(source.join("a.txt"), "later remote work\n").unwrap();
                git(&source, &["commit", "-qam", "another client"]);
                git(&source, &["push", "-q", "origin", "HEAD:refs/heads/main"]);
            }
            let started = std::time::Instant::now();
            let result = execute(plan, &revision, &|| {
                if case == "cancel-hook" && local.join("HOOK_RAN").exists() {
                    Err(ErrorCode::ControlRevoked)
                } else {
                    Ok(())
                }
            });
            if case == "stale-file" {
                assert!(matches!(result, Err(ErrorCode::RevisionConflict)));
                assert_eq!(
                    fs::read_to_string(local.join("a.txt")).unwrap(),
                    "human change after approval\n"
                );
                continue;
            }
            let result = result.unwrap_or_else(|code| panic!("{case}: {code:?}"));
            let expected_status = match case {
                "remote-changed" => Some(Status::RemoteChanged),
                "conflict" => Some(Status::Conflicted),
                "cancel-hook" => None,
                _ => Some(Status::Applied),
            };
            assert_eq!(
                result.status, expected_status,
                "{case}: {:?}",
                result.execution
            );
            if case == "cancel-hook" {
                assert!(started.elapsed() < std::time::Duration::from_secs(5));
                assert!(result.execution.interrupted.is_some());
                assert!(!local.join("LATE_EFFECT").exists());
            } else {
                assert!(result.execution.after.is_some(), "{case}");
                if case == "remote-changed" {
                    assert_eq!(git(&local, &["rev-parse", "HEAD"]).trim(), before);
                    assert_eq!(fs::read_to_string(local.join("a.txt")).unwrap(), "base\n");
                    assert_ne!(
                        result.fetched_commit.as_deref(),
                        Some(expected_remote.as_str())
                    );
                } else if case == "conflict" {
                    assert!(local.join(".git/rebase-merge").is_dir());
                    assert!(!git(&local, &["ls-files", "--unmerged"]).is_empty());
                    assert!(fs::read_to_string(local.join("a.txt"))
                        .unwrap()
                        .contains("local committed work"));
                    assert_eq!(
                        git(&local, &["rev-parse", "refs/heads/main"]).trim(),
                        before
                    );
                } else {
                    assert_eq!(
                        fs::read_to_string(local.join("a.txt")).unwrap(),
                        "remote committed work\n"
                    );
                    assert_eq!(
                        fs::read_to_string(local.join("new/incoming.txt")).unwrap(),
                        "new remote bytes\n"
                    );
                    assert!(!local.join(".git/rebase-merge").exists());
                    if rebase {
                        assert_eq!(
                            fs::read_to_string(local.join("local.txt")).unwrap(),
                            "local committed work\n"
                        );
                    }
                }
            }
        }
    }
}
