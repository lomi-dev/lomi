//! Process-local admission for Lomi project writes. External editors and PTYs
//! do not participate; effect CAS still requires the filesystem broker's proof.
use std::{
    collections::HashMap,
    fs::{self, File},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

const STOP_RUN: &str = "Stop the active coding run before changing files in this project.";
const BUSY: &str = "Wait for the current file operation to finish before starting this coding run.";
const UNAVAILABLE: &str = "Project write admission is unavailable.";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ObjectId {
    device: u64,
    inode: u64,
}
#[cfg(unix)]
fn object(meta: &fs::Metadata) -> ObjectId {
    use std::os::unix::fs::MetadataExt;
    ObjectId {
        device: meta.dev(),
        inode: meta.ino(),
    }
}
#[cfg(not(unix))]
fn object(_: &fs::Metadata) -> ObjectId {
    ObjectId {
        device: 0,
        inode: 0,
    }
}
struct Root {
    path: PathBuf,
    file: File,
    identity: ObjectId,
    ancestry: Vec<ObjectId>,
}
impl Root {
    fn open(path: &Path) -> Result<Self, String> {
        #[cfg(not(unix))]
        {
            let _ = path;
            return Err("Coding project leases are unavailable on this platform.".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let path = path
                .canonicalize()
                .map_err(|_| "Cannot resolve the coding project directory.")?;
            let mut options = fs::OpenOptions::new();
            options
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
            let file = options
                .open(&path)
                .map_err(|_| "Cannot retain the coding project directory.")?;
            let meta = file.metadata().map_err(|_| UNAVAILABLE)?;
            if !meta.is_dir() {
                return Err("The coding project must be a directory.".into());
            }
            let identity = object(&meta);
            let ancestry = directories(&path)?;
            let root = Self {
                path,
                file,
                identity,
                ancestry,
            };
            root.check()?;
            Ok(root)
        }
    }
    fn check(&self) -> Result<(), String> {
        let current = fs::symlink_metadata(&self.path).map_err(|_| STOP_RUN)?;
        let retained = self.file.metadata().map_err(|_| STOP_RUN)?;
        if !current.is_dir()
            || current.file_type().is_symlink()
            || object(&current) != self.identity
            || object(&retained) != self.identity
        {
            return Err(STOP_RUN.into());
        }
        Ok(())
    }
}
#[derive(Clone)]
struct Scope {
    path: PathBuf,
    directories: Vec<ObjectId>,
}
impl Scope {
    fn resolve(path: &Path) -> Result<Self, String> {
        if !path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
        {
            return Err("File write destinations must have an absolute resolved path.".into());
        }
        let mut existing = path;
        let mut suffix = Vec::new();
        loop {
            match fs::symlink_metadata(existing) {
                Ok(_) => break,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    suffix.push(existing.file_name().ok_or(UNAVAILABLE)?.to_owned());
                    existing = existing.parent().ok_or(UNAVAILABLE)?;
                }
                Err(_) => return Err(UNAVAILABLE.into()),
            }
        }
        // A dangling or cyclic symlink is an error, not an unowned future path.
        let canonical = existing
            .canonicalize()
            .map_err(|_| "Cannot establish file write destination ownership.")?;
        if !suffix.is_empty() && !canonical.is_dir() {
            return Err("The file write parent is not a directory.".into());
        }
        let mut resolved = canonical.clone();
        for part in suffix.iter().rev() {
            resolved.push(part);
        }
        let ancestor = if canonical.is_dir() {
            canonical.as_path()
        } else {
            canonical.parent().ok_or(UNAVAILABLE)?
        };
        Ok(Self {
            path: resolved,
            directories: directories(ancestor)?,
        })
    }
    fn overlaps(&self, root: &Root) -> bool {
        overlap(&self.path, &root.path)
            || self.directories.contains(&root.identity)
            || fs::metadata(&self.path)
                .is_ok_and(|m| m.is_dir() && root.ancestry.contains(&object(&m)))
    }
}
fn directories(path: &Path) -> Result<Vec<ObjectId>, String> {
    path.ancestors()
        .map(|ancestor| {
            let meta = fs::metadata(ancestor).map_err(|_| UNAVAILABLE)?;
            if !meta.is_dir() {
                return Err(UNAVAILABLE.into());
            }
            Ok(object(&meta))
        })
        .collect()
}
fn overlap(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}
struct ActiveLease {
    root: Arc<Root>,
    run: String,
    generation: u64,
}
#[derive(Default)]
struct Registry {
    sequence: u64,
    leases: HashMap<u64, ActiveLease>,
    writes: HashMap<u64, Option<Vec<Scope>>>,
}
type Shared = Arc<Mutex<Registry>>;
static REGISTRY: OnceLock<Shared> = OnceLock::new();
fn shared() -> Shared {
    REGISTRY
        .get_or_init(|| Arc::new(Mutex::new(Registry::default())))
        .clone()
}
impl Registry {
    fn token(&mut self) -> Result<u64, String> {
        self.sequence = self.sequence.checked_add(1).ok_or(UNAVAILABLE)?;
        Ok(self.sequence)
    }
    fn check_roots(&self) -> Result<(), String> {
        // Namespace drift cannot be safely attributed to one new pathname. Keep
        // all Lomi mutations fenced until the owning run is explicitly stopped.
        for active in self.leases.values() {
            active.root.check()?;
        }
        Ok(())
    }
}
pub(crate) struct Lease {
    registry: Shared,
    token: u64,
    root: Arc<Root>,
    run: String,
    generation: u64,
}
pub(crate) struct WriteAdmission {
    registry: Shared,
    token: u64,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Ok(mut state) = self.registry.lock() {
            state.leases.remove(&self.token);
        }
    }
}
impl Drop for WriteAdmission {
    fn drop(&mut self) {
        if let Ok(mut state) = self.registry.lock() {
            state.writes.remove(&self.token);
        }
    }
}
pub(crate) fn activate(root: &Path, run: &str, generation: u64) -> Result<Lease, String> {
    activate_in(shared(), root, run, generation)
}
fn activate_in(registry: Shared, path: &Path, run: &str, generation: u64) -> Result<Lease, String> {
    if run.is_empty() || run.len() > 256 || generation == 0 {
        return Err("Invalid coding lease owner.".into());
    }
    let mut state = registry.lock().map_err(|_| UNAVAILABLE)?;
    state.check_roots()?;
    let root = Arc::new(Root::open(path)?);
    if state.leases.values().any(|active| {
        overlap(&root.path, &active.root.path)
            || root.ancestry.contains(&active.root.identity)
            || active.root.ancestry.contains(&root.identity)
    }) {
        return Err(STOP_RUN.into());
    }
    if state.writes.values().any(|scopes| {
        scopes
            .as_ref()
            .is_none_or(|scopes| scopes.iter().any(|scope| scope.overlaps(&root)))
    }) {
        return Err(BUSY.into());
    }
    let token = state.token()?;
    state.leases.insert(
        token,
        ActiveLease {
            root: root.clone(),
            run: run.into(),
            generation,
        },
    );
    drop(state);
    Ok(Lease {
        registry,
        token,
        root,
        run: run.into(),
        generation,
    })
}
/// Hold this guard across all staging, replacement, rename and durability I/O.
pub(crate) fn admit(paths: &[&Path]) -> Result<WriteAdmission, String> {
    admit_in(shared(), paths, None, false)
}
/// Broker operations with opaque destinations conservatively fence every project.
pub(crate) fn admit_unscoped() -> Result<WriteAdmission, String> {
    admit_in(shared(), &[], None, true)
}
/// Call while holding the native runtime owner, before committing a new
/// attempt. Its active entry then prevents later MCP effect admission until
/// the worker acquires the retained project lease.
pub(crate) fn ensure_no_unscoped_effects() -> Result<(), String> {
    let registry = shared();
    let state = registry.lock().map_err(|_| UNAVAILABLE)?;
    if state.writes.values().any(Option::is_none) {
        return Err(BUSY.into());
    }
    Ok(())
}
fn admit_in(
    registry: Shared,
    paths: &[&Path],
    owner: Option<u64>,
    unscoped: bool,
) -> Result<WriteAdmission, String> {
    if !unscoped && (paths.is_empty() || paths.len() > 128) {
        return Err("Invalid file write admission scope.".into());
    }
    let mut state = registry.lock().map_err(|_| UNAVAILABLE)?;
    state.check_roots()?;
    let scopes = if unscoped {
        None
    } else {
        Some(
            paths
                .iter()
                .map(|path| Scope::resolve(path))
                .collect::<Result<Vec<_>, _>>()?,
        )
    };
    if let Some(owner) = owner {
        let active = state.leases.get(&owner).ok_or(STOP_RUN)?;
        if scopes.as_ref().is_none_or(|scopes| {
            scopes.iter().any(|scope| {
                !scope.path.starts_with(&active.root.path)
                    || !scope.directories.contains(&active.root.identity)
            })
        }) {
            return Err("The coding effect is outside its retained project lease.".into());
        }
    }
    if state.leases.iter().any(|(id, active)| {
        Some(*id) != owner
            && scopes
                .as_ref()
                .is_none_or(|scopes| scopes.iter().any(|s| s.overlaps(&active.root)))
    }) {
        return Err(STOP_RUN.into());
    }
    let token = state.token()?;
    state.writes.insert(token, scopes);
    drop(state);
    Ok(WriteAdmission { registry, token })
}
impl Lease {
    pub(crate) fn check(&self, run: &str, generation: u64) -> Result<(), String> {
        let state = self.registry.lock().map_err(|_| UNAVAILABLE)?;
        let active = state.leases.get(&self.token).ok_or(STOP_RUN)?;
        if run != self.run
            || generation != self.generation
            || active.run != run
            || active.generation != generation
        {
            return Err(STOP_RUN.into());
        }
        self.root.check()
    }
    #[cfg(test)]
    fn admit_owned(&self, paths: &[&Path]) -> Result<WriteAdmission, String> {
        self.check(&self.run, self.generation)?;
        admit_in(self.registry.clone(), paths, Some(self.token), false)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn registry() -> Shared {
        Arc::new(Mutex::new(Registry::default()))
    }
    #[test]
    fn activation_and_inflight_write_guards_are_mutually_exclusive() {
        let dir = tempfile::tempdir().unwrap();
        let shared = registry();
        let path = dir.path().join("future.txt");
        let write = admit_in(shared.clone(), &[&path], None, false).unwrap();
        assert!(activate_in(shared.clone(), dir.path(), "run_1", 1).is_err());
        drop(write);
        let lease = activate_in(shared.clone(), dir.path(), "run_1", 1).unwrap();
        assert!(admit_in(shared.clone(), &[&path], None, false).is_err());
        assert!(lease.check("run_1", 2).is_err());
        let owned = lease.admit_owned(&[&path]).unwrap();
        drop(owned);
        drop(lease);
        assert!(admit_in(shared, &[&path], None, false).is_ok());
    }
    #[test]
    fn overlapping_roots_and_directory_ancestors_are_fenced() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("nested")).unwrap();
        let shared = registry();
        let _lease = activate_in(shared.clone(), &root, "run_1", 1).unwrap();
        assert!(activate_in(shared.clone(), &root.join("nested"), "run_2", 1).is_err());
        assert!(activate_in(shared.clone(), dir.path(), "run_2", 1).is_err());
        assert!(admit_in(shared.clone(), &[dir.path()], None, false).is_err());
        assert!(admit_in(shared, &[&dir.path().join("outside.txt")], None, false).is_ok());
    }
    #[test]
    fn symlink_aliases_and_nonexistent_children_do_not_bypass_leases() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        let alias = dir.path().join("alias");
        symlink(&root, &alias).unwrap();
        let shared = registry();
        let _lease = activate_in(shared.clone(), &root, "run_1", 1).unwrap();
        assert!(admit_in(shared.clone(), &[&alias.join("new/sub/file")], None, false).is_err());
        assert!(activate_in(shared, &alias, "run_2", 1).is_err());
    }
    #[test]
    fn namespace_drift_is_conservatively_fenced_until_owner_drop() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        let shared = registry();
        let lease = activate_in(shared.clone(), &root, "run_1", 1).unwrap();
        fs::rename(&root, dir.path().join("renamed")).unwrap();
        fs::create_dir(&root).unwrap();
        assert!(lease.check("run_1", 1).is_err());
        assert!(admit_in(
            shared.clone(),
            &[&dir.path().join("otherwise_outside")],
            None,
            false
        )
        .is_err());
        drop(lease);
        assert!(admit_in(
            shared,
            &[&dir.path().join("otherwise_outside")],
            None,
            false
        )
        .is_ok());
    }
    #[test]
    fn opaque_broker_admission_fences_activation_for_its_entire_lifetime() {
        let dir = tempfile::tempdir().unwrap();
        let shared = registry();
        let write = admit_in(shared.clone(), &[], None, true).unwrap();
        assert!(activate_in(shared.clone(), dir.path(), "run_1", 1).is_err());
        drop(write);
        let _lease = activate_in(shared.clone(), dir.path(), "run_1", 1).unwrap();
        assert!(admit_in(shared, &[], None, true).is_err());
    }
}
