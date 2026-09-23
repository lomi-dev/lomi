use super::{
    git::{joined, visible},
    *,
};
use crate::{
    git_read::{self, Observation},
    project_files::{validate_relative, ProjectDirectory},
};

type Check<'a> = &'a dyn Fn() -> Result<(), ErrorCode>;
fn sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|c| c.is_ascii_hexdigit())
}
fn revision(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn utf8(bytes: &[u8]) -> Result<&str, ErrorCode> {
    std::str::from_utf8(bytes).map_err(|_| ErrorCode::UnsupportedCapability)
}
fn parents(value: &str) -> Result<Vec<String>, ErrorCode> {
    let values: Vec<_> = value.split_whitespace().collect();
    if values.len() > 128 || values.iter().any(|s| !sha(s)) {
        return Err(ErrorCode::ResourceExhausted);
    }
    Ok(values.into_iter().map(str::to_string).collect())
}
fn text_page(text: &str, start: u32, max: u16) -> Result<(String, u32, Option<u32>), ErrorCode> {
    let mut position = 0_u32;
    let mut begin = None;
    let mut end = text.len();
    let mut end_units = start;
    for (byte, c) in text.char_indices() {
        if position == start {
            begin = Some(byte);
        }
        if position < start && position + c.len_utf16() as u32 > start {
            return Err(ErrorCode::RevisionConflict);
        }
        if position >= start && position + c.len_utf16() as u32 - start > u32::from(max) {
            end = byte;
            end_units = position;
            break;
        }
        position += c.len_utf16() as u32;
        end_units = position;
    }
    let total = text.encode_utf16().count() as u32;
    if start > total {
        return Err(ErrorCode::RevisionConflict);
    }
    if start == total {
        return Ok((String::new(), total, None));
    }
    let begin = begin.ok_or(ErrorCode::RevisionConflict)?;
    Ok((
        text[begin..end].into(),
        total,
        (end_units < total).then_some(end_units),
    ))
}
impl Broker {
    fn git_observe(
        &self,
        owner: &str,
        workspace: &str,
        repository: &str,
        observe: impl FnOnce(&ProjectDirectory, Check<'_>) -> Result<Data, ErrorCode>,
    ) -> Reply {
        if !repository.is_empty() {
            if let Err(code) = validate_relative(repository) {
                return error(code);
            }
        }
        let (directory, alive, policy) = {
            let Ok(state) = self.lock_state() else {
                return error(ErrorCode::ControlRevoked);
            };
            let directory = match Self::git_access(&state, owner, workspace) {
                Ok(d) => d,
                Err(code) => return error(code),
            };
            (
                directory,
                state.sessions[owner].alive.clone(),
                state.policy_revision,
            )
        };
        let _producer = match self.file_reads.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => return error(ErrorCode::ResourceExhausted),
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        let check = || {
            if !alive.load(Ordering::SeqCst) || self.authorization.load(Ordering::SeqCst) != policy
            {
                return Err(ErrorCode::ControlRevoked);
            }
            if Instant::now() >= deadline {
                return Err(ErrorCode::DeadlineExceeded);
            }
            Ok(())
        };
        let output = match observe(&directory, &check) {
            Ok(data) => data,
            Err(code) => return error(code),
        };
        if serde_json::to_vec(&output).map_or(true, |v| v.len() > MAX_METADATA_BYTES) {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        let current = match Self::git_access(&state, owner, workspace) {
            Ok(d) => d,
            Err(code) => return error(code),
        };
        if !Arc::ptr_eq(&directory, &current) || directory.check().is_err() || check().is_err() {
            return error(ErrorCode::ControlRevoked);
        }
        Reply::ok(output)
    }
    pub(super) fn git_diff(&self, owner: &str, input: GitDiffInput) -> Reply {
        if !(2..=8192).contains(&input.max_chars)
            || input.start_utf16 > 2 * 1024 * 1024
            || (input.start_utf16 > 0 && input.expected_observation_revision.is_none())
            || input
                .expected_observation_revision
                .as_ref()
                .is_some_and(|s| !revision(s))
        {
            return error(ErrorCode::ResourceExhausted);
        }
        if let Err(code) = validate_relative(&input.relative_path) {
            return error(code);
        }
        if (input.comparison == GitComparison::Commit) != input.commit.is_some()
            || input.commit.as_ref().is_some_and(|s| !sha(s))
        {
            return error(ErrorCode::ScopeDenied);
        }
        self.git_observe(
            owner,
            &input.workspace_id,
            &input.repository_relative,
            |directory, check| observe_diff(&input, directory, check, false),
        )
    }
    pub(super) fn git_history(&self, owner: &str, input: GitHistoryInput) -> Reply {
        if !(1..=100).contains(&input.limit)
            || input.skip > 10000
            || (input.skip > 0 && input.commit.is_none())
        {
            return error(ErrorCode::ResourceExhausted);
        }
        if input.commit.as_ref().is_some_and(|s| !sha(s)) {
            return error(ErrorCode::ScopeDenied);
        }
        self.git_observe(
            owner,
            &input.workspace_id,
            &input.repository_relative,
            |directory, check| {
                let bytes = git_read::read(
                    directory,
                    &input.repository_relative,
                    Observation::History {
                        commit: input.commit.as_deref(),
                        skip: input.skip,
                        limit: input.limit,
                    },
                    check,
                )?;
                let mut records = Vec::new();
                if !bytes.is_empty() {
                    let text = utf8(&bytes)?;
                    let mut parts: Vec<_> = text.split('\0').collect();
                    if parts.pop() != Some("") {
                        return Err(ErrorCode::UnsupportedCapability);
                    }
                    // Git -z adds its record terminator after the format's final NUL.
                    for row in parts.chunks(6) {
                        if row.len() != 6
                            || !row[5].is_empty()
                            || !sha(row[0])
                            || row[2].len() > 4096
                            || row[3].len() > 64
                            || row[4].len() > 8192
                        {
                            return Err(ErrorCode::UnsupportedCapability);
                        }
                        records.push(GitCommitSummary {
                            commit: row[0].into(),
                            parents: parents(row[1])?,
                            author: row[2].into(),
                            authored_at: row[3].into(),
                            subject: row[4].into(),
                        });
                    }
                }
                let start_commit = input
                    .commit
                    .clone()
                    .or_else(|| records.first().map(|r| r.commit.clone()))
                    .ok_or(ErrorCode::TargetNotFound)?;
                let full = records.len() == usize::from(input.limit);
                let mut commits = Vec::new();
                let mut size = 1024;
                for record in records {
                    size += serde_json::to_vec(&record)
                        .map_err(|_| ErrorCode::ResourceExhausted)?
                        .len()
                        + 1;
                    if size > 48 * 1024 {
                        break;
                    }
                    commits.push(record);
                }
                if commits.is_empty() && !bytes.is_empty() {
                    return Err(ErrorCode::ResourceExhausted);
                }
                let next_skip =
                    (full || size > 48 * 1024).then_some(input.skip + commits.len() as u32);
                Ok(Data::GitHistory(Box::new(GitHistoryPage {
                    workspace_id: input.workspace_id.clone(),
                    repository_relative: input.repository_relative.clone(),
                    source: GitSource::Git,
                    consistency: GitConsistency::PerCommandSnapshot,
                    start_commit,
                    commits,
                    skip: input.skip,
                    next_skip,
                })))
            },
        )
    }
    pub(super) fn git_commit(&self, owner: &str, input: GitCommitInput) -> Reply {
        if !sha(&input.commit) {
            return error(ErrorCode::ScopeDenied);
        }
        if !(1..=200).contains(&input.files_limit) || input.files_skip > 4096 {
            return error(ErrorCode::ResourceExhausted);
        }
        self.git_observe(
            owner,
            &input.workspace_id,
            &input.repository_relative,
            |directory, check| observe_commit(&input, directory, check, false),
        )
    }
    pub(super) fn git_remotes(&self, owner: &str, input: GitRemotesInput) -> Reply {
        self.git_observe(
            owner,
            &input.workspace_id,
            &input.repository_relative,
            |directory, check| {
                let bytes = git_read::read(
                    directory,
                    &input.repository_relative,
                    Observation::Remotes,
                    check,
                )?;
                let mut remotes = Vec::new();
                if !bytes.is_empty() {
                    for entry in utf8(&bytes)?
                        .strip_suffix('\0')
                        .ok_or(ErrorCode::UnsupportedCapability)?
                        .split('\0')
                    {
                        let (key, value) = entry
                            .split_once('\n')
                            .ok_or(ErrorCode::UnsupportedCapability)?;
                        let key = key
                            .strip_prefix("remote.")
                            .ok_or(ErrorCode::UnsupportedCapability)?;
                        let (name, role) = if let Some(name) = key.strip_suffix(".pushurl") {
                            (name, GitRemoteRole::Push)
                        } else {
                            (
                                key.strip_suffix(".url")
                                    .ok_or(ErrorCode::UnsupportedCapability)?,
                                GitRemoteRole::Fetch,
                            )
                        };
                        if name.is_empty()
                            || name.len() > 128
                            || name.chars().any(char::is_control)
                            || remotes.len() >= 64
                        {
                            return Err(ErrorCode::ResourceExhausted);
                        }
                        let (transport, host, port) = remote_location(value);
                        remotes.push(GitRemote {
                            name: name.into(),
                            role,
                            transport,
                            host,
                            port,
                            location_redacted: true,
                        });
                    }
                }
                Ok(Data::GitRemotes(Box::new(GitRemotes {
                    workspace_id: input.workspace_id.clone(),
                    repository_relative: input.repository_relative.clone(),
                    source: GitSource::Git,
                    remotes,
                })))
            },
        )
    }
}
fn remote_location(raw: &str) -> (String, Option<String>, Option<u16>) {
    if (raw.contains("::") && !raw.contains("://") && !raw.starts_with('['))
        || raw.chars().any(char::is_control)
    {
        return ("unsupported".into(), None, None);
    }
    if let Ok(url) = url::Url::parse(raw) {
        if matches!(url.scheme(), "http" | "https" | "ssh" | "git" | "rsync")
            && url.host_str().is_some()
        {
            return (
                url.scheme().into(),
                url.host_str().map(str::to_string),
                url.port(),
            );
        }
        if url.scheme() == "file" {
            return ("local".into(), None, None);
        }
    }
    if raw.contains("://") {
        return ("unsupported".into(), None, None);
    }
    // Git's scp-like form is recognized only without a slash before the colon.
    // Parsing through URL validates the authority; no userinfo or path is kept.
    if let Some((authority, _)) = raw.split_once(':') {
        if !authority.is_empty()
            && !authority.contains('/')
            && !authority.contains('\\')
            && !authority.contains(['?', '#'])
            && !authority.chars().any(char::is_whitespace)
        {
            if let Ok(url) = url::Url::parse(&format!("ssh://{authority}/")) {
                if let Some(host) = url.host_str() {
                    return ("ssh".into(), Some(host.into()), url.port());
                }
            }
        }
    }
    (
        if raw.starts_with('/') || raw.starts_with('.') {
            "local"
        } else {
            "unsupported"
        }
        .into(),
        None,
        None,
    )
}

fn commit_files(
    directory: &ProjectDirectory,
    repository: &str,
    raw: &[u8],
) -> Result<(Vec<GitCommitFile>, u32), ErrorCode> {
    if raw.is_empty() {
        return Ok((Vec::new(), 0));
    }
    let text = utf8(raw)?
        .strip_suffix('\0')
        .ok_or(ErrorCode::UnsupportedCapability)?;
    let records: Vec<_> = text.split('\0').collect();
    if records.len() % 2 != 0 || records.len() > 8192 {
        return Err(ErrorCode::ResourceExhausted);
    }
    let mut files = Vec::new();
    let mut omitted = 0;
    for row in records.as_chunks::<2>().0 {
        let fields: Vec<_> = row[0].split(' ').collect();
        if fields.len() != 5
            || !fields[0].starts_with(':')
            || fields[4].len() != 1
            || !b"AMDTUXB".contains(&fields[4].as_bytes()[0])
        {
            return Err(ErrorCode::UnsupportedCapability);
        }
        let old_mode = &fields[0][1..];
        let new_mode = fields[1];
        if [old_mode, new_mode]
            .iter()
            .any(|m| !matches!(*m, "000000" | "100644" | "100755"))
            || !visible(directory, &joined(repository, row[1]), false)
        {
            omitted += 1;
            continue;
        }
        files.push(GitCommitFile {
            relative_path: row[1].into(),
            status: fields[4].into(),
            old_mode: old_mode.into(),
            new_mode: new_mode.into(),
        });
    }
    Ok((files, omitted))
}

pub(super) fn observe_diff(
    input: &GitDiffInput,
    directory: &ProjectDirectory,
    check: Check<'_>,
    full: bool,
) -> Result<Data, ErrorCode> {
    let path = joined(&input.repository_relative, &input.relative_path);
    if !visible(directory, &path, false) {
        return Err(ErrorCode::ScopeDenied);
    }
    let status = if input.commit.is_none() {
        git_read::read(
            directory,
            &input.repository_relative,
            Observation::PathStatus {
                relative: &input.relative_path,
            },
            check,
        )?
    } else {
        Vec::new()
    };
    let observation = match input.commit.as_deref() {
        Some(commit) => Observation::CommitDiff {
            relative: &input.relative_path,
            commit,
        },
        None => Observation::Diff {
            relative: &input.relative_path,
            staged: input.comparison == GitComparison::Staged,
        },
    };
    let bytes = git_read::read(directory, &input.repository_relative, observation, check)?;
    let text = git_read::guarded_patch(&bytes)?;
    let mut digest = Sha256::new();
    digest.update(&status);
    digest.update([0]);
    digest.update(&bytes);
    let observation_revision = format!("{:x}", digest.finalize());
    if input
        .expected_observation_revision
        .as_ref()
        .is_some_and(|s| s != &observation_revision)
    {
        return Err(ErrorCode::RevisionConflict);
    }
    let notice = if status.starts_with(b"?? ") && input.comparison == GitComparison::Worktree {
        Some(GitDiffNotice::Untracked)
    } else if text.lines().any(|line| line.starts_with("Binary files ")) {
        Some(GitDiffNotice::Binary)
    } else {
        None
    };
    let (patch, total_utf16, next_utf16) = if full {
        (text.to_string(), text.encode_utf16().count() as u32, None)
    } else {
        text_page(text, input.start_utf16, input.max_chars)?
    };
    check()?;
    if !visible(directory, &path, false) {
        return Err(ErrorCode::RevisionConflict);
    }
    Ok(Data::GitDiff(Box::new(GitDiffPage {
        workspace_id: input.workspace_id.clone(),
        repository_relative: input.repository_relative.clone(),
        relative_path: input.relative_path.clone(),
        comparison: input.comparison,
        commit: input.commit.clone(),
        source: GitSource::Git,
        consistency: GitConsistency::PerCommandSnapshot,
        observation_revision,
        patch,
        start_utf16: input.start_utf16,
        total_utf16,
        next_utf16,
        notice,
    })))
}

pub(super) fn observe_commit(
    input: &GitCommitInput,
    directory: &ProjectDirectory,
    check: Check<'_>,
    full: bool,
) -> Result<Data, ErrorCode> {
    let bytes = git_read::read(
        directory,
        &input.repository_relative,
        Observation::Commit {
            commit: &input.commit,
        },
        check,
    )?;
    let text = utf8(&bytes)?
        .strip_suffix('\0')
        .ok_or(ErrorCode::UnsupportedCapability)?;
    let parts: Vec<_> = text.splitn(9, '\0').collect();
    if parts.len() != 9
        || !parts[0].eq_ignore_ascii_case(&input.commit)
        || parts[8].contains('\0')
        || parts[8].len() > 65536
        || [2, 3, 5, 6].iter().any(|n| parts[*n].len() > 4096)
        || [4, 7].iter().any(|n| parts[*n].len() > 64)
    {
        return Err(ErrorCode::ResourceExhausted);
    }
    let raw = git_read::read(
        directory,
        &input.repository_relative,
        Observation::CommitFiles {
            commit: &input.commit,
        },
        check,
    )?;
    let (files, omitted_entries) = commit_files(directory, &input.repository_relative, &raw)?;
    if input.files_skip as usize > files.len() {
        return Err(ErrorCode::RevisionConflict);
    }
    let mut details = GitCommitDetails {
        workspace_id: input.workspace_id.clone(),
        repository_relative: input.repository_relative.clone(),
        source: GitSource::Git,
        commit: parts[0].into(),
        parents: parents(parts[1])?,
        author: parts[2].into(),
        author_email: parts[3].into(),
        authored_at: parts[4].into(),
        committer: parts[5].into(),
        committer_email: parts[6].into(),
        committed_at: parts[7].into(),
        message: parts[8].into(),
        files: Vec::new(),
        files_skip: input.files_skip,
        next_files_skip: None,
        omitted_entries,
    };
    if full {
        details.files = files;
        return Ok(Data::GitCommit(Box::new(details)));
    }
    let mut size = serde_json::to_vec(&details)
        .map_err(|_| ErrorCode::ResourceExhausted)?
        .len()
        + 256;
    if size > MAX_METADATA_BYTES {
        return Err(ErrorCode::ResourceExhausted);
    }
    for file in files
        .iter()
        .skip(input.files_skip as usize)
        .take(usize::from(input.files_limit))
    {
        size += serde_json::to_vec(file)
            .map_err(|_| ErrorCode::ResourceExhausted)?
            .len()
            + 1;
        if size > MAX_METADATA_BYTES {
            break;
        }
        details.files.push(file.clone());
    }
    let next = input.files_skip + details.files.len() as u32;
    if (next as usize) < files.len() {
        if next == input.files_skip {
            return Err(ErrorCode::ResourceExhausted);
        }
        details.next_files_skip = Some(next);
    }
    Ok(Data::GitCommit(Box::new(details)))
}
