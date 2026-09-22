use super::{
    catalog::Host,
    download, environment,
    installation::{checked_path, create_directories, move_directory, sync_directory},
    installer_process::InstallerChild,
    jre_archive,
    storage::{self, Directory},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::sync::watch;

const MARKER: &str = ".lomi-toolchain.json";
const JOURNAL: &str = "toolchain-installation.json";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Blob {
    pub version: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Distribution {
    pub cli: Blob,
    pub java: Blob,
    pub qualified: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Catalog {
    version: u32,
    checked_at: String,
    hosts: BTreeMap<String, Distribution>,
}

pub fn host_key(host: Host) -> &'static str {
    match host {
        Host::MacArm64 => "darwin_arm64",
        Host::MacX64 => "darwin_x86_64",
        Host::LinuxX64 => "linux_x86_64",
        Host::WindowsX64 => "windows_x86_64",
    }
}

pub fn distribution(host: Host) -> Result<Distribution, String> {
    let mut catalog: Catalog =
        serde_json::from_str(include_str!("../../android-toolchain.json")).map_err(error)?;
    if catalog.version != 1 || catalog.checked_at != "2026-09-18" {
        return Err("Unsupported bundled Android tool catalog".into());
    }
    let distribution = catalog
        .hosts
        .remove(host_key(host))
        .ok_or("No Android tools for this host")?;
    for blob in [&distribution.cli, &distribution.java] {
        if blob.size == 0
            || blob.size > 256 * 1024 * 1024
            || blob.sha256.len() != 64
            || !blob.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("Invalid bundled Android tool identity".into());
        }
    }
    Ok(distribution)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Installed {
    id: String,
    host: String,
    distribution: Distribution,
}
impl Installed {
    fn validate(&self) -> Result<(), String> {
        if !storage::valid_id(&self.id)
            || !matches!(
                self.host.as_str(),
                "darwin_arm64" | "darwin_x86_64" | "linux_x86_64" | "windows_x86_64"
            )
        {
            return Err("Invalid Android toolchain identity; preserve it for recovery".into());
        }
        for blob in [&self.distribution.cli, &self.distribution.java] {
            if blob.version.is_empty()
                || blob.version.len() > 80
                || !blob
                    .version
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".+_-".contains(&b))
                || blob.version.contains("..")
                || blob.sha256.len() != 64
                || !blob.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                || blob.size == 0
                || blob.size > 256 * 1024 * 1024
            {
                return Err("Invalid installed Android toolchain descriptor".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    version: u32,
    revision: u64,
    current: Option<Installed>,
    previous: Option<Installed>,
}
impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: 1,
            revision: 0,
            current: None,
            previous: None,
        }
    }
}
impl Manifest {
    fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || self.revision > (1 << 53) - 1
            || (self.current.is_none() && self.previous.is_some())
        {
            return Err("Unsupported Android toolchain manifest; recover it explicitly".into());
        }
        for installed in self.current.iter().chain(self.previous.iter()) {
            installed.validate()?;
        }
        if self
            .current
            .as_ref()
            .zip(self.previous.as_ref())
            .is_some_and(|(a, b)| a.id == b.id)
        {
            return Err("Duplicate Android toolchain identity".into());
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Journal {
    version: u32,
    before: Manifest,
    after: Manifest,
}
impl Journal {
    fn validate(&self) -> Result<(), String> {
        self.before.validate()?;
        self.after.validate()?;
        if self.version != 1
            || self.after.revision != self.before.revision + 1
            || self.after.current.is_none()
            || self.after.previous != self.before.current
        {
            return Err("Invalid Android toolchain publication journal".into());
        }
        let next = &self.after.current.as_ref().unwrap().id;
        if self
            .before
            .current
            .iter()
            .chain(self.before.previous.iter())
            .any(|old| &old.id == next)
        {
            return Err("Reused Android toolchain publication ID".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Tools {
    pub cli: PathBuf,
    pub java_home: PathBuf,
}

fn paths(root: &Path, installed: &Installed, staged: bool) -> Result<(PathBuf, Tools), String> {
    installed.validate()?;
    let relative = if staged {
        PathBuf::from("staging")
            .join(&installed.id)
            .join("toolchain")
    } else {
        PathBuf::from("toolchains").join(&installed.id)
    };
    let directory = checked_path(root, &relative)?;
    let java_home = directory
        .join("java")
        .join(format!("jdk-{}-jre", installed.distribution.java.version));
    let java_home = if installed.host.starts_with("darwin_") {
        java_home.join("Contents/Home")
    } else {
        java_home
    };
    let cli = directory.join(if installed.host.starts_with("windows_") {
        "android-cli.exe"
    } else {
        "android-cli"
    });
    Ok((directory, Tools { cli, java_home }))
}

fn manifest(root: &Path) -> Result<Manifest, String> {
    let value: Manifest = storage::read(&root.join("toolchain.json"))?.unwrap_or_default();
    value.validate()?;
    Ok(value)
}

pub fn installed(directory: &Directory) -> Result<Tools, String> {
    if directory.root.join(JOURNAL).try_exists().map_err(error)? {
        return Err(
            "Android tool preparation was interrupted. Use Repair in Android settings.".into(),
        );
    }
    let manifest = manifest(&directory.root)?;
    let current = manifest
        .current
        .ok_or("Android tools are not prepared. Open Android settings.")?;
    if current.host != host_key(Host::native()?) {
        return Err(
            "Android tools belong to another host. Prepare compatible tools in Settings.".into(),
        );
    }
    verified_marker(&directory.root, &current, false)
}

pub fn installed_distribution(directory: &Directory) -> Result<Option<Distribution>, String> {
    Ok(manifest(&directory.root)?
        .current
        .map(|current| current.distribution))
}

fn verified_marker(root: &Path, installed: &Installed, staged: bool) -> Result<Tools, String> {
    let (directory, tools) = paths(root, installed, staged)?;
    checked_path(
        root,
        tools
            .java_home
            .join("bin")
            .strip_prefix(root)
            .map_err(error)?,
    )?;
    let actual: Installed = storage::read(&directory.join(MARKER))?
        .ok_or("Android toolchain is incomplete; repair it in Settings")?;
    if actual != *installed {
        return Err("Android toolchain identity differs from its manifest".into());
    }
    for path in [
        &tools.cli,
        &tools
            .java_home
            .join("bin")
            .join(if installed.host.starts_with("windows_") {
                "java.exe"
            } else {
                "java"
            }),
    ] {
        let metadata = fs::symlink_metadata(path).map_err(error)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("Android tool executable must be a regular file".into());
        }
    }
    Ok(tools)
}

/// The operation owner holds the directory lock and mutation lease across this
/// future, including cancellation. Consent is checked by the installer plan.
pub async fn prepare(
    root: &Path,
    operation: &str,
    cancel: watch::Receiver<bool>,
    progress: impl Fn(&str, u64, u64),
) -> Result<Tools, String> {
    if !storage::valid_id(operation) {
        return Err("Invalid Android preparation ID".into());
    }
    if root.join(JOURNAL).try_exists().map_err(error)? {
        return Err("Repair the interrupted Android tool preparation first".into());
    }
    let before = manifest(root)?;
    let host = Host::native()?;
    let selected = Installed {
        id: operation.into(),
        host: host_key(host).into(),
        distribution: distribution(host)?,
    };
    super::disk::require(
        root,
        selected.distribution.cli.size
            + selected.distribution.java.size
            + jre_archive::EXPANDED_LIMIT,
    )?;
    let stage = checked_path(root, &PathBuf::from("staging").join(operation))?;
    create_directories(root, &stage)?;
    let (candidate, tools) = paths(root, &selected, true)?;
    fs::create_dir(&candidate).map_err(error)?;
    for relative in [
        "user",
        "user/cli-home",
        "tmp",
        "emulator-home",
        "avd",
        "toolchains",
    ] {
        create_directories(root, &root.join(relative))?;
    }
    progress("Downloading Android CLI", 0, selected.distribution.cli.size);
    download::verified(
        &selected.distribution.cli.url,
        selected.distribution.cli.size,
        &selected.distribution.cli.sha256,
        &ring::digest::SHA256,
        &tools.cli,
        cancel.clone(),
        |received, total| progress("Downloading Android CLI", received, total),
    )
    .await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tools.cli, fs::Permissions::from_mode(0o700)).map_err(error)?;
    }
    let archive = stage.join("java.archive");
    download::verified(
        &selected.distribution.java.url,
        selected.distribution.java.size,
        &selected.distribution.java.sha256,
        &ring::digest::SHA256,
        &archive,
        cancel.clone(),
        |received, total| progress("Downloading private Java runtime", received, total),
    )
    .await?;
    progress("Extracting private Java runtime", 0, 0);
    let extraction_cancel = cancel.clone();
    let target = candidate.join("java");
    let windows_zip = matches!(host, Host::WindowsX64);
    tokio::task::spawn_blocking(move || {
        jre_archive::extract(&archive, &target, windows_zip, || {
            *extraction_cancel.borrow()
        })
    })
    .await
    .map_err(error)??;
    progress("Checking Android CLI and Java", 0, 0);
    validate_programs(root, &tools, &selected.distribution, cancel.clone()).await?;
    if *cancel.borrow() {
        return Err("Android tool preparation cancelled".into());
    }
    storage::write(&candidate.join(MARKER), &selected)?;
    sync_directory(&candidate)?;
    let after = Manifest {
        version: 1,
        revision: before.revision + 1,
        current: Some(selected),
        previous: before.current.clone(),
    };
    after.validate()?;
    let journal = Journal {
        version: 1,
        before,
        after,
    };
    progress("Publishing verified Android tools", 0, 0);
    publish(root, &journal, |_| Ok(()))?;
    let current = journal.after.current.as_ref().unwrap();
    verified_marker(root, current, false)
}

async fn validate_programs(
    root: &Path,
    tools: &Tools,
    selected: &Distribution,
    cancel: watch::Receiver<bool>,
) -> Result<(), String> {
    for (program, args, expected) in [
        (
            tools.cli.clone(),
            vec!["--no-metrics", "--version"],
            selected.cli.version.as_str(),
        ),
        (
            tools
                .java_home
                .join("bin")
                .join(if cfg!(windows) { "java.exe" } else { "java" }),
            vec!["-version"],
            selected.java.version.as_str(),
        ),
    ] {
        if *cancel.borrow() {
            return Err("Android tool validation cancelled".into());
        }
        let mut command = environment::command(
            &program,
            root,
            &root.join("sdk"),
            Some(&tools.java_home),
            5037,
        )?;
        command.args(args);
        let output = run(command, Duration::from_secs(30), cancel.clone(), |_| {}).await?;
        if !output.contains(expected) {
            return Err(format!("Android tool reported an unexpected version; expected {expected}. Repair the tools."));
        }
    }
    Ok(())
}

/// Poll errors retain the process owner and lock. A future timeout is not evidence
/// of process exit; callers cancel this owned task through its watch channel.
pub async fn run(
    command: std::process::Command,
    timeout: Duration,
    cancel: watch::Receiver<bool>,
    progress: impl Fn(&str),
) -> Result<String, String> {
    let mut child = InstallerChild::spawn(command, timeout)?;
    loop {
        if *cancel.borrow() {
            child.cancel();
        }
        match child.poll() {
            Ok(Some(result)) if result.success => return Ok(result.output),
            Ok(Some(result)) => {
                return Err(format!(
                    "{}\n{}",
                    result.stop_reason.unwrap_or("Android tool failed"),
                    result.output
                ))
            }
            Ok(None) => {}
            Err(error) => {
                child.cancel();
                progress(&error);
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Step {
    Journal,
    Directory,
    Manifest,
    Cleanup,
}

fn publish(
    root: &Path,
    journal: &Journal,
    checkpoint: impl Fn(Step) -> Result<(), String>,
) -> Result<(), String> {
    journal.validate()?;
    if manifest(root)? != journal.before {
        return Err("Android toolchain changed before publication".into());
    }
    storage::write(&root.join(JOURNAL), journal)?;
    checkpoint(Step::Journal)?;
    finish(root, journal, checkpoint)
}

fn finish(
    root: &Path,
    journal: &Journal,
    checkpoint: impl Fn(Step) -> Result<(), String>,
) -> Result<(), String> {
    journal.validate()?;
    let installed = journal
        .after
        .current
        .as_ref()
        .ok_or("Missing toolchain candidate")?;
    let (target, _) = paths(root, installed, false)?;
    if !target.try_exists().map_err(error)? {
        verified_marker(root, installed, true)?;
        let (candidate, _) = paths(root, installed, true)?;
        move_directory(&candidate, &target)?;
    }
    verified_marker(root, installed, false)?;
    checkpoint(Step::Directory)?;
    storage::write(&root.join("toolchain.json"), &journal.after)?;
    checkpoint(Step::Manifest)?;
    if let Some(previous) = &journal.before.previous {
        let (directory, _) = paths(root, previous, false)?;
        if directory.try_exists().map_err(error)? {
            fs::remove_dir_all(directory).map_err(error)?;
        }
    }
    let stage = checked_path(root, &PathBuf::from("staging").join(&installed.id))?;
    if stage.try_exists().map_err(error)? {
        fs::remove_dir_all(stage).map_err(error)?;
    }
    sync_directory(&root.join("toolchains"))?;
    checkpoint(Step::Cleanup)?;
    fs::remove_file(root.join(JOURNAL)).map_err(error)?;
    sync_directory(root)
}

pub fn recover(directory: &mut Directory) -> Result<bool, String> {
    let Some(journal): Option<Journal> = storage::read(&directory.root.join(JOURNAL))? else {
        return Ok(false);
    };
    journal.validate()?;
    let current = manifest(&directory.root)?;
    if current != journal.before && current != journal.after {
        return Err(
            "Android toolchain recovery conflicts with the current manifest; files were preserved"
                .into(),
        );
    }
    finish(&directory.root, &journal, |_| Ok(()))?;
    Ok(true)
}

fn error(error: impl std::fmt::Display) -> String {
    format!("Android tool preparation: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(root: &Path, before: Manifest) -> Journal {
        let host = Host::native().unwrap();
        let selected = Installed {
            id: super::super::auth::new_id().unwrap(),
            host: host_key(host).into(),
            distribution: distribution(host).unwrap(),
        };
        let (directory, tools) = paths(root, &selected, true).unwrap();
        fs::create_dir_all(tools.java_home.join("bin")).unwrap();
        fs::write(&tools.cli, b"fixture").unwrap();
        fs::write(
            tools
                .java_home
                .join("bin")
                .join(if cfg!(windows) { "java.exe" } else { "java" }),
            b"fixture",
        )
        .unwrap();
        fs::create_dir_all(root.join("toolchains")).unwrap();
        storage::write(&directory.join(MARKER), &selected).unwrap();
        let after = Manifest {
            version: 1,
            revision: before.revision + 1,
            current: Some(selected),
            previous: before.current.clone(),
        };
        Journal {
            version: 1,
            before,
            after,
        }
    }

    #[test]
    fn interrupted_publication_recovers_and_keeps_one_previous_toolchain() {
        for interrupted in [
            Step::Journal,
            Step::Directory,
            Step::Manifest,
            Step::Cleanup,
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut directory = Directory::acquire(root.path().to_path_buf()).unwrap();
            let first = candidate(root.path(), Manifest::default());
            publish(root.path(), &first, |_| Ok(())).unwrap();
            let old = installed(&directory).unwrap();
            let second = candidate(root.path(), manifest(root.path()).unwrap());
            assert!(
                publish(root.path(), &second, |step| if step == interrupted {
                    Err("interrupted".into())
                } else {
                    Ok(())
                })
                .is_err()
            );
            assert!(old.cli.exists());
            assert!(installed(&directory).is_err());
            assert!(recover(&mut directory).unwrap());
            assert!(!recover(&mut directory).unwrap());
            assert_ne!(installed(&directory).unwrap().cli, old.cli);
            assert!(old.cli.exists());
            let third = candidate(root.path(), manifest(root.path()).unwrap());
            publish(root.path(), &third, |_| Ok(())).unwrap();
            assert!(!old.cli.exists());
            assert_eq!(
                fs::read_dir(root.path().join("toolchains"))
                    .unwrap()
                    .count(),
                2
            );
        }
    }

    #[tokio::test]
    #[ignore = "Downloads pinned official tools into a clean subdirectory of the accepted native trial"]
    async fn actual_bootstrap_downloads_and_runs_private_cli_and_java() {
        let trial = PathBuf::from(std::env::var_os("LOMI_ANDROID_PROBE_DIRECTORY").unwrap());
        let consent: serde_json::Value =
            serde_json::from_slice(&fs::read(trial.join("evidence/consent.json")).unwrap())
                .unwrap();
        assert_eq!(consent["accepted"], true);
        let temporary = tempfile::tempdir_in(trial.join("staging")).unwrap();
        let directory = Directory::acquire(temporary.path().join("managed")).unwrap();
        let operation = super::super::auth::new_id().unwrap();
        let (sender, cancel) = watch::channel(false);
        let progress = std::sync::Mutex::new(Vec::new());
        let started = std::time::Instant::now();
        let tools = prepare(
            &directory.root,
            &operation,
            cancel,
            |stage, received, total| {
                progress
                    .lock()
                    .unwrap()
                    .push(serde_json::json!({"stage":stage,"received":received,"total":total}));
            },
        )
        .await
        .unwrap();
        assert_eq!(installed(&directory).unwrap().cli, tools.cli);
        assert!(!directory.root.join("user/bin/android-cli").exists());
        assert!(!directory.root.join("user/cli-home/.androidrc").exists());
        assert!(!directory.root.join("staging").join(operation).exists());
        let mut command = environment::command(
            &tools.cli,
            &directory.root,
            &directory.root.join("sdk"),
            Some(&tools.java_home),
            15037,
        )
        .unwrap();
        command.args(["--no-metrics", "sdk", "install", "--help"]);
        let help = run(command, Duration::from_secs(30), sender.subscribe(), |_| {})
            .await
            .unwrap();
        assert!(help.contains("install"));
        let report = serde_json::json!({"version":1,"host":host_key(Host::native().unwrap()),"seconds":started.elapsed().as_secs_f64(),"tools":distribution(Host::native().unwrap()).unwrap(),"cleanManagedDirectory":true,"privateJava":true,"directCli":true,"launcherDownloadedAnotherVersion":false,"progress":progress.into_inner().unwrap(),"cliInstallHelp":help});
        fs::write(
            trial.join("evidence/native-rust-bootstrap.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        println!(
            "Native Rust bootstrap passed in {:.2}s",
            started.elapsed().as_secs_f64()
        );
    }
}
