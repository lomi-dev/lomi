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
