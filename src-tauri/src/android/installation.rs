use super::storage::{self, Directory};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

const MARKER: &str = ".lomi-package.json";
const REVISION_LIMIT: u64 = (1 << 53) - 1;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Installed {
    pub id: String,
    pub revision: String,
    pub archive_sha1: String,
}

impl Installed {
    pub(super) fn validate(&self) -> Result<(), String> {
        package_path(&self.id)?;
        if self.revision.is_empty()
            || self.revision.len() > 40
            || !self
                .revision
                .split('.')
                .all(|v| !v.is_empty() && v.parse::<u32>().is_ok())
            || self.archive_sha1.len() != 40
            || !self.archive_sha1.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("Invalid installed Android package identity.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub revision: u64,
    pub packages: BTreeMap<String, Installed>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: 1,
            revision: 0,
            packages: BTreeMap::new(),
        }
    }
}

impl Manifest {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.version != 1 || self.revision > REVISION_LIMIT || self.packages.len() > 1024 {
            return Err("Unsupported Android tools manifest; the file was preserved.".into());
        }
        for (id, package) in &self.packages {
            package.validate()?;
            if id != &package.id {
                return Err("Mismatched Android package identity.".into());
            }
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Journal {
    version: u32,
    operation: String,
    before: Manifest,
    after: Manifest,
    package: Installed,
}

impl Journal {
    fn validate(&self) -> Result<(), String> {
        if self.version != 1 || !storage::valid_id(&self.operation) {
            return Err("Invalid Android installation journal; recover it explicitly.".into());
        }
        self.before.validate()?;
        self.after.validate()?;
        self.package.validate()?;
        let mut expected = self.before.clone();
        expected.revision = expected
            .revision
            .checked_add(1)
            .ok_or("Manifest revision overflow")?;
        expected
            .packages
            .insert(self.package.id.clone(), self.package.clone());
        if expected.revision != self.after.revision || expected.packages != self.after.packages {
            return Err("Android installation journal contains inconsistent revisions.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    Journal,
    PreviousMoved,
    CandidateMoved,
    Manifest,
    RollbackRetained,
}

impl Directory {
    pub fn installed_path(&self, id: &str) -> Result<PathBuf, String> {
        let manifest = self.manifest()?;
        let package = manifest.packages.get(id).ok_or(
            "This Android package is not installed. Open Android settings to install or repair it.",
        )?;
        let path = self.active_path(id)?;
        verify_marker(&path, package)?;
        Ok(path)
    }
    pub fn manifest(&self) -> Result<Manifest, String> {
        let manifest: Manifest =
            storage::read(&self.root.join("tools-manifest.json"))?.unwrap_or_default();
        manifest.validate()?;
        Ok(manifest)
    }

    /// Call only after download integrity, extraction and native artifact checks succeed.
    /// The candidate stays private until its durable journal has been written.
    pub fn stage_verified(
        &mut self,
        operation: &str,
        package: &Installed,
    ) -> Result<PathBuf, String> {
        package.validate()?;
        let stage = self.stage_path(operation)?;
        let candidate = stage.join("candidate");
        check_directory(&candidate)?;
        storage::write(&candidate.join(MARKER), package)?;
        sync_directory(&candidate)?;
        Ok(candidate)
    }

    pub fn publish(
        &mut self,
        operation: &str,
        package: Installed,
        running: &[String],
    ) -> Result<(), String> {
        self.publish_inner(operation, package, running, |_| Ok(()))
    }

    fn publish_inner(
        &mut self,
        operation: &str,
        package: Installed,
        running: &[String],
        checkpoint: impl Fn(Step) -> Result<(), String>,
    ) -> Result<(), String> {
        if self
            .root
            .join("installation.json")
            .try_exists()
            .map_err(io_error)?
        {
            return Err(
                "An interrupted Android installation requires recovery before another mutation."
                    .into(),
            );
        }
        package.validate()?;
        let before = self.manifest()?;
        let devices = self.devices()?;
        if package.id.starts_with("system-images;") {
            let users: Vec<_> = devices
                .devices
                .iter()
                .filter(|d| d.image == package.id)
                .collect();
            if users.iter().any(|d| running.contains(&d.id)) {
                return Err(
                    "Stop every device using this image before repairing its files.".into(),
                );
            }
            if users
                .iter()
                .any(|d| d.image_revision.to_string() != package.revision)
                || (!users.is_empty()
                    && before
                        .packages
                        .get(&package.id)
                        .is_some_and(|old| old != &package))
            {
                return Err("This image revision is used by a device and is immutable, including while stopped. Create a device with a different image package.".into());
            }
        } else if !running.is_empty() {
            return Err("Stop Android devices before changing managed tools.".into());
        }
        let stage = self.stage_path(operation)?;
        let candidate = stage.join("candidate");
        verify_marker(&candidate, &package)?;
        let active = self.active_path(&package.id)?;
        match before.packages.get(&package.id) {
            Some(old) => verify_marker(&active, old)?,
            None if active.try_exists().map_err(io_error)? => return Err("An unregistered SDK directory exists. Inspect it in Android recovery before installing.".into()),
            None => {},
        }
        let mut after = before.clone();
        after.revision = after
            .revision
            .checked_add(1)
            .filter(|n| *n <= REVISION_LIMIT)
            .ok_or("Manifest revision limit reached")?;
        after.packages.insert(package.id.clone(), package.clone());
        let journal = Journal {
            version: 1,
            operation: operation.into(),
            before,
            after,
            package,
        };
        let journal_path = self.root.join("installation.json");
        storage::write(&journal_path, &journal)?;
        checkpoint(Step::Journal)?;
        if journal.before.packages.contains_key(&journal.package.id) {
            move_directory(&active, &stage.join("previous"))?;
        }
        checkpoint(Step::PreviousMoved)?;
        create_directories(&self.root, active.parent().ok_or("Missing SDK parent")?)?;
        move_directory(&candidate, &active)?;
        checkpoint(Step::CandidateMoved)?;
        storage::write(&self.root.join("tools-manifest.json"), &journal.after)?;
        checkpoint(Step::Manifest)?;
        self.finish_publication(&journal)?;
        checkpoint(Step::RollbackRetained)?;
        fs::remove_file(journal_path).map_err(io_error)?;
        sync_directory(&self.root)
    }

    /// Recovery must run while holding the directory owner lock and before any start.
    /// A missing/invalid marker blocks recovery instead of guessing which files to delete.
    pub fn recover_installation(&mut self) -> Result<bool, String> {
        let path = self.root.join("installation.json");
        let Some(journal): Option<Journal> = storage::read(&path)? else {
            return Ok(false);
        };
        journal.validate()?;
        let manifest = self.manifest()?;
        let active = self.active_path(&journal.package.id)?;
        let stage = self.stage_path(&journal.operation)?;
        if manifest.revision == journal.after.revision
            && manifest.packages == journal.after.packages
        {
            verify_marker(&active, &journal.package)?;
            self.finish_publication(&journal)?;
        } else if manifest.revision == journal.before.revision
            && manifest.packages == journal.before.packages
        {
            let previous = stage.join("previous");
            if previous.try_exists().map_err(io_error)? {
                let old = journal
                    .before
                    .packages
                    .get(&journal.package.id)
                    .ok_or("Unexpected previous package in installation journal")?;
                verify_marker(&previous, old)?;
                if active.try_exists().map_err(io_error)? {
                    verify_marker(&active, &journal.package)?;
                    move_directory(&active, &stage.join("discarded"))?;
                }
                move_directory(&previous, &active)?;
            } else if let Some(old) = journal.before.packages.get(&journal.package.id) {
                verify_marker(&active, old)?;
            } else if active.try_exists().map_err(io_error)? {
                verify_marker(&active, &journal.package)?;
                move_directory(&active, &stage.join("discarded"))?;
            }
            if stage.try_exists().map_err(io_error)? {
                remove_directory(&stage)?;
            }
        } else {
            return Err("Android manifest changed outside its installation journal. Files were preserved for recovery.".into());
        }
        fs::remove_file(path).map_err(io_error)?;
        sync_directory(&self.root)?;
        Ok(true)
    }

    fn stage_path(&self, operation: &str) -> Result<PathBuf, String> {
        if !storage::valid_id(operation) {
            return Err("Invalid Android installation ID".into());
        }
        checked_path(&self.root, &PathBuf::from("staging").join(operation))
    }

    fn active_path(&self, id: &str) -> Result<PathBuf, String> {
        checked_path(&self.root, &PathBuf::from("sdk").join(package_path(id)?))
    }

    fn finish_publication(&self, journal: &Journal) -> Result<(), String> {
        let stage = self.stage_path(&journal.operation)?;
        let previous = stage.join("previous");
        if previous.try_exists().map_err(io_error)? {
            verify_marker(
                &previous,
                journal
                    .before
                    .packages
                    .get(&journal.package.id)
                    .ok_or("Unexpected previous package")?,
            )?;
            if journal.package.id.starts_with("system-images;") {
                // Images have one immutable revision. Only tools keep one rollback copy.
                move_directory(&previous, &stage.join("discarded"))?;
            } else {
                let hash = format!("{:x}", Sha256::digest(journal.package.id.as_bytes()));
                let retained = checked_path(&self.root, &PathBuf::from("rollback").join(hash))?;
                create_directories(&self.root, retained.parent().unwrap())?;
                if retained.try_exists().map_err(io_error)? {
                    let old: Installed = storage::read(&retained.join(MARKER))?
                        .ok_or("Rollback package has no identity")?;
                    old.validate()?;
                    if old.id != journal.package.id {
                        return Err("Unexpected rollback package identity".into());
                    }
                    // Make cleanup restartable even if remove_dir_all is interrupted
                    // after deleting the identity marker but before the last artifact.
                    move_directory(&retained, &stage.join("discarded"))?;
                }
                move_directory(&previous, &retained)?;
            }
        }
        if stage.try_exists().map_err(io_error)? {
            remove_directory(&stage)?;
        }
        Ok(())
    }
}

pub(super) fn package_path(id: &str) -> Result<PathBuf, String> {
    let parts: Vec<_> = id.split(';').collect();
    let component = |value: &str| {
        !value.is_empty()
            && value.len() <= 100
            && value.as_bytes()[0].is_ascii_alphanumeric()
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            && !value.contains("..")
    };
    let allowed = matches!(parts.as_slice(), ["emulator"] | ["platform-tools"])
        || matches!(parts.as_slice(), ["cmdline-tools", version] if component(version))
        || matches!(parts.as_slice(), ["system-images", api, tag, abi] if api.starts_with("android-") && component(api) && component(tag) && matches!(*abi, "arm64-v8a" | "x86_64"));
    if !allowed {
        return Err("Unsupported managed SDK package path".into());
    }
    Ok(parts.iter().collect())
}

pub(super) fn checked_path(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    let mut result = root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            return Err("Invalid managed directory path".into());
        };
        result.push(component);
        match fs::symlink_metadata(&result) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(
                    "Managed SDK directories cannot contain links or ordinary files in their path."
                        .into(),
                )
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(error)),
        }
    }
    Ok(result)
}

fn check_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Expected a regular managed directory".into());
    }
    Ok(())
}

pub(super) fn create_directories(root: &Path, path: &Path) -> Result<(), String> {
    checked_path(root, path.strip_prefix(root).map_err(io_error)?)?;
    fs::create_dir_all(path).map_err(io_error)?;
    sync_directory(path)
}

pub(super) fn verify_marker(path: &Path, expected: &Installed) -> Result<(), String> {
    check_directory(path)?;
    let actual: Installed =
        storage::read(&path.join(MARKER))?.ok_or("Managed SDK package has no verified identity")?;
    if &actual != expected {
        return Err(
            "Managed SDK package identity differs from the manifest. Files were preserved.".into(),
        );
    }
    Ok(())
}

pub(super) fn move_directory(from: &Path, to: &Path) -> Result<(), String> {
    check_directory(from)?;
    if to.try_exists().map_err(io_error)? {
        return Err("Installation destination is occupied".into());
    }
    fs::rename(from, to).map_err(io_error)?;
    sync_directory(from.parent().ok_or("Missing source parent")?)?;
    sync_directory(to.parent().ok_or("Missing destination parent")?)
}

fn remove_directory(path: &Path) -> Result<(), String> {
    check_directory(path)?;
    fs::remove_dir_all(path).map_err(io_error)?;
    sync_directory(path.parent().ok_or("Missing managed parent")?)
}

pub(super) fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    fs::File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(io_error)?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn io_error(error: impl std::fmt::Display) -> String {
    format!("Android installation: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIRST: &str = "00000000-0000-0000-0000-000000000001";
    const SECOND: &str = "00000000-0000-0000-0000-000000000002";

    fn stage(directory: &mut Directory, operation: &str, id: &str, revision: &str) -> Installed {
        let package = Installed {
            id: id.into(),
            revision: revision.into(),
            archive_sha1: "a".repeat(40),
        };
        let candidate = directory.stage_path(operation).unwrap().join("candidate");
        fs::create_dir_all(&candidate).unwrap();
        fs::write(candidate.join("artifact"), revision).unwrap();
        directory.stage_verified(operation, &package).unwrap();
        package
    }

    #[test]
    fn interrupted_publication_recovers_every_directory_manifest_boundary() {
        for with_previous in [false, true] {
            for failure in [
                Step::Journal,
                Step::PreviousMoved,
                Step::CandidateMoved,
                Step::Manifest,
                Step::RollbackRetained,
            ] {
                let temp = tempfile::tempdir().unwrap();
                let root = temp.path().join("android");
                let mut directory = Directory::acquire(root.clone()).unwrap();
                fs::create_dir_all(root.join("avd/keep.avd")).unwrap();
                fs::write(root.join("avd/keep.avd/userdata"), "guest data").unwrap();
                if with_previous {
                    let old = stage(&mut directory, FIRST, "emulator", "1.0.0");
                    directory.publish(FIRST, old, &[]).unwrap();
                }
                let new = stage(&mut directory, SECOND, "emulator", "2.0.0");
                assert!(directory
                    .publish_inner(SECOND, new.clone(), &[], |at| if at == failure {
                        Err("Simulated interruption".into())
                    } else {
                        Ok(())
                    })
                    .is_err());
                drop(directory);
                let mut directory = Directory::acquire(root.clone()).unwrap();
                assert!(directory.recover_installation().unwrap(), "{failure:?}");
                assert!(!directory.recover_installation().unwrap());
                let manifest = directory.manifest().unwrap();
                if matches!(failure, Step::Manifest | Step::RollbackRetained) {
                    assert_eq!(manifest.packages["emulator"], new);
                    assert_eq!(
                        fs::read_to_string(root.join("sdk/emulator/artifact")).unwrap(),
                        "2.0.0"
                    );
                    if with_previous {
                        let retained = root
                            .join("rollback")
                            .join(format!("{:x}", Sha256::digest(b"emulator")));
                        assert_eq!(
                            fs::read_to_string(retained.join("artifact")).unwrap(),
                            "1.0.0"
                        );
                    }
                } else if with_previous {
                    assert_eq!(manifest.packages["emulator"].revision, "1.0.0");
                    assert_eq!(
                        fs::read_to_string(root.join("sdk/emulator/artifact")).unwrap(),
                        "1.0.0"
                    );
                } else {
                    assert!(manifest.packages.is_empty());
                    assert!(!root.join("sdk/emulator").exists());
                }
                assert_eq!(
                    fs::read_to_string(root.join("avd/keep.avd/userdata")).unwrap(),
                    "guest data"
                );
            }
        }
    }

    #[test]
    fn stopped_devices_pin_image_files_and_repairs_require_stopped_users() {
        use super::super::storage::{Device, Devices, Gpu, Hardware};
        let temp = tempfile::tempdir().unwrap();
        let mut directory = Directory::acquire(temp.path().join("android")).unwrap();
        let id = "system-images;android-36;default;arm64-v8a";
        let original = stage(&mut directory, FIRST, id, "2");
        directory.publish(FIRST, original.clone(), &[]).unwrap();
        directory
            .save_devices(
                Devices {
                    devices: vec![Device {
                        id: FIRST.into(),
                        name: "Phone".into(),
                        image: id.into(),
                        image_revision: 2,
                        profile: "Nexus 5".into(),
                        hardware: Hardware {
                            ram_mib: 2048,
                            cpu_count: 2,
                            data_gib: 2,
                            gpu: Gpu::Auto,
                            quick_boot: true,
                        },
                        input_bridge: true,
                    }],
                    ..Devices::default()
                },
                0,
            )
            .unwrap();
        let changed = stage(&mut directory, SECOND, id, "3");
        assert!(directory
            .publish(SECOND, changed, &[])
            .unwrap_err()
            .contains("immutable"));
        let repair = stage(&mut directory, SECOND, id, "2");
        assert!(directory
            .publish(SECOND, repair.clone(), &[FIRST.into()])
            .unwrap_err()
            .contains("Stop"));
        directory.publish(SECOND, repair, &[]).unwrap();
        assert_eq!(directory.manifest().unwrap().packages[id], original);
        assert!(!directory.root.join("rollback").exists());
    }

    #[test]
    fn unexpected_files_or_corrupt_journal_are_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let mut directory = Directory::acquire(temp.path().join("android")).unwrap();
        let package = stage(&mut directory, FIRST, "emulator", "1");
        fs::write(directory.root.join("installation.json"), "{broken").unwrap();
        assert!(directory.recover_installation().is_err());
        assert!(directory.publish(FIRST, package, &[]).is_err());
        assert_eq!(
            fs::read_to_string(directory.root.join("installation.json")).unwrap(),
            "{broken"
        );
        assert!(package_path("system-images;android-36;../../avd;arm64-v8a").is_err());
        assert!(directory.stage_path("../avd").is_err());
    }
}
