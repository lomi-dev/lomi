use super::{
    auth, avd, bootstrap, catalog, disk, environment, installation,
    installer::Operation,
    manager::Manager,
    runtime,
    storage::{self, Device, Devices, Directory, Hardware, Preferences},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const JOURNAL: &str = "device-operation.json";
const MARKER: &str = ".lomi-avd.json";
pub const TOOLS: &str = "cmdline-tools;23.0";

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Draft {
    pub name: String,
    pub image: String,
    pub profile: String,
    pub hardware: Hardware,
}

#[derive(Clone, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Action {
    Create {
        expected_revision: u64,
        draft: Draft,
    },
    Update {
        expected_revision: u64,
        device: Device,
    },
    Wipe {
        expected_revision: u64,
        device_id: String,
        confirmation: String,
    },
    Delete {
        expected_revision: u64,
        device_id: String,
        confirmation: String,
    },
    Recover,
}

#[derive(Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Marker {
    version: u32,
    device_id: String,
    operation: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Journal {
    version: u32,
    operation: String,
    device_id: String,
    before: Devices,
    after: Devices,
    preferences_before: Preferences,
    preferences_after: Preferences,
    replacement: bool,
}
impl Journal {
    fn validate(&self) -> Result<(), String> {
        self.before.validate()?;
        self.after.validate()?;
        self.preferences_before.validate()?;
        self.preferences_after.validate()?;
        if self.version != 1
            || !storage::valid_id(&self.operation)
            || !storage::valid_id(&self.device_id)
            || self.after.revision != self.before.revision + 1
        {
            return Err("Invalid Android device recovery journal; files were preserved".into());
        }
        let exists = self
            .before
            .devices
            .iter()
            .any(|device| device.id == self.device_id);
        let mut expected = self.before.devices.clone();
        if self.replacement && !exists {
            expected.push(
                self.after
                    .devices
                    .iter()
                    .find(|device| device.id == self.device_id)
                    .ok_or("Missing created Android device")?
                    .clone(),
            );
        } else if !self.replacement && exists {
            expected.retain(|device| device.id != self.device_id);
        } else if !self.replacement {
            return Err("Removed Android device is absent from its journal".into());
        }
        if expected != self.after.devices {
            return Err("Android recovery journal changes unrelated devices".into());
        }
        let mut preferences = self.preferences_before.clone();
        if !self.replacement && preferences.default_device_id.as_deref() == Some(&self.device_id) {
            preferences.default_device_id = None;
            preferences.revision += 1;
        }
        if preferences != self.preferences_after {
            return Err("Android recovery journal changes unrelated preferences".into());
        }
        Ok(())
    }
}

pub fn profiles(directory: &Directory) -> Result<Vec<catalog::Profile>, String> {
    catalog::profiles_from_tools(
        &directory.installed_path(TOOLS)?,
        super::rpc::MAX_DISPLAY_PIXELS,
    )
}

fn installed_api(properties: &BTreeMap<String, String>) -> Result<(u32, u32), String> {
    let value = properties
        .get("AndroidVersion.ApiLevel")
        .ok_or("Installed Android image has no API level")?;
    let (api, inline_minor) = catalog::api_level(value)
        .filter(|(api, _)| *api >= 26)
        .ok_or("Installed Android image has no valid API level")?;
    let minor = properties
        .get("AndroidVersion.ApiMinorLevel")
        .map(|value| value.parse::<u32>().map_err(|_| "Invalid image minor API"))
        .transpose()?
        .unwrap_or(inline_minor);
    if value.contains('.') && minor != inline_minor {
        return Err("Installed Android image has conflicting minor API levels".into());
    }
    Ok((api, minor))
}

#[cfg(unix)]
pub(super) fn validate_agent_device(directory: &Directory, device: &Device) -> Result<(), String> {
    profile(directory, device).map(|_| ())
}

fn profile(directory: &Directory, device: &Device) -> Result<catalog::Profile, String> {
    device.validate()?;
    let image = directory
        .manifest()?
        .packages
        .get(&device.image)
        .cloned()
        .ok_or("Install the selected Android image first")?;
    if image.revision != device.image_revision.to_string()
        || device.image.rsplit(';').next() != Some(catalog::Host::native()?.abi())
    {
        return Err("The selected system image revision is unavailable or incompatible".into());
    }
    let path = directory.installed_path(&device.image)?;
    let properties = avd::read_ini(&path.join("source.properties"))?;
    let (api, minor) = installed_api(&properties)?;
    let profile = profiles(directory)?
        .into_iter()
        .find(|profile| profile.id == device.profile)
        .ok_or("Select an available phone profile within the supported display size")?;
    if (api, minor) < (profile.min_api, profile.min_minor_api) {
        return Err("This hardware profile requires a newer Android image".into());
    }
    if device.hardware.quick_boot {
        return Err("Quick Boot has not been qualified. Select Cold Boot.".into());
    }
    if device.hardware.ram_mib < 2560 {
        return Err(
            "The qualified Android configuration requires at least 2560 MiB of guest RAM".into(),
        );
    }
    if !device.input_bridge {
        return Err("Full text input requires the Lomi input method in this device".into());
    }
    Ok(profile)
}

pub async fn perform(
    manager: &Arc<Manager>,
    operation: &str,
    action: Action,
    progress: &Operation,
) -> Result<Option<String>, String> {
    if matches!(action, Action::Recover) {
        let mut directory = manager
            .directory
            .lock()
            .map_err(|_| "Android directory failed")?;
        runtime::require_no_recovered_process(&directory.root)?;
        super::maintenance::recover(&mut directory)?;
        recover(&mut directory)?;
        directory.recover_installation()?;
        bootstrap::recover(&mut directory)?;
        return Ok(None);
    }
    let (root, before, preferences) = {
        let directory = manager
            .directory
            .lock()
            .map_err(|_| "Android directory failed")?;
        for name in [
            JOURNAL,
            "installation.json",
            "toolchain-installation.json",
            "package-removal.json",
        ] {
            if directory.root.join(name).exists() {
                return Err("Repair the interrupted Android operation first".into());
            }
        }
        (
            directory.root.clone(),
            directory.devices()?,
            directory.preferences()?,
        )
    };
    let expected = match &action {
        Action::Create {
            expected_revision, ..
        }
        | Action::Update {
            expected_revision, ..
        }
        | Action::Delete {
            expected_revision, ..
        }
        | Action::Wipe {
            expected_revision, ..
        } => *expected_revision,
        Action::Recover => unreachable!(),
    };
    if before.revision != expected {
        return Err("Android devices changed. Reload them and retry.".into());
    }
    if let Action::Update { device, .. } = action {
        let previous = before
            .devices
            .iter()
            .find(|entry| entry.id == device.id)
            .ok_or("Android device is missing")?;
        if previous.image != device.image || previous.image_revision != device.image_revision {
            return Err(
                "Changing the Android system image creates a new phone. Use Create device.".into(),
            );
        }
        if previous.hardware.data_gib != device.hardware.data_gib {
            return Err("Data partition size is fixed for this device. Create a new phone to choose another size.".into());
        }
        let mut directory = manager
            .directory
            .lock()
            .map_err(|_| "Android directory failed")?;
        profile(&directory, &device)?;
        let mut after = before;
        *after
            .devices
            .iter_mut()
            .find(|entry| entry.id == device.id)
            .unwrap() = device.clone();
        directory.save_devices(after, expected)?;
        // Only desired metadata changes here. A running AVD is never rewritten.
        return Ok(Some(device.id));
    }
    let wiping = matches!(&action, Action::Wipe { .. });
    let (device, replacement, is_new) = match action {
        Action::Create { draft, .. } => {
            let directory = manager
                .directory
                .lock()
                .map_err(|_| "Android directory failed")?;
            let revision = directory
                .manifest()?
                .packages
                .get(&draft.image)
                .ok_or("Install this Android image first")?
                .revision
                .parse()
                .map_err(|_| "Unsupported image revision")?;
            (
                Device {
                    id: auth::new_id()?,
                    name: draft.name,
                    image: draft.image,
                    image_revision: revision,
                    profile: draft.profile,
                    hardware: draft.hardware,
                    input_bridge: true,
                },
                true,
                true,
            )
        }
        Action::Wipe {
            device_id,
            confirmation,
            ..
        }
        | Action::Delete {
            device_id,
            confirmation,
            ..
        } => {
            let device = before
                .devices
                .iter()
                .find(|device| device.id == device_id)
                .ok_or("Android device is missing")?
                .clone();
            if confirmation != device.name {
                return Err("Confirm this device's exact name before erasing its data".into());
            }
            (device, wiping, false)
        }
        _ => unreachable!(),
    };
    if manager
        .statuses()?
        .iter()
        .any(|status| status.device_id == device.id && status.process_alive)
    {
        return Err("Stop this Android device before deleting or resetting its data".into());
    }
    runtime::check_previous(
        &root.join("runtime").join(format!("{}.json", device.id)),
        &device.id,
    )?;
    let stage = installation::checked_path(&root, &PathBuf::from("staging").join(operation))?;
    installation::create_directories(&root, &stage)?;
    let result = async {
        if replacement {
            create_candidate(manager, &device, operation, progress).await?;
        }
        if *progress.cancellation().borrow() {
            return Err("Android device operation cancelled".into());
        }
        let mut after = before.clone();
        if is_new {
            after.devices.push(device.clone());
        }
        if !replacement {
            after.devices.retain(|entry| entry.id != device.id);
        }
        after.revision = after
            .revision
            .checked_add(1)
            .filter(|revision| *revision < (1 << 53))
            .ok_or("Android device revision limit reached")?;
        let mut preferences_after = preferences.clone();
        if !replacement && preferences.default_device_id.as_deref() == Some(&device.id) {
            preferences_after.default_device_id = None;
            preferences_after.revision += 1;
        }
        let journal = Journal {
            version: 1,
            operation: operation.into(),
            device_id: device.id.clone(),
            before,
            after,
            preferences_before: preferences,
            preferences_after,
            replacement,
        };
        progress.progress("Publishing device changes", 0, 0);
        storage::write(&root.join(JOURNAL), &journal)?;
        let mut directory = manager
            .directory
            .lock()
            .map_err(|_| "Android directory failed")?;
        finish(&mut directory, &journal)?;
        Ok(Some(device.id))
    }
    .await;
    if !root.join(JOURNAL).exists() && stage.exists() {
        fs::remove_dir_all(&stage).map_err(|e| e.to_string())?;
    }
    result
}

async fn create_candidate(
    manager: &Arc<Manager>,
    device: &Device,
    operation: &str,
    progress: &Operation,
) -> Result<(), String> {
    let (root, tools, sdk_tools, selected) = {
        let directory = manager
            .directory
            .lock()
            .map_err(|_| "Android directory failed")?;
        (
            directory.root.clone(),
            bootstrap::installed(&directory)?,
            directory.installed_path(TOOLS)?,
            profile(&directory, device)?,
        )
    };
    disk::require(
        &root,
        u64::from(device.hardware.data_gib) * 1024 * 1024 * 1024,
    )?;
    let stage = root.join("staging").join(operation);
    let candidate = stage.join("candidate");
    let locators = stage.join("locators");
    installation::create_directories(&root, &locators)?;
    let java = tools
        .java_home
        .join("bin")
        .join(if cfg!(windows) { "java.exe" } else { "java" });
    let mut command = environment::command(
        &java,
        &root,
        &root.join("sdk"),
        Some(&tools.java_home),
        5037,
    )?;
    command
        .env("ANDROID_AVD_HOME", &locators)
        .arg("-Djava.awt.headless=true")
        .arg(format!(
            "-Dcom.android.sdkmanager.toolsdir={}",
            sdk_tools.display()
        ))
        .arg("-classpath")
        .arg(sdk_tools.join("lib/avdmanager-classpath.jar"))
        .args([
            "com.android.sdklib.tool.AvdManagerCli",
            "create",
            "avd",
            "--name",
        ])
        .arg(format!("sb_{}", device.id))
        .arg("--package")
        .arg(&device.image)
        .arg("--device")
        .arg(&device.profile)
        .arg("--path")
        .arg(&candidate);
    progress.progress("Creating Android device", 0, 0);
    bootstrap::run(
        command,
        Duration::from_secs(90),
        progress.cancellation(),
        |error| progress.progress(error, 0, 0),
    )
    .await?;
    configure(&candidate, device, &selected)?;
    storage::write(
        &candidate.join(MARKER),
        &Marker {
            version: 1,
            device_id: device.id.clone(),
            operation: operation.into(),
        },
    )?;
    installation::sync_directory(&candidate)
}

fn configure(path: &Path, device: &Device, profile: &catalog::Profile) -> Result<(), String> {
    let mut config = avd::read_ini(&path.join("config.ini"))?;
    for (key, value) in [
        ("hw.keyboard", "yes"),
        ("hw.audioInput", "no"),
        ("hw.audioOutput", "no"),
        ("hw.camera.back", "none"),
        ("hw.camera.front", "none"),
        ("hw.sdCard", "no"),
        ("showDeviceFrame", "no"),
        ("hw.multi_display_window", "no"),
        ("fastboot.forceColdBoot", "yes"),
        ("fastboot.forceFastBoot", "no"),
        ("firstboot.bootFromDownloadableSnapshot", "no"),
        ("firstboot.bootFromLocalSnapshot", "no"),
        ("firstboot.saveToLocalSnapshot", "no"),
    ] {
        config.insert(key.into(), value.into());
    }
    for key in ["sdcard.size", "sdcard.path", "hw.device.hash2"] {
        config.remove(key);
    }
    for (key, value) in [
        ("hw.lcd.width", profile.width.to_string()),
        ("hw.lcd.height", profile.height.to_string()),
        ("hw.lcd.density", profile.dpi.to_string()),
        ("hw.ramSize", device.hardware.ram_mib.to_string()),
        ("hw.cpu.ncore", device.hardware.cpu_count.to_string()),
        (
            "disk.dataPartition.size",
            format!("{}G", device.hardware.data_gib),
        ),
        ("hw.device.name", profile.id.clone()),
    ] {
        config.insert(key.into(), value);
    }
    write_ini(&path.join("config.ini"), &config)
}

pub fn apply_stopped(directory: &Directory, device: &Device) -> Result<(), String> {
    if [
        JOURNAL,
        "installation.json",
        "toolchain-installation.json",
        "package-removal.json",
    ]
    .iter()
    .any(|name| directory.root.join(name).exists())
    {
        return Err("Recover the interrupted device operation before Start".into());
    }
    let selected = profile(directory, device)?;
    let path = directory.avd_path(&device.id)?;
    marker(&path, &device.id)?;
    configure(&path, device, &selected)
}

fn write_ini(path: &Path, fields: &BTreeMap<String, String>) -> Result<(), String> {
    if fields
        .iter()
        .any(|(key, value)| key.contains(['\n', '\r', '=']) || value.contains(['\n', '\r']))
    {
        return Err("Invalid Android configuration value".into());
    }
    let parent = path.parent().ok_or("Missing AVD parent")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    for (key, value) in fields {
        writeln!(temporary, "{key}={value}").map_err(|e| e.to_string())?;
    }
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary.persist(path).map_err(|e| e.to_string())?;
    installation::sync_directory(parent)
}

fn marker(path: &Path, device: &str) -> Result<Marker, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("AVD is not an owned directory".into());
    }
    let marker: Marker = storage::read(&path.join(MARKER))?
        .ok_or("AVD ownership is missing; preserve it for recovery")?;
    if marker.version != 1 || marker.device_id != device || !storage::valid_id(&marker.operation) {
        return Err("AVD identity differs from the selected device".into());
    }
    Ok(marker)
}

fn finish(directory: &mut Directory, journal: &Journal) -> Result<(), String> {
    finish_inner(directory, journal, |_| Ok(()))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    PreviousMoved,
    CandidateMoved,
    Devices,
    Preferences,
    Cleanup,
}

fn finish_inner(
    directory: &mut Directory,
    journal: &Journal,
    checkpoint: impl Fn(Step) -> Result<(), String>,
) -> Result<(), String> {
    journal.validate()?;
    let current = directory.devices()?;
    let preferences = directory.preferences()?;
    if current != journal.before && current != journal.after {
        return Err(
            "Android devices changed after this interrupted operation; files were preserved".into(),
        );
    }
    if preferences != journal.preferences_before && preferences != journal.preferences_after {
        return Err(
            "Android preferences changed after this interrupted operation; files were preserved"
                .into(),
        );
    }
    let root = &directory.root;
    let stage =
        installation::checked_path(root, &PathBuf::from("staging").join(&journal.operation))?;
    let active = installation::checked_path(
        root,
        &PathBuf::from("avd").join(format!("sb_{}.avd", journal.device_id)),
    )?;
    let candidate = stage.join("candidate");
    let previous = stage.join("previous");
    if active.exists() {
        let active_marker = marker(&active, &journal.device_id)?;
        if active_marker.operation != journal.operation {
            if !journal
                .before
                .devices
                .iter()
                .any(|device| device.id == journal.device_id)
            {
                return Err("New Android device path is already occupied".into());
            }
            installation::move_directory(&active, &previous)?;
        }
    }
    checkpoint(Step::PreviousMoved)?;
    if journal.replacement {
        if !active.exists() {
            let identity = marker(&candidate, &journal.device_id)?;
            if identity.operation != journal.operation {
                return Err("Android candidate belongs to another operation".into());
            }
            installation::create_directories(root, active.parent().ok_or("Missing AVD parent")?)?;
            installation::move_directory(&candidate, &active)?;
        }
        let device = journal
            .after
            .devices
            .iter()
            .find(|device| device.id == journal.device_id)
            .ok_or("Missing replacement device metadata")?;
        let target = device.image.split(';').nth(1).ok_or("Missing image API")?;
        write_ini(
            &root
                .join("avd")
                .join(format!("sb_{}.ini", journal.device_id)),
            &BTreeMap::from([
                ("avd.ini.encoding".into(), "UTF-8".into()),
                (
                    "path".into(),
                    active.to_str().ok_or("AVD path is not Unicode")?.into(),
                ),
                ("target".into(), target.into()),
            ]),
        )?;
    } else {
        let locator = root
            .join("avd")
            .join(format!("sb_{}.ini", journal.device_id));
        if locator.try_exists().map_err(|e| e.to_string())? {
            fs::remove_file(locator).map_err(|e| e.to_string())?;
        }
    }
    checkpoint(Step::CandidateMoved)?;
    if current == journal.before {
        let mut after = journal.after.clone();
        after.revision = journal.before.revision;
        directory.save_devices(after, journal.before.revision)?;
    }
    checkpoint(Step::Devices)?;
    if preferences != journal.preferences_after {
        let mut after = journal.preferences_after.clone();
        after.revision = journal.preferences_before.revision;
        directory.save_preferences(after, journal.preferences_before.revision)?;
    }
    checkpoint(Step::Preferences)?;
    if stage.exists() {
        fs::remove_dir_all(stage).map_err(|e| e.to_string())?;
    }
    checkpoint(Step::Cleanup)?;
    fs::remove_file(directory.root.join(JOURNAL)).map_err(|e| e.to_string())?;
    installation::sync_directory(&directory.root)
}

pub fn recover(directory: &mut Directory) -> Result<bool, String> {
    let Some(journal): Option<Journal> = storage::read(&directory.root.join(JOURNAL))? else {
        return Ok(false);
    };
    runtime::check_previous(
        &directory
            .root
            .join("runtime")
            .join(format!("{}.json", journal.device_id)),
        &journal.device_id,
    )?;
    finish(directory, &journal)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_images_accept_decimal_api_levels_and_reject_conflicting_metadata() {
        let properties = |api: &str, minor: Option<&str>| {
            let mut values = BTreeMap::from([("AndroidVersion.ApiLevel".into(), api.into())]);
            if let Some(minor) = minor {
                values.insert("AndroidVersion.ApiMinorLevel".into(), minor.into());
            }
            values
        };
        for (api, minor, expected) in [
            ("36", None, (36, 0)),
            ("37.2", None, (37, 2)),
            ("37.10", None, (37, 10)),
            ("37", Some("2"), (37, 2)),
            ("37.2", Some("2"), (37, 2)),
        ] {
            assert_eq!(installed_api(&properties(api, minor)).unwrap(), expected);
        }
        for (api, minor) in [
            ("37.2", Some("1")),
            ("37.0", Some("2")),
            ("37", Some("preview")),
            ("37.2.1", None),
            ("37x", None),
            ("Baklava", None),
            ("4294967296", None),
            ("25", None),
            ("", None),
        ] {
            assert!(installed_api(&properties(api, minor)).is_err());
        }
        assert!(installed_api(&BTreeMap::new()).is_err());
    }

    fn fixture_device() -> Device {
        Device {
            id: auth::new_id().unwrap(),
            name: "Test phone".into(),
            image: "system-images;android-36;default;arm64-v8a".into(),
            image_revision: 2,
            profile: "small_phone".into(),
            hardware: Hardware {
                ram_mib: 2560,
                cpu_count: 2,
                data_gib: 6,
                gpu: storage::Gpu::Host,
                quick_boot: false,
            },
            input_bridge: true,
        }
    }

    #[test]
    fn interrupted_wipe_and_delete_recover_without_touching_other_devices() {
        for replacement in [true, false] {
            for interrupted in [
                Step::PreviousMoved,
                Step::CandidateMoved,
                Step::Devices,
                Step::Preferences,
                Step::Cleanup,
            ] {
                let temporary = tempfile::tempdir().unwrap();
                let mut directory = Directory::acquire(temporary.path().to_path_buf()).unwrap();
                let device = fixture_device();
                let mut other = fixture_device();
                other.name = "Other".into();
                let before = directory
                    .save_devices(
                        Devices {
                            devices: vec![device.clone(), other.clone()],
                            ..Devices::default()
                        },
                        0,
                    )
                    .unwrap();
                let preferences_before = directory
                    .save_preferences(
                        Preferences {
                            default_device_id: Some(device.id.clone()),
                            ..Preferences::default()
                        },
                        0,
                    )
                    .unwrap();
                for phone in [&device, &other] {
                    let path = directory.avd_path(&phone.id).unwrap();
                    fs::create_dir_all(&path).unwrap();
                    fs::write(path.join("userdata"), b"preserved").unwrap();
                    storage::write(
                        &path.join(MARKER),
                        &Marker {
                            version: 1,
                            device_id: phone.id.clone(),
                            operation: auth::new_id().unwrap(),
                        },
                    )
                    .unwrap();
                }
                let operation = auth::new_id().unwrap();
                let stage = directory.root.join("staging").join(&operation);
                fs::create_dir_all(&stage).unwrap();
                if replacement {
                    let candidate = stage.join("candidate");
                    fs::create_dir(&candidate).unwrap();
                    fs::write(candidate.join("userdata"), b"fresh").unwrap();
                    storage::write(
                        &candidate.join(MARKER),
                        &Marker {
                            version: 1,
                            device_id: device.id.clone(),
                            operation: operation.clone(),
                        },
                    )
                    .unwrap();
                }
                let mut after = before.clone();
                after.revision += 1;
                let mut preferences_after = preferences_before.clone();
                if !replacement {
                    after.devices.retain(|phone| phone.id != device.id);
                    preferences_after.default_device_id = None;
                    preferences_after.revision += 1;
                }
                let journal = Journal {
                    version: 1,
                    operation,
                    device_id: device.id.clone(),
                    before,
                    after,
                    preferences_before,
                    preferences_after,
                    replacement,
                };
                storage::write(&directory.root.join(JOURNAL), &journal).unwrap();
                assert!(
                    finish_inner(&mut directory, &journal, |step| if step == interrupted {
                        Err("interrupted".into())
                    } else {
                        Ok(())
                    })
                    .is_err()
                );
                recover(&mut directory).unwrap();
                assert_eq!(directory.devices().unwrap(), journal.after);
                assert_eq!(directory.preferences().unwrap(), journal.preferences_after);
                assert_eq!(
                    fs::read(directory.avd_path(&other.id).unwrap().join("userdata")).unwrap(),
                    b"preserved"
                );
                if replacement {
                    assert_eq!(
                        fs::read(directory.avd_path(&device.id).unwrap().join("userdata")).unwrap(),
                        b"fresh"
                    );
                } else {
                    assert!(!directory.avd_path(&device.id).unwrap().exists());
                }
                assert!(!stage.exists());
            }
        }
    }

    #[tokio::test]
    #[ignore = "Creates, boots, updates, wipes and deletes a real phone in the isolated Rust-installed SDK"]
    async fn native_managed_phone_creation_and_maintenance() {
        let trial = PathBuf::from(std::env::var_os("LOMI_ANDROID_PROBE_DIRECTORY").unwrap());
        let installation: serde_json::Value = serde_json::from_slice(
            &fs::read(trial.join("evidence/native-managed-installation.json")).unwrap(),
        )
        .unwrap();
        let root = PathBuf::from(installation["root"].as_str().unwrap());
        assert!(root.starts_with(&trial));
        let android = super::super::manager::Android::default();
        let manager = android.get(root.clone()).unwrap();
        manager
            .test_adb_port
            .store(15047, std::sync::atomic::Ordering::Release);
        assert!(std::net::TcpListener::bind(("127.0.0.1", 15047)).is_ok());
        let revision = manager
            .directory
            .lock()
            .unwrap()
            .devices()
            .unwrap()
            .revision;
        let result: Result<serde_json::Value, String> = async {
            let selection = fixture_device();
            let started = std::time::Instant::now();
            manager.installer.manage(&manager, Action::Create { expected_revision: revision, draft: Draft { name: selection.name, image: selection.image, profile: selection.profile, hardware: selection.hardware } })?;
            manager.installer.settle(false).await?;
            let created = manager.installer.progress().ok_or("Missing creation result")?;
            if created.phase != super::super::installer::Phase::Succeeded { return Err(format!("Device creation failed: {created:?}")); }
            let id = created.device_id.ok_or("Missing created device ID")?;
            let creation_seconds = started.elapsed().as_secs_f64();
            let started = std::time::Instant::now();
            let (one, two) = tokio::join!(manager.start(&id), manager.start(&id));
            let (one, two) = (one?, two?);
            if one.generation != two.generation || one.phase != runtime::Phase::Running { return Err("Concurrent starts did not join the real phone".into()); }
            let boot_seconds = started.elapsed().as_secs_f64();
            let runtime = manager.runtime(&id, one.generation.as_deref().unwrap())?;
            let connection = runtime.connection().await?;
            connection.client().get_status(connection.request("getStatus", ())?).await.map_err(|e| e.to_string())?;
            let before = manager.directory.lock().unwrap().devices()?;
            let mut changed = before.devices.iter().find(|device| device.id == id).unwrap().clone();
            changed.name = "Renamed native phone".into(); changed.hardware.cpu_count = 3;
            let config_path = root.join("avd").join(format!("sb_{id}.avd/config.ini"));
            let config = fs::read(&config_path).map_err(|e| e.to_string())?;
            manager.installer.manage(&manager, Action::Update { expected_revision: before.revision, device: changed.clone() })?;
            manager.installer.settle(false).await?;
            if manager.installer.progress().unwrap().phase != super::super::installer::Phase::Succeeded { return Err(format!("Device update failed: {:?}", manager.installer.progress())); }
            if fs::read(&config_path).map_err(|e| e.to_string())? != config { return Err("Hardware edit rewrote a live AVD".into()); }
            if manager.start(&id).await?.generation != one.generation { return Err("Metadata edit restarted a live phone".into()); }
            manager.stop(&id, false).await?;
            let restarted = manager.start(&id).await?;
            if restarted.generation == one.generation { return Err("Stopped phone reused its old generation".into()); }
            if avd::read_ini(&config_path)?.get("hw.cpu.ncore").map(String::as_str) != Some("3") { return Err("Pending hardware was not applied after Stop".into()); }
            manager.stop(&id, false).await?;
            let device_path = root.join("avd").join(format!("sb_{id}.avd"));
            fs::write(device_path.join("native-wipe-marker"), b"old guest data").map_err(|e| e.to_string())?;
            for wipe in [true, false] {
                let expected_revision = manager.directory.lock().unwrap().devices()?.revision;
                let action = if wipe { Action::Wipe { expected_revision, device_id: id.clone(), confirmation: changed.name.clone() } }
                    else { Action::Delete { expected_revision, device_id: id.clone(), confirmation: changed.name.clone() } };
                manager.installer.manage(&manager, action)?;
                manager.installer.settle(false).await?;
                if manager.installer.progress().unwrap().phase != super::super::installer::Phase::Succeeded { return Err(format!("Maintenance failed: {:?}", manager.installer.progress())); }
                if device_path.join("native-wipe-marker").exists() { return Err("Device data survived explicit wipe/delete".into()); }
                if wipe { manager.start(&id).await?; manager.stop(&id, false).await?; }
            }
            if device_path.exists() { return Err("Deleted AVD remains".into()); }
            Ok(serde_json::json!({"creationSeconds":creation_seconds,"bootSeconds":boot_seconds,"coalescedStarts":true,"nativeGrpc":true,"deferredHardwareChange":true,"wipeRecreatedAndBooted":true,"deleteRemovedOwnedAvd":true,"privateAdbPort":15047}))
        }.await;
        for status in manager.statuses().unwrap() {
            if status.process_alive {
                let _ = manager.stop(&status.device_id, true).await;
            }
        }
        manager.stop_private_adb_fixture().unwrap();
        let report = result.unwrap();
        fs::write(
            trial.join("evidence/native-managed-device.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        println!("{report}");
    }
}
