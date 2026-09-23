use super::*;
use crate::project_files::ProjectDirectory;

pub(in crate::broker) enum Plan {
    Index(git_execution::IndexPlan),
    Discard(git_execution::discard::Plan),
    Commit(git_execution::commit::Plan),
    Fetch(git_execution::fetch::Plan),
    Push(git_execution::push::Plan),
    Pull(Box<git_execution::pull::Plan>),
}
pub(super) struct Executed {
    pub process: git_execution::IndexOutcome,
    pub fetch: Option<GitFetched>,
    pub push: Option<GitPushed>,
    pub pull: Option<GitPulled>,
}
impl Plan {
    pub(super) fn prepare(
        directory: Arc<ProjectDirectory>,
        input: &GitMutateInput,
        environment: git_execution::Environment,
        check: &dyn Fn() -> Result<(), ErrorCode>,
    ) -> Result<Self, ErrorCode> {
        Ok(match input.operation {
            GitMutation::Pull => Self::Pull(Box::new(git_execution::pull::Plan::prepare(
                directory,
                input.repository_relative.clone(),
                GitPullTarget {
                    remote: input.remote.clone().ok_or(ErrorCode::ScopeDenied)?,
                    reference: input.reference.clone().ok_or(ErrorCode::ScopeDenied)?,
                    source_commit: input.source_commit.clone().ok_or(ErrorCode::ScopeDenied)?,
                    expected_remote_commit: input
                        .expected_remote_commit
                        .clone()
                        .ok_or(ErrorCode::ScopeDenied)?,
                    mode: input.pull_mode.ok_or(ErrorCode::ScopeDenied)?,
                },
                environment,
                check,
            )?)),
            GitMutation::Discard => Self::Discard(git_execution::discard::Plan::prepare(
                directory,
                input.repository_relative.clone(),
                input.paths.clone(),
                environment,
                check,
            )?),
            GitMutation::Push => Self::Push(git_execution::push::Plan::prepare(
                directory,
                input.repository_relative.clone(),
                GitPushTarget {
                    remote: input.remote.clone().ok_or(ErrorCode::ScopeDenied)?,
                    reference: input.reference.clone().ok_or(ErrorCode::ScopeDenied)?,
                    source_commit: input.source_commit.clone().ok_or(ErrorCode::ScopeDenied)?,
                    expected_remote_commit: input.expected_remote_commit.clone(),
                },
                environment,
                check,
            )?),
            GitMutation::Fetch => Self::Fetch(git_execution::fetch::Plan::prepare(
                directory,
                input.repository_relative.clone(),
                input.remote.as_deref().ok_or(ErrorCode::ScopeDenied)?,
                input.reference.as_deref().ok_or(ErrorCode::ScopeDenied)?,
                environment,
                check,
            )?),
            GitMutation::Commit => Self::Commit(git_execution::commit::Plan::prepare(
                directory,
                input.repository_relative.clone(),
                input.paths.clone(),
                input.message.as_deref().ok_or(ErrorCode::ScopeDenied)?,
                environment,
                check,
            )?),
            GitMutation::Stage | GitMutation::Unstage => {
                Self::Index(git_execution::IndexPlan::prepare(
                    directory,
                    input.repository_relative.clone(),
                    if input.operation == GitMutation::Stage {
                        git_execution::IndexOperation::Stage
                    } else {
                        git_execution::IndexOperation::Unstage
                    },
                    input.paths.clone(),
                    environment,
                    check,
                )?)
            }
        })
    }
    pub(super) fn revision(&self) -> &str {
        match self {
            Self::Index(plan) => &plan.revision,
            Self::Discard(plan) => &plan.revision,
            Self::Commit(plan) => &plan.preview.revision,
            Self::Fetch(plan) => &plan.revision,
            Self::Push(plan) => &plan.revision,
            Self::Pull(plan) => &plan.revision,
        }
    }
    pub(super) fn observation(&self) -> &git_execution::Snapshot {
        match self {
            Self::Index(plan) => &plan.observation,
            Self::Discard(plan) => &plan.observation,
            Self::Commit(plan) => &plan.preview.observation,
            Self::Fetch(plan) => &plan.observation,
            Self::Push(plan) => &plan.observation,
            Self::Pull(plan) => &plan.observation,
        }
    }
    pub(super) fn commit_preview(&self) -> Option<GitCommitApproval> {
        let Self::Commit(plan) = self else {
            return None;
        };
        Some(GitCommitApproval {
            message: plan.preview.stored_message.clone(),
            final_newline_added: plan.preview.final_newline_added,
            author: plan.preview.author.clone(),
            committer: plan.preview.committer.clone(),
            changes: plan.preview.changes.clone(),
        })
    }
    pub(super) fn commit_result(&self) -> Option<GitCommitCreated> {
        use sha2::{Digest, Sha256};
        let Self::Commit(plan) = self else {
            return None;
        };
        Some(GitCommitCreated {
            message_sha256: format!(
                "{:x}",
                Sha256::digest(plan.preview.stored_message.as_bytes())
            ),
            author: plan.preview.author.clone(),
            committer: plan.preview.committer.clone(),
        })
    }
    pub(super) fn network_preview(&self) -> Option<GitFetchApproval> {
        match self {
            Self::Fetch(plan) => Some(plan.preview.clone()),
            _ => None,
        }
    }
    pub(super) fn pull_preview(&self) -> Option<GitPullApproval> {
        match self {
            Self::Pull(plan) => Some(plan.preview.clone()),
            _ => None,
        }
    }
    pub(super) fn discard_preview(
        &self,
    ) -> Option<Vec<lomi_control_protocol::git::GitDiscardFile>> {
        match self {
            Self::Discard(plan) => Some(plan.files.clone()),
            _ => None,
        }
    }
    pub(super) fn push_preview(&self) -> Option<GitPushApproval> {
        match self {
            Self::Push(plan) => Some(plan.preview.clone()),
            _ => None,
        }
    }
    pub(super) fn execute(
        self,
        hash: &str,
        check: &dyn Fn() -> Result<(), ErrorCode>,
    ) -> Result<Executed, ErrorCode> {
        match self {
            Self::Discard(plan) => {
                git_execution::discard::execute(plan, hash, check).map(|process| Executed {
                    process,
                    fetch: None,
                    push: None,
                    pull: None,
                })
            }
            Self::Index(plan) => {
                git_execution::execute_index(plan, hash, check).map(|process| Executed {
                    process,
                    fetch: None,
                    push: None,
                    pull: None,
                })
            }
            Self::Commit(plan) => {
                git_execution::commit::execute(plan, hash, check).map(|process| Executed {
                    process,
                    fetch: None,
                    push: None,
                    pull: None,
                })
            }
            Self::Fetch(plan) => {
                let preview = plan.preview.clone();
                let result = git_execution::fetch::execute(plan, hash, check)?;
                Ok(Executed {
                    process: result.execution,
                    fetch: result.fetched_commit.map(|commit| GitFetched {
                        remote: preview.remote,
                        reference: preview.reference,
                        destination: preview.destination,
                        commit,
                    }),
                    push: None,
                    pull: None,
                })
            }
            Self::Pull(plan) => {
                let target = plan.preview.target.clone();
                let result = git_execution::pull::execute(*plan, hash, check)?;
                Ok(Executed {
                    process: result.execution,
                    fetch: None,
                    push: None,
                    pull: result.status.zip(result.fetched_commit).map(
                        |(outcome, fetched_commit)| GitPulled {
                            target,
                            fetched_commit,
                            outcome,
                        },
                    ),
                })
            }
            Self::Push(plan) => {
                let target = plan.preview.target.clone();
                let result = git_execution::push::execute(plan, hash, check)?;
                Ok(Executed {
                    process: result.execution,
                    fetch: None,
                    push: result.changed.map(|changed| GitPushed {
                        target,
                        changed,
                        verification: GitPushVerification::ServerAcknowledgement,
                    }),
                    pull: None,
                })
            }
        }
    }
}
