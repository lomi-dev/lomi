//! A single explicit remote branch, with local-only approval preparation.
use super::*;

pub use lomi_control_protocol::git::GitFetchApproval as Preview;
pub struct Plan {
    project: Arc<ProjectDirectory>,
    repository: String,
    environment: Environment,
    references: BTreeMap<String, String>,
    pub observation: Snapshot,
    pub preview: Preview,
    pub revision: String,
}
pub(crate) fn reference(value: &str) -> bool {
    value.starts_with("refs/")
        && value.len() <= 1024
        && !value.ends_with('/')
        && !value.contains("..")
        && !value.contains("@{")
        && !value
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c))
        && value.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && !part.ends_with('.')
                && !part.ends_with(".lock")
        })
}
pub(super) fn references(bytes: &[u8]) -> Result<BTreeMap<String, String>, ErrorCode> {
    if bytes.len() > 512 * 1024 {
        return Err(ErrorCode::ResourceExhausted);
    }
    let mut result = BTreeMap::new();
    for line in std::str::from_utf8(bytes)
        .map_err(|_| ErrorCode::UnsupportedCapability)?
        .lines()
    {
        let (name, value) = line
            .split_once('\0')
            .ok_or(ErrorCode::UnsupportedCapability)?;
        if result.len() >= 4096
            || !reference(name)
            || !matches!(value.len(), 40 | 64)
            || !value.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err(ErrorCode::UnsupportedCapability);
        }
        if result.insert(name.into(), value.into()).is_some() {
            return Err(ErrorCode::UnsupportedCapability);
        }
    }
    Ok(result)
}
pub(super) fn location(raw: &str) -> Result<String, ErrorCode> {
    if raw.is_empty()
        || raw.len() > 4096
        || raw.chars().any(char::is_control)
        || raw.contains("::") && !raw.contains("://")
    {
        return Err(ErrorCode::UnsupportedCapability);
    }
    if raw.starts_with('/') {
        return Ok(raw.into());
    }
    if let Ok(mut url) = url::Url::parse(raw) {
        if !matches!(url.scheme(), "https" | "http" | "ssh" | "git" | "file")
            || (url.scheme() != "file" && url.host_str().is_none())
            || (url.scheme() == "file" && url.host_str().is_some_and(|host| host != "localhost"))
        {
            return Err(ErrorCode::UnsupportedCapability);
        }
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
        return Ok(url.to_string());
    }
    if !raw.contains("://") {
        if let Some((authority, path)) = raw.split_once(':') {
            if !authority.contains(['/', '\\', '?', '#']) && !path.is_empty() {
                let mut url = url::Url::parse(&format!("ssh://{authority}/"))
                    .map_err(|_| ErrorCode::UnsupportedCapability)?;
                if url.host_str().is_none() || url.password().is_some() {
                    return Err(ErrorCode::UnsupportedCapability);
                }
                let _ = url.set_username("");
                return Ok(format!("{}:{path}", url.host_str().unwrap()));
            }
        }
        // Relative local remotes are shown exactly as interpreted from the repo.
        if raw.starts_with('.') && !raw.contains(':') {
            return Ok(raw.into());
        }
    }
    Err(ErrorCode::UnsupportedCapability)
}
impl Plan {
    pub fn prepare(
        project: Arc<ProjectDirectory>,
        repository: String,
        remote: &str,
        branch: &str,
        environment: Environment,
        check: &dyn Fn() -> Result<(), ErrorCode>,
    ) -> Result<Self, ErrorCode> {
        if !git_read::valid_remote_name(remote)
            || !branch.starts_with("refs/heads/")
            || !reference(branch)
        {
            return Err(ErrorCode::ScopeDenied);
        }
        let observation = snapshot(&project, &repository, &[], &environment, check)?;
        let read = |query| {
            git_read::execution_preview(&project, &repository, query, &environment.0, check)
        };
        let urls = read(Observation::RemoteUrls {
            name: remote,
            push: false,
        })?;
        let raw = std::str::from_utf8(&urls)
            .map_err(|_| ErrorCode::UnsupportedCapability)?
            .strip_suffix('\n')
            .ok_or(ErrorCode::UnsupportedCapability)?;
        let location = location(raw)?;
        let refs = references(&read(Observation::References)?)?;
        let destination = format!("refs/remotes/{remote}/{}", &branch["refs/heads/".len()..]);
        if !reference(&destination) {
            return Err(ErrorCode::ScopeDenied);
        }
        let preview = Preview {
            remote: remote.into(),
            reference: branch.into(),
            location,
            previous_commit: refs.get(&destination).cloned(),
            destination,
        };
        if read(Observation::RemoteUrls {
            name: remote,
            push: false,
        })? != urls
            || references(&read(Observation::References)?)? != refs
            || snapshot(&project, &repository, &[], &environment, check)? != observation
        {
            return Err(ErrorCode::RevisionConflict);
        }
        let revision = digest(
            &serde_json::to_vec(&(&repository, &observation, &preview, raw, &refs))
                .map_err(|_| ErrorCode::ResourceExhausted)?,
        );
        Ok(Self {
            project,
            repository,
            environment,
            observation,
            preview,
            references: refs,
            revision,
        })
    }
}
pub struct Outcome {
    pub execution: IndexOutcome,
    pub fetched_commit: Option<String>,
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
        &plan.preview.remote,
        &plan.preview.reference,
        plan.environment.clone(),
        check,
    )?;
    if fresh.revision != plan.revision {
        return Err(ErrorCode::RevisionConflict);
    }
    let mut args: Vec<String> = [
        "fetch",
        "--atomic",
        "--no-tags",
        "--no-recurse-submodules",
        "--no-prune",
        "--no-prune-tags",
        "--no-write-fetch-head",
        "--no-auto-maintenance",
        "--refmap=",
        "--",
    ]
    .map(str::to_string)
    .into();
    args.push(plan.preview.remote.clone());
    args.push(format!(
        "{}:{}",
        plan.preview.reference, plan.preview.destination
    ));
    let mut execution = execute_process(
        &plan.project,
        &plan.repository,
        &plan.environment,
        &args,
        check,
    )?;
    let mut fetched_commit = None;
    if execution.exit_code == Some(0)
        && execution.interrupted.is_none()
        && !execution.abandoned_descendants
    {
        let verify = || -> Result<(Snapshot, String), ErrorCode> {
            let after = snapshot(
                &plan.project,
                &plan.repository,
                &[],
                &plan.environment,
                check,
            )?;
            let mut refs = references(&git_read::execution_preview(
                &plan.project,
                &plan.repository,
                Observation::References,
                &plan.environment.0,
                check,
            )?)?;
            let commit = refs
                .remove(&plan.preview.destination)
                .ok_or(ErrorCode::OutcomeUnknown)?;
            let mut before = plan.references.clone();
            before.remove(&plan.preview.destination);
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
            // Verify the fetched object is an actual commit; its body stays native.
            git_read::execution_preview(
                &plan.project,
                &plan.repository,
                Observation::CommitObject { commit: &commit },
                &plan.environment.0,
                check,
            )?;
            Ok((after, commit))
        };
        match verify() {
            Ok((after, commit)) => {
                execution.after = Some(after);
                fetched_commit = Some(commit);
            }
            Err(_) => execution.interrupted = Some(ErrorCode::OutcomeUnknown),
        }
    }
    Ok(Outcome {
        execution,
        fetched_commit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command};
    #[test]
    fn exact_fetch_is_local_until_approval_and_updates_only_the_selected_tracking_ref() {
        let _serial = git_read::TEST_GATE.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let remote = root.path().join("remote.git");
        let local = root.path().join("local");
        for p in [&source, &remote, &local] {
            fs::create_dir(p).unwrap();
        }
        let environment = Environment(
            [
                ("HOME".into(), root.path().as_os_str().into()),
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
                ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
                ("LC_ALL".into(), "C".into()),
            ]
            .into(),
        );
        let git = |path: &std::path::Path, args: &[&str]| {
            let out = Command::new(git_read::trusted_executable().unwrap())
                .arg("-C")
                .arg(path)
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
        for path in [&source, &local] {
            git(path, &["init", "-q", "--initial-branch=fixture"]);
            git(path, &["config", "user.name", "Fixture Identity"]);
            git(path, &["config", "user.email", "fixture@example.invalid"]);
            fs::write(path.join("a.txt"), format!("{}\n", path.display())).unwrap();
            git(path, &["add", "--", "a.txt"]);
            git(path, &["commit", "-qm", "Private fixture base"]);
            git(path, &["remote", "add", "origin", remote.to_str().unwrap()]);
        }
        git(&source, &["tag", "fixture-tag"]);
        git(
            &source,
            &["push", "-q", "--tags", "origin", "refs/heads/fixture"],
        );
        git(
            &local,
            &[
                "config",
                "remote.origin.fetch",
                "+refs/heads/*:refs/heads/unapproved/*",
            ],
        );
        let head = git(&local, &["rev-parse", "HEAD"]);
        let content = fs::read(local.join("a.txt")).unwrap();
        let project = Arc::new(ProjectDirectory::open(&local.canonicalize().unwrap()).unwrap());
        let prepare = || {
            Plan::prepare(
                project.clone(),
                "".into(),
                "origin",
                "refs/heads/fixture",
                environment.clone(),
                &|| Ok(()),
            )
            .unwrap()
        };
        let pending = prepare();
        assert!(pending.preview.previous_commit.is_none());
        assert_eq!(git(&local, &["for-each-ref", "refs/remotes/"]), "");
        assert!(!local.join(".git/FETCH_HEAD").exists());
        let revision = pending.revision.clone();
        let outcome = execute(pending, &revision, &|| Ok(())).unwrap();
        assert_eq!(outcome.execution.exit_code, Some(0));
        assert!(
            outcome.execution.interrupted.is_none(),
            "{:?}",
            outcome.execution
        );
        assert_eq!(
            outcome.fetched_commit.as_deref(),
            Some(git(&source, &["rev-parse", "HEAD"]).trim())
        );
        assert_eq!(git(&local, &["rev-parse", "HEAD"]), head);
        assert_eq!(git(&local, &["tag"]), "");
        assert_eq!(git(&local, &["for-each-ref", "refs/heads/unapproved/"]), "");
        assert!(!local.join(".git/FETCH_HEAD").exists());
        assert_eq!(fs::read(local.join("a.txt")).unwrap(), content);
        let pending = prepare();
        let revision = pending.revision.clone();
        git(&local, &["config", "remote.origin.tagOpt", "--tags"]);
        assert!(matches!(
            execute(pending, &revision, &|| Ok(())),
            Err(ErrorCode::RevisionConflict)
        ));
        // URL expansion is previewed without running the transport. A real Git
        // child then invokes this owned SSH fixture, whose whole group is stopped.
        git(
            &local,
            &[
                "remote",
                "set-url",
                "origin",
                "ssh://fixture@example.invalid/repo.git",
            ],
        );
        let script = root.path().join("ssh-fixture");
        let marker = root.path().join("SSH_RAN");
        fs::write(
            &script,
            format!("#!/bin/sh\ntouch '{}'\nsleep 20\n", marker.display()),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        let mut env = environment.clone();
        env.0
            .insert("GIT_SSH_COMMAND".into(), script.as_os_str().into());
        env.0.insert("GIT_SSH_VARIANT".into(), "ssh".into());
        let pending = Plan::prepare(
            project,
            "".into(),
            "origin",
            "refs/heads/fixture",
            env,
            &|| Ok(()),
        )
        .unwrap();
        assert!(!marker.exists());
        assert_eq!(pending.preview.location, "ssh://example.invalid/repo.git");
        let revision = pending.revision.clone();
        let started = std::time::Instant::now();
        let outcome = execute(pending, &revision, &|| {
            if marker.exists() {
                Err(ErrorCode::ControlRevoked)
            } else {
                Ok(())
            }
        })
        .unwrap();
        assert_eq!(
            outcome.execution.interrupted,
            Some(ErrorCode::ControlRevoked)
        );
        assert!(outcome.fetched_commit.is_none());
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        assert_eq!(git(&local, &["rev-parse", "HEAD"]), head);
    }
    #[test]
    fn approval_targets_reject_options_custom_helpers_and_ambiguous_refs_and_redact_credentials() {
        for value in ["-origin", "a/b", "origin..x", "origin.lock", "origin\n"] {
            assert!(!git_read::valid_remote_name(value));
        }
        for value in [
            "refs/heads/..",
            "refs/heads/x.lock",
            "refs/heads/x:y",
            "refs/heads/x*",
            "refs/heads/a//b",
            "refs/heads/a@{x}",
        ] {
            assert!(!reference(value));
        }
        for value in [
            "ext::sh -c true",
            "helper://somewhere",
            "https://example.invalid/a\nhttps://other.invalid/b",
        ] {
            assert!(location(value).is_err());
        }
        assert_eq!(
            location("https://name:password@example.invalid/repo.git?token=secret#fragment")
                .unwrap(),
            "https://example.invalid/repo.git"
        );
        assert_eq!(
            location("git@example.invalid:org/repo.git").unwrap(),
            "example.invalid:org/repo.git"
        );
    }
}
