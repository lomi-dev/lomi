//! Fixed official native-byte admission for the reviewed macOS ARM64 clients.
//! This proves the artifact at admission/recheck, not process lifecycle ownership.
use crate::cli_catalog::TitleCli;
use std::path::{Path, PathBuf};

const CODEX_VERSION: &str = "0.160.0";
const CODEX_SHA256: &str = "112fae7a5a1223e673c8a1791d32338f37df8b527ff1159bb8adac6c4dbf1b4b";
// Official rust-v0.160.0/codex-aarch64-apple-darwin.tar.gz:
// 07c3c7ca376a8f791115342f53138dda37e97cfa29b8125d0652d93784894b5d
const CLAUDE_VERSION: &str = "2.1.287";
const CLAUDE_SHA256: &str = "6eab8333fe2121553100d8f40bfada384a3e989b94f947e18ba6677a6fcb41ea";
// Official @anthropic-ai/claude-code-darwin-arm64-2.1.287.tgz:
// 81f45ee02e22cf26a79ca4134f5b0b6f680fb9b5155694cd37088756ae6d9278
const PLATFORM: &str = "darwin-arm64";
const MAX_BINARY: u64 = 512 * 1024 * 1024;

fn failure() -> String {
    "Native Codex and Claude require the reviewed Codex 0.160.0 or Claude 2.1.287 macOS ARM64 binary. The executable changed or could not be verified; reinstall the pinned native tools.".into()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    dev: u64,
    ino: u64,
    size: u64,
    uid: u32,
    mode: u32,
    nlink: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

/// Constructible only through the fixed production digest gate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Proof {
    cli: TitleCli,
    original: PathBuf,
    canonical: PathBuf,
    version: &'static str,
    sha256: &'static str,
    identity: Identity,
}
impl Proof {
    pub(super) fn cli(&self) -> TitleCli {
        self.cli
    }
    pub(super) fn path(&self) -> &Path {
        &self.canonical
    }
    pub(super) fn version(&self) -> &'static str {
        self.version
    }
    pub(super) fn platform(&self) -> &'static str {
        PLATFORM
    }
    pub(super) fn sha256(&self) -> &'static str {
        self.sha256
    }
}

#[cfg(unix)]
fn identity(meta: &std::fs::Metadata) -> Identity {
    use std::os::unix::fs::MetadataExt;
    Identity {
        dev: meta.dev(),
        ino: meta.ino(),
        size: meta.len(),
        uid: meta.uid(),
        mode: meta.mode(),
        nlink: meta.nlink(),
        mtime: (meta.mtime(), meta.mtime_nsec()),
        ctime: (meta.ctime(), meta.ctime_nsec()),
    }
}

#[cfg(unix)]
fn safe(meta: &std::fs::Metadata) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    if !meta.is_file()
        || meta.file_type().is_symlink()
        || meta.len() == 0
        || meta.len() > MAX_BINARY
        || meta.nlink() != 1
        || meta.mode() & 0o022 != 0
        || meta.mode() & 0o6000 != 0
        || meta.mode() & 0o111 == 0
        || meta.uid() != 0 && meta.uid() != unsafe { libc::geteuid() }
    {
        return Err(failure());
    }
    Ok(())
}

#[cfg(unix)]
fn verify_file(original: &Path, expected: &str) -> Result<(PathBuf, Identity), String> {
    use sha2::{Digest, Sha256};
    use std::{fs, io::Read, os::unix::fs::OpenOptionsExt};
    if !original.is_absolute() {
        return Err(failure());
    }
    let canonical = original.canonicalize().map_err(|_| failure())?;
    let before = fs::symlink_metadata(&canonical).map_err(|_| failure())?;
    safe(&before)?;
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(&canonical)
        .map_err(|_| failure())?;
    let opened = file.metadata().map_err(|_| failure())?;
    safe(&opened)?;
    let pinned = identity(&opened);
    if identity(&before) != pinned {
        return Err(failure());
    }
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer).map_err(|_| failure())?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .filter(|size| *size <= MAX_BINARY)
            .ok_or_else(failure)?;
        hash.update(&buffer[..count]);
    }
    let after = file.metadata().map_err(|_| failure())?;
    let named = fs::symlink_metadata(&canonical).map_err(|_| failure())?;
    safe(&after)?;
    safe(&named)?;
    if total != pinned.size
        || identity(&after) != pinned
        || identity(&named) != pinned
        || original.canonicalize().map_err(|_| failure())? != canonical
        || hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
            != expected
    {
        return Err(failure());
    }
    Ok((canonical, pinned))
}

/// Must run before `--version` or any other execution of a listed client.
/// The returned canonical path is the only admitted executable, not a wrapper.
pub(super) fn admit(cli: TitleCli, path: &Path) -> Result<Option<Proof>, String> {
    let (version, sha256) = match cli {
        TitleCli::Codex => (CODEX_VERSION, CODEX_SHA256),
        TitleCli::Claude => (CLAUDE_VERSION, CLAUDE_SHA256),
        _ => return Ok(None),
    };
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return Err(failure());
    }
    #[cfg(unix)]
    {
        let (canonical, identity) = verify_file(path, sha256)?;
        Ok(Some(Proof {
            cli,
            original: path.to_owned(),
            canonical,
            version,
            sha256,
            identity,
        }))
    }
    #[cfg(not(unix))]
    {
        let _ = (version, sha256, path);
        Err(failure())
    }
}

/// Repeat immediately before each version probe or process launch. An identical
/// replacement digest does not preserve admission if its file identity changed.
pub(super) fn recheck(proof: &Proof) -> Result<(), String> {
    #[cfg(unix)]
    {
        recheck_file(
            &proof.original,
            &proof.canonical,
            &proof.identity,
            proof.sha256,
        )
    }
    #[cfg(not(unix))]
    {
        let _ = proof;
        Err(failure())
    }
}

#[cfg(unix)]
fn recheck_file(
    original: &Path,
    canonical: &Path,
    pinned: &Identity,
    digest: &str,
) -> Result<(), String> {
    let (current_path, current) = verify_file(original, digest)?;
    if current_path != canonical || current != *pinned {
        return Err(failure());
    }
    Ok(())
}
