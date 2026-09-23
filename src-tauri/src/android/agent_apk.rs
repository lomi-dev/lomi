//! Inspect an already completed, read-only private copy. No extraction or guest
//! commands occur here; Android's installer remains the signature authority.
use lomi_control_protocol::ErrorCode;
use std::{
    collections::HashSet,
    fs::File,
    io::{Read, Seek, SeekFrom},
};
const CENTRAL_LIMIT: u64 = 8 * 1024 * 1024;
const ENTRY_LIMIT: usize = 8192;
fn invalid(_: impl std::fmt::Debug) -> ErrorCode {
    ErrorCode::ArtifactInvalid
}
fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}
fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

pub(crate) fn inspect(
    copy: &File,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<(), ErrorCode> {
    metadata(copy, check).map(|_| ())
}
pub(crate) fn metadata(
    copy: &File,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<super::apk_manifest::Metadata, ErrorCode> {
    check()?;
    let mut file = copy.try_clone().map_err(invalid)?;
    let length = file.metadata().map_err(invalid)?.len();
    if !(22..=lomi_control_core::staging::MAX_IMPORT_BYTES).contains(&length) {
        return Err(ErrorCode::ArtifactInvalid);
    }
    // Bound the central directory before asking ZipArchive to allocate entries.
    // ZIP64 and multidisk packages are intentionally outside this APK contract.
    let tail_length = length.min(65557) as usize;
    file.seek(SeekFrom::End(-(tail_length as i64)))
        .map_err(invalid)?;
    let mut tail = vec![0; tail_length];
    file.read_exact(&mut tail).map_err(invalid)?;
    let end = (0..=tail_length - 22)
        .rev()
        .find(|&i| {
            &tail[i..i + 4] == b"PK\x05\x06"
                && i + 22 + usize::from(u16_at(&tail, i + 20)) == tail_length
        })
        .ok_or(ErrorCode::ArtifactInvalid)?;
    let end_offset = length - tail_length as u64 + end as u64;
    let count = usize::from(u16_at(&tail, end + 10));
    let central_length = u64::from(u32_at(&tail, end + 12));
    let central_offset = u64::from(u32_at(&tail, end + 16));
    if u16_at(&tail, end + 4) != 0
        || u16_at(&tail, end + 6) != 0
        || usize::from(u16_at(&tail, end + 8)) != count
        || !(1..=ENTRY_LIMIT).contains(&count)
        || central_length > CENTRAL_LIMIT
        || central_length < count as u64 * 46
        || central_offset + central_length != end_offset
    {
        return Err(ErrorCode::ArtifactInvalid);
    }
    file.seek(SeekFrom::Start(central_offset))
        .map_err(invalid)?;
    let mut central = vec![0; central_length as usize];
    file.read_exact(&mut central).map_err(invalid)?;
    let mut at = 0;
    let mut names = HashSet::new();
    let mut expanded = 0u64;
    for _ in 0..count {
        check()?;
        let header = central.get(at..at + 46).ok_or(ErrorCode::ArtifactInvalid)?;
        if &header[..4] != b"PK\x01\x02" {
            return Err(ErrorCode::ArtifactInvalid);
        }
        let name_length = usize::from(u16_at(header, 28));
        let extra_length = usize::from(u16_at(header, 30));
        let comment_length = usize::from(u16_at(header, 32));
        let packed = u64::from(u32_at(header, 20));
        let unpacked = u64::from(u32_at(header, 24));
        let local_offset = u64::from(u32_at(header, 42));
        expanded = expanded
            .checked_add(unpacked)
            .filter(|n| *n <= 2 * 1024 * 1024 * 1024)
            .ok_or(ErrorCode::ArtifactTooLarge)?;
        if name_length == 0
            || name_length > 4096
            || extra_length > 8192
            || comment_length > 4096
            || u16_at(header, 8) & 1 != 0
            || !matches!(u16_at(header, 10), 0 | 8)
            || u16_at(header, 34) != 0
            || unpacked > lomi_control_core::staging::MAX_IMPORT_BYTES
            || local_offset + 30 + packed > central_offset
        {
            return Err(ErrorCode::ArtifactInvalid);
        }
        let name_bytes = central
            .get(at + 46..at + 46 + name_length)
            .ok_or(ErrorCode::ArtifactInvalid)?;
        let name = std::str::from_utf8(name_bytes).map_err(invalid)?;
        if name.starts_with('/')
            || name.contains(['\\', '\0', ':'])
            || name
                .trim_end_matches('/')
                .split('/')
                .any(|p| matches!(p, "" | "." | ".."))
            || !names.insert(name.to_owned())
        {
            return Err(ErrorCode::ArtifactInvalid);
        }
        at = at
            .checked_add(46 + name_length + extra_length + comment_length)
            .filter(|n| *n <= central.len())
            .ok_or(ErrorCode::ArtifactInvalid)?;
    }
    if at != central.len() || !names.contains("AndroidManifest.xml") {
        return Err(ErrorCode::ArtifactInvalid);
    }
    check()?;
    file.seek(SeekFrom::Start(0)).map_err(invalid)?;
    let mut archive = zip::ZipArchive::new(file).map_err(invalid)?;
    if archive.len() != count || archive.offset() != 0 {
        return Err(ErrorCode::ArtifactInvalid);
    }
    let manifest = archive.by_name("AndroidManifest.xml").map_err(invalid)?;
    if !(8..=1024 * 1024).contains(&manifest.size()) {
        return Err(ErrorCode::ArtifactInvalid);
    }
    let expected = manifest.size();
    let mut bytes = Vec::new();
    manifest
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(invalid)?;
    check()?;
    if bytes.len() as u64 != expected
        || bytes[..4] != [3, 0, 8, 0]
        || u32_at(&bytes, 4) as usize != bytes.len()
    {
        return Err(ErrorCode::ArtifactInvalid);
    }
    super::apk_manifest::parse(&bytes, check)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn fixture(name: &str, manifest: &[u8]) -> File {
        let file = tempfile::tempfile().unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(manifest).unwrap();
        zip.finish().unwrap()
    }
    #[test]
    fn bounded_apk_shape_rejects_nonmanifest_xml_paths_and_cancellation() {
        let manifest = [3, 0, 8, 0, 8, 0, 0, 0];
        let mut valid = tempfile::tempfile().unwrap();
        valid
            .write_all(include_bytes!("../../android-input/lomi-input.apk"))
            .unwrap();
        let info = metadata(&valid, &|| Ok(())).unwrap();
        assert_eq!(info.package, "org.lomi.input");
        assert_eq!(info.version.as_deref(), Some("3"));
        assert_eq!(
            inspect(&fixture("AndroidManifest.xml", &manifest), &|| Ok(())),
            Err(ErrorCode::ArtifactInvalid)
        );
        assert_eq!(
            inspect(&fixture("AndroidManifest.xml", b"<manifest/>"), &|| Ok(())),
            Err(ErrorCode::ArtifactInvalid)
        );
        assert_eq!(
            inspect(&fixture("../AndroidManifest.xml", &manifest), &|| Ok(())),
            Err(ErrorCode::ArtifactInvalid)
        );
        assert_eq!(
            inspect(&fixture("AndroidManifest.xml", &manifest), &|| Err(
                ErrorCode::ControlRevoked
            )),
            Err(ErrorCode::ControlRevoked)
        );
    }
    #[test]
    fn central_directory_limits_are_checked_before_zip_allocations() {
        let manifest = [3, 0, 8, 0, 8, 0, 0, 0];
        let mut file = fixture("AndroidManifest.xml", &manifest);
        file.seek(SeekFrom::End(-12)).unwrap();
        file.write_all(&(u32::MAX - 1).to_le_bytes()).unwrap();
        assert_eq!(inspect(&file, &|| Ok(())), Err(ErrorCode::ArtifactInvalid));
        let mut file = fixture("AndroidManifest.xml", &manifest);
        file.seek(SeekFrom::End(-14)).unwrap();
        file.write_all(&u16::MAX.to_le_bytes()).unwrap();
        assert_eq!(inspect(&file, &|| Ok(())), Err(ErrorCode::ArtifactInvalid));
    }
}
