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

pub(crate) fn with_writer<T>(
    app: &tauri::AppHandle,
    write: impl FnOnce() -> Result<T, ErrorCode>,
) -> Result<T, ErrorCode> {
    use tauri::Manager;
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

#[cfg(test)]
mod tests {
    use super::*;
    fn input(start: u32, max: u16) -> FilesReadInput {
        FilesReadInput {
            workspace_id: "w".into(),
            relative_path: "a.txt".into(),
            start_utf16: start,
            max_chars: max,
            expected_disk_revision: None,
        }
    }
    #[test]
    fn disk_slices_preserve_bom_encoding_endings_and_surrogate_boundaries() {
        let text = "a🙂\r\nb\rc\n";
        let first = decode(text.as_bytes(), &input(0, 2), &|| Ok(())).unwrap();
        assert_eq!(first.content, "a");
        assert_eq!(first.next_utf16, Some(1));
        assert_eq!(first.line_endings, TextLineEndings::Mixed);
        let second = decode(text.as_bytes(), &input(1, 3), &|| Ok(())).unwrap();
        assert_eq!(second.content, "🙂\r");
        assert_eq!(second.next_utf16, Some(4));
        assert!(matches!(
            decode(text.as_bytes(), &input(2, 2), &|| Ok(())),
            Err(ErrorCode::ResourceExhausted)
        ));
        let empty = decode(text.as_bytes(), &input(first.total_utf16, 2), &|| Ok(())).unwrap();
        assert_eq!(empty.content, "");
        assert_eq!(empty.next_utf16, None);
        let mut utf16 = vec![0xff, 0xfe];
        for unit in text.encode_utf16() {
            utf16.extend(unit.to_le_bytes());
        }
        let decoded = decode(&utf16, &input(0, 100), &|| Ok(())).unwrap();
        assert_eq!(decoded.content, text);
        assert!(matches!(decoded.encoding, TextEncoding::Utf16Le));
        for invalid in [&[0xff][..], &[0xff, 0xfe, 0x00, 0xd8][..], b"binary\0value"] {
            assert!(matches!(
                decode(invalid, &input(0, 20), &|| Ok(())),
                Err(ErrorCode::UnsupportedCapability)
            ));
        }
        assert!(matches!(
            decode(text.as_bytes(), &input(0, 20), &|| Err(
                ErrorCode::ControlRevoked
            )),
            Err(ErrorCode::ControlRevoked)
        ));
    }
}
