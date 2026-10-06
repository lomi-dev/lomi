//! Compile-owned trust for the public Grok source build. Published npm binaries
//! and profile/environment supplied hashes cannot satisfy this admission.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const PIN: &str = "2bdd1d6a6369de0e8c68132ea4539e9abd9e14a8";
const SOURCE_REV: &str = "559751fdcec02d413e4c57c8832ab275e4f44980";
const VERSION: &str = "1.0.45";
const MAX_BINARY: u64 = 512 * 1024 * 1024;
const MANIFEST: &str = include_str!("../../../scripts/agent-grok-artifacts.json");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    public_commit: String,
    source_revision: String,
    version: String,
    artifacts: Vec<Artifact>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Artifact {
    target: String,
    filename: String,
    sha256: String,
}
fn unavailable() -> String {
    "Grok requires the verified source-built Lomi artifact for this platform. No request was sent."
        .into()
}
fn compiled_artifact() -> Result<Artifact, String> {
    let manifest: Manifest = serde_json::from_str(MANIFEST).map_err(|_| unavailable())?;
    if manifest.public_commit != PIN
        || manifest.source_revision != SOURCE_REV
        || manifest.version != VERSION
    {
        return Err(unavailable());
    }
    let target = env!("LOMI_AI_TARGET");
    let mut entries = manifest
        .artifacts
        .into_iter()
        .filter(|entry| entry.target == target);
    let entry = entries.next().ok_or_else(unavailable)?;
    if entries.next().is_some()
        || entry.filename != format!("grok-{target}")
        || entry.sha256.len() != 64
        || !entry
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(unavailable());
    }
    Ok(entry)
}
pub(super) fn compiled_available() -> bool {
    cfg!(unix) && compiled_artifact().is_ok()
}
pub(super) fn version_matches(bytes: &[u8]) -> bool {
    compiled_available() && bytes == b"grok 1.0.45 (2bdd1d6a6369)\n"
}
/// Bundle resources take precedence. The only unbundled fallback is owned by
/// the source tree, never PATH, profile input or a process environment variable.
pub(super) fn resolve(resource_dir: &Path) -> Result<PathBuf, String> {
    let entry = compiled_artifact()?;
    let bundled = resource_dir.join("agent-grok").join(&entry.filename);
    if fs::symlink_metadata(&bundled).is_ok() {
        admit(&bundled)?;
        return Ok(bundled);
    }
    // Local release --no-bundle builds use this compile-owned source root too.
    // A mutable checkout still must pass the exact hash/owner/mode admission.
    let development = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("resources/grok")
        .join(entry.filename);
    admit(&development)?;
    Ok(development)
}

/// Compare the returned identity again at the final owner fence, before launch.
/// The hash is read from the executable itself on every admission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FileIdentity {
    dev: u64,
    ino: u64,
    len: u64,
    uid: u32,
    mode: u32,
    mtime: (i64, i64),
    ctime: (i64, i64),
    sha256: String,
}
#[cfg(unix)]
fn identity(meta: &fs::Metadata, sha256: String) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;
    FileIdentity {
        dev: meta.dev(),
        ino: meta.ino(),
        len: meta.len(),
        uid: meta.uid(),
        mode: meta.mode(),
        mtime: (meta.mtime(), meta.mtime_nsec()),
        ctime: (meta.ctime(), meta.ctime_nsec()),
        sha256,
    }
}
#[cfg(unix)]
pub(super) fn admit(path: &Path) -> Result<FileIdentity, String> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let expected = compiled_artifact()?;
    let before = fs::symlink_metadata(path).map_err(|_| unavailable())?;
    if !before.is_file()
        || before.file_type().is_symlink()
        || before.len() == 0
        || before.len() > MAX_BINARY
        || before.mode() & 0o022 != 0
        || before.mode() & 0o111 == 0
        || (before.uid() != unsafe { libc::geteuid() } && before.uid() != 0)
    {
        return Err(unavailable());
    }
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| unavailable())?;
    let opened = file.metadata().map_err(|_| unavailable())?;
    if identity(&before, String::new()) != identity(&opened, String::new()) {
        return Err(unavailable());
    }
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|_| unavailable())?;
        if n == 0 {
            break;
        }
        total = total.checked_add(n as u64).ok_or_else(unavailable)?;
        if total > MAX_BINARY {
            return Err(unavailable());
        }
        hash.update(&buffer[..n]);
    }
    let after = file.metadata().map_err(|_| unavailable())?;
    let named = fs::symlink_metadata(path).map_err(|_| unavailable())?;
    if total != opened.len()
        || identity(&after, String::new()) != identity(&opened, String::new())
        || identity(&named, String::new()) != identity(&opened, String::new())
        || named.file_type().is_symlink()
    {
        return Err(unavailable());
    }
    let digest = format!("{:x}", hash.finalize());
    if digest != expected.sha256 {
        return Err(unavailable());
    }
    Ok(identity(&after, digest))
}
#[cfg(not(unix))]
pub(super) fn admit(_: &Path) -> Result<FileIdentity, String> {
    Err(unavailable())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn published_version_cannot_supply_source_identity() {
        assert!(!version_matches(b"grok 1.0.45\n"));
        assert!(!version_matches(b"grok 1.0.46\n"));
        assert!(!version_matches(b"grok 1.0.45 (unknown)\n"));
    }
    #[test]
    fn missing_executable_is_never_admitted() {
        assert!(admit(Path::new("/lomi-no-such-grok-artifact")).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn arbitrary_bytes_symlink_and_writable_artifact_are_rejected() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let temporary = tempfile::tempdir().unwrap();
        let executable = temporary.path().join("grok");
        fs::write(&executable, b"unreviewed executable").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(admit(&executable).is_err());
        let link = temporary.path().join("linked-grok");
        symlink(&executable, &link).unwrap();
        assert!(admit(&link).is_err());
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(admit(&executable).is_err());
    }
}
