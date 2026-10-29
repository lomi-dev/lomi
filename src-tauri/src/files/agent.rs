//! The MCP file adapter shares editor decoding but reads only a broker-pinned
//! descriptor. It never reopens an agent-provided pathname through the UI API.
use lomi_control_core::broker::DecodedFile;
use lomi_control_protocol::{files::*, ErrorCode};

#[cfg(target_os = "macos")]
pub(crate) fn trash_staged(path: &std::path::Path) -> Result<(), ErrorCode> {
    use trash::macos::{DeleteMethod, TrashContextExtMacos};
    let mut context = trash::TrashContext::new();
    context.set_delete_method(DeleteMethod::NsFileManager);
    context
        .delete(path)
        .map_err(|_| ErrorCode::StorageUnavailable)
}

pub(crate) fn encode_preserving(original: &[u8], content: &str) -> Result<Vec<u8>, ErrorCode> {
    let (_, encoding) =
        super::editor::decode(original).map_err(|_| ErrorCode::UnsupportedCapability)?;
    super::editor::encode(content, encoding).map_err(|_| ErrorCode::ResourceExhausted)
}

pub(crate) fn publish_editor_save(
    app: &tauri::AppHandle,
    destination: &std::path::Path,
    project_root: &str,
    relative_path: &str,
    expected_disk_revision: &str,
    content: &str,
    principal: &str,
    operation_id: &str,
    check: &mut dyn FnMut() -> Result<(), ErrorCode>,
) -> Result<String, lomi_control_core::atomic_file::ReplaceError> {
    use lomi_control_core::atomic_file::ReplaceError;
    use lomi_remote_protocol::{
        ipc::{LocalNativeEditorEffect, LocalNativeRelativePath, LocalRootPath},
        HostKeyCommitmentHex,
    };

    let effect = LocalNativeEditorEffect::Save {
        root: LocalRootPath::try_from(project_root.to_owned())
            .map_err(|_| ErrorCode::UnsupportedCapability)?,
        relative: LocalNativeRelativePath::try_from(relative_path.to_owned())
            .map_err(|_| ErrorCode::UnsupportedCapability)?,
        expected_revision: HostKeyCommitmentHex::try_from(expected_disk_revision.to_owned())
            .map_err(|_| ErrorCode::RevisionConflict)?,
    };
    let result = crate::native_file_effects::save_guarded(
        app,
        destination,
        content,
        effect,
        crate::native_file_effects::ExpectedResult::File,
        principal,
        operation_id,
        check,
    )
    .map_err(|error| match error {
        crate::native_file_effects::GuardedSaveError::Authorization(code)
        | crate::native_file_effects::GuardedSaveError::Before(code) => ReplaceError::Before(code),
        crate::native_file_effects::GuardedSaveError::Uncertain => ReplaceError::Uncertain,
    })?;
    match result {
        lomi_native_files::NativeSaveResult::File(file)
            if file.path == destination.to_str().unwrap_or_default()
                && file.relative == relative_path =>
        {
            Ok(file.revision)
        }
        _ => Err(ReplaceError::Uncertain),
    }
}

pub(crate) fn publish_project_mutation(
    app: &tauri::AppHandle,
    root: &std::path::Path,
    principal: &str,
    operation: &str,
    mutation: &lomi_native_project_files::NativeProjectMutation,
    check: &mut dyn FnMut() -> Result<(), ErrorCode>,
) -> Result<
    lomi_native_project_files::NativeProjectMutationResult,
    lomi_native_project_files::NativeProjectMutationError,
> {
    crate::native_file_effects::project_mutation_guarded(
        app, root, principal, operation, mutation, check,
    )
}

pub(crate) fn publish_artifact_create(
    app: &tauri::AppHandle,
    root: &std::path::Path,
    principal: &str,
    operation: &str,
    relative_path: &str,
    expected_parent_revision: &str,
    bytes: &[u8],
    check: &mut dyn FnMut() -> Result<(), ErrorCode>,
) -> Result<
    lomi_native_project_files::NativeArtifactCreated,
    lomi_native_project_files::NativeProjectMutationError,
> {
    crate::native_file_effects::artifact_create_guarded(
        app,
        root,
        principal,
        operation,
        relative_path,
        expected_parent_revision,
        bytes,
        check,
    )
}

pub(crate) fn with_writer<T>(
    app: &tauri::AppHandle,
    write: impl FnOnce() -> Result<T, ErrorCode>,
) -> Result<T, ErrorCode> {
    use tauri::Manager;
    let _admission =
        crate::project_write_guard::admit_unscoped().map_err(|_| ErrorCode::TargetBusy)?;
    let state = app.state::<super::editor::EditorFiles>();
    let _guard = state.writes.try_lock().map_err(|_| ErrorCode::TargetBusy)?;
    write()
}

pub(crate) fn decode_full(bytes: &[u8]) -> Result<(String, TextEncoding), ErrorCode> {
    let (text, encoding) =
        super::editor::decode(bytes).map_err(|_| ErrorCode::UnsupportedCapability)?;
    use super::editor::Encoding;
    Ok((
        text,
        match encoding {
            Encoding::Utf8 => TextEncoding::Utf8,
            Encoding::Utf8Bom => TextEncoding::Utf8Bom,
            Encoding::Utf16Le => TextEncoding::Utf16Le,
            Encoding::Utf16Be => TextEncoding::Utf16Be,
        },
    ))
}

pub(crate) fn decode(
    bytes: &[u8],
    input: &FilesReadInput,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<DecodedFile, ErrorCode> {
    check()?;
    let (text, encoding) =
        super::editor::decode(bytes).map_err(|_| ErrorCode::UnsupportedCapability)?;
    let mut units = 0_u32;
    let mut start = None;
    let mut end = (0, 0);
    let end_limit = input
        .start_utf16
        .checked_add(u32::from(input.max_chars))
        .ok_or(ErrorCode::ResourceExhausted)?;
    let (mut cr, mut lf, mut crlf) = (0_u32, 0_u32, 0_u32);
    let mut previous_cr = false;
    for (count, (index, ch)) in text.char_indices().enumerate() {
        if count % 4096 == 0 {
            check()?;
        }
        if units == input.start_utf16 {
            start = Some(index);
        }
        if units <= end_limit {
            end = (index, units);
        }
        units += ch.len_utf16() as u32;
        if ch == '\r' {
            cr += 1;
        }
        if ch == '\n' {
            if previous_cr {
                cr -= 1;
                crlf += 1;
            } else {
                lf += 1;
            }
        }
        previous_cr = ch == '\r';
    }
    if units == input.start_utf16 {
        start = Some(text.len());
    }
    if units <= end_limit {
        end = (text.len(), units);
    }
    let start = start.ok_or(ErrorCode::ResourceExhausted)?;
    check()?;
    let line_endings = match (cr > 0, lf > 0, crlf > 0) {
        (false, false, false) => TextLineEndings::None,
        (true, false, false) => TextLineEndings::Cr,
        (false, true, false) => TextLineEndings::Lf,
        (false, false, true) => TextLineEndings::CrLf,
        _ => TextLineEndings::Mixed,
    };
    use super::editor::Encoding;
    Ok(DecodedFile {
        content: text[start..end.0].into(),
        total_utf16: units,
        next_utf16: (end.1 < units).then_some(end.1),
        line_endings,
        encoding: match encoding {
            Encoding::Utf8 => TextEncoding::Utf8,
            Encoding::Utf8Bom => TextEncoding::Utf8Bom,
            Encoding::Utf16Le => TextEncoding::Utf16Le,
            Encoding::Utf16Be => TextEncoding::Utf16Be,
        },
    })
}
