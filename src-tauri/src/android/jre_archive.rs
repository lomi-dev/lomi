use super::download::safe_parts;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
    path::Path,
    time::{Duration, Instant},
};

pub const EXPANDED_LIMIT: u64 = 1024 * 1024 * 1024;
const ENTRY_LIMIT: usize = 20_000;

struct Entry {
    spelling: String,
    directory: bool,
    explicit: bool,
    link: Option<(String, String)>,
}

struct Extraction<'a> {
    root: &'a Path,
    entries: BTreeMap<String, Entry>,
    size: u64,
    started: Instant,
    cancelled: &'a dyn Fn() -> bool,
}

/// Extract into a newly created private directory. Links are created last, after
/// every ancestor and target has been checked across the complete archive.
pub fn extract(
    archive: &Path,
    target: &Path,
    windows_zip: bool,
    cancelled: impl Fn() -> bool,
) -> Result<(), String> {
    fs::create_dir(target).map_err(error)?;
    let mut extraction = Extraction {
        root: target,
        entries: BTreeMap::new(),
        size: 0,
        started: Instant::now(),
        cancelled: &cancelled,
    };
    if windows_zip {
        let mut archive =
            zip::ZipArchive::new(File::open(archive).map_err(error)?).map_err(error)?;
        if archive.len() > ENTRY_LIMIT {
            return Err("Java archive has too many entries".into());
        }
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(error)?;
            let name = entry.name().trim_end_matches('/').to_owned();
            let mode = entry.unix_mode().unwrap_or(0);
            if !matches!(mode & 0o170000, 0 | 0o100000 | 0o040000) {
                return Err("Java ZIP contains an unsupported link or special file".into());
            }
            let directory = entry.is_dir();
            let size = entry.size();
            extraction.entry(&name, directory, None, size, mode, &mut entry)?;
        }
    } else {
        let gzip = flate2::read::GzDecoder::new(File::open(archive).map_err(error)?);
        let mut archive = tar::Archive::new(gzip);
        for entry in archive.entries().map_err(error)? {
            let mut entry = entry.map_err(error)?;
            let path = entry.path().map_err(error)?.into_owned();
            let name = path
                .to_str()
                .ok_or("Java archive path is not Unicode")?
                .trim_end_matches('/');
            let kind = entry.header().entry_type();
            if !kind.is_file() && !kind.is_dir() && !kind.is_symlink() {
                return Err(
                    "Java archive contains an unsupported hard link or special file".into(),
                );
            }
            let link = entry
                .link_name()
                .map_err(error)?
                .map(|path| path.into_owned());
            if kind.is_symlink() != link.is_some() {
                return Err("Invalid Java archive link header".into());
            }
            let mode = entry.header().mode().map_err(error)?;
            let size = entry.size();
            extraction.entry(name, kind.is_dir(), link.as_deref(), size, mode, &mut entry)?;
        }
    }
    extraction.finish()
}

impl Extraction<'_> {
    fn check(&self) -> Result<(), String> {
        if (self.cancelled)() {
            return Err("Java extraction cancelled".into());
        }
        if self.started.elapsed() > Duration::from_secs(10 * 60) {
            return Err("Java extraction exceeded its deadline".into());
        }
        Ok(())
    }

    fn entry(
        &mut self,
        name: &str,
        directory: bool,
        link: Option<&Path>,
        size: u64,
        mode: u32,
        reader: &mut impl Read,
    ) -> Result<(), String> {
        self.check()?;
        let parts = safe_parts(name)?;
        self.size = self
            .size
            .checked_add(size)
            .filter(|size| *size <= EXPANDED_LIMIT)
            .ok_or("Java archive exceeds its expanded size limit")?;
        for length in 1..=parts.len() {
            let spelling = parts[..length].join("/");
            let key = spelling.to_lowercase();
            let final_entry = length == parts.len();
            if let Some(previous) = self.entries.get(&key) {
                if previous.spelling != spelling
                    || !previous.directory
                    || previous.link.is_some()
                    || (final_entry && (previous.explicit || !directory || link.is_some()))
                {
                    return Err("Java archive contains colliding paths or a link ancestor".into());
                }
            }
            if !final_entry {
                self.entries.entry(key).or_insert(Entry {
                    spelling,
                    directory: true,
                    explicit: false,
                    link: None,
                });
            }
        }
        let link = link
            .map(|target| -> Result<_, String> {
                if directory || size != 0 {
                    return Err("Invalid Java archive link".into());
                }
                let text = target.to_str().ok_or("Invalid Java link encoding")?;
                if text.is_empty()
                    || text.len() > 1024
                    || text.starts_with('/')
                    || text.contains(['\\', ':', '\0'])
                {
                    return Err("Java archive link escapes its directory".into());
                }
                let mut resolved = parts[..parts.len() - 1].to_vec();
                for part in text.split('/') {
                    match part {
                        "" | "." => {}
                        ".." => {
                            resolved.pop().ok_or("Java link escapes its archive")?;
                        }
                        part => {
                            safe_parts(part)?;
                            resolved.push(part);
                        }
                    }
                }
                if resolved.is_empty() {
                    return Err("Java link points at its archive root".into());
                }
                Ok((text.to_owned(), resolved.join("/")))
            })
            .transpose()?;
        self.entries.insert(
            name.to_lowercase(),
            Entry {
                spelling: name.into(),
                directory,
                explicit: true,
                link: link.clone(),
            },
        );
        if self.entries.len() > ENTRY_LIMIT {
            return Err("Java archive has too many paths".into());
        }
        let target = self.root.join(name);
        if link.is_some() {
            return Ok(());
        }
        if directory {
            if size != 0 {
                return Err("Java archive directory contains data".into());
            }
            fs::create_dir_all(target).map_err(error)?;
            return Ok(());
        }
        fs::create_dir_all(target.parent().ok_or("Java entry has no parent")?).map_err(error)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(error)?;
        let mut buffer = [0; 64 * 1024];
        let mut remaining = size;
        while remaining > 0 {
            self.check()?;
            let length = remaining.min(buffer.len() as u64) as usize;
            let count = reader.read(&mut buffer[..length]).map_err(error)?;
            if count == 0 {
                return Err("Java archive entry is truncated".into());
            }
            file.write_all(&buffer[..count]).map_err(error)?;
            remaining -= count as u64;
        }
        if reader.read(&mut buffer[..1]).map_err(error)? != 0 {
            return Err("Java archive entry exceeded its declared size".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(if mode & 0o111 != 0 {
                0o700
            } else {
                0o600
            }))
            .map_err(error)?;
        }
        #[cfg(not(unix))]
        let _ = mode;
        file.sync_all().map_err(error)
    }

    fn finish(self) -> Result<(), String> {
        if self.entries.is_empty() {
            return Err("Java archive is empty".into());
        }
        for entry in self.entries.values() {
            self.check()?;
            let Some((_, target)) = &entry.link else {
                continue;
            };
            let mut target = target.as_str();
            let mut depth = 0;
            loop {
                depth += 1;
                if depth > 32 {
                    return Err("Java archive contains a cyclic link".into());
                }
                for (offset, _) in target.match_indices('/') {
                    if self
                        .entries
                        .get(&target[..offset].to_lowercase())
                        .is_some_and(|parent| !parent.directory || parent.link.is_some())
                    {
                        return Err("Java archive link traverses another link".into());
                    }
                }
                let resolved = self
                    .entries
                    .get(&target.to_lowercase())
                    .ok_or("Java archive contains a dangling link")?;
                if resolved.spelling != target {
                    return Err("Java archive link has inconsistent casing".into());
                }
                match &resolved.link {
                    Some((_, next)) => target = next,
                    None => break,
                }
            }
        }
        for entry in self.entries.values() {
            self.check()?;
            if let Some((target, _)) = &entry.link {
                let path = self.root.join(&entry.spelling);
                fs::create_dir_all(path.parent().ok_or("Java link has no parent")?)
                    .map_err(error)?;
                #[cfg(unix)]
                std::os::unix::fs::symlink(target, path).map_err(error)?;
                #[cfg(not(unix))]
                {
                    let _ = (target, path);
                    return Err("Java links are unsupported on this host".into());
                }
            }
        }
        Ok(())
    }
}

fn error(error: impl std::fmt::Display) -> String {
    format!("Java extraction: {error}")
}
