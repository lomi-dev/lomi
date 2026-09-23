use super::*;
use sha2::{Digest, Sha256};

pub struct DecodedFile {
    pub content: String,
    pub encoding: TextEncoding,
    pub line_endings: TextLineEndings,
    pub total_utf16: u32,
    pub next_utf16: Option<u32>,
}
pub type FilesReadDispatch = Arc<
    dyn Fn(
            &[u8],
            &FilesReadInput,
            &dyn Fn() -> Result<(), ErrorCode>,
        ) -> Result<DecodedFile, ErrorCode>
        + Send
        + Sync,
>;

impl Broker {
    pub fn set_files_read_dispatch(&self, dispatch: FilesReadDispatch) -> io::Result<()> {
        *self.files_read_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    pub(super) fn files_read(&self, owner: &str, input: FilesReadInput) -> Reply {
        if !(2..=8192).contains(&input.max_chars)
            || (input.start_utf16 > 0 && input.expected_disk_revision.is_none())
            || input.expected_disk_revision.as_ref().is_some_and(|s| {
                s.len() != 64
                    || !s
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
        {
            return error(ErrorCode::ResourceExhausted);
        }
        if let Err(e) = crate::project_files::validate_relative(&input.relative_path) {
            return error(e);
        }
        let (directory, connected, policy_revision) = {
            let Ok(state) = self.lock_state() else {
                return error(ErrorCode::ControlRevoked);
            };
            let directory = match Self::project_file_access(&state, owner, &input.workspace_id) {
                Ok(d) => d,
                Err(e) => return error(e),
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
        let Some(dispatch) = self.files_read_dispatch.lock().ok().and_then(|d| d.clone()) else {
            return error(ErrorCode::UnsupportedCapability);
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        let check = || {
            if !connected.load(Ordering::SeqCst)
                || self.authorization.load(Ordering::SeqCst) != policy_revision
            {
                return Err(ErrorCode::ControlRevoked);
            }
            if Instant::now() >= deadline {
                return Err(ErrorCode::DeadlineExceeded);
            }
            Ok(())
        };
        let read = || -> Result<FileText, ErrorCode> {
            check()?;
            let bytes = directory
                .open_file(&input.relative_path, 4 * 1024 * 1024)
                .map_err(|e| {
                    if e == ErrorCode::ArtifactTooLarge {
                        ErrorCode::ResourceExhausted
                    } else {
                        e
                    }
                })?
                .read_bytes(4 * 1024 * 1024, check)?;
            let revision = format!("{:x}", Sha256::digest(&bytes));
            if input
                .expected_disk_revision
                .as_ref()
                .is_some_and(|r| r != &revision)
            {
                return Err(ErrorCode::RevisionConflict);
            }
            check()?;
            let decoded = dispatch(&bytes, &input, &check)?;
            check()?;
            directory.check()?;
            let units = decoded.content.encode_utf16().count() as u32;
            let end = input
                .start_utf16
                .checked_add(units)
                .ok_or(ErrorCode::ResourceExhausted)?;
            if units > u32::from(input.max_chars)
                || end > decoded.total_utf16
                || decoded.content.len() > 32768
                || decoded.next_utf16 != (end < decoded.total_utf16).then_some(end)
                || (units == 0 && end < decoded.total_utf16)
            {
                return Err(ErrorCode::OutcomeUnknown);
            }
            Ok(FileText {
                workspace_id: input.workspace_id.clone(),
                relative_path: input.relative_path.clone(),
                source: DiskSource::Disk,
                disk_revision: revision,
                encoding: decoded.encoding,
                line_endings: decoded.line_endings,
                content: decoded.content,
                start_utf16: input.start_utf16,
                total_utf16: decoded.total_utf16,
                next_utf16: decoded.next_utf16,
                truncated: decoded.next_utf16.is_some(),
            })
        };
        let output = match read() {
            Ok(r) => r,
            Err(e) => return error(e),
        };
        if serde_json::to_vec(&output).map_or(true, |v| v.len() > MAX_METADATA_BYTES) {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(state) = self.lock_state() else {
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
        Reply::ok(Data::FileText(Box::new(output)))
    }
}

impl Broker {
    pub(super) fn files_list(&self, owner: &str, input: FilesListInput) -> Reply {
        if !(1..=200).contains(&input.limit) || input.cursor.as_ref().is_some_and(|c| c.len() > 96)
        {
            return error(ErrorCode::ResourceExhausted);
        }
        if !input.relative_directory.is_empty() {
            if let Err(e) = crate::project_files::validate_relative(&input.relative_directory) {
                return error(e);
            }
        }
        let view = format!("files:{}:{}", input.workspace_id, input.relative_directory);
        let (directory, connected, policy, previous) = {
            let Ok(state) = self.lock_state() else {
                return error(ErrorCode::ControlRevoked);
            };
            let directory = match Self::project_file_access(&state, owner, &input.workspace_id) {
                Ok(d) => d,
                Err(e) => return error(e),
            };
            let session = &state.sessions[owner];
            let previous = if let Some(cursor) = &input.cursor {
                let Some(c) = session.cursors.get(cursor).filter(|c| c.view == view) else {
                    return error(ErrorCode::CursorExpired);
                };
                Some((c.revision.clone(), c.offset))
            } else {
                None
            };
            (
                directory,
                session.alive.clone(),
                state.policy_revision,
                previous,
            )
        };
        let _producer = match self.file_reads.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => return error(ErrorCode::ResourceExhausted),
        };
        let deadline = Instant::now() + Duration::from_secs(5);
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
        let snapshot = match directory.list(&input.relative_directory, check) {
            Ok(s) => s,
            Err(e) => return error(e),
        };
        let offset = if let Some((revision, offset)) = previous {
            if snapshot.revision != revision || offset >= snapshot.entries.len() {
                return error(ErrorCode::CursorExpired);
            }
            offset
        } else {
            0
        };
        let mut result = FilesList {
            workspace_id: input.workspace_id.clone(),
            relative_directory: input.relative_directory,
            directory_revision: snapshot.revision.clone(),
            entries: Vec::new(),
            next_cursor: None,
            filtered: true,
        };
        let mut bytes = serde_json::to_vec(&result).map_or(MAX_METADATA_BYTES, |v| v.len()) + 128;
        for entry in snapshot
            .entries
            .iter()
            .skip(offset)
            .take(usize::from(input.limit))
        {
            let size = serde_json::to_vec(entry).map_or(MAX_METADATA_BYTES, |v| v.len() + 1);
            if bytes + size > 49152 {
                break;
            }
            bytes += size;
            result.entries.push(entry.clone());
        }
        let next = offset + result.entries.len();
        if next == offset && next < snapshot.entries.len() {
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
        if next < snapshot.entries.len() {
            let Ok(id) = new_id() else {
                return error(ErrorCode::ResourceExhausted);
            };
            let cursors = &mut state.sessions.get_mut(owner).unwrap().cursors;
            if cursors.len() >= 64 {
                cursors.clear();
            }
            cursors.insert(
                id.clone(),
                Cursor {
                    revision: snapshot.revision,
                    offset: next,
                    view,
                },
            );
            result.next_cursor = Some(id);
        }
        Reply::ok(Data::FilesList(Box::new(result)))
    }
}
