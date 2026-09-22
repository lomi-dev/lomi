use super::{catalog, download};
use ring::digest::{Context, SHA1_FOR_LEGACY_USE_ONLY};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

/// The manager may refresh its own catalog. Verify the actual extracted bytes against
/// our approved archive before publication, independently of its exit code or metadata.
#[cfg(test)]
pub fn verify(
    package: &catalog::Package,
    license: &catalog::License,
    archive: &Path,
    installed: &Path,
) -> Result<(), String> {
    verify_with_cancel(package, license, archive, installed, || false)
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{
        io::Write,
        time::{Duration, Instant},
    };
    use zip::{write::SimpleFileOptions, ZipWriter};

    fn fixture(root: &Path) -> (catalog::Package, catalog::License) {
        let archive = root.join("archive.zip");
        let mut zip = ZipWriter::new(File::create(&archive).unwrap());
        zip.start_file("platform-tools/tool", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"verified tool").unwrap();
        zip.finish().unwrap();
        let license = catalog::License {
            id: "test-license".into(),
            text: "Accepted terms".into(),
            digest: format!("{:x}", Sha256::digest(b"Accepted terms")),
        };
        let bytes = fs::read(&archive).unwrap();
        let hash = ring::digest::digest(&SHA1_FOR_LEGACY_USE_ONLY, &bytes);
        let package = catalog::Package {
            id: "platform-tools".into(),
            revision: "37.0.1".into(),
            name: "Tools".into(),
            license: license.id.clone(),
            url: String::new(),
            size: bytes.len() as u64,
            sha1: hash
                .as_ref()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            image: None,
            dependencies: vec![],
        };
        let candidate = root.join("candidate");
        fs::create_dir(&candidate).unwrap();
        fs::write(candidate.join("tool"), "verified tool").unwrap();
        fs::write(candidate.join("package.xml"), "<repository><license id='test-license'>Accepted terms</license><localPackage path='platform-tools'><revision><major>37</major><minor>0</minor><micro>1</micro></revision><uses-license ref='test-license'/></localPackage></repository>").unwrap();
        (package, license)
    }

    #[test]
    fn matching_metadata_cannot_hide_modified_or_extra_artifacts() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let (package, license) = fixture(root);
        let archive = root.join("archive.zip");
        let candidate = root.join("candidate");
        verify(&package, &license, &archive, &candidate).unwrap();
        fs::write(candidate.join("tool"), "modified tool").unwrap();
        assert!(verify(&package, &license, &archive, &candidate)
            .unwrap_err()
            .contains("differs"));
        assert_eq!(fs::read(candidate.join("tool")).unwrap(), b"modified tool");
        fs::write(candidate.join("tool"), "verified tool").unwrap();
        fs::write(candidate.join("unexpected"), "not approved").unwrap();
        assert!(verify(&package, &license, &archive, &candidate)
            .unwrap_err()
            .contains("absent"));
        fs::remove_file(candidate.join("unexpected")).unwrap();
        fs::write(&archive, b"truncated").unwrap();
        assert!(verify(&package, &license, &archive, &candidate)
            .unwrap_err()
            .contains("size changed"));
    }

    #[cfg(unix)]
    #[test]
    fn artifact_verification_does_not_follow_replaced_files() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let (package, license) = fixture(root);
        let candidate = root.join("candidate");
        fs::write(root.join("outside"), "verified tool").unwrap();
        fs::remove_file(candidate.join("tool")).unwrap();
        std::os::unix::fs::symlink(root.join("outside"), candidate.join("tool")).unwrap();
        assert!(verify(&package, &license, &root.join("archive.zip"), &candidate).is_err());
        assert_eq!(fs::read(root.join("outside")).unwrap(), b"verified tool");
    }

    #[tokio::test]
    #[ignore = "Real CLI download/install/verification/publication in the accepted native fixture"]
    async fn actual_cli_installation_is_verified_before_atomic_publication() {
        use super::super::{
            environment, installation::Installed, installer_process::InstallerChild,
            storage::Directory,
        };
        let root = PathBuf::from(std::env::var_os("LOMI_ANDROID_PROBE_DIRECTORY").unwrap());
        let consent: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("evidence/consent.json")).unwrap()).unwrap();
        assert_eq!(consent["accepted"], true);
        assert_eq!(consent["scope"], "Isolated native stage 0 only");
        assert!(matches!(
            catalog::Host::native().unwrap(),
            catalog::Host::MacArm64
        ));
        let cli = root.join("user/bin/android-cli");
        assert_eq!(
            format!("{:x}", Sha256::digest(fs::read(&cli).unwrap())),
            "d1ebf7fc1517aba98d6a34961ea89f843ea3acc7c1ced693fa103a0e279052fd"
        );
        let catalog = catalog::packages(
            &fs::read_to_string(root.join("evidence/repository2-3.xml")).unwrap(),
            "https://dl.google.com/android/repository/",
            catalog::Host::native().unwrap(),
        )
        .unwrap();
        let package = catalog
            .packages
            .iter()
            .find(|package| package.id == "platform-tools" && package.revision == "37.0.1")
            .unwrap();
        let license = catalog
            .licenses
            .iter()
            .find(|license| license.id == package.license)
            .unwrap();
        assert_eq!(consent["sha256"], license.digest);
        let temporary = tempfile::tempdir_in(root.join("staging")).unwrap();
        let mut owner = Directory::acquire(temporary.path().join("managed")).unwrap();
        let operation = "00000000-0000-0000-0000-000000000001";
        let stage = owner.root.join("staging").join(operation);
        let sdk = stage.join("sdk");
        for path in [
            sdk.join(".sdk/arch"),
            sdk.join("licenses"),
            owner.root.join("user/cli-home"),
            owner.root.join("tmp"),
        ] {
            fs::create_dir_all(path).unwrap();
        }
        // Consent applies to this isolated trial only; no interactive tool receives a yes answer.
        let license_hash: String =
            ring::digest::digest(&SHA1_FOR_LEGACY_USE_ONLY, license.text.as_bytes())
                .as_ref()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
        fs::write(
            sdk.join("licenses").join(&license.id),
            format!("\n{license_hash}\n"),
        )
        .unwrap();
        let archive = stage.join("archive.zip");
        let (_sender, cancel) = tokio::sync::watch::channel(false);
        let progress = std::sync::Mutex::new(Vec::new());
        let started = Instant::now();
        download::package(package, &archive, cancel, |received, total| {
            progress.lock().unwrap().push((received, total))
        })
        .await
        .unwrap();
        fs::copy(&archive, sdk.join(".sdk/arch").join(&package.sha1)).unwrap();
        let mut command = environment::command(&cli, &owner.root, &sdk, None, 15037).unwrap();
        command
            .arg("--no-metrics")
            .arg(format!("--sdk={}", sdk.display()))
            .args(["sdk", "install", "platform-tools@37.0.1"]);
        let mut child = InstallerChild::spawn(command, Duration::from_secs(120)).unwrap();
        let outcome = loop {
            if let Some(outcome) = child.poll().unwrap() {
                break outcome;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        assert!(outcome.success, "{}", outcome.output);
        assert!(owner.manifest().unwrap().packages.is_empty());
        let extracted = sdk.join("platform-tools");
        verify(package, license, &archive, &extracted).unwrap();
        fs::rename(extracted, stage.join("candidate")).unwrap();
        let installed = Installed {
            id: package.id.clone(),
            revision: package.revision.clone(),
            archive_sha1: package.sha1.clone(),
        };
        owner.stage_verified(operation, &installed).unwrap();
        owner.publish(operation, installed.clone(), &[]).unwrap();
        assert_eq!(
            owner.manifest().unwrap().packages["platform-tools"],
            installed
        );
        assert!(!stage.exists());
        let mut command = environment::command(
            &owner.root.join("sdk/platform-tools/adb"),
            &owner.root,
            &owner.root.join("sdk"),
            None,
            15037,
        )
        .unwrap();
        command.arg("version");
        let version = command.output().unwrap();
        assert!(version.status.success());
        let version = String::from_utf8(version.stdout).unwrap();
        assert!(version.contains("Version 37.0.1-15733141"));
        let progress = progress.into_inner().unwrap();
        assert_eq!(progress.first(), Some(&(0, package.size)));
        assert_eq!(progress.last(), Some(&(package.size, package.size)));
        let evidence = serde_json::json!({
            "host": "macOS ARM64",
            "cli": "1.0.16261425",
            "package": package.id,
            "revision": package.revision,
            "archiveSha1": package.sha1,
            "archiveBytes": package.size,
            "seconds": started.elapsed().as_secs_f64(),
            "downloadProgressEvents": progress.len(),
            "licenseDigestMatchedAcceptedText": true,
            "installedBytesMatchedApprovedArchive": true,
            "publishedAtomically": true,
            "stagingRemoved": true,
            "executableVersion": version.trim(),
        });
        fs::write(
            root.join("evidence/rust-installer-trial.json"),
            serde_json::to_vec_pretty(&evidence).unwrap(),
        )
        .unwrap();
        println!("{evidence}");
    }
}
