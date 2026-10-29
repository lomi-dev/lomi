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
