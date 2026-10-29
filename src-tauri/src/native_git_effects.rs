use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use lomi_remote_protocol::{
    rpc::{
        CommitMessage, GitCommitParams, GitMutationContext, GitNetworkParams, GitPreconditions,
        GitPullParams, GitRemote, GitRemoteSelector, GitRemoteSelectorKind, GitRepository,
        GitStageParams, GitStagePath, GitStatusEntry, GitStatusParams, GitUnstageParams, PageLimit,
        PullMode, RefId, RemoteId, RpcCall, RpcResult, WorkspaceReadParams,
    },
    Effect, EventCursor, Method, OperationId, OperationReceipt, OperationResult, OperationState,
    OperationStatusParams, RepoStateToken, RepositoryId, RequestId,
};
use ring::rand::SecureRandom;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    thread,
    time::Duration,
};
use tauri::{AppHandle, Manager};

use crate::shared_host::{NativeGitBinding, SharedHost};

const JOURNAL_LIMIT: u64 = 1024 * 1024;
const JOURNAL_ENTRIES: usize = 512;
const STATUS_POLLS: usize = 5;
const STATUS_INTERVAL: Duration = Duration::from_millis(200);
const STATUS_PAGE_LIMIT: u32 = 200;
const STATUS_ENTRY_LIMIT: usize = 100_000;

static MUTATION_ADMISSION: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum LegacyGitAction<'a> {
    Stage { paths: &'a [String], stage: bool },
    Commit { message: &'a str },
    Fetch { remote: Option<&'a str> },
    Pull,
    Push { remote: Option<&'a str> },
}

#[derive(Clone, Copy)]
enum NetworkAction {
    Fetch,
    Pull,
    Push,
}

#[derive(Clone)]
struct SelectedRemoteReference {
    remote_id: RemoteId,
    ref_id: RefId,
    refs_state_token: RepoStateToken,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PendingGitEffect {
    operation_id: OperationId,
    method: Method,
    intent_fingerprint: String,
    request_fingerprint: String,
    request_sent: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GitEffectJournal {
    version: u32,
    operations: BTreeMap<String, PendingGitEffect>,
}

impl Default for GitEffectJournal {
    fn default() -> Self {
        Self {
            version: 1,
            operations: BTreeMap::new(),
        }
    }
}

struct CurrentRepository {
    repository: GitRepository,
    status: lomi_remote_protocol::rpc::GitStatusResult,
    entries: Vec<GitStatusEntry>,
}

enum ReceiptResolution {
    Succeeded(RepoStateToken),
    NoEffect(String),
    Unknown,
    EffectNeedsReview,
}

pub(crate) fn stage(
    app: &AppHandle,
    host: &SharedHost,
    root: &str,
    paths: &[String],
    stage: bool,
) -> Result<(), String> {
    let mut paths = paths.to_vec();
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return Ok(());
    }
    if paths.len() > 200 {
        return Err("Select at most 200 files for a Git index change.".into());
    }
    run(
        app,
        host,
        root,
        LegacyGitAction::Stage {
            paths: &paths,
            stage,
        },
    )
}

pub(crate) fn commit(
    app: &AppHandle,
    host: &SharedHost,
    root: &str,
    message: &str,
) -> Result<(), String> {
    if message.trim().is_empty() {
        return Err("Enter a commit message.".into());
    }
    CommitMessage::try_from(message.to_owned())
        .map_err(|_| "The commit message exceeds the supported limit.".to_owned())?;
    run(app, host, root, LegacyGitAction::Commit { message })
}

pub(crate) fn fetch(
    app: &AppHandle,
    host: &SharedHost,
    root: &str,
    remote: Option<&str>,
) -> Result<(), String> {
    run(app, host, root, LegacyGitAction::Fetch { remote })
}

pub(crate) fn pull(
    app: &AppHandle,
    host: &SharedHost,
    root: &str,
    rebase: bool,
) -> Result<(), String> {
    if rebase {
        return Err("Pull with rebase is not supported by the shared Git host.".into());
    }
    run(app, host, root, LegacyGitAction::Pull)
}

pub(crate) fn push(
    app: &AppHandle,
    host: &SharedHost,
    root: &str,
    remote: Option<&str>,
    force: bool,
) -> Result<(), String> {
    if force {
        return Err("Force push is not supported by the shared Git host.".into());
    }
    run(app, host, root, LegacyGitAction::Push { remote })
}

fn run(
    app: &AppHandle,
    host: &SharedHost,
    root: &str,
    action: LegacyGitAction<'_>,
) -> Result<(), String> {
    let _admission = MUTATION_ADMISSION
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "Native Git operation state is unavailable.")?;
    let repository_path = fs::canonicalize(root)
        .map_err(|_| "The selected Git repository is unavailable.".to_owned())?;
    if !repository_path.is_dir() {
        return Err("The selected Git repository is unavailable.".into());
    }
    let binding = host.native_git_binding(app, &repository_path)?;
    let repository_id = resolve_repository_id(&binding, &repository_path)?;
    let repository = find_host_repository(host, &binding, repository_id)?;
    let needs_all_entries = matches!(
        &action,
        LegacyGitAction::Stage { .. } | LegacyGitAction::Pull
    );
    let current = current_repository(host, &binding, repository, repository_id, needs_all_entries)?;
    if matches!(&action, LegacyGitAction::Pull) {
        if !current.entries.is_empty() || current.status.truncated {
            return Err("Fast-forward pull requires a clean working tree.".into());
        }
    }

    let operation_key = repository_key(&binding, repository_id);
    let selected_reference = select_action_reference(&current.repository, &action)?;
    let intent_fingerprint = intent_fingerprint(
        &binding,
        repository_id,
        &action,
        selected_reference.as_ref(),
    )?;
    let journal_path = journal_path(app)?;
    let mut journal = read_journal(&journal_path)?;
    match recover_existing(
        host,
        &binding,
        &operation_key,
        &intent_fingerprint,
        &current.status.state_token,
        &journal_path,
        &mut journal,
    )? {
        Some(()) => return Ok(()),
        None => {}
    }

    let operation_id = new_operation_id()?;
    let call = build_call(
        &binding,
        repository_id,
        &current,
        &action,
        selected_reference.as_ref(),
        operation_id.clone(),
    )?;
    let method = call.method();
    let request_fingerprint = request_fingerprint(&call)?;
    if journal.operations.len() >= JOURNAL_ENTRIES {
        return Err(
            "Too many Git operations are awaiting confirmation. Resolve a pending operation before continuing."
                .into(),
        );
    }
    let pending = PendingGitEffect {
        operation_id: operation_id.clone(),
        method,
        intent_fingerprint,
        request_fingerprint,
        request_sent: false,
    };
    journal.operations.insert(operation_key.clone(), pending);
    write_journal(&journal_path, &journal)?;

    journal
        .operations
        .get_mut(&operation_key)
        .ok_or("The pending Git operation could not be recorded.")?
        .request_sent = true;
    write_journal(&journal_path, &journal)?;

    let response = host.native_git_rpc(new_request_id()?, call);
    match response {
        Ok(lomi_remote_protocol::RpcResponse::Result {
            result,
            request_id: _,
            ..
        }) => {
            if let Some(receipt) = mutation_receipt(result) {
                if valid_receipt(&receipt, operation_id, method) {
                    return finish_first_receipt(
                        host,
                        &binding,
                        &operation_key,
                        operation_id,
                        method,
                        &current.status.state_token,
                        &journal_path,
                        &mut journal,
                        receipt,
                    );
                }
            }
            resolve_after_send(
                host,
                &binding,
                &operation_key,
                operation_id,
                method,
                &current.status.state_token,
                &journal_path,
                &mut journal,
            )
        }
        Ok(lomi_remote_protocol::RpcResponse::Error { error, .. })
            if error.effect() == Effect::None =>
        {
            clear_operation(&journal_path, &mut journal, &operation_key)?;
            Err(error.message().to_owned())
        }
        Ok(_) | Err(_) => resolve_after_send(
            host,
            &binding,
            &operation_key,
            operation_id,
            method,
            &current.status.state_token,
            &journal_path,
            &mut journal,
        ),
    }
}

fn resolve_repository_id(
    binding: &NativeGitBinding,
    repository_path: &Path,
) -> Result<RepositoryId, String> {
    let service =
        lomi_git::GitService::open(&binding.root_path, &[], binding.workspace.context.clone())
            .map_err(|_| {
                "The Git repository cannot be safely resolved in this imported workspace."
            })?;
    let control = lomi_git::RequestControl::with_timeout(
        lomi_git::CancellationToken::new(),
        Duration::from_secs(20),
    )
    .map_err(|_| "Git repository discovery is unavailable.")?;
    service
        .repository_id_for_path(repository_path, &control)
        .map_err(|_| {
            "The Git repository changed or is unavailable in the imported workspace.".into()
        })
}

fn find_host_repository(
    host: &SharedHost,
    binding: &NativeGitBinding,
    repository_id: RepositoryId,
) -> Result<GitRepository, String> {
    let result = rpc_result(
        host,
        RpcCall::GitRepositories(WorkspaceReadParams {
            context: binding.workspace.context.clone(),
        }),
    )?;
    let RpcResult::GitRepositories(repositories) = result else {
        return Err("Local host returned an invalid Git repository response.".into());
    };
    repositories
        .repositories
        .0
        .into_iter()
        .find(|repository| repository.repository_id == repository_id)
        .ok_or_else(|| "The selected repository is not part of the imported host workspace.".into())
}

fn current_repository(
    host: &SharedHost,
    binding: &NativeGitBinding,
    repository: GitRepository,
    repository_id: RepositoryId,
    all_entries: bool,
) -> Result<CurrentRepository, String> {
    let limit =
        PageLimit::try_from(STATUS_PAGE_LIMIT).map_err(|_| "Git status page size is invalid.")?;
    let first = status_page(host, binding, repository_id, None, None, limit.clone())?;
    let expected_state = first.state_token.clone();
    let expected_head = first.head.clone();
    let expected_index = first.index.clone();
    let mut entries = first.entries.0.clone();
    let mut next_cursor = first.next_cursor.clone();
    let mut truncated = first.truncated;
    let mut pages = 1usize;
    while all_entries && truncated {
        let cursor = next_cursor
            .take()
            .ok_or("Git status did not return a cursor for a truncated result.")?;
        if pages >= STATUS_ENTRY_LIMIT / STATUS_PAGE_LIMIT as usize {
            return Err("Git status exceeds the supported repository limit.".into());
        }
        let next = status_page(
            host,
            binding,
            repository_id,
            Some(expected_state.clone()),
            Some(cursor),
            limit.clone(),
        )?;
        if next.state_token != expected_state
            || next.head != expected_head
            || next.index != expected_index
        {
            return Err("The Git repository changed while reading its status.".into());
        }
        if entries.len().saturating_add(next.entries.0.len()) > STATUS_ENTRY_LIMIT {
            return Err("Git status exceeds the supported repository limit.".into());
        }
        entries.extend(next.entries.0);
        truncated = next.truncated;
        next_cursor = next.next_cursor;
        pages += 1;
    }
    let mut status = first;
    if all_entries {
        status.truncated = truncated;
        status.next_cursor = next_cursor;
    }
    Ok(CurrentRepository {
        repository,
        status,
        entries,
    })
}

fn status_page(
    host: &SharedHost,
    binding: &NativeGitBinding,
    repository_id: RepositoryId,
    state_token: Option<RepoStateToken>,
    cursor: Option<EventCursor>,
    limit: PageLimit,
) -> Result<lomi_remote_protocol::rpc::GitStatusResult, String> {
    match rpc_result(
        host,
        RpcCall::GitStatus(GitStatusParams {
            context: binding.workspace.context.clone(),
            repository_id,
            state_token,
            cursor,
            limit,
        }),
    )? {
        RpcResult::GitStatus(status) => Ok(status),
        _ => Err("Local host returned an invalid Git status response.".into()),
    }
}

fn build_call(
    binding: &NativeGitBinding,
    repository_id: RepositoryId,
    current: &CurrentRepository,
    action: &LegacyGitAction<'_>,
    selected_reference: Option<&SelectedRemoteReference>,
    operation_id: OperationId,
) -> Result<RpcCall, String> {
    let mutation = GitMutationContext {
        operation_id,
        context: binding.workspace.context.clone(),
    };
    let expected = GitPreconditions {
        head: current.status.head.clone(),
        index: current.status.index.clone(),
    };
    match action {
        LegacyGitAction::Stage { paths, stage } => {
            let entries = entries_by_path(&current.entries)?;
            let mut selected = Vec::with_capacity(paths.len());
            for path in paths.iter() {
                validate_relative_path(path)?;
                if let Some(entry) = entries.get(path.as_bytes()).copied() {
                    selected.push(entry);
                }
            }
            if selected.len() != paths.len() {
                return Err("The Git file changed. Refresh Source Control and try again.".into());
            }
            if *stage {
                let paths = selected
                    .into_iter()
                    .map(|entry| GitStagePath {
                        entry_id: entry.entry_id.clone(),
                        file_revision: entry.file_revision.clone(),
                    })
                    .collect();
                Ok(RpcCall::GitStage(GitStageParams {
                    mutation,
                    repository_id,
                    expected,
                    paths: lomi_remote_protocol::rpc::BoundedList(paths),
                }))
            } else {
                let paths = selected
                    .into_iter()
                    .map(|entry| entry.entry_id.clone())
                    .collect();
                Ok(RpcCall::GitUnstage(GitUnstageParams {
                    mutation,
                    repository_id,
                    expected,
                    paths: lomi_remote_protocol::rpc::BoundedList(paths),
                }))
            }
        }
        LegacyGitAction::Commit { message } => Ok(RpcCall::GitCommit(GitCommitParams {
            mutation,
            repository_id,
            expected,
            message: CommitMessage::try_from((*message).to_owned())
                .map_err(|_| "The commit message exceeds the supported limit.")?,
        })),
        LegacyGitAction::Fetch { .. } => {
            let selected =
                selected_reference.ok_or("No supported Git fetch selector is available.")?;
            Ok(RpcCall::GitFetch(GitNetworkParams {
                mutation,
                repository_id,
                remote_id: selected.remote_id.clone(),
                ref_id: selected.ref_id.clone(),
                expected_refs: selected.refs_state_token.clone(),
            }))
        }
        LegacyGitAction::Pull => {
            let selected =
                selected_reference.ok_or("No supported Git pull selector is available.")?;
            Ok(RpcCall::GitPull(GitPullParams {
                mutation,
                repository_id,
                remote_id: selected.remote_id.clone(),
                ref_id: selected.ref_id.clone(),
                expected_refs: selected.refs_state_token.clone(),
                expected_worktree: current.status.state_token.clone(),
                expected,
                mode: PullMode::FastForwardOnly,
            }))
        }
        LegacyGitAction::Push { .. } => {
            let selected =
                selected_reference.ok_or("No supported Git push selector is available.")?;
            Ok(RpcCall::GitPush(GitNetworkParams {
                mutation,
                repository_id,
                remote_id: selected.remote_id.clone(),
                ref_id: selected.ref_id.clone(),
                expected_refs: selected.refs_state_token.clone(),
            }))
        }
    }
}

fn select_action_reference(
    repository: &GitRepository,
    action: &LegacyGitAction<'_>,
) -> Result<Option<SelectedRemoteReference>, String> {
    match action {
        LegacyGitAction::Fetch { remote } => Ok(Some(select_remote_reference(
            repository,
            *remote,
            NetworkAction::Fetch,
        )?)),
        LegacyGitAction::Pull => Ok(Some(select_remote_reference(
            repository,
            None,
            NetworkAction::Pull,
        )?)),
        LegacyGitAction::Push { remote } => Ok(Some(select_remote_reference(
            repository,
            *remote,
            NetworkAction::Push,
        )?)),
        LegacyGitAction::Stage { .. } | LegacyGitAction::Commit { .. } => Ok(None),
    }
}

fn select_remote_reference(
    repository: &GitRepository,
    requested_remote: Option<&str>,
    action: NetworkAction,
) -> Result<SelectedRemoteReference, String> {
    let remotes = if let Some(name) = requested_remote {
        let matching = repository
            .remotes
            .0
            .iter()
            .filter(|remote| remote.display_name.as_str() == name)
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            return Err(
                "This Git remote is no longer uniquely configured. Refresh Source Control.".into(),
            );
        }
        matching
    } else {
        repository.remotes.0.iter().collect::<Vec<_>>()
    };
    for remote in &remotes {
        validate_remote_selectors(remote)?;
    }
    let candidates = remotes
        .iter()
        .flat_map(|remote| {
            remote
                .selectors
                .0
                .iter()
                .filter(move |selector| match action {
                    NetworkAction::Fetch => selector.fetch,
                    NetworkAction::Pull => selector.pull,
                    NetworkAction::Push => selector.push,
                })
                .map(move |selector| (*remote, selector))
        })
        .collect::<Vec<_>>();
    let preferred_kind = match action {
        NetworkAction::Fetch => GitRemoteSelectorKind::Fetch,
        NetworkAction::Pull => GitRemoteSelectorKind::Fetch,
        NetworkAction::Push => GitRemoteSelectorKind::Push,
    };
    let preferred = candidates
        .iter()
        .filter(|(_, selector)| selector.kind == preferred_kind)
        .cloned()
        .collect::<Vec<_>>();
    let candidates = if preferred.is_empty() {
        candidates
    } else {
        preferred
    };
    if candidates.len() != 1 {
        let (operation, message) = match action {
            NetworkAction::Fetch => (
                "fetch",
                "Choose a remote and reference in the shared Git panel.",
            ),
            NetworkAction::Pull => (
                "fast-forward pull",
                "Choose a remote and reference in the shared Git panel.",
            ),
            NetworkAction::Push => ("push", "Choose a push reference in the shared Git panel."),
        };
        if candidates.is_empty() {
            if matches!(action, NetworkAction::Pull) {
                return Err("Fast-forward pull is unavailable because the current branch has no unique configured upstream that this remote can safely fetch.".into());
            }
            return Err(format!("No supported {operation} selector is configured."));
        }
        return Err(message.into());
    }
    let (remote, selector) = candidates[0];
    Ok(SelectedRemoteReference {
        remote_id: remote.remote_id.clone(),
        ref_id: selector.ref_id.clone(),
        refs_state_token: remote.refs_state_token.clone(),
    })
}

fn validate_remote_selectors(remote: &GitRemote) -> Result<(), String> {
    let mut ref_ids = HashSet::with_capacity(remote.refs.0.len());
    for reference in &remote.refs.0 {
        if !ref_ids.insert(reference.as_str()) {
            return Err("The local host returned duplicate Git reference IDs.".into());
        }
    }
    let mut selector_ids = HashSet::with_capacity(remote.selectors.0.len());
    for selector in &remote.selectors.0 {
        if !selector_ids.insert(selector.ref_id.as_str())
            || !ref_ids.contains(selector.ref_id.as_str())
        {
            return Err("The local host returned invalid Git selector metadata.".into());
        }
        let valid = match selector.kind {
            GitRemoteSelectorKind::Head => selector.fetch && !selector.pull && !selector.push,
            GitRemoteSelectorKind::Fetch => selector.fetch && !selector.push,
            GitRemoteSelectorKind::Push => !selector.fetch && !selector.pull && selector.push,
        };
        if !valid {
            return Err("The local host returned inconsistent Git selector capabilities.".into());
        }
    }
    if selector_ids.len() != ref_ids.len() {
        return Err("The local host returned incomplete Git selector metadata.".into());
    }
    Ok(())
}

fn entries_by_path(
    entries: &[GitStatusEntry],
) -> Result<HashMap<Vec<u8>, &GitStatusEntry>, String> {
    let mut by_path = HashMap::with_capacity(entries.len());
    for entry in entries {
        let path = URL_SAFE_NO_PAD
            .decode(entry.path_id.as_str())
            .map_err(|_| "The local host returned an invalid Git file path.")?;
        if path.is_empty() || by_path.insert(path, entry).is_some() {
            return Err("The local host returned duplicate Git file paths.".into());
        }
    }
    Ok(by_path)
}

fn validate_relative_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.contains('\0')
        || path.starts_with('/')
        || path.starts_with('\\')
        || path
            .split(['/', '\\'])
            .any(|part| part == ".." || part == ".git")
    {
        return Err("Git paths must stay inside the selected repository and outside .git.".into());
    }
    Ok(())
}

fn recover_existing(
    host: &SharedHost,
    binding: &NativeGitBinding,
    operation_key: &str,
    intent_fingerprint: &str,
    current_state: &RepoStateToken,
    path: &Path,
    journal: &mut GitEffectJournal,
) -> Result<Option<()>, String> {
    let Some(pending) = journal.operations.get(operation_key) else {
        return Ok(None);
    };
    let pending = PendingGitEffect {
        operation_id: pending.operation_id,
        method: pending.method,
        intent_fingerprint: pending.intent_fingerprint.clone(),
        request_fingerprint: pending.request_fingerprint.clone(),
        request_sent: pending.request_sent,
    };
    if !pending.request_sent {
        clear_operation(path, journal, operation_key)?;
        return Ok(None);
    }
    let resolution = resolve_status(host, binding, pending.operation_id, pending.method)?;
    match resolution {
        ReceiptResolution::Succeeded(result_state) => {
            clear_operation(path, journal, operation_key)?;
            if pending.intent_fingerprint == intent_fingerprint && &result_state == current_state {
                Ok(Some(()))
            } else {
                Err("A previous Git operation completed, but repository state or intent changed. Refresh and review Source Control before continuing.".into())
            }
        }
        ReceiptResolution::NoEffect(message) => {
            clear_operation(path, journal, operation_key)?;
            let _ = message;
            Ok(None)
        }
        ReceiptResolution::EffectNeedsReview => Err(
            "A previous Git operation has a committed or partial effect that needs review. Inspect repository state before another mutation.".into(),
        ),
        ReceiptResolution::Unknown => Err(
            "A previous Git operation could not be confirmed. Inspect repository state; no retry was sent.".into(),
        ),
    }
}

fn finish_first_receipt(
    host: &SharedHost,
    binding: &NativeGitBinding,
    operation_key: &str,
    operation_id: OperationId,
    method: Method,
    current_state: &RepoStateToken,
    path: &Path,
    journal: &mut GitEffectJournal,
    receipt: OperationReceipt,
) -> Result<(), String> {
    match classify_receipt(&receipt, operation_id, method) {
        ReceiptResolution::Succeeded(_) => {
            clear_operation(path, journal, operation_key)
        }
        ReceiptResolution::NoEffect(message) => {
            clear_operation(path, journal, operation_key)?;
            Err(message)
        }
        ReceiptResolution::EffectNeedsReview => Err(
            "The Git operation has a committed or partial effect that needs review. Inspect repository state before continuing.".into(),
        ),
        ReceiptResolution::Unknown => resolve_after_send(
            host,
            binding,
            operation_key,
            operation_id,
            method,
            current_state,
            path,
            journal,
        ),
    }
}

fn resolve_after_send(
    host: &SharedHost,
    binding: &NativeGitBinding,
    operation_key: &str,
    operation_id: OperationId,
    method: Method,
    _current_state: &RepoStateToken,
    path: &Path,
    journal: &mut GitEffectJournal,
) -> Result<(), String> {
    match resolve_status(host, binding, operation_id, method)? {
        ReceiptResolution::Succeeded(_) => clear_operation(path, journal, operation_key),
        ReceiptResolution::NoEffect(message) => {
            clear_operation(path, journal, operation_key)?;
            Err(message)
        }
        ReceiptResolution::EffectNeedsReview => Err(
            "The Git operation has a committed or partial effect that needs review. Inspect repository state before continuing.".into(),
        ),
        ReceiptResolution::Unknown => Err(
            "The Git operation outcome is uncertain. Inspect repository state; the operation was not replayed.".into(),
        ),
    }
}

fn resolve_status(
    host: &SharedHost,
    binding: &NativeGitBinding,
    operation_id: OperationId,
    method: Method,
) -> Result<ReceiptResolution, String> {
    for attempt in 0..STATUS_POLLS {
        let response = host.native_git_rpc(
            new_request_id()?,
            RpcCall::OperationStatus(OperationStatusParams {
                context: binding.workspace.context.clone(),
                operation_id: operation_id.clone(),
            }),
        );
        let receipt = match response {
            Ok(lomi_remote_protocol::RpcResponse::Result {
                result: RpcResult::OperationStatus(receipt),
                ..
            }) if valid_receipt(&receipt, operation_id, method) => receipt,
            _ => return Ok(ReceiptResolution::Unknown),
        };
        let resolution = classify_receipt(&receipt, operation_id, method);
        if !matches!(
            receipt.state(),
            OperationState::Accepted | OperationState::Running
        ) {
            return Ok(resolution);
        }
        if attempt + 1 < STATUS_POLLS {
            thread::sleep(STATUS_INTERVAL);
        }
    }
    Ok(ReceiptResolution::Unknown)
}

fn classify_receipt(
    receipt: &OperationReceipt,
    operation_id: OperationId,
    method: Method,
) -> ReceiptResolution {
    if !valid_receipt(receipt, operation_id, method) {
        return ReceiptResolution::Unknown;
    }
    match (receipt.state(), receipt.effect()) {
        (OperationState::Succeeded, Effect::Committed) => match receipt.result() {
            Some(OperationResult::Git { repo_state_token }) => {
                ReceiptResolution::Succeeded(repo_state_token.clone())
            }
            _ => ReceiptResolution::Unknown,
        },
        (OperationState::Failed, Effect::None) | (OperationState::Cancelled, Effect::None) => {
            ReceiptResolution::NoEffect(
                "The Git operation was rejected without changing repository state.".into(),
            )
        }
        (OperationState::Failed, Effect::Committed | Effect::Partial)
        | (OperationState::OutcomeUnknown, Effect::Unknown) => ReceiptResolution::EffectNeedsReview,
        (OperationState::Accepted | OperationState::Running, Effect::None) => {
            ReceiptResolution::Unknown
        }
        _ => ReceiptResolution::Unknown,
    }
}

fn valid_receipt(receipt: &OperationReceipt, id: OperationId, method: Method) -> bool {
    receipt.operation_id() == id && receipt.stored_rights().original_method() == method
}

fn mutation_receipt(result: RpcResult) -> Option<OperationReceipt> {
    match result {
        RpcResult::GitStage(receipt)
        | RpcResult::GitUnstage(receipt)
        | RpcResult::GitCommit(receipt)
        | RpcResult::GitFetch(receipt)
        | RpcResult::GitPush(receipt)
        | RpcResult::GitPull(receipt) => Some(receipt),
        _ => None,
    }
}

fn rpc_result(host: &SharedHost, call: RpcCall) -> Result<RpcResult, String> {
    match host.native_git_rpc(new_request_id()?, call)? {
        lomi_remote_protocol::RpcResponse::Result { result, .. } => Ok(result),
        lomi_remote_protocol::RpcResponse::Error { error, .. } => Err(error.message().to_owned()),
    }
}

fn intent_fingerprint(
    binding: &NativeGitBinding,
    repository_id: RepositoryId,
    action: &LegacyGitAction<'_>,
    selected_reference: Option<&SelectedRemoteReference>,
) -> Result<String, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Intent<'a> {
        workspace_id: lomi_remote_protocol::WorkspaceId,
        repository_id: RepositoryId,
        action: &'a LegacyGitAction<'a>,
        selected_remote_id: Option<RemoteId>,
        selected_ref_id: Option<RefId>,
    }
    let bytes = serde_json::to_vec(&Intent {
        workspace_id: binding.workspace.context.workspace_id.clone(),
        repository_id,
        action,
        selected_remote_id: selected_reference.map(|selected| selected.remote_id.clone()),
        selected_ref_id: selected_reference.map(|selected| selected.ref_id.clone()),
    })
    .map_err(|_| "The Git operation could not be fingerprinted.")?;
    Ok(hash(&bytes))
}

fn request_fingerprint(call: &RpcCall) -> Result<String, String> {
    let mut value =
        serde_json::to_value(call).map_err(|_| "The Git operation could not be fingerprinted.")?;
    let mutation = value
        .get_mut("params")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|params| params.get_mut("mutation"))
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("The Git operation has an invalid mutation context.")?;
    mutation.remove("operationId");
    let bytes =
        serde_json::to_vec(&value).map_err(|_| "The Git operation could not be fingerprinted.")?;
    Ok(hash(&bytes))
}

fn repository_key(binding: &NativeGitBinding, repository_id: RepositoryId) -> String {
    let workspace_id: String = binding.workspace.context.workspace_id.clone().into();
    let repository_id: String = repository_id.into();
    hash(format!("{workspace_id}:{repository_id}").as_bytes())
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn journal_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("git-native-effects.json"))
}

fn read_journal(path: &Path) -> Result<GitEffectJournal, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(GitEffectJournal::default());
        }
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > JOURNAL_LIMIT {
        return Err("The pending Git operation journal is unavailable or invalid.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.mode() & 0o077 != 0
        {
            return Err("The pending Git operation journal is not owner-private.".into());
        }
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = options.open(path).map_err(|error| error.to_string())?;
    let opened = file.metadata().map_err(|error| error.to_string())?;
    if !opened.is_file() || opened.file_type().is_symlink() {
        return Err("The pending Git operation journal is not a regular file.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.dev() != metadata.dev()
            || opened.ino() != metadata.ino()
            || opened.uid() != unsafe { libc::geteuid() }
            || opened.nlink() != 1
            || opened.mode() & 0o077 != 0
        {
            return Err("The pending Git operation journal changed while it was opened.".into());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if opened.file_attributes() & 0x400 != 0 {
            return Err("The pending Git operation journal is not a regular file.".into());
        }
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    Read::by_ref(&mut file)
        .take(JOURNAL_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > JOURNAL_LIMIT {
        return Err("The pending Git operation journal is too large.".into());
    }
    let journal: GitEffectJournal = serde_json::from_slice(&bytes)
        .map_err(|_| "The pending Git operation journal is invalid.")?;
    if journal.version != 1 || journal.operations.len() > JOURNAL_ENTRIES {
        return Err("The pending Git operation journal has an unsupported version.".into());
    }
    if journal.operations.keys().any(|key| !valid_hash(key))
        || journal.operations.values().any(|pending| {
            !valid_hash(&pending.intent_fingerprint) || !valid_hash(&pending.request_fingerprint)
        })
    {
        return Err("The pending Git operation journal contains invalid identifiers.".into());
    }
    let operation_ids = journal
        .operations
        .values()
        .map(|pending| pending.operation_id)
        .collect::<HashSet<_>>();
    if operation_ids.len() != journal.operations.len() {
        return Err("The pending Git operation journal reuses an operation ID.".into());
    }
    Ok(journal)
}

fn write_journal(path: &Path, journal: &GitEffectJournal) -> Result<(), String> {
    crate::files::write_json(path, journal, JOURNAL_LIMIT as usize)
        .map_err(|error| error.to_string())
}

fn clear_operation(path: &Path, journal: &mut GitEffectJournal, key: &str) -> Result<(), String> {
    journal.operations.remove(key);
    write_journal(path, journal)
}

fn new_operation_id() -> Result<OperationId, String> {
    OperationId::try_from(new_uuid()?).map_err(|_| "Git operation IDs are unavailable.".into())
}

fn new_request_id() -> Result<String, String> {
    let request_id = new_uuid()?;
    RequestId::try_from(request_id.clone())
        .map_err(|_| "Git request IDs are unavailable.".to_owned())?;
    Ok(request_id)
}

fn new_uuid() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "Secure randomness is unavailable.".to_owned())?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    ))
}
