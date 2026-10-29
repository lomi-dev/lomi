use super::{process::valid_id, storage, store::Store};
use image::{ImageFormat, ImageReader, Limits};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
};
const MAX_IMAGE: usize = 10 * 1024 * 1024;
const MAX_TEXT: usize = 1024 * 1024;
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: usize,
}
fn failure<T>(result: rusqlite::Result<T>) -> Result<T, String> {
    result.map_err(|_| "storage: Cannot save the attachment reference.".into())
}
pub fn sensitive(name: &str) -> bool {
    let name = name.to_lowercase();
    name == ".env"
        || name.starts_with(".env.")
        || name.starts_with("id_rsa")
        || name.starts_with("id_ed25519")
        || name.contains("credentials")
        || name.ends_with(".pem")
        || name.ends_with(".key")
}
pub fn read_selected(path: &Path) -> Result<Vec<u8>, String> {
    storage::reject_link(path)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    let file = options
        .open(path)
        .map_err(|_| "Cannot open this attachment. Choose a regular file.")?;
    let metadata = file.metadata().map_err(|_| "Cannot inspect attachment.")?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("Attachments cannot be reparse points.".into());
        }
    }
    if !metadata.is_file() {
        return Err("Attach a regular file, not a directory or device.".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_IMAGE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read attachment.")?;
    if bytes.len() > MAX_IMAGE {
        return Err("Images are limited to 10 MiB each.".into());
    }
    Ok(bytes)
}
pub fn inspect(name: &str, bytes: &[u8]) -> Result<&'static str, String> {
    if name.is_empty()
        || name.len() > 255
        || name.contains(['\0', '/', '\\'])
        || bytes.len() > MAX_IMAGE
    {
        return Err("Invalid attachment name or size.".into());
    }
    let ext = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let expected = match ext.as_str() {
        "png" => Some(ImageFormat::Png),
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        "webp" => Some(ImageFormat::WebP),
        _ => None,
    };
    if let Ok(format) = image::guess_format(bytes) {
        if !matches!(
            format,
            ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
        ) || expected != Some(format)
        {
            return Err("Image signature and extension must match PNG, JPEG or WebP.".into());
        }
        let dimensions = ImageReader::with_format(Cursor::new(bytes), format)
            .into_dimensions()
            .map_err(|_| "The image is corrupt or unsupported.")?;
        if u64::from(dimensions.0) * u64::from(dimensions.1) > 20_000_000 {
            return Err("Images are limited to 20 megapixels.".into());
        }
        let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
        let mut limits = Limits::default();
        limits.max_alloc = Some(100 * 1024 * 1024);
        limits.max_image_width = Some(20_000_000);
        limits.max_image_height = Some(20_000_000);
        reader.limits(limits);
        reader
            .decode()
            .map_err(|_| "The image cannot be decoded within the attachment limits.")?;
        return Ok(match format {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            _ => "image/webp",
        });
    }
    if expected.is_some() {
        return Err("The image signature is invalid.".into());
    }
    if bytes.len() > MAX_TEXT {
        return Err("Text attachments are limited to 1 MiB each.".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| {
        "Text attachments must use UTF-8. Convert the file or paste text explicitly."
    })?;
    if text.contains('\0') {
        return Err("Binary attachments are not supported.".into());
    }
    Ok("text/plain")
}
pub fn folder(root: &Path) -> Result<PathBuf, String> {
    let path = root.join("attachments");
    storage::reject_link(&path)?;
    fs::create_dir_all(&path).map_err(|_| "storage: Cannot create the attachment directory.")?;
    storage::private(&path, true)?;
    Ok(path)
}
pub struct Import<'a> {
    pub conversation: &'a str,
    pub id: &'a str,
    pub name: &'a str,
    pub bytes: &'a [u8],
    pub expected: i64,
    pub approved: bool,
}
pub fn import(store: &mut Store, root: &Path, input: Import<'_>) -> Result<Attachment, String> {
    let Import {
        conversation,
        id,
        name,
        bytes,
        expected,
        approved,
    } = input;
    if !valid_id(id) {
        return Err("Invalid attachment ID.".into());
    }
    if sensitive(name) && !approved {
        return Err(
            "sensitive: This file may contain secrets. Confirm explicitly before attaching it."
                .into(),
        );
    }
    let mime = inspect(name, bytes)?;
    let draft = store.draft(conversation)?;
    if draft.revision != expected {
        return Err("conflict: The shared draft changed before the attachment was added.".into());
    }
    let total:i64=failure(store.connection.query_row("SELECT coalesce(sum(size),0) FROM attachments JOIN draft_attachments ON attachment_id=id WHERE conversation_id=?1",[conversation],|r|r.get(0)))?;
    if draft.attachments.len() >= 10 || total as usize + bytes.len() > 20 * 1024 * 1024 {
        return Err("Attach at most 10 files, totalling at most 20 MiB.".into());
    }
    let folder = folder(root)?;
    let destination = folder.join(id);
    storage::reject_link(&destination)?;
    if destination.exists() {
        return Err("Attachment ID already exists.".into());
    }
    let mut temp = tempfile::NamedTempFile::new_in(&folder)
        .map_err(|_| "storage: Cannot stage attachment.")?;
    storage::private(temp.path(), false)?;
    temp.write_all(bytes)
        .and_then(|()| temp.as_file().sync_all())
        .map_err(|_| "storage: Cannot save attachment.")?;
    temp.persist_noclobber(&destination)
        .map_err(|_| "storage: Cannot publish attachment.")?;
    #[cfg(unix)]
    fs::File::open(&folder)
        .and_then(|file| file.sync_all())
        .map_err(|_| "storage: Cannot flush attachment.")?;
    // Reconciliation removes a published object if this transaction never commits.
    let tx = failure(store.connection.transaction())?;
    failure(tx.execute(
        "INSERT INTO attachments(id,name,mime,size,hash,object) VALUES (?1,?2,?3,?4,?5,?1)",
        params![
            id,
            name,
            mime,
            bytes.len() as i64,
            format!("{:x}", Sha256::digest(bytes))
        ],
    ))?;
    failure(tx.execute(
        "INSERT INTO draft_attachments VALUES (?1,?2,?3)",
        params![conversation, id, draft.attachments.len() as i64],
    ))?;
    failure(tx.execute(
        "UPDATE drafts SET revision=revision+1 WHERE conversation_id=?1",
        [conversation],
    ))?;
    failure(tx.commit())?;
    Ok(Attachment {
        id: id.into(),
        name: name.into(),
        mime: mime.into(),
        size: bytes.len(),
    })
}
pub fn read_object(store: &Store, root: &Path, id: &str) -> Result<(Attachment, Vec<u8>), String> {
    if !valid_id(id) {
        return Err("Invalid attachment ID.".into());
    }
    let (name, mime, size, hash, object): (String, String, i64, String, String) =
        failure(store.connection.query_row(
            "SELECT name,mime,size,hash,object FROM attachments WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        ))?;
    if !valid_id(&object) {
        return Err("Invalid stored attachment.".into());
    }
    let bytes = read_selected(&folder(root)?.join(object))?;
    if bytes.len() != size as usize || format!("{:x}", Sha256::digest(&bytes)) != hash {
        return Err("The saved attachment changed or is corrupt.".into());
    }
    Ok((
        Attachment {
            id: id.into(),
            name,
            mime,
            size: bytes.len(),
        },
        bytes,
    ))
}
pub fn reconcile(store: &mut Store, root: &Path) -> Result<(), String> {
    let folder = folder(root)?;
    failure(store.connection.execute("DELETE FROM attachments WHERE NOT EXISTS(SELECT 1 FROM message_attachments WHERE attachment_id=id) AND NOT EXISTS(SELECT 1 FROM draft_attachments WHERE attachment_id=id)",[]))?;
    for entry in fs::read_dir(folder).map_err(|_| "storage: Cannot inspect attachments.")? {
        let entry = entry.map_err(|_| "storage: Cannot inspect attachment.")?;
        if !entry
            .file_type()
            .map_err(|_| "storage: Cannot inspect attachment.")?
            .is_file()
        {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let used: Option<i64> = failure(
            store
                .connection
                .query_row("SELECT 1 FROM attachments WHERE object=?1", [name], |r| {
                    r.get(0)
                })
                .optional(),
        )?;
        if used.is_none() {
            fs::remove_file(entry.path()).map_err(|_| {
                "storage: Attachment cleanup is pending. Retry after restoring disk access."
            })?;
        }
    }
    Ok(())
}
