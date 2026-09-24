use super::{
    artifact, auth, bootstrap,
    catalog::{self, License, Package},
    disk, download, environment,
    installation::{self, Installed},
    manager::Manager,
    repository, storage,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::watch;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    pub id: String,
    pub revision: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub id: String,
    pub catalog_revision: String,
    pub packages: Vec<Package>,
    pub licenses: Vec<License>,
    pub bootstrap: Option<bootstrap::Distribution>,
    pub download_bytes: u64,
}
struct PendingPlan {
    plan: Plan,
    created: Instant,
    manifest_revision: u64,
    devices_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Running,
    Cancelling,
    Succeeded,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub operation_id: String,
    pub package_ids: Vec<String>,
    pub phase: Phase,
    pub stage: String,
    pub received: u64,
    pub total: u64,
    pub error: Option<String>,
    pub device_id: Option<String>,
}

pub(super) struct Operation {
    progress: watch::Sender<Progress>,
    cancel: watch::Sender<bool>,
    manager: std::sync::Weak<Manager>,
}
impl Operation {
    pub(super) fn progress(&self, stage: &str, received: u64, total: u64) {
        self.progress.send_modify(|progress| {
            progress.stage = stage.into();
            progress.received = received;
            progress.total = total;
        });
        self.notify();
    }
    fn notify(&self) {
        if let Some(manager) = self.manager.upgrade() {
            manager.emit(super::events::Event::Operation(
                self.progress.borrow().clone(),
            ));
        }
    }
    pub(super) fn cancellation(&self) -> watch::Receiver<bool> {
        self.cancel.subscribe()
    }
    async fn wait(&self) -> Progress {
        let mut progress = self.progress.subscribe();
        loop {
            let value = progress.borrow_and_update().clone();
            if !matches!(value.phase, Phase::Running | Phase::Cancelling) {
                return value;
            }
            if progress.changed().await.is_err() {
                return progress.borrow().clone();
            }
        }
    }
}

#[derive(Default)]
pub struct Installer {
    state: Mutex<State>,
}
#[derive(Default)]
struct State {
    catalog: Option<repository::Snapshot>,
    plan: Option<PendingPlan>,
    operation: Option<Arc<Operation>>,
}

enum Work {
    Device(super::devices::Action),
    Maintenance(super::maintenance::Action),
}

impl Installer {
    pub async fn catalog(&self, refresh: bool) -> Result<repository::Snapshot, String> {
        if !refresh {
            if let Some(catalog) = self
                .state
                .lock()
                .map_err(|_| "Android catalog failed")?
                .catalog
                .clone()
            {
                return Ok(catalog);
            }
        }
        let snapshot = repository::fetch(catalog::Host::native()?).await?;
        self.state
            .lock()
            .map_err(|_| "Android catalog failed")?
            .catalog = Some(snapshot.clone());
        Ok(snapshot)
    }

    pub fn plan(
        &self,
        manager: &Arc<Manager>,
        revision: &str,
        selection: Vec<Selection>,
        prepare_tools: bool,
    ) -> Result<Plan, String> {
        if selection.len() > 16 || (selection.is_empty() && !prepare_tools) {
            return Err("Select Android packages to install".into());
        }
        super::host::require_acceleration()?;
        let mut state = self.state.lock().map_err(|_| "Android installer failed")?;
        if state.operation.as_ref().is_some_and(|operation| {
            matches!(
                operation.progress.borrow().phase,
                Phase::Running | Phase::Cancelling
            )
        }) {
            return Err("An Android installation is already in progress".into());
        }
        let catalog = state
            .catalog
            .as_ref()
            .ok_or("Load the Android catalog first")?;
        if catalog.revision != revision {
            return Err("Android catalog changed. Review the installation again.".into());
        }
        let directory = manager
            .directory
            .lock()
            .map_err(|_| "Android directory failed")?;
        let manifest = directory.manifest()?;
        let devices = directory.devices()?;
        if directory.root.join("device-operation.json").exists()
            || directory.root.join("package-removal.json").exists()
            || directory.root.join("installation.json").exists()
            || directory.root.join("toolchain-installation.json").exists()
        {
            return Err(
                "Repair the interrupted Android installation before installing more packages"
                    .into(),
            );
        }
        let mut packages = Vec::new();
        let mut identifiers = BTreeSet::new();
        for selected in selection {
            if !identifiers.insert(selected.id.clone()) {
                return Err("Select only one revision of each Android package".into());
            }
            let package = catalog
                .packages
                .iter()
                .find(|package| package.id == selected.id && package.revision == selected.revision)
                .ok_or("The selected Android package is no longer available")?;
            check_image_revision(package, &manifest, &devices)?;
            packages.push(package.clone());
        }
        for package in &packages {
            for dependency in &package.dependencies {
                let revision = packages
                    .iter()
                    .find(|p| p.id == dependency.id)
                    .map(|p| p.revision.as_str())
                    .or_else(|| {
                        manifest
                            .packages
                            .get(&dependency.id)
                            .map(|p| p.revision.as_str())
                    })
                    .ok_or_else(|| {
                        format!(
                            "{} requires {}. Include that tool in the installation.",
                            package.name, dependency.id
                        )
                    })?;
                if dependency
                    .minimum_revision
                    .as_ref()
                    .is_some_and(|minimum| version(revision) < version(minimum))
                {
                    return Err(format!(
                        "{} requires {} {} or newer",
                        package.name,
                        dependency.id,
                        dependency.minimum_revision.as_deref().unwrap()
                    ));
                }
            }
        }
        // Stable ordering publishes tools before images; no global SDK update is invoked.
        packages.sort_by_key(|package| (package.image.is_some(), package.id.clone()));
        let bootstrap = if prepare_tools {
            Some(bootstrap::distribution(catalog::Host::native()?)?)
        } else {
            bootstrap::installed(&directory)?;
            None
        };
        let mut licenses: Vec<_> = catalog
            .licenses
            .iter()
            .filter(|license| packages.iter().any(|p| p.license == license.id))
            .cloned()
            .collect();
        if bootstrap.is_some() {
            let sdk = catalog
                .licenses
                .iter()
                .find(|license| license.id == "android-sdk-license")
                .ok_or("Android SDK terms are missing from the catalog")?;
            if !licenses.iter().any(|license| license.id == sdk.id) {
                licenses.push(sdk.clone());
            }
            let text = include_str!("../../licenses/android-java.txt").to_string();
            licenses.push(License {
                id: "lomi-temurin-21".into(),
                digest: format!("{:x}", Sha256::digest(text.as_bytes())),
                text,
            });
        }
        let download_bytes = packages.iter().map(|p| p.size).sum::<u64>()
            + bootstrap
                .as_ref()
                .map_or(0, |tools| tools.cli.size + tools.java.size);
        // The expanded package estimate is checked again against actual ZIP entries.
        disk::require(&directory.root, download_bytes + 1024 * 1024 * 1024)?;
        let plan = Plan {
            id: auth::new_id()?,
            catalog_revision: revision.into(),
            packages,
            licenses,
            bootstrap,
            download_bytes,
        };
        state.plan = Some(PendingPlan {
            plan: plan.clone(),
            created: Instant::now(),
            manifest_revision: manifest.revision,
            devices_revision: devices.revision,
        });
        Ok(plan)
    }

    pub fn start(
        &self,
        manager: &Arc<Manager>,
        id: &str,
        accepted: Vec<String>,
    ) -> Result<Progress, String> {
        self.start_guarded(manager, id, accepted, &|| Ok(()))
    }

    pub(super) fn start_guarded(
        &self,
        manager: &Arc<Manager>,
        id: &str,
        accepted: Vec<String>,
        guard: &dyn Fn() -> Result<(), String>,
    ) -> Result<Progress, String> {
        let mut state = self.state.lock().map_err(|_| "Android installer failed")?;
        let pending = state
            .plan
            .as_ref()
            .ok_or("Review an Android installation plan first")?;
        if pending.plan.id != id || pending.created.elapsed() > Duration::from_secs(30 * 60) {
            return Err("This Android installation plan expired. Review the current downloads and terms again.".into());
        }
        let expected: BTreeSet<_> = pending
            .plan
            .licenses
            .iter()
            .map(|license| license.digest.clone())
            .collect();
        if accepted.iter().cloned().collect::<BTreeSet<_>>() != expected
            || accepted.len() != expected.len()
        {
            return Err(
                "Accept each displayed provider's terms before downloading Android components"
                    .into(),
            );
        }
        let lease = manager.begin_mutation(true)?;
        let root = {
            let directory = manager
                .directory
                .lock()
                .map_err(|_| "Android directory failed")?;
            if directory.manifest()?.revision != pending.manifest_revision
                || directory.devices()?.revision != pending.devices_revision
            {
                return Err(
                    "Android devices or tools changed. Review the installation again.".into(),
                );
            }
            directory.root.clone()
        };
        guard()?;
        let pending = state.plan.take().unwrap();
        let progress = Progress {
            operation_id: lease.id.clone(),
            package_ids: pending
                .plan
                .packages
                .iter()
                .map(|package| package.id.clone())
                .collect(),
            phase: Phase::Running,
            stage: "Preparing installation".into(),
            received: 0,
            total: 0,
            error: None,
            device_id: None,
        };
        let (sender, _) = watch::channel(progress.clone());
        let (cancel, _) = watch::channel(false);
        let operation = Arc::new(Operation {
            progress: sender,
            cancel,
            manager: Arc::downgrade(manager),
        });
        state.operation = Some(operation.clone());
        let manager = manager.clone();
        tauri::async_runtime::spawn(async move {
            let mut result = install(&manager, &root, &pending.plan, &operation).await;
            if result.is_ok() {
                let cleanup_root = root.clone();
                result = tokio::task::spawn_blocking(move || super::maintenance::prune_cli_cache(&cleanup_root))
                    .await.map_err(|e| e.to_string()).and_then(|result| result)
                    .map_err(|error| format!("Android packages are installed, but cache cleanup needs attention: {error}. Use Maintenance cleanup."));
            }
            // The mutation gate opens only after all native children and publication
            // steps have settled, even if Settings closed or its invoke was dropped.
            drop(lease);
            operation.progress.send_modify(|progress| {
                progress.phase = if result.is_ok() {
                    Phase::Succeeded
                } else if *operation.cancel.borrow() {
                    Phase::Cancelled
                } else {
                    Phase::Failed
                };
                progress.stage = if result.is_ok() {
                    "Installation complete".into()
                } else {
                    "Installation stopped".into()
                };
                progress.error = result.err();
            });
            operation.notify();
            manager.emit(super::events::Event::Metadata);
        });
        Ok(progress)
    }

    pub fn progress(&self) -> Option<Progress> {
        self.state
            .lock()
            .ok()?
            .operation
            .as_ref()
            .map(|operation| operation.progress.borrow().clone())
    }

    pub fn manage(
        &self,
        manager: &Arc<Manager>,
        action: super::devices::Action,
    ) -> Result<Progress, String> {
        self.operation(manager, Work::Device(action))
    }

    pub fn maintain(
        &self,
        manager: &Arc<Manager>,
        action: super::maintenance::Action,
    ) -> Result<Progress, String> {
        self.operation(manager, Work::Maintenance(action))
    }

    #[cfg(unix)]
    pub(super) fn manage_guarded(
        &self,
        manager: &Arc<Manager>,
        action: super::devices::Action,
        guard: &dyn Fn() -> Result<(), String>,
    ) -> Result<Progress, String> {
        self.operation_guarded(manager, Work::Device(action), guard)
    }
    #[cfg(unix)]
    pub(super) fn maintain_guarded(
        &self,
        manager: &Arc<Manager>,
        action: super::maintenance::Action,
        guard: &dyn Fn() -> Result<(), String>,
    ) -> Result<Progress, String> {
        self.operation_guarded(manager, Work::Maintenance(action), guard)
    }
    fn operation(&self, manager: &Arc<Manager>, work: Work) -> Result<Progress, String> {
        self.operation_guarded(manager, work, &|| Ok(()))
    }
    fn operation_guarded(
        &self,
        manager: &Arc<Manager>,
        work: Work,
        guard: &dyn Fn() -> Result<(), String>,
    ) -> Result<Progress, String> {
        let mut state = self.state.lock().map_err(|_| "Android installer failed")?;
        let lease = manager.begin_mutation(
            matches!(work, Work::Maintenance(_))
                || matches!(work, Work::Device(super::devices::Action::Recover)),
        )?;
        guard()?;
        let progress = Progress {
            operation_id: lease.id.clone(),
            package_ids: Vec::new(),
            phase: Phase::Running,
            stage: "Preparing Android operation".into(),
            received: 0,
            total: 0,
            error: None,
            device_id: None,
        };
        let (sender, _) = watch::channel(progress.clone());
        let (cancel, _) = watch::channel(false);
        let operation = Arc::new(Operation {
            progress: sender,
            cancel,
            manager: Arc::downgrade(manager),
        });
        state.operation = Some(operation.clone());
        let manager = manager.clone();
        tauri::async_runtime::spawn(async move {
            let result = match work {
                Work::Device(action) => {
                    super::devices::perform(&manager, &lease.id, action, &operation).await
                }
                Work::Maintenance(action) => {
                    super::maintenance::perform(&manager, &lease.id, action, &operation).await
                }
            };
            drop(lease);
            operation.progress.send_modify(|progress| {
                progress.phase = if result.is_ok() {
                    Phase::Succeeded
                } else if *operation.cancel.borrow() {
                    Phase::Cancelled
                } else {
                    Phase::Failed
                };
                progress.stage = if result.is_ok() {
                    "Android operation complete".into()
                } else {
                    "Android operation stopped".into()
                };
                match result {
                    Ok(id) => progress.device_id = id,
                    Err(error) => progress.error = Some(error),
                }
            });
            operation.notify();
            manager.emit(super::events::Event::Metadata);
        });
        Ok(progress)
    }

    pub fn cancel(&self, id: &str) -> Result<(), String> {
        let state = self.state.lock().map_err(|_| "Android installer failed")?;
        let operation = state
            .operation
            .as_ref()
            .ok_or("No Android installation is active")?;
        if operation.progress.borrow().operation_id != id {
            return Err("This Android installation is no longer active".into());
        }
        if matches!(
            operation.progress.borrow().phase,
            Phase::Running | Phase::Cancelling
        ) {
            operation.cancel.send_replace(true);
            operation
                .progress
                .send_modify(|progress| progress.phase = Phase::Cancelling);
            operation.notify();
        }
        Ok(())
    }

    #[cfg(unix)]
    pub(super) async fn wait_owned(
        &self,
        id: &str,
        permit: &lomi_control_core::broker::NativePermit,
    ) -> Result<Progress, String> {
        let operation = self
            .state
            .lock()
            .map_err(|_| "Android installer failed")?
            .operation
            .clone()
            .ok_or("Android operation is unavailable")?;
        if operation.progress.borrow().operation_id != id {
            return Err("Android operation changed".into());
        }
        let mut interval = tokio::time::interval(Duration::from_millis(50));
        loop {
            tokio::select! {
                result = operation.wait() => return Ok(result),
                _ = interval.tick() => {
                    if permit.check().is_err() { operation.cancel.send_replace(true); }
                }
            }
        }
    }

    pub async fn settle(&self, cancel: bool) -> Result<(), String> {
        let operation = self
            .state
            .lock()
            .map_err(|_| "Android installer failed")?
            .operation
            .clone();
        if let Some(operation) = operation {
            if cancel {
                operation.cancel.send_replace(true);
            }
            operation.wait().await;
        }
        Ok(())
    }
}

fn check_image_revision(
    package: &Package,
    manifest: &installation::Manifest,
    devices: &storage::Devices,
) -> Result<(), String> {
    if package.image.is_some()
        && devices
            .devices
            .iter()
            .any(|device| device.image == package.id)
    {
        let old = manifest
            .packages
            .get(&package.id)
            .ok_or("The used Android image needs recovery before installation")?;
        if old.revision != package.revision || old.archive_sha1 != package.sha1 {
            return Err("This system image is used by a device and cannot change revision. Create a new device with a different image package.".into());
        }
    }
    Ok(())
}

fn version(value: &str) -> Vec<u32> {
    let mut version: Vec<_> = value
        .split('.')
        .filter_map(|part| part.parse().ok())
        .collect();
    version.resize(3, 0);
    version
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Consents {
    version: u32,
    accepted: BTreeMap<String, String>,
}

async fn install(
    manager: &Arc<Manager>,
    root: &Path,
    plan: &Plan,
    operation: &Operation,
) -> Result<(), String> {
    let mut consents: Consents = storage::read(&root.join("licenses.json"))?.unwrap_or(Consents {
        version: 1,
        accepted: BTreeMap::new(),
    });
    if consents.version != 1 || consents.accepted.len() > 256 {
        return Err(
            "Android license records need explicit recovery; the file was preserved".into(),
        );
    }
    for license in &plan.licenses {
        consents
            .accepted
            .insert(license.id.clone(), license.digest.clone());
    }
    storage::write(&root.join("licenses.json"), &consents)?;
    let mut stages = Vec::new();
    let result = async {
        if plan.bootstrap.is_some() {
            let id = auth::new_id()?;
            stages.push(id.clone());
            bootstrap::prepare(
                root,
                &id,
                operation.cancel.subscribe(),
                |stage, received, total| operation.progress(stage, received, total),
            )
            .await?;
        }
        let tools = {
            let directory = manager
                .directory
                .lock()
                .map_err(|_| "Android directory failed")?;
            bootstrap::installed(&directory)?
        };
        for package in &plan.packages {
            if *operation.cancel.borrow() {
                return Err("Android installation cancelled".into());
            }
            let id = auth::new_id()?;
            stages.push(id.clone());
            let license = plan
                .licenses
                .iter()
                .find(|license| license.id == package.license)
                .ok_or("Approved package license is missing")?;
            install_package(manager, root, &tools, &id, package, license, operation).await?;
        }
        Ok(())
    }
    .await;
    // A publication journal owns its stage until explicit recovery. Other stages
    // contain only this operation's scratch data and never any AVD files.
    if !root.join("installation.json").exists()
        && !root.join("toolchain-installation.json").exists()
    {
        for id in stages {
            let stage = installation::checked_path(root, &PathBuf::from("staging").join(id))?;
            if stage.exists() {
                fs::remove_dir_all(stage).map_err(|e| e.to_string())?;
            }
        }
    }
    result
}

async fn install_package(
    manager: &Arc<Manager>,
    root: &Path,
    tools: &bootstrap::Tools,
    id: &str,
    package: &Package,
    license: &License,
    operation: &Operation,
) -> Result<(), String> {
    let stage = installation::checked_path(root, &PathBuf::from("staging").join(id))?;
    installation::create_directories(root, &stage)?;
    let sdk = stage.join("sdk");
    for path in [
        sdk.join(".sdk/arch"),
        sdk.join("licenses"),
        root.join("user/cli-home"),
        root.join("tmp"),
    ] {
        installation::create_directories(root, &path)?;
    }
    let license_hash: String = ring::digest::digest(
        &ring::digest::SHA1_FOR_LEGACY_USE_ONLY,
        license.text.as_bytes(),
    )
    .as_ref()
    .iter()
    .map(|byte| format!("{byte:02x}"))
    .collect();
    fs::write(
        sdk.join("licenses").join(&license.id),
        format!("\n{license_hash}\n"),
    )
    .map_err(|e| e.to_string())?;
    for dependency in &package.dependencies {
        let directory = manager
            .directory
            .lock()
            .map_err(|_| "Android directory failed")?;
        let installed = directory.installed_path(&dependency.id)?;
        let target = sdk.join(installation::package_path(&dependency.id)?);
        installation::create_directories(root, &target)?;
        // Only metadata is copied. The SDK manager cannot mutate active dependency files.
        let metadata = installed.join("package.xml");
        let status = fs::symlink_metadata(&metadata).map_err(|e| e.to_string())?;
        if !status.is_file() || status.len() > 8 * 1024 * 1024 {
            return Err("Invalid installed dependency metadata".into());
        }
        fs::copy(metadata, target.join("package.xml")).map_err(|e| e.to_string())?;
    }
    disk::require(root, package.size)?;
    let archive = stage.join("archive.zip");
    let summary = download::package(
        package,
        &archive,
        operation.cancel.subscribe(),
        |received, total| {
            operation.progress(&format!("Downloading {}", package.name), received, total)
        },
    )
    .await?;
    disk::require(root, summary.expanded_bytes + package.size)?;
    fs::hard_link(&archive, sdk.join(".sdk/arch").join(&package.sha1))
        .map_err(|e| e.to_string())?;
    let mut command = environment::command(&tools.cli, root, &sdk, Some(&tools.java_home), 5037)?;
    let argument = if package.id.starts_with("cmdline-tools;") {
        package.id.clone()
    } else {
        format!("{}@{}", package.id, package.revision)
    };
    command
        .arg("--no-metrics")
        .arg(format!("--sdk={}", sdk.display()))
        .args(["sdk", "install", &argument]);
    operation.progress(&format!("Installing {}", package.name), 0, 0);
    let output = bootstrap::run(
        command,
        Duration::from_secs(20 * 60),
        operation.cancel.subscribe(),
        |error| operation.progress(error, 0, 0),
    )
    .await?;
    let extracted = sdk.join(installation::package_path(&package.id)?);
    operation.progress(&format!("Verifying {}", package.name), 0, 0);
    let (package_copy, license_copy, archive_copy, extracted_copy, cancel) = (
        package.clone(),
        license.clone(),
        archive.clone(),
        extracted.clone(),
        operation.cancel.subscribe(),
    );
    tokio::task::spawn_blocking(move || {
        artifact::verify_with_cancel(
            &package_copy,
            &license_copy,
            &archive_copy,
            &extracted_copy,
            || *cancel.borrow(),
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    if *operation.cancel.borrow() {
        return Err("Android installation cancelled".into());
    }
    fs::rename(extracted, stage.join("candidate")).map_err(|e| e.to_string())?;
    let installed = Installed {
        id: package.id.clone(),
        revision: package.revision.clone(),
        archive_sha1: package.sha1.clone(),
    };
    operation.progress(&format!("Publishing {}", package.name), 0, 0);
    let mut directory = manager
        .directory
        .lock()
        .map_err(|_| "Android directory failed")?;
    directory.stage_verified(id, &installed)?;
    directory.publish(id, installed, &[])?;
    // Provider output is bounded by InstallerChild; avoid retaining full installation histories.
    let logs = root.join("logs");
    installation::create_directories(root, &logs)?;
    let log = logs.join("installer.log");
    let mut temporary = tempfile::NamedTempFile::new_in(&logs).map_err(|e| e.to_string())?;
    use std::io::Write;
    temporary
        .write_all(output.as_bytes())
        .map_err(|e| e.to_string())?;
    temporary.persist(log).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn synchronous_native_commands_schedule_owned_work_without_an_ambient_runtime() {
        let directory = tempfile::tempdir().unwrap();
        let state = crate::android::manager::Android::default();
        let manager = state.get(directory.path().join("android")).unwrap();
        manager
            .installer
            .manage(&manager, crate::android::devices::Action::Recover)
            .unwrap();
        tauri::async_runtime::block_on(manager.installer.settle(false)).unwrap();
        assert_eq!(
            manager.installer.progress().unwrap().phase,
            super::Phase::Succeeded
        );
    }
    use super::*;

    #[test]
    fn package_identity_survives_progress_snapshots_and_guarded_cancellation() {
        let package_ids =
            vec!["system-images;android-37.2;google_apis_playstore_ps16k;arm64-v8a".into()];
        let (sender, _) = watch::channel(Progress {
            operation_id: "download-1".into(),
            package_ids: package_ids.clone(),
            phase: Phase::Running,
            stage: "Preparing installation".into(),
            received: 0,
            total: 0,
            error: None,
            device_id: None,
        });
        let (cancel, _) = watch::channel(false);
        let operation = Arc::new(Operation {
            progress: sender,
            cancel,
            manager: std::sync::Weak::new(),
        });
        let installer = Installer::default();
        installer.state.lock().unwrap().operation = Some(operation.clone());
        operation.progress("Downloading image", 25, 100);
        let snapshot = installer.progress().unwrap();
        assert_eq!(snapshot.package_ids, package_ids);
        assert_eq!(snapshot.received, 25);
        assert_eq!(
            serde_json::to_value(snapshot).unwrap()["packageIds"],
            serde_json::json!(package_ids)
        );
        operation.progress("Verifying image", 0, 0);
        assert!(installer.cancel("previous-download").is_err());
        assert!(!*operation.cancel.borrow());
        installer.cancel("download-1").unwrap();
        let snapshot = installer.progress().unwrap();
        assert_eq!(snapshot.phase, Phase::Cancelling);
        assert_eq!(snapshot.package_ids, package_ids);
        assert_eq!(snapshot.total, 0);
        assert!(*operation.cancel.borrow());
    }

    #[test]
    fn consent_and_stale_metadata_are_checked_before_starting_any_tool() {
        let temporary = tempfile::tempdir().unwrap();
        let android = super::super::manager::Android::default();
        let manager = android.get(temporary.path().join("managed")).unwrap();
        let license = License {
            id: "android-sdk-license".into(),
            text: "Test terms".into(),
            digest: format!("{:x}", Sha256::digest(b"Test terms")),
        };
        manager.installer.state.lock().unwrap().catalog = Some(repository::Snapshot {
            revision: "test".into(),
            packages: vec![],
            licenses: vec![license.clone()],
        });
        // Consent/revision rejection must be testable without host virtualization.
        // Seed only the pending plan; both start attempts still use production guards.
        let plan = Plan {
            id: auth::new_id().unwrap(),
            catalog_revision: "test".into(),
            packages: vec![],
            licenses: vec![license],
            bootstrap: None,
            download_bytes: 0,
        };
        manager.installer.state.lock().unwrap().plan = Some(PendingPlan {
            plan: plan.clone(),
            created: Instant::now(),
            manifest_revision: 0,
            devices_revision: 0,
        });
        assert!(manager
            .installer
            .start(&manager, &plan.id, vec![])
            .unwrap_err()
            .contains("Accept"));
        assert!(manager.installer.progress().is_none());
        assert!(!temporary.path().join("managed/staging").exists());
        manager
            .directory
            .lock()
            .unwrap()
            .save_devices(storage::Devices::default(), 0)
            .unwrap();
        let accepted = plan
            .licenses
            .iter()
            .map(|license| license.digest.clone())
            .collect();
        assert!(manager
            .installer
            .start(&manager, &plan.id, accepted)
            .unwrap_err()
            .contains("changed"));
        assert!(manager.begin_mutation(true).is_ok());
        assert_eq!(version("23"), version("23.0.0"));
    }

    #[tokio::test]
    #[ignore = "Installs a real SDK and image in the accepted isolated native trial; retains its managed directory"]
    async fn actual_installer_prepares_clean_sdk_and_cancels_safely_during_exit() {
        let trial = PathBuf::from(std::env::var_os("LOMI_ANDROID_PROBE_DIRECTORY").unwrap());
        let consent: serde_json::Value =
            serde_json::from_slice(&fs::read(trial.join("evidence/consent.json")).unwrap())
                .unwrap();
        assert_eq!(consent["accepted"], true);
        assert_eq!(consent["scope"], "Isolated native stage 0 only");
        let java_consent: serde_json::Value =
            serde_json::from_slice(&fs::read(trial.join("evidence/java-consent.json")).unwrap())
                .unwrap();
        assert_eq!(java_consent["accepted"], true);
        let root = trial.join(format!("native-managed-{}", auth::new_id().unwrap()));
        let android = super::super::manager::Android::default();
        let manager = android.get(root.clone()).unwrap();
        let catalog = manager.installer.catalog(true).await.unwrap();
        let select = || {
            [
                ("cmdline-tools;23.0", "23.0"),
                ("emulator", "37.1.11"),
                ("platform-tools", "37.0.1"),
                ("system-images;android-36;default;arm64-v8a", "2"),
            ]
            .into_iter()
            .map(|(id, revision)| Selection {
                id: id.into(),
                revision: revision.into(),
            })
            .collect()
        };
        let plan = manager
            .installer
            .plan(&manager, &catalog.revision, select(), true)
            .unwrap();
        assert!(plan
            .licenses
            .iter()
            .filter(|license| license.id != "lomi-temurin-21")
            .all(|license| consent["sha256"] == license.digest));
        assert_eq!(
            java_consent["licenseSha256"],
            plan.licenses
                .iter()
                .find(|license| license.id == "lomi-temurin-21")
                .unwrap()
                .digest
        );
        let progress = manager
            .installer
            .start(
                &manager,
                &plan.id,
                plan.licenses
                    .iter()
                    .map(|license| license.digest.clone())
                    .collect(),
            )
            .unwrap();
        // Exit preparation freezes starts, then installer cancellation waits for
        // the actual operation owner before the application could stop devices.
        let close = manager.begin_exit().unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let progress = manager.installer.progress().unwrap();
            if progress.received > 0 {
                break;
            }
            assert_eq!(progress.phase, Phase::Running, "{progress:?}");
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        manager.installer.cancel(&progress.operation_id).unwrap();
        manager.installer.settle(true).await.unwrap();
        assert_eq!(
            manager.installer.progress().unwrap().phase,
            Phase::Cancelled
        );
        manager.stop_all(&close, false).await.unwrap();
        manager.resume(&close).unwrap();
        assert!(!root.join("toolchain.json").exists());
        assert_eq!(fs::read_dir(root.join("staging")).unwrap().count(), 0);
        let plan = manager
            .installer
            .plan(&manager, &catalog.revision, select(), true)
            .unwrap();
        let started = Instant::now();
        manager
            .installer
            .start(
                &manager,
                &plan.id,
                plan.licenses
                    .iter()
                    .map(|license| license.digest.clone())
                    .collect(),
            )
            .unwrap();
        let mut progress = manager
            .installer
            .state
            .lock()
            .unwrap()
            .operation
            .as_ref()
            .unwrap()
            .progress
            .subscribe();
        let mut stages = Vec::new();
        loop {
            let snapshot = progress.borrow_and_update().clone();
            if stages.last() != Some(&snapshot.stage) {
                println!("{}", snapshot.stage);
                stages.push(snapshot.stage.clone());
            }
            if !matches!(snapshot.phase, Phase::Running | Phase::Cancelling) {
                assert_eq!(snapshot.phase, Phase::Succeeded, "{snapshot:?}");
                break;
            }
            progress.changed().await.unwrap();
        }
        let directory = manager.directory.lock().unwrap();
        let tools = bootstrap::installed(&directory).unwrap();
        let manifest = directory.manifest().unwrap();
        assert_eq!(manifest.packages.len(), 4);
        assert_eq!(fs::read_dir(root.join("staging")).unwrap().count(), 0);
        assert!(!root.join("user/bin/android-cli").exists());
        assert!(tools.java_home.join("bin/java").exists());
        let profiles = catalog::profiles_from_tools(
            &directory.installed_path("cmdline-tools;23.0").unwrap(),
            921_600,
        )
        .unwrap();
        assert!(!profiles.is_empty());
        let report = serde_json::json!({"version":1,"host":bootstrap::host_key(catalog::Host::native().unwrap()),"root":root,"seconds":started.elapsed().as_secs_f64(),"packages":manifest.packages,"profiles":profiles,"stages":stages,"exitCancellationReapedAndReleased":true,"restartAfterCancellation":true,"privateJava":true,"noStudio":true});
        fs::write(
            trial.join("evidence/native-managed-installation.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        println!("Managed native SDK prepared at {}", root.display());
    }
}
