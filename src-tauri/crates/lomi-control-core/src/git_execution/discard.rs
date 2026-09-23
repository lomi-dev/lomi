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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command};
    #[test]
    fn restores_only_approved_index_bytes_and_refuses_changed_or_untracked_files() {
        let _serial = git_read::TEST_GATE.lock().unwrap();
        for case in [
            "partial",
            "missing",
            "binary",
            "literal",
            "stale-file",
            "stale-index",
            "untracked",
            "symlink",
            "cancel",
        ] {
            let root = tempfile::tempdir().unwrap();
            let environment = Environment(
                [
                    ("PATH".into(), "/usr/bin:/bin".into()),
                    ("HOME".into(), root.path().as_os_str().into()),
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
                out.stdout
            };
            git(&["init", "-q"]);
            git(&["config", "user.name", "Fixture"]);
            git(&["config", "user.email", "fixture@example.invalid"]);
            let name = if case == "literal" {
                "[a].txt"
            } else {
                "a.txt"
            };
            let path = root.path().join(name);
            fs::write(&path, "base\n").unwrap();
            fs::write(root.path().join("untouched.txt"), "base\n").unwrap();
            git(&["add", "."]);
            git(&["commit", "-qm", "fixture base"]);
            let staged: &[u8] = if case == "binary" {
                b"staged\0bytes"
            } else {
                "staged 🙂\n".as_bytes()
            };
            fs::write(&path, staged).unwrap();
            git(&["--literal-pathspecs", "add", "--", name]);
            fs::write(&path, "unstaged newer\n").unwrap();
            fs::write(root.path().join("untouched.txt"), "keep this\n").unwrap();
            if case == "missing" {
                fs::remove_file(&path).unwrap();
            }
            if case == "untracked" {
                git(&["rm", "--cached", "-f", "--", name]);
            }
            if case == "symlink" {
                fs::remove_file(&path).unwrap();
                std::os::unix::fs::symlink("untouched.txt", &path).unwrap();
            }
            let project =
                Arc::new(ProjectDirectory::open(&root.path().canonicalize().unwrap()).unwrap());
            let plan = Plan::prepare(
                project,
                "".into(),
                vec![name.into()],
                environment.clone(),
                &|| Ok(()),
            );
            if matches!(case, "untracked" | "symlink") {
                assert!(plan.is_err(), "{case}");
                continue;
            }
            let plan = plan.unwrap();
            assert!(!plan.files[0].patch.is_empty());
            let revision = plan.revision.clone();
            let index = git(&["ls-files", "--stage", "-z"]);
            let head = git(&["rev-parse", "HEAD"]);
            if case.starts_with("stale") {
                fs::write(&path, "later human bytes\n").unwrap();
            }
            if case == "stale-index" {
                git(&["add", "--", name]);
            }
            let result = execute(plan, &revision, &|| {
                if case == "cancel" {
                    Err(ErrorCode::ControlRevoked)
                } else {
                    Ok(())
                }
            });
            if case.starts_with("stale") {
                assert!(matches!(result, Err(ErrorCode::RevisionConflict)), "{case}");
                assert_eq!(fs::read_to_string(&path).unwrap(), "later human bytes\n");
            } else if case == "cancel" {
                assert!(matches!(result, Err(ErrorCode::ControlRevoked)));
                assert_eq!(fs::read_to_string(&path).unwrap(), "unstaged newer\n");
            } else {
                let result = result.unwrap();
                assert_eq!(result.exit_code, Some(0), "{case}: {result:?}");
                assert!(result.interrupted.is_none(), "{case}: {result:?}");
                assert!(result.after.is_some());
                assert_eq!(fs::read(&path).unwrap(), staged);
                assert_eq!(git(&["ls-files", "--stage", "-z"]), index);
            }
            assert_eq!(git(&["rev-parse", "HEAD"]), head);
            assert_eq!(
                fs::read_to_string(root.path().join("untouched.txt")).unwrap(),
                "keep this\n"
            );
        }
    }
}
