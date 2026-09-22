use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const LIMIT: u64 = 1024 * 1024;
const MAX_REVISION: u64 = (1 << 53) - 1;

/// Held for the entire lifetime of SDK management and every owned process.
pub struct Directory {
    pub root: PathBuf,
    _lock: fs::File,
}

impl Drop for Directory {
    fn drop(&mut self) {
        // Closing alone can leave a Unix flock alive in a concurrent fork until exec.
        // The owner releases explicitly after every managed operation has settled.
        let _ = self._lock.unlock();
    }
}

impl Directory {
    pub fn acquire(root: PathBuf) -> Result<Self, String> {
        if root
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
        {
            return Err("The managed Android directory must not be a symbolic link.".into());
        }
        fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        let path = root.join("owner.lock");
        reject_link(&path)?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| e.to_string())?;
        lock.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => "Another Lomi process owns this Android directory. Close its Android session and retry.".into(),
            fs::TryLockError::Error(error) => format!("Cannot lock the Android directory: {error}"),
        })?;
        Ok(Self { root, _lock: lock })
    }

    pub fn preferences(&self) -> Result<Preferences, String> {
        let data: Preferences = read(&self.root.join("preferences.json"))?.unwrap_or_default();
        data.validate()?;
        Ok(data)
    }

    pub fn devices(&self) -> Result<Devices, String> {
        let data: Devices = read(&self.root.join("devices.json"))?.unwrap_or_default();
        data.validate()?;
        Ok(data)
    }

    pub fn save_preferences(
        &mut self,
        mut data: Preferences,
        expected: u64,
    ) -> Result<Preferences, String> {
        if self.preferences()?.revision != expected || data.revision != expected {
            return Err("Android preferences changed in another view. Reload and retry.".into());
        }
        data.validate()?;
        if let Some(id) = &data.default_device_id {
            if !self
                .devices()?
                .devices
                .iter()
                .any(|device| &device.id == id)
            {
                return Err("The default Android device no longer exists.".into());
            }
        }
        data.revision = next_revision(expected)?;
        write(&self.root.join("preferences.json"), &data)?;
        Ok(data)
    }

    pub fn save_devices(&mut self, mut data: Devices, expected: u64) -> Result<Devices, String> {
        if self.devices()?.revision != expected || data.revision != expected {
            return Err("Android devices changed in another view. Reload and retry.".into());
        }
        data.validate()?;
        data.revision = next_revision(expected)?;
        write(&self.root.join("devices.json"), &data)?;
        Ok(data)
    }

    pub fn avd_path(&self, id: &str) -> Result<PathBuf, String> {
        if !valid_id(id) {
            return Err("Invalid Android device ID".into());
        }
        Ok(self.root.join("avd").join(format!("sb_{id}.avd")))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub revision: u64,
    pub default_device_id: Option<String>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            revision: 0,
            default_device_id: None,
        }
    }
}
impl Preferences {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || self.revision > MAX_REVISION
            || self
                .default_device_id
                .as_ref()
                .is_some_and(|id| !valid_id(id))
        {
            return Err("Unsupported or invalid Android preferences; the file was preserved. Open recovery in Android settings.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Devices {
    pub version: u32,
    pub revision: u64,
    pub devices: Vec<Device>,
}
impl Default for Devices {
    fn default() -> Self {
        Self {
            version: 1,
            revision: 0,
            devices: Vec::new(),
        }
    }
}
impl Devices {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.version != 1 || self.revision > MAX_REVISION {
            return Err("Unsupported Android devices schema; the file was preserved.".into());
        }
        let mut ids = HashSet::new();
        for device in &self.devices {
            device.validate()?;
            if !ids.insert(&device.id) {
                return Err("Duplicate Android device ID; the file was preserved.".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub image: String,
    pub image_revision: u32,
    pub profile: String,
    pub hardware: Hardware,
    pub input_bridge: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Hardware {
    pub ram_mib: u32,
    pub cpu_count: u32,
    pub data_gib: u32,
    pub gpu: Gpu,
    pub quick_boot: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Gpu {
    Auto,
    Host,
    Software,
}

impl Device {
    pub fn validate(&self) -> Result<(), String> {
        let parts: Vec<_> = self.image.split(';').collect();
        let valid_image = parts.len() == 4
            && parts[0] == "system-images"
            && parts[1].strip_prefix("android-").is_some_and(|api| {
                component(api)
                    && api.split(['.', '-']).next().is_some_and(|major| {
                        major.parse::<u32>().is_ok_and(|n| (26..=999).contains(&n))
                    })
            })
            && component(parts[2])
            && matches!(parts[3], "arm64-v8a" | "x86_64");
        if !valid_id(&self.id)
            || self.name.trim() != self.name
            || self.name.is_empty()
            || self.name.chars().count() > 80
            || self.name.chars().any(char::is_control)
            || !valid_image
            || self.image_revision == 0
            || self.profile.is_empty()
            || self.profile.len() > 160
            || self.profile.starts_with('-')
            || self.profile.chars().any(char::is_control)
            || !(512..=32768).contains(&self.hardware.ram_mib)
            || !(1..=32).contains(&self.hardware.cpu_count)
            || !(2..=128).contains(&self.hardware.data_gib)
        {
            return Err("Invalid Android device configuration; no files were changed.".into());
        }
        Ok(())
    }
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}
fn component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value != "."
        && value != ".."
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
fn next_revision(revision: u64) -> Result<u64, String> {
    revision
        .checked_add(1)
        .filter(|value| *value <= MAX_REVISION)
        .ok_or_else(|| "Android metadata revision limit reached.".into())
}
fn reject_link(path: &Path) -> Result<(), String> {
    if path
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink() || !m.is_file())
    {
        return Err(format!(
            "{} must be a regular managed file.",
            path.display()
        ));
    }
    Ok(())
}
pub(super) fn read<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    reject_link(path)?;
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("Android metadata exceeds 1 MiB; the file was preserved.".into());
    }
    serde_json::from_slice(&bytes).map(Some).map_err(|e| {
        format!(
            "Cannot read {}: {e}. The file was preserved; use Android settings recovery.",
            path.display()
        )
    })
}
pub(super) fn write(path: &Path, value: &impl Serialize) -> Result<(), String> {
    reject_link(path)?;
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("Android metadata exceeds 1 MiB.".into());
    }
    if let Some(name @ ("preferences.json" | "devices.json")) =
        path.file_name().and_then(|s| s.to_str())
    {
        if let Some(previous) = read::<serde_json::Value>(path)? {
            let valid = if name == "preferences.json" {
                serde_json::from_value::<Preferences>(previous.clone())
                    .map_err(|e| e.to_string())
                    .and_then(|data| data.validate())
            } else {
                serde_json::from_value::<Devices>(previous.clone())
                    .map_err(|e| e.to_string())
                    .and_then(|data| data.validate())
            };
            if valid.is_ok()
                && previous
                    != serde_json::from_slice::<serde_json::Value>(&bytes)
                        .map_err(|e| e.to_string())?
            {
                write(&path.with_extension("previous.json"), &previous)?;
            }
        }
    }
    write_bytes(path, &bytes)
}

pub(super) fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    reject_link(path)?;
    let parent = path.parent().ok_or("Missing metadata directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temporary.write_all(bytes).map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary.persist(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn owner_release_is_not_delayed_by_an_inherited_file_description() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("android");
        let owner = Directory::acquire(root.clone()).unwrap();
        let inherited = owner._lock.try_clone().unwrap();
        drop(owner);
        let next = Directory::acquire(root.clone()).unwrap();
        drop(inherited);
        assert!(Directory::acquire(root.clone()).is_err());
        drop(next);
        assert!(Directory::acquire(root).is_ok());
    }

    #[test]
    fn directory_lock_and_corrupt_metadata_preserve_ownership() {
        let temp = tempfile::tempdir().unwrap();
        let mut directory = Directory::acquire(temp.path().join("android")).unwrap();
        assert!(Directory::acquire(directory.root.clone()).is_err());
        let path = directory.root.join("devices.json");
        fs::write(&path, b"{broken").unwrap();
        assert!(directory.save_devices(Devices::default(), 0).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{broken");
        let root = directory.root.clone();
        drop(directory);
        assert!(Directory::acquire(root).is_ok());
    }

    #[test]
    fn stale_writes_and_missing_default_do_not_replace_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let mut directory = Directory::acquire(temp.path().join("android")).unwrap();
        let prefs = directory
            .save_preferences(Preferences::default(), 0)
            .unwrap();
        assert_eq!(prefs.revision, 1);
        assert!(directory
            .save_preferences(Preferences::default(), 0)
            .is_err());
        let before = fs::read(directory.root.join("preferences.json")).unwrap();
        let invalid = Preferences {
            default_device_id: Some("00000000-0000-0000-0000-000000000001".into()),
            ..prefs
        };
        assert!(directory.save_preferences(invalid, 1).is_err());
        assert_eq!(
            fs::read(directory.root.join("preferences.json")).unwrap(),
            before
        );
        assert!(directory.avd_path("../another-sdk").is_err());
    }

    #[test]
    fn directory_lock_is_shared_between_processes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("android");
        let directory = Directory::acquire(root.clone()).unwrap();
        let child = |available: bool| {
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "android::storage::tests::directory_lock_child",
                    "--ignored",
                ])
                .env("LOMI_LOCK_TEST_DIRECTORY", &root)
                .env(
                    "LOMI_LOCK_TEST_AVAILABLE",
                    if available { "1" } else { "0" },
                )
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stdout)
            );
        };
        child(false);
        drop(directory);
        child(true);
    }

    #[test]
    #[ignore = "Subprocess fixture for directory_lock_is_shared_between_processes"]
    fn directory_lock_child() {
        let path = std::env::var_os("LOMI_LOCK_TEST_DIRECTORY").expect("Missing test directory");
        let available = std::env::var("LOMI_LOCK_TEST_AVAILABLE").unwrap() == "1";
        assert_eq!(Directory::acquire(path.into()).is_ok(), available);
    }

    #[test]
    fn unknown_fields_and_unsafe_revisions_require_explicit_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let mut directory = Directory::acquire(temp.path().join("android")).unwrap();
        let path = directory.root.join("preferences.json");
        for value in [
            serde_json::json!({"version":1,"revision":0,"defaultDeviceId":null,"future":true}),
            serde_json::json!({"version":1,"revision":MAX_REVISION+1,"defaultDeviceId":null}),
        ] {
            let bytes = serde_json::to_vec(&value).unwrap();
            fs::write(&path, &bytes).unwrap();
            assert!(directory.preferences().is_err());
            assert!(directory
                .save_preferences(Preferences::default(), 0)
                .is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }
}
