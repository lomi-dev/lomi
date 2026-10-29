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
        .env("ANDROID_AVD_HOME", environment::external_path(&locators))
        .arg("-Djava.awt.headless=true")
        .arg(format!(
            "-Dcom.android.sdkmanager.toolsdir={}",
            environment::external_path(&sdk_tools).display()
        ))
        .arg("-classpath")
        .arg(environment::external_path(
            &sdk_tools.join("lib/avdmanager-classpath.jar"),
        ))
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
        .arg(environment::external_path(&candidate));
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
                    environment::external_path(&active)
                        .to_str()
                        .ok_or("AVD path is not Unicode")?
                        .into(),
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
