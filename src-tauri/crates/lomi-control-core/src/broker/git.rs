use super::*;
use crate::{
    git_read::{self, Observation},
    project_files::{validate_relative, ProjectDirectory},
};

pub(super) struct CachedStatus {
    owner: String,
    input: GitStatusInput,
    directory: Arc<ProjectDirectory>,
    policy: u64,
    expires: Instant,
    page: GitStatusPage,
}
pub(super) fn joined(repository: &str, relative: &str) -> String {
    if repository.is_empty() {
        relative.into()
    } else {
        format!("{repository}/{relative}")
    }
}
pub(super) fn visible(directory: &ProjectDirectory, relative: &str, folder: bool) -> bool {
    use rustix::fs::{openat, statat, AtFlags, FileType, Mode, OFlags};
    if validate_relative(relative).is_err() {
        return false;
    }
    let Ok(mut parent) = directory.open_directory("") else {
        return false;
    };
    let mut parts = relative.split('/').peekable();
    while let Some(name) = parts.next() {
        let metadata = match statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(metadata) => metadata,
            Err(rustix::io::Errno::NOENT) => return !folder,
            Err(_) => return false,
        };
        let kind = FileType::from_raw_mode(metadata.st_mode);
        if parts.peek().is_none() {
            return if folder {
                kind == FileType::Directory
            } else {
                kind == FileType::RegularFile && metadata.st_nlink == 1
            };
        }
        if kind != FileType::Directory {
            return false;
        }
        let Ok(next) = openat(
            &parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) else {
            return false;
        };
        parent = std::fs::File::from(next);
    }
    false
}

fn parse(
    directory: &ProjectDirectory,
    repository: &str,
    bytes: &[u8],
) -> Result<(Vec<GitChange>, u32), ErrorCode> {
    if !bytes.is_empty() && !bytes.ends_with(&[0]) {
        return Err(ErrorCode::UnsupportedCapability);
    }
    let mut parts = bytes.split(|b| *b == 0).peekable();
    let mut changes = Vec::new();
    let mut omitted = 0;
    let mut size = 0;
    while let Some(entry) = parts.next() {
        if entry.is_empty() && parts.peek().is_none() {
            break;
        }
        if entry.len() < 4
            || entry[2] != b' '
            || !entry[..2].iter().all(|b| b" MADRCUT?!".contains(b))
        {
            return Err(ErrorCode::UnsupportedCapability);
        }
        let raw = std::str::from_utf8(&entry[3..]).map_err(|_| ErrorCode::UnsupportedCapability)?;
        let folder = raw.ends_with('/');
        let path = joined(repository, raw.strip_suffix('/').unwrap_or(raw));
        let original_path = if entry[..2].iter().any(|b| b"RC".contains(b)) {
            Some(joined(
                repository,
                std::str::from_utf8(parts.next().ok_or(ErrorCode::UnsupportedCapability)?)
                    .map_err(|_| ErrorCode::UnsupportedCapability)?,
            ))
        } else {
            None
        };
        if !visible(directory, &path, folder)
            || original_path
                .as_ref()
                .is_some_and(|p| !visible(directory, p, false))
        {
            omitted += 1;
            continue;
        }
        let change = GitChange {
            relative_path: path,
            original_path,
            index: char::from(entry[0]).to_string(),
            worktree: char::from(entry[1]).to_string(),
            directory: folder,
        };
        size += serde_json::to_vec(&change)
            .map_err(|_| ErrorCode::ResourceExhausted)?
            .len();
        if changes.len() >= 4096 || size > 1024 * 1024 {
            return Err(ErrorCode::ResourceExhausted);
        }
        changes.push(change);
    }
    Ok((changes, omitted))
}
impl Broker {
    pub(super) fn git_status(&self, owner: &str, input: GitStatusInput) -> Reply {
        if !(1..=200).contains(&input.limit) || input.cursor.as_ref().is_some_and(|s| s.len() > 96)
        {
            return error(ErrorCode::ResourceExhausted);
        }
        if !input.repository_relative.is_empty() {
            if let Err(code) = validate_relative(&input.repository_relative) {
                return error(code);
            }
        }
        let (directory, connected, policy) = {
            let Ok(mut state) = self.lock_state() else {
                return error(ErrorCode::ControlRevoked);
            };
            let directory = match Self::git_access(&state, owner, &input.workspace_id) {
                Ok(d) => d,
                Err(e) => return error(e),
            };
            let policy = state.policy_revision;
            state
                .git_statuses
                .retain(|_, s| s.expires > Instant::now() && s.policy == policy);
            if let Some(cursor) = &input.cursor {
                let Some((id, offset)) = cursor.split_once(':') else {
                    return error(ErrorCode::CursorExpired);
                };
                let Some(cached) = state.git_statuses.get(id) else {
                    return error(ErrorCode::CursorExpired);
                };
                let Some(offset) = offset
                    .parse::<usize>()
                    .ok()
                    .filter(|n| *n > 0 && n.to_string() == offset)
                else {
                    return error(ErrorCode::CursorExpired);
                };
                if cached.owner != owner
                    || cached.input.workspace_id != input.workspace_id
                    || cached.input.repository_relative != input.repository_relative
                    || !Arc::ptr_eq(&directory, &cached.directory)
                    || offset >= cached.page.changes.len()
                {
                    return error(ErrorCode::CursorExpired);
                }
                if directory.check().is_err() {
                    return error(ErrorCode::ControlRevoked);
                }
                if self.authorization.load(Ordering::SeqCst) != policy {
                    return error(ErrorCode::ControlRevoked);
                }
                return page(id, cached, offset, input.limit);
            }
            if state.git_statuses.len() >= 4 {
                return error(ErrorCode::ResourceExhausted);
            }
            (directory, state.sessions[owner].alive.clone(), policy)
        };
        let _producer = match self.file_reads.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => return error(ErrorCode::ResourceExhausted),
        };
        let check = || {
            if !connected.load(Ordering::SeqCst)
                || self.authorization.load(Ordering::SeqCst) != policy
            {
                Err(ErrorCode::ControlRevoked)
            } else {
                Ok(())
            }
        };
        let observed = (|| {
            let version = git_read::read(
                &directory,
                &input.repository_relative,
                Observation::Version,
                &check,
            )?;
            let git_version = std::str::from_utf8(&version)
                .map_err(|_| ErrorCode::UnsupportedCapability)?
                .trim();
            if !git_version.starts_with("git version ") || git_version.len() > 128 {
                return Err(ErrorCode::UnsupportedCapability);
            }
            let bytes = git_read::read(
                &directory,
                &input.repository_relative,
                Observation::Status,
                &check,
            )?;
            let observation_revision = format!("{:x}", Sha256::digest(&bytes));
            let (changes, omitted_entries) = parse(&directory, &input.repository_relative, &bytes)?;
            check()?;
            directory.check()?;
            Ok(GitStatusPage {
                workspace_id: input.workspace_id.clone(),
                repository_relative: input.repository_relative.clone(),
                source: GitSource::Git,
                consistency: GitConsistency::PerCommandSnapshot,
                git_version: git_version.into(),
                observation_revision,
                changes,
                omitted_entries,
                next_cursor: None,
            })
        })();
        let result = match observed {
            Ok(result) => result,
            Err(code) => return error(code),
        };
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        let current = match Self::git_access(&state, owner, &input.workspace_id) {
            Ok(d) => d,
            Err(code) => return error(code),
        };
        if !Arc::ptr_eq(&directory, &current) || check().is_err() {
            return error(ErrorCode::ControlRevoked);
        }
        let id = match new_id() {
            Ok(id) => id,
            Err(_) => return error(ErrorCode::ResourceExhausted),
        };
        let cached = CachedStatus {
            owner: owner.into(),
            input,
            directory,
            policy,
            expires: Instant::now() + Duration::from_secs(60),
            page: result,
        };
        let reply = page(&id, &cached, 0, cached.input.limit);
        if matches!(&reply,Reply::Ok {data:Data::GitStatus(p),..} if p.next_cursor.is_some()) {
            if state.git_statuses.len() >= 4 {
                return error(ErrorCode::ResourceExhausted);
            }
            state.git_statuses.insert(id, cached);
        }
        reply
    }
    pub(super) fn git_access(
        state: &State,
        owner: &str,
        workspace: &str,
    ) -> Result<Arc<ProjectDirectory>, ErrorCode> {
        let directory = Self::project_file_access(state, owner, workspace)?;
        if !state.sessions[owner].grant.scopes.contains("git.read") {
            return Err(ErrorCode::ScopeDenied);
        }
        Ok(directory)
    }
}
fn page(id: &str, cached: &CachedStatus, offset: usize, limit: u16) -> Reply {
    let mut page = cached.page.clone();
    page.changes.clear();
    let mut bytes = serde_json::to_vec(&page).map_or(MAX_METADATA_BYTES, |v| v.len()) + 128;
    for change in cached
        .page
        .changes
        .iter()
        .skip(offset)
        .take(usize::from(limit))
    {
        let size = serde_json::to_vec(change).map_or(MAX_METADATA_BYTES, |v| v.len() + 1);
        if bytes + size > 49152 {
            break;
        }
        bytes += size;
        page.changes.push(change.clone());
    }
    let next = offset + page.changes.len();
    if next < cached.page.changes.len() {
        if next == offset {
            return error(ErrorCode::ResourceExhausted);
        }
        page.next_cursor = Some(format!("{id}:{next}"));
    }
    Reply::ok(Data::GitStatus(Box::new(page)))
}
