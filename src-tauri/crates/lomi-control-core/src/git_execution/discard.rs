//! Restore exact tracked worktree paths from the approved index. Untracked
//! removal belongs to the existing reversible Trash operation, never Git clean.
use super::*;
pub use lomi_control_protocol::git::GitDiscardFile as Preview;

pub struct Plan {
    project: Arc<ProjectDirectory>,
    repository: String,
    paths: Vec<String>,
    environment: Environment,
    index_entries_revision: String,
    pub observation: Snapshot,
    pub files: Vec<Preview>,
    pub revision: String,
}
impl Plan {
    pub fn prepare(
        project: Arc<ProjectDirectory>,
        repository: String,
        paths: Vec<String>,
        environment: Environment,
        check: &dyn Fn() -> Result<(), ErrorCode>,
    ) -> Result<Self, ErrorCode> {
        if paths.is_empty() || paths.len() > 64 {
            return Err(ErrorCode::ResourceExhausted);
        }
        let observation = snapshot(&project, &repository, &paths, &environment, check)?;
        let index_entries_revision = digest(&git_read::execution_preview(
            &project,
            &repository,
            Observation::IndexedFiles,
            &environment.0,
            check,
        )?);
        let mut files = Vec::new();
        for file in &observation.files {
            let read = |query| {
                git_read::execution_preview(&project, &repository, query, &environment.0, check)
            };
            let raw = read(Observation::IndexEntry {
                relative: &file.relative_path,
            })?;
            let entry = std::str::from_utf8(&raw).map_err(|_| ErrorCode::UnsupportedCapability)?;
            let entry = entry
                .strip_suffix('\0')
                .ok_or(ErrorCode::UnsupportedCapability)?;
            let (header, path) = entry
                .split_once('\t')
                .ok_or(ErrorCode::UnsupportedCapability)?;
            let parts: Vec<_> = header.split(' ').collect();
            let [mode, object, stage] = parts.as_slice() else {
                return Err(ErrorCode::UnsupportedCapability);
            };
            if path != file.relative_path
                || !matches!(*mode, "100644" | "100755")
                || *stage != "0"
                || !push::oid(object)
            {
                return Err(ErrorCode::ScopeDenied);
            }
            let raw = read(Observation::Diff {
                relative: path,
                staged: false,
            })?;
            let patch = git_read::guarded_patch(&raw)?;
            if patch.is_empty() {
                return Err(ErrorCode::RevisionConflict);
            }
            files.push(Preview {
                relative_path: path.into(),
                index_object: (*object).into(),
                index_mode: (*mode).into(),
                patch: patch.into(),
            });
            // Bound the complete preview before retaining it or asking main.
            if serde_json::to_vec(&files)
                .map_err(|_| ErrorCode::ResourceExhausted)?
                .len()
                > 24 * 1024
            {
                return Err(ErrorCode::ResourceExhausted);
            }
        }
        if snapshot(&project, &repository, &paths, &environment, check)? != observation {
            return Err(ErrorCode::RevisionConflict);
        }
        let revision = digest(
            &serde_json::to_vec(&(&observation, &files))
                .map_err(|_| ErrorCode::ResourceExhausted)?,
        );
        Ok(Self {
            project,
            repository,
            paths,
            environment,
            observation,
            index_entries_revision,
            files,
            revision,
        })
    }
}

pub fn execute(
    plan: Plan,
    approved_revision: &str,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<IndexOutcome, ErrorCode> {
    if approved_revision != plan.revision {
        return Err(ErrorCode::ControlRevoked);
    }
    let current = Plan::prepare(
        plan.project.clone(),
        plan.repository.clone(),
        plan.paths.clone(),
        plan.environment.clone(),
        check,
    )?;
    if current.revision != plan.revision {
        return Err(ErrorCode::RevisionConflict);
    }
    let mut args: Vec<String> = ["--literal-pathspecs", "restore", "--worktree", "--"]
        .map(str::to_string)
        .into();
    args.extend(plan.paths.iter().cloned());
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
            let before = &plan.observation;
            if after.head != before.head
                || after.branch != before.branch
                || after.references_revision != before.references_revision
                || after.configuration_revision != before.configuration_revision
                || after.files.iter().any(|f| f.sha256.is_none())
            {
                return Err(ErrorCode::OutcomeUnknown);
            }
            if digest(&git_read::execution_preview(
                &plan.project,
                &plan.repository,
                Observation::IndexedFiles,
                &plan.environment.0,
                check,
            )?) != plan.index_entries_revision
            {
                return Err(ErrorCode::OutcomeUnknown);
            }
            for path in &plan.paths {
                let diff = git_read::execution_preview(
                    &plan.project,
                    &plan.repository,
                    Observation::Diff {
                        relative: path,
                        staged: false,
                    },
                    &plan.environment.0,
                    check,
                )?;
                if !diff.is_empty() {
                    return Err(ErrorCode::OutcomeUnknown);
                }
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
            Ok(after)
        };
        match verify() {
            Ok(after) => outcome.after = Some(after),
            Err(_) => outcome.interrupted = Some(ErrorCode::OutcomeUnknown),
        }
    }
    Ok(outcome)
}
