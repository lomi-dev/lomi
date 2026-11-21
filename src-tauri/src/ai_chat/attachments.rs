//! Chat attachments: kind detection, limits, names and provider content parts.
//! Files live in `<ai-chat dir>/attachments/<id>.<ext>` (legacy: `<id>`); rows live in threads.sqlite.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub(crate) const MAX_IMAGE: usize = 10 * 1024 * 1024;
pub(crate) const MAX_PDF: usize = 20 * 1024 * 1024;
pub(crate) const MAX_TEXT_FILE: usize = 512 * 1024;
pub(crate) const MAX_COUNT: usize = 10;
pub(crate) const MAX_TOTAL: u64 = 40 * 1024 * 1024;
const UNSUPPORTED: &str = "Unsupported file type.";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Image,
    Text,
    Pdf,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Attachment {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub kind: Kind,
    pub size: u64,
}

pub(crate) fn dir(chat_dir: &Path) -> PathBuf {
    chat_dir.join("attachments")
}

/// On-disk name; the extension lets CLIs that infer the type from it treat files correctly.
pub(crate) fn file_name(a: &Attachment) -> String {
    let ext: String = match (a.kind, a.mime.as_str()) {
        (Kind::Pdf, _) => "pdf".into(),
        (Kind::Image, "image/png") => "png".into(),
        (Kind::Image, "image/jpeg") => "jpg".into(),
        (Kind::Image, "image/gif") => "gif".into(),
        (Kind::Image, _) => "webp".into(),
        // Text keeps its own short extension, but never one that claims another type.
        (Kind::Text, _) => a
            .name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .filter(|e| {
                (1..=8).contains(&e.len())
                    && e.bytes().all(|b| b.is_ascii_alphanumeric())
                    && !matches!(e.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "pdf")
            })
            .unwrap_or_else(|| "txt".into()),
    };
    format!("{}.{ext}", a.id)
}

/// The one place that resolves a file; falls back to the legacy extension-less name.
pub(crate) fn path(dir: &Path, a: &Attachment) -> PathBuf {
    let p = dir.join(file_name(a));
    if p.exists() {
        return p;
    }
    let legacy = dir.join(&a.id);
    if legacy.exists() {
        legacy
    } else {
        p
    }
}

/// Detects the kind by content, never by extension, and enforces the per-file limit.
pub(crate) fn classify(data: &[u8]) -> Result<(Kind, &'static str), String> {
    let image = if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if data.starts_with(b"\xFF\xD8\xFF") {
        Some("image/jpeg")
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    };
    let (kind, mime, max) = match image {
        Some(mime) => (Kind::Image, mime, MAX_IMAGE),
        None if data.starts_with(b"%PDF") => (Kind::Pdf, "application/pdf", MAX_PDF),
        None if !data.contains(&0) && std::str::from_utf8(data).is_ok() => {
            (Kind::Text, "text/plain", MAX_TEXT_FILE)
        }
        None => return Err(UNSUPPORTED.into()),
    };
    if data.len() > max {
        return Err(
            "This file is too large for its type (images 10 MB, PDFs 20 MB, text 512 KB).".into(),
        );
    }
    Ok((kind, mime))
}

/// Basename only, without characters that could break the `<file name="...">` wrapper.
pub(crate) fn sanitize_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = base
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '"' | '<' | '>') {
                '_'
            } else {
                c
            }
        })
        .take(128)
        .collect();
    let clean = clean.trim();
    if clean.is_empty() || clean.chars().all(|c| c == '.') {
        "file".into()
    } else {
        clean.into()
    }
}

/// Reads at most `MAX_PDF + 1` bytes so an oversized or endless file never fills memory.
pub(crate) fn read_capped(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let fail = |_| "Cannot read this file.".to_string();
    let file = std::fs::File::open(path).map_err(fail)?;
    if !file.metadata().map_err(fail)?.is_file() {
        return Err("Only regular files can be attached.".into());
    }
    let mut data = Vec::new();
    file.take(MAX_PDF as u64 + 1)
        .read_to_end(&mut data)
        .map_err(fail)?;
    Ok(data)
}

pub(crate) fn read(dir: &Path, a: &Attachment) -> Result<Vec<u8>, String> {
    std::fs::read(path(dir, a)).map_err(|_| "An attachment file is missing.".into())
}

pub(crate) fn data_url(mime: &str, data: &[u8]) -> String {
    use base64::Engine;
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(data)
    )
}

fn b64(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn text_part(a: &Attachment, data: &[u8]) -> String {
    format!(
        "<file name=\"{}\">\n{}\n</file>",
        a.name,
        String::from_utf8_lossy(data)
    )
}

/// Content parts of one user turn for Chat Completions. `pdf` is true only for OpenAI.
pub(crate) fn openai_parts(
    text: &str,
    files: &[Attachment],
    dir: &Path,
    pdf: bool,
) -> Result<Value, String> {
    let mut parts = Vec::new();
    if !text.is_empty() {
        parts.push(json!({"type": "text", "text": text}));
    }
    for a in files {
        let data = read(dir, a)?;
        parts.push(match a.kind {
            Kind::Image => {
                json!({"type": "image_url", "image_url": {"url": data_url(&a.mime, &data)}})
            }
            Kind::Text => json!({"type": "text", "text": text_part(a, &data)}),
            Kind::Pdf if pdf => json!({"type": "file", "file": {
                "filename": a.name, "file_data": data_url("application/pdf", &data)}}),
            Kind::Pdf => return Err("This provider can't read PDF files.".into()),
        });
    }
    Ok(Value::Array(parts))
}

/// Content blocks of one user turn for the Anthropic Messages API.
pub(crate) fn anthropic_parts(
    text: &str,
    files: &[Attachment],
    dir: &Path,
) -> Result<Value, String> {
    let mut parts = Vec::new();
    if !text.is_empty() {
        parts.push(json!({"type": "text", "text": text}));
    }
    for a in files {
        let data = read(dir, a)?;
        let source = |mime: &str| json!({"type": "base64", "media_type": mime, "data": b64(&data)});
        parts.push(match a.kind {
            Kind::Image => json!({"type": "image", "source": source(&a.mime)}),
            Kind::Pdf => json!({"type": "document", "source": source("application/pdf")}),
            Kind::Text => json!({"type": "text", "text": text_part(a, &data)}),
        });
    }
    Ok(Value::Array(parts))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n0000";

    #[test]
    fn detects_kinds_by_content_and_enforces_limits() {
        assert_eq!(classify(PNG).unwrap(), (Kind::Image, "image/png"));
        assert_eq!(classify(b"\xFF\xD8\xFFx").unwrap().1, "image/jpeg");
        assert_eq!(classify(b"GIF89a..").unwrap().1, "image/gif");
        assert_eq!(classify(b"RIFF\0\0\0\0WEBPVP8 ").unwrap().1, "image/webp");
        assert_eq!(
            classify(b"%PDF-1.7").unwrap(),
            (Kind::Pdf, "application/pdf")
        );
        assert_eq!(classify("zażółć".as_bytes()).unwrap().0, Kind::Text);
        assert_eq!(classify(b"a\0b").unwrap_err(), UNSUPPORTED);
        assert_eq!(classify(b"\xFF\xFE\x00x").unwrap_err(), UNSUPPORTED);
        assert_eq!(classify(b"\xC3\x28").unwrap_err(), UNSUPPORTED);
        let big_text = vec![b'a'; MAX_TEXT_FILE + 1];
        assert!(classify(&big_text).is_err());
        assert!(classify(&big_text[..MAX_TEXT_FILE]).is_ok());
        let mut img = PNG.to_vec();
        img.resize(MAX_IMAGE + 1, 0);
        assert!(classify(&img).is_err());
        img.truncate(MAX_IMAGE);
        assert!(classify(&img).is_ok());
        let mut pdf = b"%PDF".to_vec();
        pdf.resize(MAX_PDF + 1, 0);
        assert!(classify(&pdf).is_err());
    }

    #[test]
    fn files_get_typed_extensions_and_legacy_fallback() {
        let at = |name: &str, kind, mime: &str| Attachment {
            id: "ab".into(),
            name: name.into(),
            mime: mime.into(),
            kind,
            size: 1,
        };
        let n = |name, kind, mime| file_name(&at(name, kind, mime));
        assert_eq!(n("x", Kind::Image, "image/jpeg"), "ab.jpg");
        assert_eq!(n("x", Kind::Image, "image/webp"), "ab.webp");
        assert_eq!(n("x.txt", Kind::Pdf, "application/pdf"), "ab.pdf");
        assert_eq!(n("a.RS", Kind::Text, "text/plain"), "ab.rs");
        assert_eq!(n("a.png", Kind::Text, "text/plain"), "ab.txt");
        assert_eq!(n("a.toolongext1", Kind::Text, "text/plain"), "ab.txt");
        assert_eq!(n("a.é", Kind::Text, "text/plain"), "ab.txt");
        assert_eq!(n("noext", Kind::Text, "text/plain"), "ab.txt");
        let d = tempfile::tempdir().unwrap();
        let a = at("x.png", Kind::Image, "image/png");
        assert_eq!(path(d.path(), &a), d.path().join("ab.png"));
        std::fs::write(d.path().join("ab"), b"old").unwrap();
        assert_eq!(read(d.path(), &a).unwrap(), b"old");
        std::fs::write(d.path().join("ab.png"), b"new").unwrap();
        assert_eq!(read(d.path(), &a).unwrap(), b"new");
    }

    #[test]
    fn names_are_basenames() {
        assert_eq!(sanitize_name("/a/b/c.txt"), "c.txt");
        assert_eq!(sanitize_name("C:\\x\\y.png"), "y.png");
        assert_eq!(sanitize_name("a\"<b>\n.md"), "a__b__.md");
        assert_eq!(sanitize_name(".."), "file");
        assert_eq!(sanitize_name("dir/"), "file");
        assert_eq!(sanitize_name(&"n".repeat(500)).len(), 128);
    }
}
