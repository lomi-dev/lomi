use super::{catalog, download};
use ring::digest::{Context, SHA1_FOR_LEGACY_USE_ONLY};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

pub fn verify_with_cancel(
    package: &catalog::Package,
    license: &catalog::License,
    archive: &Path,
    installed: &Path,
    cancelled: impl Fn() -> bool,
) -> Result<(), String> {
    let started = std::time::Instant::now();
    let check = || {
        if cancelled() {
            Err("Android artifact verification cancelled".to_string())
        } else if started.elapsed() > std::time::Duration::from_secs(20 * 60) {
            Err("Android artifact verification exceeded its deadline".to_string())
        } else {
            Ok(())
        }
    };
    check()?;
    let mut file = File::open(archive).map_err(error)?;
    if file.metadata().map_err(error)?.len() != package.size {
        return Err("SDK archive size changed before installation verification.".into());
    }
    let mut hash = Context::new(&SHA1_FOR_LEGACY_USE_ONLY);
    let mut buffer = [0; 64 * 1024];
    loop {
        check()?;
        let count = file.read(&mut buffer).map_err(error)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    let actual: String = hash
        .finish()
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if actual != package.sha1.to_ascii_lowercase() {
        return Err("SDK archive checksum changed before installation verification.".into());
    }
    download::inspect_with_cancel(archive, &cancelled)?;
    if !fs::symlink_metadata(installed).map_err(error)?.is_dir() {
        return Err("SDK manager did not create a regular package directory.".into());
    }
    let metadata = regular_file(installed, Path::new("package.xml"))?;
    let mut xml = String::new();
    metadata
        .take(8 * 1024 * 1024 + 1)
        .read_to_string(&mut xml)
        .map_err(error)?;
    catalog::verify_local_package(&xml, package, license)?;
    let prefix = if package.id.starts_with("cmdline-tools;") {
        "cmdline-tools"
    } else if package.image.is_some() {
        package.id.rsplit(';').next().ok_or("Missing image ABI")?
    } else {
        package.id.as_str()
    };
    let mut zip = zip::ZipArchive::new(File::open(archive).map_err(error)?).map_err(error)?;
    let mut expected = BTreeSet::from([PathBuf::from("package.xml")]);
    for index in 0..zip.len() {
        check()?;
        let mut entry = zip.by_index(index).map_err(error)?;
        let name = entry.name().trim_end_matches('/').to_string();
        if name == prefix && entry.is_dir() {
            continue;
        }
        let relative = name
            .strip_prefix(prefix)
            .and_then(|name| name.strip_prefix('/'))
            .ok_or("SDK archive contains files outside the selected package")?;
        let relative = Path::new(relative);
        let path = checked_path(installed, relative)?;
        let metadata = fs::symlink_metadata(&path).map_err(error)?;
        let link = entry.unix_mode().unwrap_or(0) & 0o170000 == 0o120000;
        if link {
            if !metadata.file_type().is_symlink() {
                return Err("SDK manager changed an archive symbolic link.".into());
            }
            let mut target = String::new();
            entry.read_to_string(&mut target).map_err(error)?;
            if fs::read_link(&path).map_err(error)? != Path::new(&target) {
                return Err("SDK manager changed an archive link target.".into());
            }
        } else if entry.is_dir() {
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err("SDK manager changed an archive directory.".into());
            }
        } else {
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() != entry.size()
            {
                return Err("SDK manager produced an incomplete or unexpected artifact.".into());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // The selected CLI uses owner-only executable permissions (0755 -> 0744).
                // SDK directories are private; group/other execute bits are unnecessary.
                if metadata.permissions().mode() & 0o6000 != 0
                    || entry
                        .unix_mode()
                        .is_some_and(|mode| mode & 0o100 != metadata.permissions().mode() & 0o100)
                {
                    return Err(format!("SDK manager changed executable permissions for {}: archive {:o}, installed {:o}.", relative.display(), entry.unix_mode().unwrap_or(0), metadata.permissions().mode()));
                }
            }
            let mut extracted = File::open(&path).map_err(error)?;
            let mut actual = [0; 64 * 1024];
            loop {
                check()?;
                let count = entry.read(&mut buffer).map_err(error)?;
                if count == 0 {
                    break;
                }
                extracted.read_exact(&mut actual[..count]).map_err(error)?;
                if actual[..count] != buffer[..count] {
                    return Err("SDK manager artifact differs from the verified archive.".into());
                }
            }
            if extracted.read(&mut actual[..1]).map_err(error)? != 0 {
                return Err("SDK artifact changed during verification.".into());
            }
        }
        expected.insert(relative.to_path_buf());
    }
    let mut pending = vec![installed.to_path_buf()];
    let mut visited = 0;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).map_err(error)? {
            check()?;
            visited += 1;
            if visited > 100_001 {
                return Err("SDK manager produced too many artifacts.".into());
            }
            let entry = entry.map_err(error)?;
            let path = entry.path();
            let kind = entry.file_type().map_err(error)?;
            if kind.is_dir() {
                pending.push(path);
            } else if !expected.contains(path.strip_prefix(installed).map_err(error)?) {
                return Err(
                    "SDK manager produced an artifact absent from the approved archive.".into(),
                );
            }
        }
    }
    Ok(())
}

fn checked_path(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    let mut path = root.to_path_buf();
    let components: Vec<_> = relative.components().collect();
    for (index, component) in components.iter().enumerate() {
        let std::path::Component::Normal(component) = component else {
            return Err("Invalid SDK artifact path".into());
        };
        path.push(component);
        if index + 1 < components.len() {
            let metadata = fs::symlink_metadata(&path).map_err(error)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err("SDK artifact has a linked or invalid parent directory.".into());
            }
        }
    }
    Ok(path)
}

fn regular_file(root: &Path, relative: &Path) -> Result<File, String> {
    let path = checked_path(root, relative)?;
    if !fs::symlink_metadata(&path).map_err(error)?.is_file() {
        return Err("Expected a regular SDK artifact.".into());
    }
    File::open(path).map_err(error)
}

fn error(error: impl std::fmt::Display) -> String {
    format!("Android artifact verification: {error}")
}
