use super::*;
use crate::project_files::ProjectDirectory;

#[derive(Default)]
pub struct FileSearchBatch {
    pub matches: Vec<FileSearchMatch>,
    pub limited: bool,
    pub skipped: u32,
    pub files_scanned: u32,
}
pub type FilesSearchDispatch = Arc<
    dyn Fn(
            &ProjectDirectory,
            &FilesSearchInput,
            &dyn Fn() -> Result<(), ErrorCode>,
        ) -> Result<FileSearchBatch, ErrorCode>
        + Send
        + Sync,
>;
pub(super) struct CachedSearch {
    pub owner: String,
    input: FilesSearchInput,
    directory: Arc<ProjectDirectory>,
    policy: u64,
    expires: Instant,
    batch: FileSearchBatch,
}
impl Broker {
    pub fn set_files_search_dispatch(&self, dispatch: FilesSearchDispatch) -> io::Result<()> {
        *self.files_search_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    pub(super) fn files_search(&self, owner: &str, input: FilesSearchInput) -> Reply {
        if !(1..=200).contains(&input.limit)
            || input.query.text.is_empty()
            || input.query.text.len() > 1024
            || input.query.text.contains(['\r', '\n'])
            || input.query.include.len() > 4096
            || input.query.exclude.len() > 4096
            || input.cursor.as_ref().is_some_and(|c| c.len() > 80)
        {
            return error(ErrorCode::ResourceExhausted);
        }
        if !input.relative_directory.is_empty() {
            if let Err(e) = crate::project_files::validate_relative(&input.relative_directory) {
                return error(e);
            }
        }
        let (directory, connected, policy) = {
            let Ok(mut state) = self.lock_state() else {
                return error(ErrorCode::ControlRevoked);
            };
            let policy = state.policy_revision;
            state
                .file_searches
                .retain(|_, b| b.expires > Instant::now() && b.policy == policy);
            let directory = match Self::project_file_access(&state, owner, &input.workspace_id) {
                Ok(d) => d,
                Err(e) => return error(e),
            };
            if let Some(cursor) = &input.cursor {
                let Some((id, offset)) = cursor.split_once(':') else {
                    return error(ErrorCode::CursorExpired);
                };
                let Some(cached) = state.file_searches.get(id).filter(|b| {
                    b.owner == owner
                        && Arc::ptr_eq(&directory, &b.directory)
                        && b.input.workspace_id == input.workspace_id
                        && b.input.relative_directory == input.relative_directory
                        && b.input.query == input.query
                }) else {
                    return error(ErrorCode::CursorExpired);
                };
                let Some(offset) = offset.parse::<usize>().ok().filter(|n| {
                    n.to_string() == offset && *n > 0 && *n < cached.batch.matches.len()
                }) else {
                    return error(ErrorCode::CursorExpired);
                };
                if let Err(e) = directory.check() {
                    return error(e);
                }
                if self.authorization.load(Ordering::SeqCst) != policy {
                    return error(ErrorCode::ControlRevoked);
                }
                return page(id, cached, offset, input.limit);
            }
            if state.file_searches.len() >= 8 {
                return error(ErrorCode::ResourceExhausted);
            }
            (directory, state.sessions[owner].alive.clone(), policy)
        };
        let _producer = match self.file_reads.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => return error(ErrorCode::ResourceExhausted),
        };
        let Some(dispatch) = self
            .files_search_dispatch
            .lock()
            .ok()
            .and_then(|d| d.clone())
        else {
            return error(ErrorCode::UnsupportedCapability);
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        let check = || {
            if !connected.load(Ordering::SeqCst)
                || self.authorization.load(Ordering::SeqCst) != policy
            {
                return Err(ErrorCode::ControlRevoked);
            }
            if Instant::now() >= deadline {
                return Err(ErrorCode::DeadlineExceeded);
            }
            Ok(())
        };
        let batch = match check().and_then(|()| dispatch(&directory, &input, &check)) {
            Ok(b) => b,
            Err(e) => return error(e),
        };
        let mut bytes = 0;
        for m in &batch.matches {
            if crate::project_files::validate_relative(&m.relative_path).is_err()
                || (!input.relative_directory.is_empty()
                    && !m
                        .relative_path
                        .starts_with(&format!("{}/", input.relative_directory)))
                || m.disk_revision.len() != 64
                || !m
                    .disk_revision
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || m.line == 0
                || m.column == 0
                || m.preview.len() > 1204
            {
                return error(ErrorCode::OutcomeUnknown);
            }
            bytes += serde_json::to_vec(m).map_or(131073, |v| v.len() + 1);
        }
        if batch.matches.len() > 256 || bytes > 131072 || batch.files_scanned > 10000 {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        let current = match Self::project_file_access(&state, owner, &input.workspace_id) {
            Ok(d) => d,
            Err(e) => return error(e),
        };
        if !Arc::ptr_eq(&current, &directory) {
            return error(ErrorCode::ControlRevoked);
        }
        if let Err(e) = check() {
            return error(e);
        }
        if state.file_searches.len() >= 8 {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(id) = new_id() else {
            return error(ErrorCode::ResourceExhausted);
        };
        let cached = CachedSearch {
            owner: owner.into(),
            input,
            directory,
            policy,
            expires: Instant::now() + Duration::from_secs(60),
            batch,
        };
        let reply = page(&id, &cached, 0, cached.input.limit);
        if matches!(&reply, Reply::Ok { data: Data::FilesSearch(p), .. } if p.next_cursor.is_some())
        {
            state.file_searches.insert(id, cached);
        }
        reply
    }
}
fn page(id: &str, cached: &CachedSearch, offset: usize, limit: u16) -> Reply {
    let mut result = FilesSearch {
        workspace_id: cached.input.workspace_id.clone(),
        relative_directory: cached.input.relative_directory.clone(),
        matches: Vec::new(),
        next_cursor: None,
        limited: cached.batch.limited,
        skipped: cached.batch.skipped,
        files_scanned: cached.batch.files_scanned,
        filtered: true,
        consistency: SearchConsistency::PerFileSnapshot,
        filter_policy: SearchFilterPolicy::ExplicitPatternsAndSecretPaths,
    };
    let mut bytes = serde_json::to_vec(&result).map_or(MAX_METADATA_BYTES, |v| v.len()) + 128;
    for m in cached
        .batch
        .matches
        .iter()
        .skip(offset)
        .take(usize::from(limit))
    {
        let size = serde_json::to_vec(m).map_or(MAX_METADATA_BYTES, |v| v.len() + 1);
        if bytes + size > 49152 {
            break;
        }
        bytes += size;
        result.matches.push(m.clone());
    }
    let next = offset + result.matches.len();
    if next < cached.batch.matches.len() {
        if next == offset {
            return error(ErrorCode::ResourceExhausted);
        }
        result.next_cursor = Some(format!("{id}:{next}"));
    }
    Reply::ok(Data::FilesSearch(Box::new(result)))
}
