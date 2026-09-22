use super::{
    disk,
    installation::{self, Installed, Manifest},
    installer::Operation,
    manager::Manager,
    storage::{self, Directory},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

const JOURNAL: &str = "package-removal.json";
const MARKER: &str = ".lomi-package.json";

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Action {
    Cleanup,
    RestoreMetadata {
        file: super::recovery::File,
        digest: String,
        reset: bool,
    },
    Remove {
        package_id: String,
        expected_revision: u64,
    },
    Rollback {
        package_id: String,
        expected_revision: u64,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Removal {
    version: u32,
    operation: String,
    before: Manifest,
    after: Manifest,
    package: Installed,
    rollback: Option<Installed>,
}
impl Removal {
    fn validate(&self) -> Result<(), String> {
        self.before.validate()?;
        self.after.validate()?;
        self.package.validate()?;
        if let Some(rollback) = &self.rollback {
            rollback.validate()?;
            if rollback.id != self.package.id {
                return Err("Invalid Android rollback identity in removal journal".into());
            }
        }
        let mut expected = self.before.packages.clone();
        if self.version != 1
            || !storage::valid_id(&self.operation)
            || expected.remove(&self.package.id).as_ref() != Some(&self.package)
            || expected != self.after.packages
            || self.before.revision.checked_add(1) != Some(self.after.revision)
        {
            return Err("Invalid Android package-removal journal. Files were preserved.".into());
        }
        Ok(())
    }
}

fn rollback_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    installation::package_path(id)?;
    installation::checked_path(
        root,
        &Path::new("rollback").join(format!("{:x}", Sha256::digest(id.as_bytes()))),
    )
}

pub fn rollbacks(directory: &Directory) -> Result<Vec<Installed>, String> {
    let mut packages = Vec::new();
    for id in directory
        .manifest()?
        .packages
        .keys()
        .filter(|id| !id.starts_with("system-images;"))
    {
        let path = rollback_path(&directory.root, id)?;
        if let Some(package) = storage::read::<Installed>(&path.join(MARKER))? {
            package.validate()?;
            if &package.id != id {
                return Err("Android rollback identity does not match its package".into());
            }
            installation::verify_marker(&path, &package)?;
            packages.push(package);
        }
    }
    Ok(packages)
}

pub async fn perform(
    manager: &Arc<Manager>,
    operation: &str,
    action: Action,
    progress: &Arc<Operation>,
) -> Result<Option<String>, String> {
    let manager = manager.clone();
    let operation = operation.to_owned();
    let progress = progress.clone();
    tokio::task::spawn_blocking(move || {
        let mut directory = manager.directory.lock().map_err(|_| "Android directory failed")?;
        for journal in [JOURNAL, "installation.json", "toolchain-installation.json", "device-operation.json"] {
            if directory.root.join(journal).exists() { return Err("Repair the interrupted Android operation before changing packages.".into()); }
        }
        match action {
            Action::RestoreMetadata { file, digest, reset } => {
                progress.progress("Recovering Android metadata", 0, 0);
                super::recovery::restore(&directory, file, &digest, reset)?;
            }
            Action::Cleanup => {
                progress.progress("Cleaning Android temporary files", 0, 0);
                cleanup(&directory.root, &progress)?;
            }
            Action::Remove { package_id, expected_revision } => {
                progress.progress("Removing managed package", 0, 0);
                if *progress.cancellation().borrow() { return Err("Android package removal cancelled".into()); }
                remove(&mut directory, &operation, &package_id, expected_revision, |_| Ok(()))?;
            }
            Action::Rollback { package_id, expected_revision } => {
                let manifest = directory.manifest()?;
                if manifest.revision != expected_revision { return Err("Android packages changed. Reload and try again.".into()); }
                if package_id.starts_with("system-images;") { return Err("Android images are immutable; create a different device instead of rolling back its system.".into()); }
                let package = rollbacks(&directory)?.into_iter().find(|package| package.id == package_id).ok_or("No previous verified version is retained for this tool")?;
                let source = rollback_path(&directory.root, &package_id)?;
                let size = disk::measure(&source)?.logical_bytes;
                disk::require(&directory.root, size)?;
                let stage = installation::checked_path(&directory.root, &Path::new("staging").join(&operation))?;
                let candidate = stage.join("candidate");
                if stage.exists() { return Err("Android rollback staging directory is occupied".into()); }
                installation::create_directories(&directory.root, &candidate)?;
                let result = (|| {
                    copy_package(&source, &candidate, size, &progress)?;
                    installation::verify_marker(&candidate, &package)?;
                    if *progress.cancellation().borrow() { return Err("Android rollback cancelled".into()); }
                    directory.publish(&operation, package, &[])
                })();
                if stage.exists() && !directory.root.join("installation.json").exists() { fs::remove_dir_all(&stage).map_err(|e| e.to_string())?; }
                result?;
            }
        }
        Ok(None)
    }).await.map_err(|e| e.to_string())?
}

#[derive(Clone, Copy, Debug)]
enum Step {
    Journal,
    ActiveMoved,
    RollbackMoved,
    Manifest,
    Cleanup,
}

fn remove(
    directory: &mut Directory,
    operation: &str,
    id: &str,
    expected_revision: u64,
    checkpoint: impl Fn(Step) -> Result<(), String>,
) -> Result<(), String> {
    if !storage::valid_id(operation) {
        return Err("Invalid Android operation identity".into());
    }
    let before = directory.manifest()?;
    if before.revision != expected_revision {
        return Err("Android packages changed. Reload and try again.".into());
    }
    let package = before
        .packages
        .get(id)
        .cloned()
        .ok_or("Android package is not installed")?;
    if directory
        .devices()?
        .devices
        .iter()
        .any(|device| device.image == id)
    {
        return Err("This system image is used by a device, including while stopped. Remove that device before removing its image.".into());
    }
    let active = directory.installed_path(id)?;
    let retained = rollback_path(&directory.root, id)?;
    let rollback: Option<Installed> = storage::read(&retained.join(MARKER))?;
    if let Some(package) = &rollback {
        installation::verify_marker(&retained, package)?;
    }
    if retained.exists() && rollback.is_none() {
        return Err(
            "Android rollback has no verified identity; repair it before removing this package."
                .into(),
        );
    }
    let mut after = before.clone();
    after.revision = before
        .revision
        .checked_add(1)
        .ok_or("Android manifest revision overflow")?;
    after.packages.remove(id);
    let journal = Removal {
        version: 1,
        operation: operation.into(),
        before,
        after,
        package,
        rollback,
    };
    journal.validate()?;
    let stage = installation::checked_path(&directory.root, &Path::new("staging").join(operation))?;
    if stage.exists() {
        return Err("Android removal staging directory is occupied".into());
    }
    installation::create_directories(&directory.root, &stage)?;
    storage::write(&directory.root.join(JOURNAL), &journal)?;
    checkpoint(Step::Journal)?;
    installation::move_directory(&active, &stage.join("active"))?;
    checkpoint(Step::ActiveMoved)?;
    if journal.rollback.is_some() {
        installation::move_directory(&retained, &stage.join("rollback"))?;
    }
    checkpoint(Step::RollbackMoved)?;
    storage::write(&directory.root.join("tools-manifest.json"), &journal.after)?;
    checkpoint(Step::Manifest)?;
    fs::remove_dir_all(&stage).map_err(|e| e.to_string())?;
    checkpoint(Step::Cleanup)?;
    fs::remove_file(directory.root.join(JOURNAL)).map_err(|e| e.to_string())?;
    installation::sync_directory(&directory.root)
}

pub fn recover(directory: &mut Directory) -> Result<bool, String> {
    let Some(journal): Option<Removal> = storage::read(&directory.root.join(JOURNAL))? else {
        return Ok(false);
    };
    journal.validate()?;
    if directory
        .devices()?
        .devices
        .iter()
        .any(|device| device.image == journal.package.id)
    {
        return Err("A device references the package being removed. Preserve its staged image and resolve the metadata conflict before recovery.".into());
    }
    let manifest = directory.manifest()?;
    let stage = installation::checked_path(
        &directory.root,
        &Path::new("staging").join(&journal.operation),
    )?;
    let active = installation::checked_path(
        &directory.root,
        &Path::new("sdk").join(installation::package_path(&journal.package.id)?),
    )?;
    let retained = rollback_path(&directory.root, &journal.package.id)?;
    if manifest.revision == journal.before.revision && manifest.packages == journal.before.packages
    {
        for (source, target, package) in [
            (stage.join("active"), active, Some(&journal.package)),
            (stage.join("rollback"), retained, journal.rollback.as_ref()),
        ] {
            if let Some(package) = package {
                if source.exists() {
                    installation::verify_marker(&source, package)?;
                    installation::move_directory(&source, &target)?;
                } else {
                    installation::verify_marker(&target, package)?;
                }
            }
        }
    } else if manifest.revision == journal.after.revision
        && manifest.packages == journal.after.packages
    {
        if active.exists() || retained.exists() {
            return Err(
                "Unexpected files appeared after Android package removal. Files were preserved."
                    .into(),
            );
        }
    } else {
        return Err(
            "Android manifest changed outside its removal journal. Files were preserved.".into(),
        );
    }
    if stage.exists() {
        fs::remove_dir_all(stage).map_err(|e| e.to_string())?;
    }
    fs::remove_file(directory.root.join(JOURNAL)).map_err(|e| e.to_string())?;
    installation::sync_directory(&directory.root)?;
    Ok(true)
}

fn copy_package(
    source: &Path,
    target: &Path,
    total: u64,
    progress: &Operation,
) -> Result<(), String> {
    let mut pending = vec![(source.to_owned(), target.to_owned())];
    let mut directories = vec![target.to_owned()];
    let mut count = 0;
    let mut copied = 0;
    let mut last = std::time::Instant::now();
    let cancelled = progress.cancellation();
    let mut buffer = [0; 65536];
    while let Some((from, to)) = pending.pop() {
        if *cancelled.borrow() {
            return Err("Android rollback cancelled".into());
        }
        count += 1;
        if count > 100000 {
            return Err("Android rollback exceeds its file limit".into());
        }
        for entry in fs::read_dir(from).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let from = entry.path();
            let to = to.join(entry.file_name());
            let metadata = fs::symlink_metadata(&from).map_err(|e| e.to_string())?;
            count += 1;
            if count > 100000 {
                return Err("Android rollback exceeds its file limit".into());
            }
            if metadata.file_type().is_symlink() {
                return Err("Android rollback contains an unexpected symbolic link".into());
            }
            if metadata.is_dir() {
                fs::create_dir(&to).map_err(|e| e.to_string())?;
                directories.push(to.clone());
                pending.push((from, to));
            } else if metadata.is_file() {
                let mut input = fs::File::open(&from).map_err(|e| e.to_string())?;
                let mut output = fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&to)
                    .map_err(|e| e.to_string())?;
                loop {
                    if *cancelled.borrow() {
                        return Err("Android rollback cancelled".into());
                    }
                    let size = input.read(&mut buffer).map_err(|e| e.to_string())?;
                    if size == 0 {
                        break;
                    }
                    copied += size as u64;
                    if copied > total || copied > 16 * 1024 * 1024 * 1024 {
                        return Err("Android rollback grew beyond its verified size".into());
                    }
                    output
                        .write_all(&buffer[..size])
                        .map_err(|e| e.to_string())?;
                    if last.elapsed() >= std::time::Duration::from_millis(100) {
                        progress.progress("Preparing previous tool version", copied, total);
                        last = std::time::Instant::now();
                    }
                }
                output.sync_all().map_err(|e| e.to_string())?;
                fs::set_permissions(to, metadata.permissions()).map_err(|e| e.to_string())?;
            } else {
                return Err("Android rollback contains an unsupported file type".into());
            }
        }
    }
    if copied != total {
        return Err("Android rollback changed during preparation".into());
    }
    for directory in directories.into_iter().rev() {
        installation::sync_directory(&directory)?;
    }
    Ok(())
}

fn cleanup(root: &Path, progress: &Operation) -> Result<(), String> {
    let cancel = progress.cancellation();
    for relative in ["cache", "tmp", "user/cli/bundles", "staging"] {
        let parent = installation::checked_path(root, Path::new(relative))?;
        if !parent.exists() {
            continue;
        }
        for entry in fs::read_dir(&parent).map_err(|e| e.to_string())? {
            if *cancel.borrow() {
                return Err("Android cleanup cancelled".into());
            }
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if relative == "staging"
                && (!storage::valid_id(&name) || contains_avd_data(&entry.path())?)
            {
                continue;
            }
            let metadata = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                fs::remove_dir_all(entry.path()).map_err(|e| e.to_string())?;
            } else {
                fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
            }
        }
        installation::sync_directory(&parent)?;
    }
    prune_logs(root)
}

fn contains_avd_data(root: &Path) -> Result<bool, String> {
    let mut pending = vec![root.to_owned()];
    let mut entries = 0;
    while let Some(path) = pending.pop() {
        entries += 1;
        if entries > 100000 {
            return Ok(true);
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.ends_with(".avd")
            || name.ends_with(".qcow2")
            || name.starts_with("userdata")
            || name == ".lomi-avd.json"
        {
            return Ok(true);
        }
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
                if pending.len() >= 100000 {
                    return Ok(true);
                }
                pending.push(entry.map_err(|e| e.to_string())?.path());
            }
        }
    }
    Ok(false)
}

pub fn prune_logs(root: &Path) -> Result<(), String> {
    let path = installation::checked_path(root, Path::new("logs"))?;
    if !path.exists() {
        return Ok(());
    }
    let mut logs = Vec::new();
    for (index, entry) in fs::read_dir(&path).map_err(|e| e.to_string())?.enumerate() {
        if index >= 4096 {
            return Err(
                "Android logs exceed their file-count limit. Inspect the managed logs folder."
                    .into(),
            );
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.strip_suffix(".log").is_some_and(storage::valid_id) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
        if metadata.is_file() && !metadata.file_type().is_symlink() {
            logs.push((
                metadata.modified().map_err(|e| e.to_string())?,
                entry.path(),
            ));
        }
    }
    logs.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    for (_, path) in logs.into_iter().skip(32) {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn prune_cli_cache(root: &Path) -> Result<(), String> {
    let path = installation::checked_path(root, Path::new("user/cli/bundles"))?;
    if !path.exists() {
        return Ok(());
    }
    let mut bundles = Vec::new();
    for (index, entry) in fs::read_dir(&path).map_err(|e| e.to_string())?.enumerate() {
        if index >= 1025 {
            return Err(
                "CLI cache exceeds its entry limit. Use Android Maintenance cleanup.".into(),
            );
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let metadata = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
        // The pinned launcher keeps an empty advisory-lock file beside its bundles.
        // Preserve its inode so cleanup cannot split ownership across two locks.
        if name == "lock" {
            if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != 0 {
                return Err("Unexpected CLI cache lock file type or contents".into());
            }
            continue;
        }
        if name.len() != 40 || !name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Unknown CLI cache entry; inspect it before cleanup.".into());
        }
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("Unexpected CLI cache file type".into());
        }
        bundles.push((
            metadata.modified().map_err(|e| e.to_string())?,
            entry.path(),
        ));
    }
    if bundles.len() > 1024 {
        return Err("CLI cache exceeds its entry limit. Use Android Maintenance cleanup.".into());
    }
    bundles.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    let mut retained = 0;
    for (index, (_, path)) in bundles.into_iter().enumerate() {
        let size = disk::measure(&path)?.allocated_bytes;
        if index >= 2 || retained + size > 512 * 1024 * 1024 {
            fs::remove_dir_all(path).map_err(|e| e.to_string())?;
        } else {
            retained += size;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cli_cache_preserves_launcher_lock_and_bounds_retained_bundles() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("user/cli/bundles");
        fs::create_dir_all(&cache).unwrap();
        let lock_path = cache.join("lock");
        let lock = fs::File::create(&lock_path).unwrap();
        lock.try_lock().unwrap();
        for name in ["a", "b", "c"] {
            let bundle = cache.join(name.repeat(40));
            fs::create_dir(&bundle).unwrap();
            fs::write(bundle.join("main.jar"), b"fixture").unwrap();
        }
        prune_cli_cache(root.path()).unwrap();
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 3);
        assert!(lock_path.is_file());
        assert!(fs::File::open(&lock_path).unwrap().try_lock().is_err());
        lock.unlock().unwrap();
        fs::write(&lock_path, b"unexpected").unwrap();
        assert!(prune_cli_cache(root.path()).unwrap_err().contains("lock"));
        assert_eq!(fs::read(&lock_path).unwrap(), b"unexpected");
    }

    #[cfg(unix)]
    #[test]
    fn cli_cache_rejects_a_symlink_in_place_of_the_launcher_lock() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("user/cli/bundles");
        fs::create_dir_all(&cache).unwrap();
        let unrelated = root.path().join("unrelated");
        fs::write(&unrelated, []).unwrap();
        std::os::unix::fs::symlink(&unrelated, cache.join("lock")).unwrap();
        assert!(prune_cli_cache(root.path()).unwrap_err().contains("lock"));
        assert!(unrelated.is_file());
        assert!(cache.join("lock").is_symlink());
    }

    #[test]
    fn cleanup_keeps_device_data_and_limits_only_owned_temporary_files() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::manager::Android::default();
        let manager = state.get(root.path().join("android")).unwrap();
        let directory = manager.directory.lock().unwrap();
        let root = directory.root.clone();
        for relative in [
            "avd/phone.avd",
            "staging/00000000-0000-0000-0000-000000000001/old.avd",
            "staging/00000000-0000-0000-0000-000000000002/candidate",
            "cache",
            "logs",
        ] {
            fs::create_dir_all(root.join(relative)).unwrap();
            fs::write(root.join(relative).join("test"), "retained").unwrap();
        }
        for _ in 0..35 {
            fs::write(
                root.join("logs")
                    .join(format!("{}.log", super::super::auth::new_id().unwrap())),
                "diagnostic",
            )
            .unwrap();
        }
        drop(directory);
        manager
            .installer
            .maintain(&manager, Action::Cleanup)
            .unwrap();
        tauri::async_runtime::block_on(manager.installer.settle(false)).unwrap();
        assert_eq!(
            manager.installer.progress().unwrap().phase,
            super::super::installer::Phase::Succeeded
        );
        assert!(root.join("avd/phone.avd/test").exists());
        assert!(root
            .join("staging/00000000-0000-0000-0000-000000000001/old.avd/test")
            .exists());
        assert!(!root
            .join("staging/00000000-0000-0000-0000-000000000002")
            .exists());
        assert!(!root.join("cache/test").exists());
        assert_eq!(fs::read_dir(root.join("logs")).unwrap().count(), 33);
    }

    #[test]
    fn removing_an_image_referenced_by_a_stopped_device_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let mut directory = Directory::acquire(root.path().join("android")).unwrap();
        let image = "system-images;android-36;default;arm64-v8a";
        install(&mut directory, image, "2");
        let device = storage::Device {
            id: super::super::auth::new_id().unwrap(),
            name: "Retained phone".into(),
            image: image.into(),
            image_revision: 2,
            profile: "small_phone".into(),
            input_bridge: true,
            hardware: storage::Hardware {
                ram_mib: 2560,
                cpu_count: 2,
                data_gib: 2,
                gpu: storage::Gpu::Auto,
                quick_boot: false,
            },
        };
        directory
            .save_devices(
                storage::Devices {
                    devices: vec![device],
                    ..Default::default()
                },
                0,
            )
            .unwrap();
        let revision = directory.manifest().unwrap().revision;
        assert!(remove(
            &mut directory,
            &super::super::auth::new_id().unwrap(),
            image,
            revision,
            |_| Ok(())
        )
        .unwrap_err()
        .contains("used by a device"));
        assert!(directory.installed_path(image).is_ok());
        assert!(!directory.root.join(JOURNAL).exists());
    }
    fn install(directory: &mut Directory, id: &str, revision: &str) {
        let operation = super::super::auth::new_id().unwrap();
        let stage = directory
            .root
            .join("staging")
            .join(&operation)
            .join("candidate");
        fs::create_dir_all(&stage).unwrap();
        fs::write(stage.join("test-artifact"), revision).unwrap();
        let package = Installed {
            id: id.into(),
            revision: revision.into(),
            archive_sha1: "a".repeat(40),
        };
        directory.stage_verified(&operation, &package).unwrap();
        directory.publish(&operation, package, &[]).unwrap();
    }

    #[test]
    fn interrupted_removal_recovers_directories_manifest_and_previous_copy() {
        for boundary in 0..5 {
            let root = tempfile::tempdir().unwrap();
            let mut directory = Directory::acquire(root.path().join("android")).unwrap();
            install(&mut directory, "emulator", "1");
            install(&mut directory, "emulator", "2");
            install(&mut directory, "platform-tools", "1");
            fs::create_dir_all(directory.root.join("avd/untouched.avd")).unwrap();
            fs::write(directory.root.join("avd/untouched.avd/userdata"), "keep").unwrap();
            let revision = directory.manifest().unwrap().revision;
            let operation = super::super::auth::new_id().unwrap();
            let result = remove(&mut directory, &operation, "emulator", revision, |step| {
                if step as usize == boundary {
                    Err("interrupted".into())
                } else {
                    Ok(())
                }
            });
            assert!(result.is_err());
            assert!(recover(&mut directory).unwrap());
            assert!(directory.installed_path("platform-tools").is_ok());
            assert_eq!(
                fs::read(directory.root.join("avd/untouched.avd/userdata")).unwrap(),
                b"keep"
            );
            if boundary < Step::Manifest as usize {
                assert_eq!(
                    directory.manifest().unwrap().packages["emulator"].revision,
                    "2"
                );
                assert_eq!(rollbacks(&directory).unwrap()[0].revision, "1");
                assert_eq!(
                    fs::read(
                        directory
                            .installed_path("emulator")
                            .unwrap()
                            .join("test-artifact")
                    )
                    .unwrap(),
                    b"2"
                );
            } else {
                assert!(!directory
                    .manifest()
                    .unwrap()
                    .packages
                    .contains_key("emulator"));
                assert!(!directory.root.join("sdk/emulator").exists());
                assert!(!rollback_path(&directory.root, "emulator").unwrap().exists());
            }
            assert!(!directory.root.join("staging").join(operation).exists());
        }
    }

    #[test]
    fn tool_rollback_publishes_the_previous_copy_and_retains_the_replaced_version() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::manager::Android::default();
        let manager = state.get(root.path().join("android")).unwrap();
        let revision = {
            let mut directory = manager.directory.lock().unwrap();
            install(&mut directory, "emulator", "1");
            install(&mut directory, "emulator", "2");
            directory.manifest().unwrap().revision
        };
        manager
            .installer
            .maintain(
                &manager,
                Action::Rollback {
                    package_id: "emulator".into(),
                    expected_revision: revision,
                },
            )
            .unwrap();
        tauri::async_runtime::block_on(manager.installer.settle(false)).unwrap();
        let progress = manager.installer.progress().unwrap();
        assert_eq!(
            progress.phase,
            super::super::installer::Phase::Succeeded,
            "{progress:?}"
        );
        let directory = manager.directory.lock().unwrap();
        assert_eq!(
            directory.manifest().unwrap().packages["emulator"].revision,
            "1"
        );
        assert_eq!(rollbacks(&directory).unwrap()[0].revision, "2");
        assert_eq!(
            fs::read(
                directory
                    .installed_path("emulator")
                    .unwrap()
                    .join("test-artifact")
            )
            .unwrap(),
            b"1"
        );
    }
}
