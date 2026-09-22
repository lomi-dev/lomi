use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

const LIMIT: u64 = 1024 * 1024;

pub fn read(path: &Path) -> Result<Option<String>, String> {
    match fs::metadata(path) {
        Ok(metadata) if !metadata.is_file() => {
            return Err("CLI configuration must be a regular file.".into())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
        _ => {}
    }
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("CLI configuration must be a regular file.".into());
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("CLI configuration exceeds 1 MiB. The file was left intact.".into());
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "CLI configuration is not UTF-8. The file was left intact.".into())
}

pub fn revision(source: Option<&str>) -> Option<String> {
    source.map(|source| format!("{:x}", Sha256::digest(source.as_bytes())))
}

pub fn write(path: &Path, source: Option<&str>, mut output: String) -> Result<(), String> {
    let conflict =
        "CLI configuration changed. Check the settings again before allowing the update.";
    if source
        .as_ref()
        .is_some_and(|source| source.contains("\r\n"))
    {
        output = output.replace("\r\n", "\n").replace('\n', "\r\n");
    }
    if output.len() as u64 > LIMIT {
        return Err(
            "Updated CLI configuration would exceed 1 MiB. The file was left intact.".into(),
        );
    }
    let directory = path
        .parent()
        .ok_or("CLI configuration has no parent directory.")?;
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".lomi-cli-")
        .tempfile_in(directory)
        .map_err(|error| error.to_string())?;
    temporary
        .write_all(output.as_bytes())
        .map_err(|error| error.to_string())?;
    if let Some(source) = &source {
        let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
        if metadata.permissions().readonly() {
            return Err("CLI configuration is read-only. The file was left intact.".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let created = temporary
                .as_file()
                .metadata()
                .map_err(|error| error.to_string())?;
            if created.uid() != metadata.uid() || created.gid() != metadata.gid() {
                std::os::unix::fs::chown(
                    temporary.path(),
                    Some(metadata.uid()),
                    Some(metadata.gid()),
                )
                .map_err(|error| error.to_string())?;
            }
        }
        let mut backup = tempfile::Builder::new()
            .prefix(&format!(
                "{}.lomi-backup-",
                path.file_name().unwrap_or_default().to_string_lossy()
            ))
            .tempfile_in(directory)
            .map_err(|error| error.to_string())?;
        backup
            .write_all(source.as_bytes())
            .map_err(|error| error.to_string())?;
        backup
            .as_file()
            .sync_all()
            .map_err(|error| error.to_string())?;
        backup.keep().map_err(|error| error.to_string())?;
        temporary
            .as_file()
            .set_permissions(metadata.permissions())
            .map_err(|error| error.to_string())?;
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    if read(path)?.as_deref() != source {
        return Err(conflict.into());
    }
    if source.is_none() {
        temporary
            .persist_noclobber(path)
            .map_err(|error| error.to_string())?;
    } else {
        temporary.persist(path).map_err(|error| error.to_string())?;
    }
    Ok(())
}
