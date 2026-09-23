use super::{
    operations::{storage_error, UiMutation},
    *,
};

pub(super) fn revision_number(document: &str, revision: &str) -> Option<u64> {
    let n = revision.strip_prefix(document)?.strip_prefix(':')?;
    n.parse::<u64>()
        .ok()
        .filter(|v| v.to_string() == n && *v < 9_007_199_254_740_991)
}
pub(super) fn next_revision(document: &str, previous: &str, next: &str) -> bool {
    revision_number(document, previous)
        .zip(revision_number(document, next))
        .is_some_and(|(a, b)| a + 1 == b)
}
fn valid_edits(input: &EditorEditsInput) -> bool {
    if input.edits.is_empty()
        || input.edits.len() > 64
        || !valid_id(&input.document_id)
        || revision_number(&input.document_id, &input.expected_buffer_revision).is_none()
        || input.expected_disk_revision.len() != 64
        || !input
            .expected_disk_revision
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return false;
    }
    let mut previous: Option<&EditorEdit> = None;
    let mut bytes = 0;
    for edit in &input.edits {
        bytes += edit.insert.len();
        if edit.from_utf16 > edit.to_utf16
            || edit.to_utf16 > 16 * 1024 * 1024
            || edit.insert.contains(['\r', '\0'])
            || bytes > 65536
            || (edit.from_utf16 == edit.to_utf16 && edit.insert.is_empty())
            || previous
                .is_some_and(|p| edit.from_utf16 < p.to_utf16 || edit.from_utf16 <= p.from_utf16)
        {
            return false;
        }
        previous = Some(edit);
    }
    true
}
impl Broker {
    pub(super) fn validate_editor_edits(
        state: &State,
        owner: &str,
        input: &EditorEditsInput,
    ) -> Result<(), ErrorCode> {
        let directory = Self::editor_read_access(state, owner, &input.read_target())?;
        if !state.sessions[owner].grant.scopes.contains("editor.write") {
            return Err(ErrorCode::ScopeDenied);
        }
        crate::project_files::validate_relative(&input.relative_path)?;
        directory.open_file(&input.relative_path, 16 * 1024 * 1024)?;
        Ok(())
    }
    pub(super) fn editor_edits(self: &Arc<Self>, owner: &str, input: EditorEditsInput) -> Reply {
        if !valid_edits(&input)
            || !valid_id(&input.request_key)
            || input.expected_revision.parse::<u64>().is_err()
        {
            return error(ErrorCode::ResourceExhausted);
        }
        if let Err(e) = crate::project_files::validate_relative(&input.relative_path) {
            return error(e);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        let directory = match Self::project_file_access(&state, owner, &input.workspace_id) {
            Ok(d) => d,
            Err(e) => return error(e),
        };
        let session = &state.sessions[owner];
        if !["editor.read", "editor.write"]
            .iter()
            .all(|s| session.grant.scopes.contains(*s))
        {
            return error(ErrorCode::ScopeDenied);
        }
        if input.retry_epoch != session.retry_epoch {
            return error(ErrorCode::RetryWindowExpired);
        }
        let Some(root) = session.grant.workspace(&input.workspace_id) else {
            return error(ErrorCode::TargetNotFound);
        };
        let project = root.project_id.clone();
        let project_path = root.project_path.clone();
        let hash = match receipts::fingerprint(
            &(&input, &project_path),
            &receipts::Target {
                workspace_id: &input.workspace_id,
                resource_id: &input.panel_id,
                generation: &input.document_id,
                revision: &input.expected_buffer_revision,
            },
        ) {
            Ok(h) => h,
            Err(e) => return storage_error(e),
        };
        let key = receipts::Key {
            pairing_id: owner,
            project_id: &project,
            retry_epoch: &input.retry_epoch,
            request_key: &input.request_key,
            tool: "lomi_editor_apply_edits",
        };
        match self.replay(&state, owner, &key, hash) {
            Ok(Some(r)) => return r,
            Err(e) => return *e,
            Ok(None) => {}
        }
        if let Err(e) = Self::validate_editor_edits(&state, owner, &input) {
            return error(e);
        }
        if let Err(e) = directory.check() {
            return error(e);
        }
        let not_after_millis = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            + 30_000)
            .to_string();
        self.enqueue_ui(
            &mut state,
            owner,
            UiMutation {
                workspace: input.workspace_id.clone(),
                project,
                revision: input.expected_revision.clone(),
                retry_epoch: input.retry_epoch.clone(),
                request_key: input.request_key.clone(),
                tool: "lomi_editor_apply_edits",
                hash,
                action: UiAction::EditorEdits(EditorEditsCommand {
                    workspace_id: input.workspace_id.clone(),
                    project_path,
                    not_after_millis,
                    input,
                }),
            },
        )
    }
}
