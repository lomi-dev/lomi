//! Exact staged-state approval and verified native commit execution. Preparation
//! does not run hooks; approved execution preserves the user's configuration.
use super::*;

pub use lomi_control_protocol::git::{
    GitCommitIdentity as Identity, GitStagedChange as StagedChange,
};
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitPreview {
    pub observation: Snapshot,
    pub changes: Vec<StagedChange>,
    pub author: Identity,
    pub committer: Identity,
    pub stored_message: String,
    pub final_newline_added: bool,
    pub revision: String,
}

pub struct Plan {
    project: Arc<ProjectDirectory>,
    repository: String,
    paths: Vec<String>,
    environment: Environment,
    pub preview: CommitPreview,
}
impl Plan {
    pub fn prepare(
        project: Arc<ProjectDirectory>,
        repository: String,
        paths: Vec<String>,
        input_message: &str,
        environment: Environment,
        check: &dyn Fn() -> Result<(), ErrorCode>,
    ) -> Result<Self, ErrorCode> {
        let preview = prepare(
            &project,
            &repository,
            &paths,
            input_message,
            &environment,
            check,
        )?;
        Ok(Self {
            project,
            repository,
            paths,
            environment,
            preview,
        })
    }
}

fn verify_object(bytes: &[u8], preview: &CommitPreview) -> Result<(), ErrorCode> {
    let body = std::str::from_utf8(bytes).map_err(|_| ErrorCode::OutcomeUnknown)?;
    let (headers, message) = body.split_once("\n\n").ok_or(ErrorCode::OutcomeUnknown)?;
    if message != preview.stored_message {
        return Err(ErrorCode::OutcomeUnknown);
    }
    let mut parents = Vec::new();
    let mut author = None;
    let mut committer = None;
    let mut tree = None;
    for line in headers.lines() {
        if let Some(value) = line.strip_prefix("parent ") {
            if !object(value) {
                return Err(ErrorCode::OutcomeUnknown);
            }
            parents.push(value);
        } else if let Some(value) = line.strip_prefix("tree ") {
            if tree.replace(value).is_some() || !object(value) {
                return Err(ErrorCode::OutcomeUnknown);
            }
        } else if let Some(value) = line.strip_prefix("author ") {
            if author
                .replace(identity(format!("{value}\n").as_bytes())?)
                .is_some()
            {
                return Err(ErrorCode::OutcomeUnknown);
            }
        } else if let Some(value) = line.strip_prefix("committer ") {
            if committer
                .replace(identity(format!("{value}\n").as_bytes())?)
                .is_some()
            {
                return Err(ErrorCode::OutcomeUnknown);
            }
        }
    }
    let expected: Vec<_> = preview.observation.head.as_deref().into_iter().collect();
    if tree.is_none()
        || parents != expected
        || author.as_ref() != Some(&preview.author)
        || committer.as_ref() != Some(&preview.committer)
    {
        return Err(ErrorCode::OutcomeUnknown);
    }
    Ok(())
}

/// The caller retains the shared repository lock and a durable running receipt.
/// Every error after spawn remains an outcome with uncertain effects.
pub fn execute(
    plan: Plan,
    approved_revision: &str,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<IndexOutcome, ErrorCode> {
    if plan.preview.revision != approved_revision {
        return Err(ErrorCode::ControlRevoked);
    }
    let current = prepare(
        &plan.project,
        &plan.repository,
        &plan.paths,
        &plan.preview.stored_message,
        &plan.environment,
        check,
    )?;
    // The first preview records whether an LF was supplied by Git; the stored
    // message and its bound revision are identical during the final recheck.
    if current.revision != plan.preview.revision {
        return Err(ErrorCode::RevisionConflict);
    }
    let args = vec![
        "commit".into(),
        "--cleanup=verbatim".into(),
        "-m".into(),
        plan.preview.stored_message.clone(),
    ];
    let mut outcome = execute_process(
        &plan.project,
        &plan.repository,
        &plan.environment,
        &args,
        check,
    )?;
    if outcome.exit_code == Some(0)
        && outcome.interrupted.is_none()
        && !outcome.abandoned_descendants
    {
        let verify = || -> Result<Snapshot, ErrorCode> {
            let after = snapshot(
                &plan.project,
                &plan.repository,
                &plan.paths,
                &plan.environment,
                check,
            )?;
            let head = after.head.as_deref().ok_or(ErrorCode::OutcomeUnknown)?;
            if after.head == plan.preview.observation.head
                || after.branch != plan.preview.observation.branch
            {
                return Err(ErrorCode::OutcomeUnknown);
            }
            let read = |query| {
                git_read::execution_preview(
                    &plan.project,
                    &plan.repository,
                    query,
                    &plan.environment.0,
                    check,
                )
            };
            verify_object(
                &read(Observation::CommitObject { commit: head })?,
                &plan.preview,
            )?;
            if parse_changes(&read(Observation::CommittedChanges { commit: head })?)?
                != plan.preview.changes
                || !read(Observation::StagedChanges)?.is_empty()
            {
                return Err(ErrorCode::OutcomeUnknown);
            }
            if snapshot(
                &plan.project,
                &plan.repository,
                &plan.paths,
                &plan.environment,
                check,
            )? != after
            {
                return Err(ErrorCode::OutcomeUnknown);
            }
            idle(&plan.project, &plan.repository)?;
            Ok(after)
        };
        match verify() {
            Ok(after) => outcome.after = Some(after),
            Err(_) => outcome.interrupted = Some(ErrorCode::OutcomeUnknown),
        }
    }
    Ok(outcome)
}

fn message(input: &str) -> Result<(String, bool), ErrorCode> {
    if input.contains('\0') || input.trim().is_empty() || input.len() > 64 * 1024 {
        return Err(ErrorCode::ResourceExhausted);
    }
    let added = !input.ends_with('\n');
    let mut stored = input.to_owned();
    if added {
        stored.push('\n');
    }
    if stored.len() > 64 * 1024 {
        return Err(ErrorCode::ResourceExhausted);
    }
    Ok((stored, added))
}
fn object(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}
pub(super) fn parse_changes(bytes: &[u8]) -> Result<Vec<StagedChange>, ErrorCode> {
    if bytes.is_empty() {
        return Err(ErrorCode::RevisionConflict);
    }
    let bytes = bytes
        .strip_suffix(b"\0")
        .ok_or(ErrorCode::UnsupportedCapability)?;
    let mut parts = bytes.split(|b| *b == 0);
    let mut changes = BTreeMap::new();
    while let Some(header) = parts.next() {
        if changes.len() >= 64 || header.len() > 256 {
            return Err(ErrorCode::ResourceExhausted);
        }
        let path = parts.next().ok_or(ErrorCode::UnsupportedCapability)?;
        let path = std::str::from_utf8(path).map_err(|_| ErrorCode::UnsupportedCapability)?;
        validate_relative(path)?;
        let header = std::str::from_utf8(header).map_err(|_| ErrorCode::UnsupportedCapability)?;
        let header = header
            .strip_prefix(':')
            .ok_or(ErrorCode::UnsupportedCapability)?;
        let fields: Vec<_> = header.split(' ').collect();
        let [old_mode, new_mode, old_object, new_object, status] = fields.as_slice() else {
            return Err(ErrorCode::UnsupportedCapability);
        };
        if ![old_mode, new_mode]
            .into_iter()
            .all(|mode| matches!(*mode, "000000" | "100644" | "100755"))
            || !object(old_object)
            || !object(new_object)
            || !matches!(*status, "A" | "D" | "M" | "T")
        {
            return Err(ErrorCode::UnsupportedCapability);
        }
        let change = StagedChange {
            relative_path: path.into(),
            old_mode: (*old_mode).into(),
            new_mode: (*new_mode).into(),
            old_object: (*old_object).into(),
            new_object: (*new_object).into(),
            status: (*status).into(),
        };
        if changes.insert(path.to_string(), change).is_some() {
            return Err(ErrorCode::UnsupportedCapability);
        }
    }
    Ok(changes.into_values().collect())
}
fn identity(bytes: &[u8]) -> Result<Identity, ErrorCode> {
    if bytes.len() > 4096 {
        return Err(ErrorCode::ResourceExhausted);
    }
    let value = std::str::from_utf8(bytes).map_err(|_| ErrorCode::UnsupportedCapability)?;
    let value = value
        .strip_suffix('\n')
        .ok_or(ErrorCode::UnsupportedCapability)?;
    let (person, zone) = value
        .rsplit_once(' ')
        .ok_or(ErrorCode::UnsupportedCapability)?;
    let (person, timestamp) = person
        .rsplit_once(' ')
        .ok_or(ErrorCode::UnsupportedCapability)?;
    if timestamp.parse::<i64>().is_err()
        || zone.len() != 5
        || !matches!(zone.as_bytes()[0], b'+' | b'-')
        || !zone.as_bytes()[1..].iter().all(u8::is_ascii_digit)
    {
        return Err(ErrorCode::UnsupportedCapability);
    }
    let (name, email) = person
        .rsplit_once(" <")
        .ok_or(ErrorCode::UnsupportedCapability)?;
    let email = email
        .strip_suffix('>')
        .ok_or(ErrorCode::UnsupportedCapability)?;
    if name.is_empty()
        || email.is_empty()
        || name.len() > 1024
        || email.len() > 1024
        || name.chars().chain(email.chars()).any(char::is_control)
    {
        return Err(ErrorCode::UnsupportedCapability);
    }
    Ok(Identity {
        name: name.into(),
        email: email.into(),
    })
}
pub(super) fn idle(project: &ProjectDirectory, repository: &str) -> Result<(), ErrorCode> {
    use rustix::fs::{openat, statat, AtFlags, Mode, OFlags};
    let directory = project.open_directory(repository)?;
    let git = openat(
        directory,
        ".git",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| ErrorCode::ScopeDenied)?;
    for marker in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "BISECT_START",
        "rebase-merge",
        "rebase-apply",
        "sequencer",
    ] {
        match statat(&git, marker, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => return Err(ErrorCode::TargetBusy),
            Err(rustix::io::Errno::NOENT) => {}
            Err(_) => return Err(ErrorCode::ScopeDenied),
        }
    }
    Ok(())
}
/// Hash-only configuration and exact staged objects form the future main-view
/// preview. The public broker cannot dispatch commit with this preparation alone.
pub fn prepare(
    project: &Arc<ProjectDirectory>,
    repository: &str,
    paths: &[String],
    input_message: &str,
    environment: &Environment,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<CommitPreview, ErrorCode> {
    let (stored_message, final_newline_added) = message(input_message)?;
    idle(project, repository)?;
    let observation = snapshot(project, repository, paths, environment, check)?;
    let read =
        |query| git_read::execution_preview(project, repository, query, &environment.0, check);
    let changes = parse_changes(&read(Observation::StagedChanges)?)?;
    let expected: std::collections::BTreeSet<_> = paths.iter().map(String::as_str).collect();
    if changes
        .iter()
        .map(|c| c.relative_path.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        != expected
    {
        return Err(ErrorCode::RevisionConflict);
    }
    let author = identity(&read(Observation::Identity { author: true })?)?;
    let committer = identity(&read(Observation::Identity { author: false })?)?;
    if snapshot(project, repository, paths, environment, check)? != observation
        || parse_changes(&read(Observation::StagedChanges)?)? != changes
        || identity(&read(Observation::Identity { author: true })?)? != author
        || identity(&read(Observation::Identity { author: false })?)? != committer
    {
        return Err(ErrorCode::RevisionConflict);
    }
    idle(project, repository)?;
    let revision = digest(
        &serde_json::to_vec(&(
            repository,
            &observation,
            &changes,
            &author,
            &committer,
            &stored_message,
        ))
        .map_err(|_| ErrorCode::ResourceExhausted)?,
    );
    Ok(CommitPreview {
        observation,
        changes,
        author,
        committer,
        stored_message,
        final_newline_added,
        revision,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command};
    #[test]
    fn approved_commits_verify_exact_index_identity_message_and_preserve_hook_effects() {
        let _serial = git_read::TEST_GATE.lock().unwrap();
        for case in [
            "initial",
            "ordinary",
            "stale",
            "message-hook",
            "index-hook",
            "failed-hook",
            "cancel-hook",
        ] {
            let root = tempfile::tempdir().unwrap();
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
            let git = |args: &[&str]| {
                let out = Command::new(git_read::trusted_executable().unwrap())
                    .arg("-C")
                    .arg(root.path())
                    .args(args)
                    .env_clear()
                    .envs(&environment.0)
                    .output()
                    .unwrap();
                assert!(
                    out.status.success(),
                    "{case}: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
                String::from_utf8(out.stdout).unwrap()
            };
            git(&["init", "-q", "--initial-branch=fixture"]);
            git(&["config", "user.name", "Fixture Identity"]);
            git(&["config", "user.email", "fixture@example.invalid"]);
            let path = root.path().join("a.txt");
            fs::write(&path, "baseline\n").unwrap();
            if case != "initial" {
                git(&["add", "--", "a.txt"]);
                git(&["commit", "-qm", "Fixture base"]);
            }
            fs::write(&path, "exact staged 🙂\n").unwrap();
            git(&["add", "--", "a.txt"]);
            fs::write(&path, "newer unstaged 🙂\n").unwrap();
            let hook = match case {
                "message-hook" => Some((
                    "commit-msg",
                    "printf 'Changed by configured hook\\n' > \"$1\"\n".into(),
                )),
                "index-hook" => Some((
                    "pre-commit",
                    format!(
                        "printf 'hook bytes\\n' > b.txt\n'{}' add -- b.txt\n",
                        git_read::trusted_executable().unwrap()
                    ),
                )),
                "failed-hook" => Some(("pre-commit", "touch HOOK_RAN\nexit 1\n".into())),
                "cancel-hook" => Some((
                    "pre-commit",
                    "touch HOOK_RAN\nsleep 20\ntouch LATE_EFFECT\n".into(),
                )),
                _ => None,
            };
            if let Some((name, body)) = hook {
                use std::os::unix::fs::PermissionsExt;
                let path = root.path().join(".git/hooks").join(name);
                fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
            }
            let project =
                Arc::new(ProjectDirectory::open(&root.path().canonicalize().unwrap()).unwrap());
            let message = "  Approved commit 🙂  \n\nBody  \n\n";
            let plan = Plan::prepare(
                project,
                "".into(),
                vec!["a.txt".into()],
                message,
                environment.clone(),
                &|| Ok(()),
            )
            .unwrap();
            assert!(!root.path().join("HOOK_RAN").exists());
            let before = plan.preview.observation.head.clone();
            let revision = plan.preview.revision.clone();
            if case == "stale" {
                fs::write(&path, "changed after approval\n").unwrap();
            }
            let start = std::time::Instant::now();
            let result = execute(plan, &revision, &|| {
                if case == "cancel-hook" && root.path().join("HOOK_RAN").exists() {
                    Err(ErrorCode::ControlRevoked)
                } else {
                    Ok(())
                }
            });
            if case == "stale" {
                assert!(matches!(result, Err(ErrorCode::RevisionConflict)));
                assert_eq!(git(&["rev-parse", "HEAD"]).trim(), before.unwrap());
                continue;
            }
            let result = result.unwrap();
            if matches!(case, "initial" | "ordinary") {
                assert_eq!(result.exit_code, Some(0));
                assert!(result.interrupted.is_none(), "{case}: {result:?}");
                let after = result.after.unwrap();
                assert_ne!(after.head, before);
                assert_eq!(git(&["show", "HEAD:a.txt"]), "exact staged 🙂\n");
                assert_eq!(fs::read_to_string(path).unwrap(), "newer unstaged 🙂\n");
                let object = git(&["cat-file", "commit", "HEAD"]);
                assert_eq!(object.split_once("\n\n").unwrap().1, message);
            } else {
                assert!(result.after.is_none());
                if matches!(case, "message-hook" | "index-hook") {
                    assert_eq!(result.exit_code, Some(0));
                    assert_eq!(result.interrupted, Some(ErrorCode::OutcomeUnknown));
                    assert_ne!(git(&["rev-parse", "HEAD"]).trim(), before.unwrap());
                } else {
                    assert_ne!(result.exit_code, Some(0));
                    assert!(root.path().join("HOOK_RAN").exists());
                    assert_eq!(git(&["rev-parse", "HEAD"]).trim(), before.unwrap());
                }
                if case == "cancel-hook" {
                    assert_eq!(result.interrupted, Some(ErrorCode::ControlRevoked));
                    assert!(start.elapsed() < std::time::Duration::from_secs(5));
                    assert!(!root.path().join("LATE_EFFECT").exists());
                }
            }
        }
    }
    #[test]
    fn exact_commit_preview_requires_all_staged_paths_and_preserves_identity_and_message() {
        let _serial = git_read::TEST_GATE.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
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
        let git = |args: &[&str]| {
            let out = Command::new(git_read::trusted_executable().unwrap())
                .arg("-C")
                .arg(root.path())
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
        git(&["init", "-q", "--initial-branch=fixture"]);
        git(&["config", "user.name", "Fixture Identity"]);
        git(&["config", "user.email", "fixture@example.invalid"]);
        fs::write(root.path().join("a.txt"), "staged bytes\n").unwrap();
        git(&["add", "--", "a.txt"]);
        fs::write(root.path().join("a.txt"), "newer unstaged bytes\n").unwrap();
        let project =
            Arc::new(ProjectDirectory::open(&root.path().canonicalize().unwrap()).unwrap());
        let paths = vec!["a.txt".into()];
        let input = "  Exact subject 🙂  \n\nBody  \n\n";
        let plan = prepare(&project, "", &paths, input, &environment, &|| Ok(())).unwrap();
        assert_eq!(plan.stored_message, input);
        assert!(!plan.final_newline_added);
        assert_eq!(plan.author.name, "Fixture Identity");
        assert_eq!(plan.committer.email, "fixture@example.invalid");
        assert_eq!(
            plan.changes[0].new_object,
            git(&["rev-parse", ":a.txt"]).trim()
        );
        assert_eq!(plan.changes[0].status, "A");
        let no_lf = prepare(&project, "", &paths, "Subject", &environment, &|| Ok(())).unwrap();
        assert_eq!(no_lf.stored_message, "Subject\n");
        assert!(no_lf.final_newline_added);
        assert_ne!(plan.revision, no_lf.revision);
        let hooks = root.path().join("hooks");
        fs::create_dir(&hooks).unwrap();
        let hook = hooks.join("pre-commit");
        fs::write(&hook, "#!/bin/sh\ntouch HOOK_RAN\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(hook, fs::Permissions::from_mode(0o700)).unwrap();
        git(&["config", "core.hooksPath", hooks.to_str().unwrap()]);
        prepare(&project, "", &paths, input, &environment, &|| Ok(())).unwrap();
        assert!(!root.path().join("HOOK_RAN").exists());
        fs::write(root.path().join("other.txt"), "extra staged data").unwrap();
        git(&["add", "--", "other.txt"]);
        assert!(matches!(
            prepare(&project, "", &paths, input, &environment, &|| Ok(())),
            Err(ErrorCode::RevisionConflict)
        ));
        git(&["rm", "--cached", "--", "other.txt"]);
        fs::write(root.path().join(".env"), "fixture secret").unwrap();
        git(&["add", "--", ".env"]);
        assert!(matches!(
            prepare(&project, "", &paths, input, &environment, &|| Ok(())),
            Err(ErrorCode::ScopeDenied)
        ));
        git(&["rm", "--cached", "--", ".env"]);
        fs::write(root.path().join(".git/MERGE_HEAD"), "0".repeat(40)).unwrap();
        assert!(matches!(
            prepare(&project, "", &paths, input, &environment, &|| Ok(())),
            Err(ErrorCode::TargetBusy)
        ));
        for input in [" ", "\0", &"x".repeat(65536)] {
            assert!(message(input).is_err());
        }
        assert_eq!(git(&["show", ":a.txt"]), "staged bytes\n");
        assert_eq!(
            fs::read_to_string(root.path().join("a.txt")).unwrap(),
            "newer unstaged bytes\n"
        );
    }
}
