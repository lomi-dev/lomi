//! Proof of the standard Pi entry and its complete script-free npm closure.
//! The digest was derived from an isolated npm ci using the embedded installer
//! lock, public registry integrity pins and empty configs, not a live receipt.
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
const ENTRY: &str = "node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js";
const CLOSURE: &str = "007dd63a973108bd475255b06a91873f217e32de84eb86ddfe0257a1c0ffd1dd";
const COUNT: usize = 15025;
// Official node-v22.22.3-darwin-arm64.tar.gz SHA256:
// 0da7ff74ef8611328c8212f17943368713a2ad953fb7d89a8c8a0eae87c23207
const NODE: &str = "5d9d3872911e2340a43b707962e68143de8a4e8d54628845c0c4f2de1fb7cd5c";
fn failure() -> String {
    "Managed Pi requires the reviewed Pi 1.0.1 dependency closure and Node 22.22.3 standard entry; reinstall the pinned native tools.".into()
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Proof {
    pub(super) node: PathBuf,
    root: PathBuf,
    entry: PathBuf,
}
fn safe(m: &fs::Metadata) -> Result<(), String> {
    if m.uid() != unsafe { libc::geteuid() }
        || (!m.file_type().is_symlink() && m.mode() & 0o022 != 0)
    {
        return Err(failure());
    }
    Ok(())
}
fn hash(path: &Path, budget: &mut u64) -> Result<String, String> {
    let mut f = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| failure())?;
    let before = f.metadata().map_err(|_| failure())?;
    safe(&before)?;
    if !before.is_file() || before.nlink() != 1 || before.len() > 128 * 1024 * 1024 {
        return Err(failure());
    }
    *budget = budget
        .checked_add(before.len())
        .filter(|n| *n <= 1024 * 1024 * 1024)
        .ok_or_else(failure)?;
    let mut h = Sha256::new();
    let mut b = [0u8; 65536];
    loop {
        let n = f.read(&mut b).map_err(|_| failure())?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    let after = f.metadata().map_err(|_| failure())?;
    let named = fs::symlink_metadata(path).map_err(|_| failure())?;
    if (
        before.dev(),
        before.ino(),
        before.len(),
        before.mtime(),
        before.mtime_nsec(),
    ) != (
        after.dev(),
        after.ino(),
        after.len(),
        after.mtime(),
        after.mtime_nsec(),
    ) || named.ino() != before.ino()
        || named.dev() != before.dev()
        || !named.is_file()
    {
        return Err(failure());
    }
    Ok(format!("{:x}", h.finalize()))
}
fn closure(root: &Path) -> Result<(usize, String), String> {
    fn walk(
        root: &Path,
        path: &Path,
        rows: &mut Vec<(String, &'static str, String)>,
        budget: &mut u64,
        depth: usize,
        visited: &mut usize,
    ) -> Result<(), String> {
        *visited += 1;
        if depth > 32 || rows.len() > 20000 || *visited > 30000 {
            return Err(failure());
        }
        let m = fs::symlink_metadata(path).map_err(|_| failure())?;
        safe(&m)?;
        if m.is_dir() {
            for e in fs::read_dir(path).map_err(|_| failure())? {
                walk(
                    root,
                    &e.map_err(|_| failure())?.path(),
                    rows,
                    budget,
                    depth + 1,
                    visited,
                )?;
            }
            let after = fs::symlink_metadata(path).map_err(|_| failure())?;
            if !after.is_dir()
                || (m.dev(), m.ino(), m.mtime(), m.mtime_nsec())
                    != (after.dev(), after.ino(), after.mtime(), after.mtime_nsec())
            {
                return Err(failure());
            }
        } else {
            let name = path
                .strip_prefix(root)
                .map_err(|_| failure())?
                .to_str()
                .ok_or_else(failure)?
                .to_owned();
            if m.file_type().is_symlink() {
                // Unix link mode is conventionally 0777; only ownership matters.
                let target = fs::read_link(path).map_err(|_| failure())?;
                let canonical = path.canonicalize().map_err(|_| failure())?;
                if target.is_absolute() || !canonical.starts_with(root) || !canonical.is_file() {
                    return Err(failure());
                }
                rows.push((name, "l", target.to_str().ok_or_else(failure)?.into()));
            } else if m.is_file() {
                rows.push((name, "f", hash(path, budget)?));
            } else {
                return Err(failure());
            }
        }
        Ok(())
    }
    let m = fs::symlink_metadata(root).map_err(|_| failure())?;
    safe(&m)?;
    if !m.is_dir() {
        return Err(failure());
    }
    let mut rows = Vec::new();
    let mut budget = 0;
    let mut visited = 0;
    for name in ["package.json", "package-lock.json", "node_modules"] {
        walk(
            root,
            &root.join(name),
            &mut rows,
            &mut budget,
            0,
            &mut visited,
        )?;
    }
    rows.sort();
    let mut h = Sha256::new();
    for (p, k, v) in &rows {
        for x in [p.as_str(), *k, v.as_str()] {
            h.update(x.as_bytes());
            h.update([0]);
        }
    }
    Ok((rows.len(), format!("{:x}", h.finalize())))
}
pub(super) fn admit(entry: &Path) -> Result<Proof, String> {
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return Err(failure());
    }
    let entry = entry.canonicalize().map_err(|_| failure())?;
    let root = entry.ancestors().nth(6).ok_or_else(failure)?.to_owned();
    if root.join(ENTRY) != entry || closure(&root)? != (COUNT, CLOSURE.into()) {
        return Err(failure());
    }
    let mut paths: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| super::native_launch::trusted_search_directory(p))
    {
        paths.push(home.join(".local/bin"));
    }
    paths.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
    if paths.len() > 256 {
        return Err(failure());
    }
    let node = paths
        .into_iter()
        .filter(|p| super::native_launch::trusted_search_directory(p))
        .map(|p| p.join("node"))
        .find_map(|p| {
            let p = p.canonicalize().ok()?;
            let mut budget = 0;
            (hash(&p, &mut budget).ok()?.as_str() == NODE).then_some(p)
        })
        .ok_or_else(failure)?;
    Ok(Proof { node, root, entry })
}
pub(super) fn recheck(proof: &Proof) -> Result<(), String> {
    let mut budget = 0;
    if hash(&proof.node, &mut budget)? != NODE
        || closure(&proof.root)? != (COUNT, CLOSURE.into())
        || proof.root.join(ENTRY) != proof.entry
    {
        return Err(failure());
    }
    Ok(())
}
