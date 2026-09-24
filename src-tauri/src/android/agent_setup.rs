use super::{catalog, devices, installer, manager::Manager as AndroidManager, storage};
use lomi_control_core::broker::{
    AndroidManagementPlan, AndroidManagementRequest, AndroidPrepareInput, AndroidSetupRead,
    AndroidTerms,
};
use lomi_control_protocol::{control::*, ErrorCode};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, sync::Arc, time::Duration};
use tauri::Manager;

fn present(
    app: tauri::AppHandle,
) -> Arc<dyn Fn(lomi_control_core::broker::NativePermit) + Send + Sync> {
    Arc::new(move |permit| {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ =
                crate::settings_window::request_checked(&app, Some("agent-control".into()), || {
                    permit
                        .check()
                        .map_err(|_| "Android approval ended".to_string())
                })
                .await;
        });
    })
}
fn failure(_: impl std::fmt::Display) -> ErrorCode {
    ErrorCode::StorageUnavailable
}
fn setup_error(message: &str) -> ErrorCode {
    if message.starts_with("Android needs at least ")
        || message == "Android growth estimate overflow"
        || message == "Android disk estimate overflow"
    {
        ErrorCode::ResourceExhausted
    } else if message.contains("already in progress")
        || message.starts_with("Android installation or device management is in progress.")
    {
        ErrorCode::TargetBusy
    } else {
        ErrorCode::RevisionConflict
    }
}
fn download(package: &catalog::Package) -> AndroidDownload {
    AndroidDownload {
        id: package.id.clone(),
        revision: package.revision.clone(),
        name: package.name.clone(),
        url: package.url.clone(),
        bytes: package.size,
        checksum: format!("sha1:{}", package.sha1),
        license_id: Some(package.license.clone()),
        dependencies: package
            .dependencies
            .iter()
            .map(|d| AndroidPackageSelection {
                id: d.id.clone(),
                revision: d.minimum_revision.clone().unwrap_or_default(),
            })
            .collect(),
    }
}
fn hardware(value: &storage::Hardware) -> AndroidHardware {
    AndroidHardware {
        ram_mib: value.ram_mib,
        cpu_count: value.cpu_count,
        data_gib: value.data_gib,
        gpu: match value.gpu {
            storage::Gpu::Auto => AndroidGpu::Auto,
            storage::Gpu::Host => AndroidGpu::Host,
            storage::Gpu::Software => AndroidGpu::Software,
        },
        quick_boot: value.quick_boot,
    }
}
fn native_hardware(value: &AndroidHardware) -> storage::Hardware {
    storage::Hardware {
        ram_mib: value.ram_mib,
        cpu_count: value.cpu_count,
        data_gib: value.data_gib,
        gpu: match value.gpu {
            AndroidGpu::Auto => storage::Gpu::Auto,
            AndroidGpu::Host => storage::Gpu::Host,
            AndroidGpu::Software => storage::Gpu::Software,
        },
        quick_boot: value.quick_boot,
    }
}
fn revision(value: &impl serde::Serialize) -> Result<String, ErrorCode> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(failure)?)
    ))
}
// Fingerprints remain native. They bind approval to actual metadata and runtime
// identities without disclosing unselected device names to the MCP client.
fn identity(manager: &Arc<AndroidManager>) -> Result<String, ErrorCode> {
    let mut statuses: Vec<_> = manager
        .statuses()
        .map_err(failure)?
        .into_iter()
        .map(|s| (s.device_id, s.generation, s.phase, s.process_alive))
        .collect();
    statuses.sort_by(|a, b| a.0.cmp(&b.0));
    let directory = manager.directory.lock().map_err(failure)?;
    let mut hash = Sha256::new();
    hash.update(serde_json::to_vec(&statuses).map_err(failure)?);
    for name in [
        "devices.json",
        "preferences.json",
        "devices.previous.json",
        "preferences.previous.json",
        "tools-manifest.json",
        "installation.json",
        "toolchain-installation.json",
        "device-operation.json",
        "package-removal.json",
    ] {
        hash.update(name);
        let path = directory.root.join(name);
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => hash.update([0]),
            Err(e) => return Err(failure(e)),
            Ok(meta) => {
                if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 1024 * 1024 {
                    return Err(ErrorCode::ResourceExhausted);
                }
                let mut bytes = Vec::new();
                fs::File::open(path)
                    .map_err(failure)?
                    .take(1024 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(failure)?;
                if bytes.len() > 1024 * 1024 {
                    return Err(ErrorCode::ResourceExhausted);
                }
                hash.update([1]);
                hash.update((bytes.len() as u64).to_le_bytes());
                hash.update(bytes);
            }
        }
    }
    // Use the validated manifest as well; malformed metadata is handled in the
    // existing Settings recovery flow, never replaced by a guessed empty value.
    if let Ok(manifest) = directory.manifest() {
        hash.update(serde_json::to_vec(&manifest).map_err(failure)?);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub(crate) fn prepare(
    app: &tauri::AppHandle,
    input: AndroidPrepareInput,
    allowed: &[String],
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<AndroidSetupRead, ErrorCode> {
    check()?;
    let window = app.get_window("main").ok_or(ErrorCode::UiNotReady)?;
    let manager = super::commands::backend(&window, &app.state::<super::manager::Android>())
        .map_err(failure)?;
    match input {
        AndroidPrepareInput::Setup(AndroidSetupQuery::Inventory) => {
            let statuses = manager.statuses().map_err(failure)?;
            let directory = manager.directory.lock().map_err(failure)?;
            let devices = directory.devices();
            let manifest = directory.manifest();
            let recovery = super::recovery::plans(&directory).map_err(failure)?;
            let rollbacks = super::maintenance::rollbacks(&directory).unwrap_or_default();
            let recovery_required = !recovery.is_empty()
                || devices.is_err()
                || manifest.is_err()
                || [
                    "installation.json",
                    "toolchain-installation.json",
                    "device-operation.json",
                    "package-removal.json",
                ]
                .iter()
                .any(|name| directory.root.join(name).exists());
            let profiles = devices::profiles(&directory)
                .unwrap_or_default()
                .into_iter()
                .map(|p| AndroidProfile {
                    id: p.id,
                    name: p.name,
                    width: p.width,
                    height: p.height,
                    min_api: p.min_api,
                    min_minor_api: p.min_minor_api,
                })
                .collect();
            let view = AndroidSetupView::Inventory {
                devices_revision: devices.as_ref().ok().map(|d| d.revision.to_string()),
                manifest_revision: manifest.as_ref().ok().map(|m| m.revision.to_string()),
                host_qualified: super::host::require_qualification().is_ok(),
                devices: devices
                    .as_ref()
                    .ok()
                    .map(|d| {
                        d.devices
                            .iter()
                            .filter(|d| allowed.contains(&d.id))
                            .map(|d| {
                                let status = statuses.iter().find(|s| s.device_id == d.id);
                                AndroidManagedDevice {
                                    device_id: d.id.clone(),
                                    name: d.name.clone(),
                                    image: d.image.clone(),
                                    image_revision: d.image_revision.to_string(),
                                    profile: d.profile.clone(),
                                    hardware: hardware(&d.hardware),
                                    generation: status.and_then(|s| s.generation.clone()),
                                    process_alive: status.is_some_and(|s| s.process_alive),
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                installed: manifest
                    .as_ref()
                    .ok()
                    .map(|m| {
                        m.packages
                            .values()
                            .map(|p| AndroidPackageSelection {
                                id: p.id.clone(),
                                revision: p.revision.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                profiles,
                recovery_required,
                recovery: recovery
                    .into_iter()
                    .map(|p| AndroidMetadataRecovery {
                        file: match p.file {
                            super::recovery::File::Devices => AndroidMetadataFile::Devices,
                            super::recovery::File::Preferences => AndroidMetadataFile::Preferences,
                        },
                        digest: p.digest,
                        backup_revision: p.backup_revision.map(|r| r.to_string()),
                    })
                    .collect(),
                rollbacks: rollbacks
                    .into_iter()
                    .map(|p| AndroidPackageSelection {
                        id: p.id,
                        revision: p.revision,
                    })
                    .collect(),
            };
            check()?;
            Ok(AndroidSetupRead { view, plan: None })
        }
        AndroidPrepareInput::Setup(AndroidSetupQuery::Catalog {
            refresh,
            offset,
            limit,
            expected_catalog_revision,
        }) => {
            if !(1..=32).contains(&limit)
                || (refresh && offset != 0)
                || offset > 4096
                || (offset > 0 && expected_catalog_revision.is_none())
            {
                return Err(ErrorCode::ResourceExhausted);
            }
            let catalog = tauri::async_runtime::block_on(async {
                tokio::time::timeout(Duration::from_secs(40), manager.installer.catalog(refresh))
                    .await
                    .map_err(|_| ErrorCode::DeadlineExceeded)?
                    .map_err(failure)
            })?;
            check()?;
            if expected_catalog_revision
                .as_ref()
                .is_some_and(|r| *r != catalog.revision)
            {
                return Err(ErrorCode::RevisionConflict);
            }
            let end = (offset as usize + limit as usize).min(catalog.packages.len());
            if offset as usize > catalog.packages.len() {
                return Err(ErrorCode::RevisionConflict);
            }
            Ok(AndroidSetupRead {
                view: AndroidSetupView::Catalog {
                    catalog_revision: catalog.revision,
                    packages: catalog.packages[offset as usize..end]
                        .iter()
                        .map(download)
                        .collect(),
                    next_offset: (end < catalog.packages.len()).then_some(end as u32),
                },
                plan: None,
            })
        }
        AndroidPrepareInput::Setup(AndroidSetupQuery::Prepare {
            catalog_revision,
            packages,
            prepare_tools,
        }) => {
            super::host::require_qualification().map_err(|_| ErrorCode::HostUnqualified)?;
            if catalog_revision.len() != 64
                || packages.len() > 16
                || packages
                    .iter()
                    .any(|p| p.id.len() > 256 || p.revision.len() > 64)
            {
                return Err(ErrorCode::ResourceExhausted);
            }
            check()?;
            let plan = manager
                .installer
                .plan(
                    &manager,
                    &catalog_revision,
                    packages
                        .into_iter()
                        .map(|p| installer::Selection {
                            id: p.id,
                            revision: p.revision,
                        })
                        .collect(),
                    prepare_tools,
                )
                .map_err(|error| setup_error(&error))?;
            let mut downloads: Vec<_> = plan.packages.iter().map(download).collect();
            if let Some(bootstrap) = &plan.bootstrap {
                for (id, blob) in [
                    ("private-sdk-cli", &bootstrap.cli),
                    ("private-java", &bootstrap.java),
                ] {
                    downloads.push(AndroidDownload {
                        id: id.into(),
                        revision: blob.version.clone(),
                        name: id.into(),
                        url: blob.url.clone(),
                        bytes: blob.size,
                        checksum: format!("sha256:{}", blob.sha256),
                        license_id: None,
                        dependencies: vec![],
                    });
                }
            }
            let view = AndroidPreparedPlan {
                plan_id: plan.id.clone(),
                revision: revision(&plan)?,
                downloads,
                licenses: plan
                    .licenses
                    .iter()
                    .map(|l| AndroidLicenseSummary {
                        id: l.id.clone(),
                        digest: l.digest.clone(),
                    })
                    .collect(),
                download_bytes: plan.download_bytes,
                target: "Lomi's private Android SDK and Java; existing devices retain their images"
                    .into(),
                expires_in_seconds: 1800,
            };
            let licenses = plan
                .licenses
                .into_iter()
                .map(|l| AndroidTerms {
                    id: l.id,
                    digest: l.digest,
                    text: l.text,
                })
                .collect();
            let id = plan.id;
            let runtime = manager.clone();
            let prepared = AndroidManagementPlan {
                view: view.clone(),
                present: present(app.clone()),
                licenses,
                apply: Arc::new(move |request| {
                    request.permit.check()?;
                    // Mark only after the mutation gate is held and final authority is checked.
                    let progress = runtime
                        .installer
                        .start_guarded(&runtime, &id, request.accepted.clone(), &|| {
                            request
                                .mark_dispatching()
                                .map_err(|_| "Android approval ended".to_string())
                        })
                        .map_err(|error| {
                            request
                                .permit
                                .check()
                                .err()
                                .unwrap_or_else(|| setup_error(&error))
                        })?;
                    complete(&runtime, &request, progress)
                }),
            };
            check()?;
            Ok(AndroidSetupRead {
                view: AndroidSetupView::Prepared(view),
                plan: Some(prepared),
            })
        }
        AndroidPrepareInput::Device(action) => {
            prepare_device(app.clone(), manager, action, allowed, check)
        }
    }
}
fn native_action(
    manager: &Arc<AndroidManager>,
    action: &AndroidDeviceAction,
    allowed: &[String],
) -> Result<Option<devices::Action>, ErrorCode> {
    if matches!(action, AndroidDeviceAction::Recover) {
        return Ok(Some(devices::Action::Recover));
    }
    if matches!(
        action,
        AndroidDeviceAction::Cleanup
            | AndroidDeviceAction::RestoreMetadata { .. }
            | AndroidDeviceAction::RemovePackage { .. }
            | AndroidDeviceAction::RollbackPackage { .. }
    ) {
        maintenance_action(manager, action)?;
        return Ok(None);
    }
    if matches!(
        action,
        AndroidDeviceAction::Create { .. } | AndroidDeviceAction::Wipe { .. }
    ) {
        super::host::require_qualification().map_err(|_| ErrorCode::HostUnqualified)?;
    }
    let statuses = manager.statuses().map_err(failure)?;
    let directory = manager.directory.lock().map_err(failure)?;
    let current = directory.devices().map_err(failure)?;
    let (expected, device_id, generation) = match action {
        AndroidDeviceAction::Create {
            expected_devices_revision,
            ..
        } => (expected_devices_revision, None, None),
        AndroidDeviceAction::Modify {
            expected_devices_revision,
            device_id,
            generation,
            ..
        }
        | AndroidDeviceAction::Wipe {
            expected_devices_revision,
            device_id,
            generation,
            ..
        }
        | AndroidDeviceAction::Delete {
            expected_devices_revision,
            device_id,
            generation,
            ..
        } => (expected_devices_revision, Some(device_id), Some(generation)),
        _ => unreachable!(),
    };
    if expected.parse::<u64>().ok() != Some(current.revision) {
        return Err(ErrorCode::RevisionConflict);
    }
    let old = if let Some(id) = device_id {
        if !allowed.contains(id) {
            return Err(ErrorCode::ScopeDenied);
        }
        let status = statuses.iter().find(|s| s.device_id == *id);
        if status.and_then(|s| s.generation.as_ref()) != generation.unwrap().as_ref() {
            return Err(ErrorCode::StaleGeneration);
        }
        if status.is_some_and(|s| s.process_alive) {
            return Err(ErrorCode::TargetBusy);
        }
        Some(
            current
                .devices
                .iter()
                .find(|d| d.id == *id)
                .ok_or(ErrorCode::TargetNotFound)?
                .clone(),
        )
    } else {
        None
    };
    let result = match action {
        AndroidDeviceAction::Create {
            name,
            image,
            profile,
            hardware,
            ..
        } => {
            let image_revision = directory
                .manifest()
                .map_err(failure)?
                .packages
                .get(image)
                .ok_or(ErrorCode::TargetNotFound)?
                .revision
                .parse()
                .map_err(failure)?;
            let candidate = storage::Device {
                id: super::auth::new_id().map_err(failure)?,
                name: name.clone(),
                image: image.clone(),
                image_revision,
                profile: profile.clone(),
                hardware: native_hardware(hardware),
                input_bridge: true,
            };
            devices::validate_agent_device(&directory, &candidate)
                .map_err(|_| ErrorCode::UnsupportedCapability)?;
            devices::Action::Create {
                expected_revision: current.revision,
                draft: devices::Draft {
                    name: name.clone(),
                    image: image.clone(),
                    profile: profile.clone(),
                    hardware: native_hardware(hardware),
                },
            }
        }
        AndroidDeviceAction::Modify {
            name,
            profile,
            hardware,
            ..
        } => {
            let mut device = old.unwrap();
            if device.hardware.data_gib != hardware.data_gib {
                return Err(ErrorCode::RevisionConflict);
            }
            device.name = name.clone();
            device.profile = profile.clone();
            device.hardware = native_hardware(hardware);
            devices::validate_agent_device(&directory, &device)
                .map_err(|_| ErrorCode::UnsupportedCapability)?;
            devices::Action::Update {
                expected_revision: current.revision,
                device,
            }
        }
        AndroidDeviceAction::Wipe { confirmation, .. }
        | AndroidDeviceAction::Delete { confirmation, .. } => {
            let device = old.unwrap();
            if confirmation != &device.name {
                return Err(ErrorCode::ScopeDenied);
            }
            if matches!(action, AndroidDeviceAction::Wipe { .. }) {
                devices::Action::Wipe {
                    expected_revision: current.revision,
                    device_id: device.id,
                    confirmation: confirmation.clone(),
                }
            } else {
                devices::Action::Delete {
                    expected_revision: current.revision,
                    device_id: device.id,
                    confirmation: confirmation.clone(),
                }
            }
        }
        _ => unreachable!(),
    };
    Ok(Some(result))
}
fn maintenance_action(
    manager: &Arc<AndroidManager>,
    action: &AndroidDeviceAction,
) -> Result<super::maintenance::Action, ErrorCode> {
    use super::maintenance::Action;
    let directory = manager.directory.lock().map_err(failure)?;
    match action {
        AndroidDeviceAction::Cleanup => Ok(Action::Cleanup),
        AndroidDeviceAction::RestoreMetadata {
            file,
            digest,
            reset,
        } => {
            if *reset && matches!(file, AndroidMetadataFile::Devices) {
                return Err(ErrorCode::UnsupportedCapability);
            }
            let native = match file {
                AndroidMetadataFile::Devices => super::recovery::File::Devices,
                AndroidMetadataFile::Preferences => super::recovery::File::Preferences,
            };
            let plan = super::recovery::plans(&directory)
                .map_err(failure)?
                .into_iter()
                .find(|p| std::mem::discriminant(&p.file) == std::mem::discriminant(&native))
                .ok_or(ErrorCode::TargetNotFound)?;
            if plan.digest != *digest || (!reset && plan.backup_revision.is_none()) {
                return Err(ErrorCode::RevisionConflict);
            }
            Ok(Action::RestoreMetadata {
                file: native,
                digest: digest.clone(),
                reset: *reset,
            })
        }
        AndroidDeviceAction::RemovePackage {
            package_id,
            expected_manifest_revision,
        }
        | AndroidDeviceAction::RollbackPackage {
            package_id,
            expected_manifest_revision,
        } => {
            let manifest = directory.manifest().map_err(failure)?;
            if expected_manifest_revision.parse::<u64>().ok() != Some(manifest.revision) {
                return Err(ErrorCode::RevisionConflict);
            }
            if !manifest.packages.contains_key(package_id) {
                return Err(ErrorCode::TargetNotFound);
            }
            if directory
                .devices()
                .map_err(failure)?
                .devices
                .iter()
                .any(|d| d.image == *package_id)
            {
                return Err(ErrorCode::TargetBusy);
            }
            if matches!(action, AndroidDeviceAction::RemovePackage { .. }) {
                Ok(Action::Remove {
                    package_id: package_id.clone(),
                    expected_revision: manifest.revision,
                })
            } else {
                if !super::maintenance::rollbacks(&directory)
                    .map_err(failure)?
                    .iter()
                    .any(|p| p.id == *package_id)
                {
                    return Err(ErrorCode::TargetNotFound);
                }
                Ok(Action::Rollback {
                    package_id: package_id.clone(),
                    expected_revision: manifest.revision,
                })
            }
        }
        _ => Err(ErrorCode::UnsupportedCapability),
    }
}
fn prepare_device(
    app: tauri::AppHandle,
    manager: Arc<AndroidManager>,
    action: AndroidDeviceAction,
    allowed: &[String],
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<AndroidSetupRead, ErrorCode> {
    // Validate without acquiring a mutation gate, allocating an AVD or changing files.
    let before = identity(&manager)?;
    native_action(&manager, &action, allowed)?;
    if identity(&manager)? != before {
        return Err(ErrorCode::RevisionConflict);
    }
    check()?;
    let target = match &action {
        AndroidDeviceAction::Create { name, .. } => format!("Create Android device: {name}"),
        AndroidDeviceAction::Modify {
            name, device_id, ..
        } => format!("Modify Android device {device_id}: {name}; apply hardware on next start"),
        AndroidDeviceAction::Wipe {
            confirmation,
            device_id,
            ..
        } => format!("Erase all data on Android device {device_id}: {confirmation}"),
        AndroidDeviceAction::Delete {
            confirmation,
            device_id,
            ..
        } => format!("Delete Android device {device_id} and all its data: {confirmation}"),
        AndroidDeviceAction::Recover => {
            "Recover interrupted operations in Lomi's private Android directory".into()
        }
        AndroidDeviceAction::RestoreMetadata { file, reset, .. } => format!(
            "{} Android {:?} metadata; preserve the current file as a recovery copy",
            if *reset { "Reset" } else { "Restore backed-up" },
            file
        ),
        AndroidDeviceAction::RemovePackage { package_id, .. } => {
            format!("Remove managed Android package: {package_id}; refuse if any device uses it")
        }
        AndroidDeviceAction::RollbackPackage { package_id, .. } => {
            format!("Restore the previous managed Android tools: {package_id}")
        }
        AndroidDeviceAction::Cleanup => {
            "Clean owned Android caches, staging and logs; preserve device data".into()
        }
    };
    let view = AndroidPreparedPlan {
        plan_id: super::auth::new_id().map_err(failure)?,
        revision: revision(&(&action, &before))?,
        downloads: vec![],
        licenses: vec![],
        download_bytes: 0,
        target,
        expires_in_seconds: 600,
    };
    let allowed = allowed.to_vec();
    let prepared = AndroidManagementPlan {
        view: view.clone(),
        present: present(app),
        licenses: vec![],
        apply: Arc::new(move |request| {
            request.permit.check()?;
            let native = native_action(&manager, &action, &allowed)?;
            let guard = || {
                request
                    .permit
                    .check()
                    .map_err(|_| "Android approval ended".to_string())?;
                if identity(&manager).map_err(|_| "Android metadata unavailable")? != before {
                    return Err("Android approval is stale".into());
                }
                request
                    .mark_dispatching()
                    .map_err(|_| "Android approval ended".to_string())
            };
            let progress = match native {
                Some(action) => manager.installer.manage_guarded(&manager, action, &guard),
                None => manager.installer.maintain_guarded(
                    &manager,
                    maintenance_action(&manager, &action)?,
                    &guard,
                ),
            }
            .map_err(|error| {
                request
                    .permit
                    .check()
                    .err()
                    .unwrap_or_else(|| setup_error(&error))
            })?;
            complete(&manager, &request, progress)
        }),
    };
    Ok(AndroidSetupRead {
        view: AndroidSetupView::Prepared(view),
        plan: Some(prepared),
    })
}
fn complete(
    manager: &Arc<AndroidManager>,
    request: &AndroidManagementRequest,
    progress: installer::Progress,
) -> Result<AndroidManagementResult, ErrorCode> {
    let result = tauri::async_runtime::block_on(
        manager
            .installer
            .wait_owned(&progress.operation_id, &request.permit),
    )
    .map_err(failure)?;
    request.permit.check()?;
    if result.phase != installer::Phase::Succeeded {
        return Err(ErrorCode::OutcomeUnknown);
    }
    let directory = manager.directory.lock().map_err(failure)?;
    Ok(AndroidManagementResult {
        workspace_id: request.workspace_id.clone(),
        native_operation_id: result.operation_id,
        device_id: result.device_id,
        devices_revision: directory.devices().ok().map(|d| d.revision.to_string()),
        manifest_revision: directory.manifest().ok().map(|m| m.revision.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_resource_and_mutation_gate_failures_have_typed_mcp_errors() {
        let root = tempfile::tempdir().unwrap();
        let manager = super::super::manager::Android::default()
            .get(root.path().join("android"))
            .unwrap();
        let error =
            super::super::disk::require(&manager.directory.lock().unwrap().root, u64::MAX / 4)
                .unwrap_err();
        assert_eq!(setup_error(&error), ErrorCode::ResourceExhausted);
        let _held = manager.begin_mutation(false).unwrap();
        let Err(error) = manager.begin_mutation(false) else {
            panic!("Mutation exclusion failed")
        };
        assert_eq!(setup_error(&error), ErrorCode::TargetBusy);
        assert!(manager.installer.progress().is_none());
    }

    #[test]
    fn destructive_preflight_binds_name_generation_and_metadata_without_mutation() {
        let root = tempfile::tempdir().unwrap();
        let backend = super::super::manager::Android::default();
        let manager = backend.get(root.path().join("android")).unwrap();
        let id = "00000000-0000-4000-8000-000000000001";
        let device = storage::Device {
            id: id.into(),
            name: "Exact phone".into(),
            image: "system-images;android-36;google_apis;arm64-v8a".into(),
            image_revision: 1,
            profile: "pixel".into(),
            hardware: storage::Hardware {
                ram_mib: 2560,
                cpu_count: 2,
                data_gib: 4,
                gpu: storage::Gpu::Host,
                quick_boot: false,
            },
            input_bridge: true,
        };
        {
            let mut directory = manager.directory.lock().unwrap();
            let mut devices = directory.devices().unwrap();
            devices.devices.push(device);
            directory.save_devices(devices, 0).unwrap();
        }
        let before = identity(&manager).unwrap();
        let mut action = AndroidDeviceAction::Delete {
            expected_devices_revision: "1".into(),
            device_id: id.into(),
            generation: None,
            confirmation: "wrong".into(),
        };
        assert!(matches!(
            native_action(&manager, &action, &[id.into()]),
            Err(ErrorCode::ScopeDenied)
        ));
        if let AndroidDeviceAction::Delete {
            confirmation,
            generation,
            ..
        } = &mut action
        {
            *confirmation = "Exact phone".into();
            *generation = Some(id.into());
        }
        assert!(matches!(
            native_action(&manager, &action, &[id.into()]),
            Err(ErrorCode::StaleGeneration)
        ));
        if let AndroidDeviceAction::Delete { generation, .. } = &mut action {
            *generation = None;
        }
        assert!(matches!(
            native_action(&manager, &action, &[]),
            Err(ErrorCode::ScopeDenied)
        ));
        assert!(native_action(&manager, &action, &[id.into()])
            .unwrap()
            .is_some());
        assert_eq!(identity(&manager).unwrap(), before);
        let mut directory = manager.directory.lock().unwrap();
        let mut preferences = directory.preferences().unwrap();
        preferences.default_device_id = Some(id.into());
        directory.save_preferences(preferences, 0).unwrap();
        drop(directory);
        assert_ne!(
            identity(&manager).unwrap(),
            before,
            "An unrelated native metadata edit invalidates prepared approval"
        );
    }

    #[test]
    fn mutation_guard_runs_under_exclusion_and_preserves_failed_recovery() {
        let root = tempfile::tempdir().unwrap();
        let manager = super::super::manager::Android::default()
            .get(root.path().join("android"))
            .unwrap();
        let path = manager
            .directory
            .lock()
            .unwrap()
            .root
            .join("preferences.json");
        fs::write(&path, b"malformed metadata").unwrap();
        let before = identity(&manager).unwrap();
        let plan = super::super::recovery::plans(&manager.directory.lock().unwrap())
            .unwrap()
            .remove(0);
        let action = AndroidDeviceAction::RestoreMetadata {
            file: AndroidMetadataFile::Preferences,
            digest: plan.digest.clone(),
            reset: true,
        };
        assert!(maintenance_action(&manager, &action).is_ok());
        let result = manager.installer.maintain_guarded(
            &manager,
            maintenance_action(&manager, &action).unwrap(),
            &|| {
                assert!(
                    manager.begin_mutation(false).is_err(),
                    "Approval must be rechecked after excluding concurrent starts and writes"
                );
                Err("Revoked fixture approval".into())
            },
        );
        assert!(result.is_err());
        assert!(manager.installer.progress().is_none());
        assert_eq!(identity(&manager).unwrap(), before);
        fs::write(&path, b"changed malformed metadata").unwrap();
        assert!(matches!(
            maintenance_action(&manager, &action),
            Err(ErrorCode::RevisionConflict)
        ));
        assert!(
            manager.begin_mutation(false).is_ok(),
            "A refused request releases only its own mutation gate"
        );
    }
}
