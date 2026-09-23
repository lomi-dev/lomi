//! Bounded APK manifest metadata, following AOSP androidfw ResourceTypes.h:
//! https://android.googlesource.com/platform/frameworks/base/+/refs/heads/main/libs/androidfw/include/androidfw/ResourceTypes.h
//! Resource references are not evaluated. Signature/installability belong to Android.
use lomi_control_protocol::ErrorCode;
type Result<T> = std::result::Result<T, ErrorCode>;
const INVALID: ErrorCode = ErrorCode::ArtifactInvalid;
#[derive(Debug)]
pub(crate) struct Metadata {
    pub package: String,
    pub version: Option<String>,
}
fn word(b: &[u8], i: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        b.get(i..i + 2).ok_or(INVALID)?.try_into().unwrap(),
    ))
}
fn dword(b: &[u8], i: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(i..i + 4).ok_or(INVALID)?.try_into().unwrap(),
    ))
}
fn length8(b: &[u8], i: &mut usize) -> Result<usize> {
    let first = *b.get(*i).ok_or(INVALID)?;
    *i += 1;
    Ok(if first & 0x80 == 0 {
        usize::from(first)
    } else {
        let last = *b.get(*i).ok_or(INVALID)?;
        *i += 1;
        (usize::from(first & 0x7f) << 8) | usize::from(last)
    })
}
fn length16(b: &[u8], i: &mut usize) -> Result<usize> {
    let first = word(b, *i)?;
    *i += 2;
    Ok(if first & 0x8000 == 0 {
        usize::from(first)
    } else {
        let last = word(b, *i)?;
        *i += 2;
        (usize::from(first & 0x7fff) << 16) | usize::from(last)
    })
}
fn pool(b: &[u8]) -> Result<Vec<String>> {
    let header = usize::from(word(b, 2)?);
    let count = dword(b, 8)? as usize;
    let styles = dword(b, 12)? as usize;
    let flags = dword(b, 16)?;
    let start = dword(b, 20)? as usize;
    let style_start = dword(b, 24)? as usize;
    if header < 28
        || count > 8192
        || styles > 8192
        || start < header + (count + styles) * 4
        || start > b.len()
        || (style_start != 0 && (style_start < start || style_start > b.len()))
    {
        return Err(INVALID);
    }
    let end = if style_start == 0 {
        b.len()
    } else {
        style_start
    };
    let mut values = Vec::with_capacity(count);
    let mut total = 0usize;
    for index in 0..count {
        let offset = dword(b, header + index * 4)? as usize;
        let mut at = start
            .checked_add(offset)
            .filter(|n| *n < end)
            .ok_or(INVALID)?;
        let source = &b[..end];
        let value = if flags & 0x100 != 0 {
            let units = length8(source, &mut at)?;
            let bytes = length8(source, &mut at)?;
            if bytes > 8192 || units > 4096 {
                return Err(INVALID);
            }
            let value = std::str::from_utf8(source.get(at..at + bytes).ok_or(INVALID)?)
                .map_err(|_| INVALID)?;
            if source.get(at + bytes) != Some(&0) || value.encode_utf16().count() != units {
                return Err(INVALID);
            }
            value.to_owned()
        } else {
            let units = length16(source, &mut at)?;
            if units > 4096 {
                return Err(INVALID);
            }
            let raw = source.get(at..at + units * 2).ok_or(INVALID)?;
            if word(source, at + units * 2)? != 0 {
                return Err(INVALID);
            }
            String::from_utf16(
                &raw.as_chunks::<2>()
                    .0
                    .iter()
                    .map(|p| u16::from_le_bytes([p[0], p[1]]))
                    .collect::<Vec<_>>(),
            )
            .map_err(|_| INVALID)?
        };
        total += value.len();
        if total > 2 * 1024 * 1024 {
            return Err(INVALID);
        }
        values.push(value);
    }
    Ok(values)
}
fn string(strings: &[String], index: u32) -> Result<&str> {
    strings
        .get(index as usize)
        .map(String::as_str)
        .ok_or(INVALID)
}
pub(crate) fn package_valid(package: &str) -> bool {
    package.len() <= 255
        && package.contains('.')
        && package.split('.').all(|s| {
            !s.is_empty()
                && s.as_bytes()[0].is_ascii_alphabetic()
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
}
pub(crate) fn parse(bytes: &[u8], check: &dyn Fn() -> Result<()>) -> Result<Metadata> {
    if bytes.len() > 1024 * 1024
        || word(bytes, 0)? != 3
        || word(bytes, 2)? != 8
        || dword(bytes, 4)? as usize != bytes.len()
    {
        return Err(INVALID);
    }
    let mut at = 8;
    let mut strings = None;
    let mut stack = Vec::new();
    let mut package = None;
    let mut version = None;
    let mut version_major = 0u32;
    let mut root_seen = false;
    let mut nodes = 0;
    while at < bytes.len() {
        check()?;
        nodes += 1;
        if nodes > 4096 {
            return Err(INVALID);
        }
        let kind = word(bytes, at)?;
        let header = usize::from(word(bytes, at + 2)?);
        let size = dword(bytes, at + 4)? as usize;
        if header < 8 || size < header || !size.is_multiple_of(4) {
            return Err(INVALID);
        }
        let b = bytes
            .get(at..at.checked_add(size).ok_or(INVALID)?)
            .ok_or(INVALID)?;
        match kind {
            1 => {
                if strings.is_some() || root_seen {
                    return Err(INVALID);
                }
                strings = Some(pool(b)?);
            }
            0x180 => {
                if strings.is_none() || root_seen || header != 8 {
                    return Err(INVALID);
                }
            }
            0x100 | 0x101 => {
                if header != 16 || size < 24 {
                    return Err(INVALID);
                }
                let strings = strings.as_ref().ok_or(INVALID)?;
                string(strings, dword(b, header + 4)?)?;
            }
            0x102 => {
                let strings = strings.as_ref().ok_or(INVALID)?;
                if header != 16 || size < header + 20 || stack.len() >= 64 {
                    return Err(INVALID);
                }
                let ns = dword(b, header)?;
                let name = dword(b, header + 4)?;
                let root = stack.is_empty();
                if root && (root_seen || ns != u32::MAX || string(strings, name)? != "manifest") {
                    return Err(INVALID);
                }
                if root {
                    root_seen = true;
                }
                let attr_start = usize::from(word(b, header + 8)?);
                let attr_size = usize::from(word(b, header + 10)?);
                let count = usize::from(word(b, header + 12)?);
                if attr_start < 20
                    || attr_size != 20
                    || count > 256
                    || header + attr_start + count * attr_size > size
                {
                    return Err(INVALID);
                }
                let mut seen = std::collections::HashSet::new();
                for n in 0..count {
                    let a = &b[header + attr_start + n * attr_size..][..attr_size];
                    let ns = dword(a, 0)?;
                    let name = string(strings, dword(a, 4)?)?;
                    if !seen.insert((ns, name)) || word(a, 12)? != 8 || a[14] != 0 {
                        return Err(INVALID);
                    }
                    let raw = dword(a, 8)?;
                    let value_type = a[15];
                    let data = dword(a, 16)?;
                    let raw_value = if raw != u32::MAX {
                        Some(string(strings, raw)?)
                    } else {
                        None
                    };
                    let value = if value_type == 3 {
                        Some(string(strings, data)?)
                    } else {
                        None
                    };
                    if root && ns == u32::MAX && name == "package" {
                        let p = value.or(raw_value).ok_or(INVALID)?;
                        if !package_valid(p) {
                            return Err(INVALID);
                        }
                        package = Some(p.to_owned());
                    }
                    if root
                        && ns != u32::MAX
                        && string(strings, ns)? == "http://schemas.android.com/apk/res/android"
                        && name == "versionCode"
                        && matches!(value_type, 0x10 | 0x11)
                    {
                        version = Some(data);
                    }
                    if root
                        && ns != u32::MAX
                        && string(strings, ns)? == "http://schemas.android.com/apk/res/android"
                        && name == "versionCodeMajor"
                        && matches!(value_type, 0x10 | 0x11)
                    {
                        version_major = data;
                    }
                }
                stack.push((ns, name));
            }
            0x103 => {
                if header != 16
                    || size < header + 8
                    || stack.pop() != Some((dword(b, header)?, dword(b, header + 4)?))
                {
                    return Err(INVALID);
                }
            }
            0x104 => {
                if header != 16 || size < header + 12 || stack.is_empty() {
                    return Err(INVALID);
                }
            }
            _ => return Err(INVALID),
        }
        at += size;
    }
    if !root_seen || !stack.is_empty() {
        return Err(INVALID);
    }
    Ok(Metadata {
        package: package.ok_or(INVALID)?,
        version: version.map(|low| ((u64::from(version_major) << 32) | u64::from(low)).to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};
    fn manifest() -> Vec<u8> {
        let mut archive = zip::ZipArchive::new(Cursor::new(include_bytes!(
            "../../android-input/lomi-input.apk"
        )))
        .unwrap();
        let mut bytes = Vec::new();
        archive
            .by_name("AndroidManifest.xml")
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    }
    #[test]
    fn real_compiled_manifest_preserves_package_version_and_checks_bounds() {
        let bytes = manifest();
        let metadata = parse(&bytes, &|| Ok(())).unwrap();
        assert_eq!(metadata.package, "org.lomi.input");
        assert_eq!(metadata.version.as_deref(), Some("3"));
        for end in 0..bytes.len() {
            assert!(parse(&bytes[..end], &|| Ok(())).is_err());
        }
        // Damage chunk sizes/offsets from a real compiler output. No unchecked
        // indexing or oversized string/attribute allocation is permitted.
        for at in (8..bytes.len().min(512) - 4).step_by(4) {
            let mut changed = bytes.clone();
            changed[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            let _ = parse(&changed, &|| Ok(()));
        }
        assert!(matches!(
            parse(&bytes, &|| Err(ErrorCode::ControlRevoked)),
            Err(ErrorCode::ControlRevoked)
        ));
    }
    #[test]
    fn package_identifiers_cannot_be_guest_shell_options_or_expressions() {
        for package in ["org.lomi.input", "com.example.test_1"] {
            assert!(package_valid(package));
        }
        for package in [
            "-org.example",
            "org.example;id",
            "org.example\n",
            "org.$(id)",
            "org..example",
            "../org.example",
            "org.éxample",
            "org.example/activity",
            "",
        ] {
            assert!(!package_valid(package));
        }
    }
}
