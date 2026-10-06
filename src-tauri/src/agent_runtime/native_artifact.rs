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

/// The optional hook is private and used only by deterministic race fixtures.
/// Production always passes a no-op; no caller can supply an expected digest.
#[cfg(unix)]
fn verify_file(
    original: &Path,
    expected: &str,
    opened_hook: impl FnOnce(&Path, &std::fs::File),
) -> Result<(PathBuf, Identity), String> {
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
    opened_hook(&canonical, &file);
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
        let (canonical, identity) = verify_file(path, sha256, |_, _| {})?;
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
    let (current_path, current) = verify_file(original, digest, |_, _| {})?;
    if current_path != canonical || current != *pinned {
        return Err(failure());
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        os::unix::fs::{symlink, PermissionsExt},
    };
    const BYTES: &[u8] = b"private artifact fixture bytes; never executed";
    fn fixture() -> (tempfile::TempDir, PathBuf, String) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("native");
        fs::write(&path, BYTES).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        (
            root,
            path,
            Sha256::digest(BYTES)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        )
    }
    #[test]
    fn native_artifact_private_fixture_accepts_exact_bytes_and_resolved_normal_alias() {
        let (root, path, digest) = fixture();
        let expected = verify_file(&path, &digest, |_, _| {}).unwrap();
        let alias = root.path().join("cli");
        symlink(&path, &alias).unwrap();
        assert_eq!(verify_file(&alias, &digest, |_, _| {}).unwrap(), expected);
        assert!(admit(TitleCli::Codex, &path).is_err());
        assert!(admit(TitleCli::Claude, &alias).is_err());
        assert!(admit(TitleCli::Pi, &path).unwrap().is_none());
    }
    #[test]
    fn native_artifact_rejects_wrappers_mutations_permissions_and_hardlinks() {
        let (root, path, digest) = fixture();
        fs::write(&path, b"#!/bin/sh\necho codex 0.160.0\n").unwrap();
        assert!(verify_file(&path, &digest, |_, _| {}).is_err());
        fs::write(&path, BYTES).unwrap();
        for mode in [0o777, 0o720, 0o702, 0o600, 0o4700, 0o2700] {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            assert!(verify_file(&path, &digest, |_, _| {}).is_err());
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        fs::hard_link(&path, root.path().join("hardlink")).unwrap();
        assert!(verify_file(&path, &digest, |_, _| {}).is_err());
    }
    #[test]
    fn native_artifact_recheck_rejects_same_digest_replacement_and_identity_changes() {
        let (root, path, digest) = fixture();
        let (canonical, pinned) = verify_file(&path, &digest, |_, _| {}).unwrap();
        recheck_file(&path, &canonical, &pinned, &digest).unwrap();
        let replacement = root.path().join("replacement");
        fs::write(&replacement, BYTES).unwrap();
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o700)).unwrap();
        fs::rename(&replacement, &path).unwrap();
        assert!(recheck_file(&path, &canonical, &pinned, &digest).is_err());
        let (canonical, pinned) = verify_file(&path, &digest, |_, _| {}).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o500)).unwrap();
        assert!(recheck_file(&path, &canonical, &pinned, &digest).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let (canonical, pinned) = verify_file(&path, &digest, |_, _| {}).unwrap();
        fs::write(&path, b"mutated after admission").unwrap();
        assert!(recheck_file(&path, &canonical, &pinned, &digest).is_err());
    }
    #[test]
    fn native_artifact_rejects_path_replacement_and_alias_retarget_during_read() {
        let (root, path, digest) = fixture();
        let replacement = root.path().join("replacement");
        fs::write(&replacement, BYTES).unwrap();
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(verify_file(&path, &digest, |canonical, _| {
            fs::rename(&replacement, canonical).unwrap();
        })
        .is_err());
        let alias = root.path().join("alias");
        symlink(&path, &alias).unwrap();
        let other = root.path().join("other");
        fs::write(&other, BYTES).unwrap();
        fs::set_permissions(&other, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(verify_file(&alias, &digest, |_, _| {
            fs::remove_file(&alias).unwrap();
            symlink(&other, &alias).unwrap();
        })
        .is_err());
    }
    #[test]
    fn native_artifact_brackets_in_place_edits_mode_changes_and_bounded_file_kind() {
        let (root, path, digest) = fixture();
        assert!(verify_file(&path, &digest, |canonical, _| {
            fs::write(canonical, b"different bytes").unwrap();
        })
        .is_err());
        fs::write(&path, BYTES).unwrap();
        assert!(verify_file(&path, &digest, |canonical, _| {
            fs::set_permissions(canonical, fs::Permissions::from_mode(0o500)).unwrap();
        })
        .is_err());
        assert!(verify_file(root.path(), &digest, |_, _| {}).is_err());
        assert!(verify_file(Path::new("relative"), &digest, |_, _| {}).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_BINARY + 1)
            .unwrap();
        assert!(verify_file(&path, &digest, |_, _| {}).is_err());
    }
    #[test]
    #[ignore = "Opt-in independently downloaded official macOS ARM64 artifacts; never executes code"]
    fn native_artifact_exact_official_public_fixtures() {
        let root = std::env::var_os("LOMI_NATIVE_ARTIFACT_FIXTURE_ROOT")
            .expect("Provide isolated public artifact fixture root");
        for (cli, name, version, digest) in [
            (TitleCli::Codex, "codex", CODEX_VERSION, CODEX_SHA256),
            (TitleCli::Claude, "claude", CLAUDE_VERSION, CLAUDE_SHA256),
        ] {
            let proof = admit(cli, &PathBuf::from(&root).join(name))
                .unwrap()
                .unwrap();
            assert_eq!(proof.cli(), cli);
            assert!(proof.path().is_absolute());
            assert_eq!(proof.version(), version);
            assert_eq!(proof.platform(), PLATFORM);
            assert_eq!(proof.sha256(), digest);
            recheck(&proof).unwrap();
        }
    }
}
