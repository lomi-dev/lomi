//! Exact compare-and-swap push with an independent fast-forward proof.
use super::*;

pub use lomi_control_protocol::git::{GitPushApproval as Preview, GitPushTarget as Target};
pub struct Plan {
    project: Arc<ProjectDirectory>,
    repository: String,
    environment: Environment,
    references: BTreeMap<String, String>,
    pub observation: Snapshot,
    pub preview: Preview,
    pub revision: String,
}
pub(crate) fn oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|c| c.is_ascii_hexdigit())
}
pub(super) fn full_history(project: &ProjectDirectory, repository: &str) -> Result<(), ErrorCode> {
    use rustix::fs::{openat, statat, AtFlags, Mode, OFlags};
    let repository = project.open_directory(repository)?;
    let git = openat(
        repository,
        ".git",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| ErrorCode::ScopeDenied)?;
    let absent = |dir, name| match statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        _ => Err(ErrorCode::UnsupportedCapability),
    };
    absent(&git, "shallow")?;
    match openat(
        &git,
        "info",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(info) => absent(&info, "grafts"),
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Err(_) => Err(ErrorCode::ScopeDenied),
    }
}
impl Plan {
    pub fn prepare(
        project: Arc<ProjectDirectory>,
        repository: String,
        target: Target,
        environment: Environment,
        check: &dyn Fn() -> Result<(), ErrorCode>,
    ) -> Result<Self, ErrorCode> {
        if !git_read::valid_remote_name(&target.remote)
            || !target.reference.starts_with("refs/heads/")
            || !fetch::reference(&target.reference)
            || !oid(&target.source_commit)
            || target
                .expected_remote_commit
                .as_deref()
                .is_some_and(|v| !oid(v))
        {
            return Err(ErrorCode::ScopeDenied);
        }
        full_history(&project, &repository)?;
        let observation = snapshot(&project, &repository, &[], &environment, check)?;
        if observation.head.as_ref() != Some(&target.source_commit) {
            return Err(ErrorCode::RevisionConflict);
        }
        let read = |query| {
            git_read::execution_preview(&project, &repository, query, &environment.0, check)
        };
        let urls = read(Observation::RemoteUrls {
            name: &target.remote,
            push: true,
        })?;
        let raw = std::str::from_utf8(&urls)
            .map_err(|_| ErrorCode::UnsupportedCapability)?
            .strip_suffix('\n')
            .ok_or(ErrorCode::UnsupportedCapability)?;
        let location = fetch::location(raw)?;
        let specs = read(Observation::RemoteFetchSpecs {
            name: &target.remote,
        })?;
        let specs = std::str::from_utf8(&specs).map_err(|_| ErrorCode::UnsupportedCapability)?;
        let mapping = format!("refs/heads/*:refs/remotes/{}/*", target.remote);
        if specs.lines().count() != 1 || specs.trim_end().trim_start_matches('+') != mapping {
            return Err(ErrorCode::UnsupportedCapability);
        }
        if let Some(expected) = &target.expected_remote_commit {
            let base = read(Observation::MergeBase {
                ancestor: expected,
                descendant: &target.source_commit,
            })?;
            if std::str::from_utf8(&base)
                .map_err(|_| ErrorCode::UnsupportedCapability)?
                .trim()
                != expected
            {
                return Err(ErrorCode::RevisionConflict);
            }
        }
        let refs = fetch::references(&read(Observation::References)?)?;
        full_history(&project, &repository)?;
        if read(Observation::RemoteUrls {
            name: &target.remote,
            push: true,
        })? != urls
            || fetch::references(&read(Observation::References)?)? != refs
            || snapshot(&project, &repository, &[], &environment, check)? != observation
        {
            return Err(ErrorCode::RevisionConflict);
        }
        let preview = Preview { target, location };
        let revision = digest(
            &serde_json::to_vec(&(&repository, &observation, &preview, raw, &refs))
                .map_err(|_| ErrorCode::ResourceExhausted)?,
        );
        Ok(Self {
            project,
            repository,
            environment,
            references: refs,
            observation,
            preview,
            revision,
        })
    }
}
fn acknowledged(bytes: &[u8], target: &Target) -> Result<bool, ErrorCode> {
    let text = std::str::from_utf8(bytes).map_err(|_| ErrorCode::OutcomeUnknown)?;
    let mut status = None;
    for line in text.lines().filter(|line| line.contains('\t')) {
        let mut fields = line.split('\t');
        let flag = fields.next().ok_or(ErrorCode::OutcomeUnknown)?;
        let update = fields.next().ok_or(ErrorCode::OutcomeUnknown)?;
        if fields.next().is_none()
            || fields.next().is_some()
            || !matches!(flag, " " | "*" | "=")
            || update != format!("{}:{}", target.source_commit, target.reference)
            || status.is_some()
        {
            return Err(ErrorCode::OutcomeUnknown);
        }
        if (flag == "*" && target.expected_remote_commit.is_some())
            || (flag == " " && target.expected_remote_commit.is_none())
            || (flag == "="
                && target.expected_remote_commit.as_ref() != Some(&target.source_commit))
        {
            return Err(ErrorCode::OutcomeUnknown);
        }
        status = Some(flag != "=");
    }
    status.ok_or(ErrorCode::OutcomeUnknown)
}
pub struct Outcome {
    pub execution: IndexOutcome,
    pub changed: Option<bool>,
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
    let target = &plan.preview.target;
    let mut args: Vec<String> = [
        "push",
        "--porcelain",
        "--atomic",
        "--no-mirror",
        "--no-follow-tags",
        "--recurse-submodules=no",
    ]
    .map(str::to_string)
    .into();
    // The independent ancestry proof above rules out history rewrites. The
    // explicit expected value prevents a concurrent server change from passing.
    args.extend([
        format!(
            "--force-with-lease={}:{}",
            target.reference,
            target.expected_remote_commit.as_deref().unwrap_or("")
        ),
        "--".into(),
        target.remote.clone(),
        format!("{}:{}", target.source_commit, target.reference),
    ]);
    let (mut execution, stdout) = execute_process_captured(
        &plan.project,
        &plan.repository,
        &plan.environment,
        &args,
        true,
        check,
    )?;
    let mut changed = None;
    if execution.exit_code == Some(0)
        && execution.interrupted.is_none()
        && !execution.abandoned_descendants
    {
        let verify = || -> Result<(Snapshot, bool), ErrorCode> {
            let changed = acknowledged(&stdout, target)?;
            let after = snapshot(
                &plan.project,
                &plan.repository,
                &[],
                &plan.environment,
                check,
            )?;
            let mut refs = fetch::references(&git_read::execution_preview(
                &plan.project,
                &plan.repository,
                Observation::References,
                &plan.environment.0,
                check,
            )?)?;
            let tracking = format!(
                "refs/remotes/{}/{}",
                target.remote,
                &target.reference["refs/heads/".len()..]
            );
            if refs
                .remove(&tracking)
                .is_some_and(|value| value != target.source_commit)
            {
                return Err(ErrorCode::OutcomeUnknown);
            }
            let mut before = plan.references.clone();
            before.remove(&tracking);
            if before != refs
                || after.head != plan.observation.head
                || after.branch != plan.observation.branch
                || after.index_revision != plan.observation.index_revision
                || after.status_revision != plan.observation.status_revision
                || after.configuration_revision != plan.observation.configuration_revision
            {
                return Err(ErrorCode::OutcomeUnknown);
            }
            if snapshot(
                &plan.project,
                &plan.repository,
                &[],
                &plan.environment,
                check,
            )? != after
            {
                return Err(ErrorCode::OutcomeUnknown);
            }
            Ok((after, changed))
        };
        match verify() {
            Ok((after, value)) => {
                execution.after = Some(after);
                changed = Some(value);
            }
            Err(_) => execution.interrupted = Some(ErrorCode::OutcomeUnknown),
        }
    }
    Ok(Outcome { execution, changed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command};
    #[test]
    fn push_creates_or_fast_forwards_exact_ref_and_never_overwrites_a_server_race() {
        let _serial = git_read::TEST_GATE.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        let local = root.path().join("local");
        let remote = root.path().join("remote.git");
        for dir in [&local, &remote] {
            fs::create_dir(dir).unwrap();
        }
        let environment = Environment(
            [
                ("HOME".into(), root.path().as_os_str().into()),
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
                ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
                ("GIT_NO_REPLACE_OBJECTS".into(), "1".into()),
                ("GIT_NO_LAZY_FETCH".into(), "1".into()),
                ("LC_ALL".into(), "C".into()),
            ]
            .into(),
        );
        let git = |dir: &std::path::Path, args: &[&str]| {
            let out = Command::new(git_read::trusted_executable().unwrap())
                .arg("-C")
                .arg(dir)
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
            String::from_utf8(out.stdout).unwrap()
        };
        git(&remote, &["init", "-q", "--bare"]);
        git(&local, &["init", "-q", "--initial-branch=fixture"]);
        git(&local, &["config", "user.name", "Push Fixture"]);
        git(&local, &["config", "user.email", "fixture@example.invalid"]);
        fs::write(local.join("a.txt"), "base\n").unwrap();
        git(&local, &["add", "--", "a.txt"]);
        git(&local, &["commit", "-qm", "Fixture base"]);
        git(
            &local,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(&local, &["config", "push.followTags", "true"]);
        git(&local, &["tag", "-am", "Fixture tag", "private-tag"]);
        let base = git(&local, &["rev-parse", "HEAD"]).trim().to_string();
        let project = Arc::new(ProjectDirectory::open(&local.canonicalize().unwrap()).unwrap());
        let prepare = |source: &str, expected: Option<&str>, branch: &str| {
            Plan::prepare(
                project.clone(),
                "".into(),
                Target {
                    remote: "origin".into(),
                    reference: branch.into(),
                    source_commit: source.into(),
                    expected_remote_commit: expected.map(str::to_string),
                },
                environment.clone(),
                &|| Ok(()),
            )
        };
        let run = |plan: Plan| {
            let revision = plan.revision.clone();
            execute(plan, &revision, &|| Ok(())).unwrap()
        };
        let plan = prepare(&base, None, "refs/heads/fixture").unwrap();
        assert!(git(&remote, &["for-each-ref"]).is_empty());
        let result = run(plan);
        assert_eq!(result.execution.exit_code, Some(0));
        assert!(
            result.execution.interrupted.is_none(),
            "{:?}",
            result.execution
        );
        assert_eq!(result.changed, Some(true));
        assert_eq!(
            git(&remote, &["rev-parse", "refs/heads/fixture"]).trim(),
            base
        );
        assert!(git(&remote, &["tag"]).is_empty());
        // Git can report up-to-date before testing a lease. Do not turn that
        // into confirmation of a contradictory expected remote value.
        let result = run(prepare(&base, None, "refs/heads/fixture").unwrap());
        assert_eq!(result.changed, None);
        assert_eq!(
            result.execution.interrupted,
            Some(ErrorCode::OutcomeUnknown)
        );
        let result = run(prepare(&base, Some(&base), "refs/heads/fixture").unwrap());
        assert_eq!(result.changed, Some(false));
        fs::write(local.join("a.txt"), "local advance\n").unwrap();
        git(&local, &["add", "--", "a.txt"]);
        git(&local, &["commit", "-qm", "Fixture advance"]);
        let next = git(&local, &["rev-parse", "HEAD"]).trim().to_string();
        let plan = prepare(&next, Some(&base), "refs/heads/fixture").unwrap();
        let other = root.path().join("other");
        git(
            root.path(),
            &[
                "clone",
                "-q",
                "--branch",
                "fixture",
                remote.to_str().unwrap(),
                other.to_str().unwrap(),
            ],
        );
        git(&other, &["config", "user.name", "Other Fixture"]);
        git(&other, &["config", "user.email", "other@example.invalid"]);
        fs::write(other.join("b.txt"), "concurrent remote work\n").unwrap();
        git(&other, &["add", "--", "b.txt"]);
        git(&other, &["commit", "-qm", "Concurrent fixture advance"]);
        git(&other, &["push", "-q", "origin", "refs/heads/fixture"]);
        let competing = git(&other, &["rev-parse", "HEAD"]).trim().to_string();
        let result = run(plan);
        assert_ne!(result.execution.exit_code, Some(0));
        assert_eq!(result.changed, None);
        assert_eq!(
            git(&remote, &["rev-parse", "refs/heads/fixture"]).trim(),
            competing
        );
        git(&local, &["fetch", "-q", "origin"]);
        assert!(matches!(
            prepare(&next, Some(&competing), "refs/heads/fixture"),
            Err(ErrorCode::RevisionConflict)
        ));
        // Reset only this owned fixture's remote to exercise a valid advance.
        git(
            &remote,
            &["update-ref", "refs/heads/fixture", &base, &competing],
        );
        let result = run(prepare(&next, Some(&base), "refs/heads/fixture").unwrap());
        assert_eq!(result.changed, Some(true));
        assert_eq!(
            git(&remote, &["rev-parse", "refs/heads/fixture"]).trim(),
            next
        );
        let stale = prepare(&next, Some(&next), "refs/heads/fixture").unwrap();
        git(&local, &["config", "push.followTags", "false"]);
        let revision = stale.revision.clone();
        assert!(matches!(
            execute(stale, &revision, &|| Ok(())),
            Err(ErrorCode::RevisionConflict)
        ));
        fs::write(local.join(".git/info/grafts"), "").unwrap();
        assert!(matches!(
            prepare(&next, Some(&next), "refs/heads/fixture"),
            Err(ErrorCode::UnsupportedCapability)
        ));
        fs::remove_file(local.join(".git/info/grafts")).unwrap();
        let marker = root.path().join("PRE_PUSH");
        let hook = local.join(".git/hooks/pre-push");
        fs::write(
            &hook,
            format!("#!/bin/sh\ntouch '{}'\nsleep 20\n", marker.display()),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(hook, fs::Permissions::from_mode(0o700)).unwrap();
        let plan = prepare(&next, None, "refs/heads/cancelled").unwrap();
        assert!(!marker.exists());
        let revision = plan.revision.clone();
        let start = std::time::Instant::now();
        let result = execute(plan, &revision, &|| {
            if marker.exists() {
                Err(ErrorCode::ControlRevoked)
            } else {
                Ok(())
            }
        })
        .unwrap();
        assert_eq!(
            result.execution.interrupted,
            Some(ErrorCode::ControlRevoked)
        );
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        assert!(git(&remote, &["for-each-ref", "refs/heads/cancelled"]).is_empty());
    }
}
