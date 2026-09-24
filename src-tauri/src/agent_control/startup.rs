use serde::{Deserialize, Serialize};
use std::{fs, io::Read, path::Path};

const LIMIT: u64 = 4 * 1024;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Preferences {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auto_start: Option<bool>,
    #[serde(default)]
    yolo_mode: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlStartupState {
    pub supported: bool,
    pub auto_start: Option<bool>,
    pub yolo_mode: bool,
    pub error: Option<String>,
}

fn read(path: &Path) -> Result<Preferences, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "Cannot read Agent control preferences ({error}). The file was left intact."
            ))
        }
    };
    let Some(metadata) = metadata else {
        return Ok(Preferences {
            version: 1,
            auto_start: None,
            yolo_mode: false,
        });
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(
            "Agent control preferences are not a regular file. The file was left intact.".into(),
        );
    }
    let file = fs::File::open(path).map_err(|error| {
        format!("Cannot read Agent control preferences ({error}). The file was left intact.")
    })?;
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            format!("Cannot read Agent control preferences ({error}). The file was left intact.")
        })?;
    if bytes.len() as u64 > LIMIT {
        return Err("Agent control preferences exceed 4 KiB. The file was left intact.".into());
    }
    let preferences: Preferences = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "Agent control preferences are corrupt or unsupported ({error}). The file was left intact."
        )
    })?;
    if preferences.version != 1 {
        return Err(
            "Agent control preferences use an unsupported version. The file was left intact."
                .into(),
        );
    }
    Ok(preferences)
}

pub(super) fn save(path: &Path, auto_start: Option<bool>, yolo_mode: bool) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Invalid Agent control preferences path.".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    crate::files::write_json(
        path,
        &Preferences {
            version: 1,
            auto_start,
            yolo_mode,
        },
        LIMIT as usize,
    )
}

#[derive(Default)]
pub(super) struct Runtime {
    loaded: bool,
    auto_start: Option<bool>,
    yolo_mode: bool,
    launch_auto_start: Option<Option<bool>>,
    blocked: bool,
    load_error: Option<String>,
    startup_error: Option<String>,
    initialized: bool,
}

impl Runtime {
    pub(super) fn load(&mut self, path: &Path) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        match read(path) {
            Ok(auto_start) => {
                self.auto_start = auto_start.auto_start;
                self.yolo_mode = auto_start.yolo_mode;
                self.launch_auto_start = Some(auto_start.auto_start);
            }
            Err(error) => {
                self.blocked = true;
                self.load_error = Some(error);
                self.launch_auto_start = Some(None);
            }
        }
    }

    pub(super) fn fail_load(&mut self, error: String) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        self.blocked = true;
        self.load_error = Some(error);
        self.launch_auto_start = Some(None);
    }

    pub(super) fn can_record_choice(&self) -> bool {
        self.loaded && !self.blocked && self.auto_start.is_none()
    }

    pub(super) fn can_save(&self) -> bool {
        self.loaded && !self.blocked
    }

    pub(super) fn auto_start(&self) -> Option<bool> {
        self.auto_start
    }

    pub(super) fn yolo_mode(&self) -> bool {
        self.yolo_mode
    }

    pub(super) fn record_saved_choice(&mut self, enabled: bool) {
        self.auto_start = Some(enabled);
    }

    pub(super) fn record_yolo_mode(&mut self, enabled: bool) {
        self.yolo_mode = enabled;
    }

    pub(super) fn begin_initialization(&mut self, supported: bool) -> bool {
        if self.initialized {
            return false;
        }
        self.initialized = true;
        supported && self.launch_auto_start == Some(Some(true))
    }

    pub(super) fn suppress_initialization(&mut self) {
        self.initialized = true;
    }

    pub(super) fn set_startup_error(&mut self, error: Option<String>) {
        self.startup_error = error;
    }

    pub(super) fn state(&self, supported: bool) -> ControlStartupState {
        ControlStartupState {
            supported,
            auto_start: self.auto_start,
            yolo_mode: self.yolo_mode,
            error: self
                .load_error
                .clone()
                .or_else(|| self.startup_error.clone()),
        }
    }
}

pub(super) fn check_enable_supported(enabled: bool, supported: bool) -> Result<(), String> {
    if enabled && !supported {
        Err("This host has not been qualified for Agent control.".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_preferences_are_unknown() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agent-control-preferences.json");
        let preferences = read(&path).unwrap();
        assert_eq!(preferences.auto_start, None);
        assert!(!preferences.yolo_mode);
        let mut runtime = Runtime::default();
        runtime.load(&path);
        assert!(!runtime.state(true).yolo_mode);
    }

    #[test]
    fn changing_yolo_mode_preserves_auto_start_including_unknown() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agent-control-preferences.json");
        for auto_start in [None, Some(false), Some(true)] {
            save(&path, auto_start, false).unwrap();
            let mut runtime = Runtime::default();
            runtime.load(&path);
            save(&path, runtime.auto_start(), true).unwrap();
            let preferences = read(&path).unwrap();
            assert_eq!(preferences.auto_start, auto_start);
            assert!(preferences.yolo_mode);
        }
    }

    #[test]
    fn changing_auto_start_preserves_yolo_mode() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agent-control-preferences.json");
        for yolo_mode in [false, true] {
            save(&path, None, yolo_mode).unwrap();
            let mut runtime = Runtime::default();
            runtime.load(&path);
            save(&path, Some(true), runtime.yolo_mode()).unwrap();
            let preferences = read(&path).unwrap();
            assert_eq!(preferences.auto_start, Some(true));
            assert_eq!(preferences.yolo_mode, yolo_mode);
        }
    }

    #[test]
    fn legacy_preferences_without_yolo_mode_load_disabled() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agent-control-preferences.json");
        fs::write(&path, r#"{"version":1,"autoStart":true}"#).unwrap();
        let preferences = read(&path).unwrap();
        assert_eq!(preferences.auto_start, Some(true));
        assert!(!preferences.yolo_mode);
    }

    #[test]
    fn saved_preferences_round_trip_all_auto_start_and_yolo_values() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agent-control-preferences.json");
        for auto_start in [None, Some(false), Some(true)] {
            for yolo_mode in [false, true] {
                save(&path, auto_start, yolo_mode).unwrap();
                let preferences = read(&path).unwrap();
                assert_eq!(preferences.auto_start, auto_start);
                assert_eq!(preferences.yolo_mode, yolo_mode);
            }
        }
    }

    #[test]
    fn invalid_preferences_are_preserved_and_block_writes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agent-control-preferences.json");
        let oversized = " ".repeat(LIMIT as usize + 1);
        for bytes in [
            "{broken".to_string(),
            r#"{"version":99,"autoStart":true}"#.to_string(),
            oversized,
        ] {
            fs::write(&path, &bytes).unwrap();
            let mut runtime = Runtime::default();
            runtime.load(&path);
            assert!(runtime.state(true).error.is_some());
            assert!(!runtime.can_record_choice());
            assert!(!runtime.can_save());
            assert!(read(&path).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn startup_is_one_shot_and_manual_disable_suppresses_it() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agent-control-preferences.json");
        save(&path, Some(true), false).unwrap();
        let mut runtime = Runtime::default();
        runtime.load(&path);
        assert!(runtime.begin_initialization(true));
        assert!(!runtime.begin_initialization(true));

        let mut manually_disabled = Runtime::default();
        manually_disabled.load(&path);
        manually_disabled.suppress_initialization();
        assert!(!manually_disabled.begin_initialization(true));
    }

    #[test]
    fn saved_choices_apply_on_the_next_launch_only() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agent-control-preferences.json");

        save(&path, Some(false), false).unwrap();
        let mut current_launch = Runtime::default();
        current_launch.load(&path);
        assert!(!current_launch.begin_initialization(true));

        save(&path, Some(true), false).unwrap();
        current_launch.record_saved_choice(true);
        assert!(!current_launch.begin_initialization(true));
        assert_eq!(current_launch.state(true).auto_start, Some(true));

        let mut next_launch = Runtime::default();
        next_launch.load(&path);
        assert!(next_launch.begin_initialization(true));
    }

    #[test]
    fn first_choice_after_launch_initialization_is_future_only() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agent-control-preferences.json");
        let mut current_launch = Runtime::default();
        current_launch.load(&path);

        assert!(!current_launch.begin_initialization(true));
        save(&path, Some(true), false).unwrap();
        current_launch.record_saved_choice(true);
        assert!(!current_launch.begin_initialization(true));

        let mut next_launch = Runtime::default();
        next_launch.load(&path);
        assert!(next_launch.begin_initialization(true));
    }

    #[test]
    fn failed_save_leaves_the_first_choice_available_for_retry() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("preferences");
        fs::create_dir(&parent).unwrap();
        let path = parent.join("agent-control-preferences.json");
        let mut runtime = Runtime::default();
        runtime.load(&path);

        fs::remove_dir(&parent).unwrap();
        fs::write(&parent, "not a directory").unwrap();
        assert!(save(&path, Some(true), false).is_err());
        assert_eq!(runtime.auto_start(), None);
        assert!(runtime.can_record_choice());

        fs::remove_file(&parent).unwrap();
        fs::create_dir(&parent).unwrap();
        save(&path, Some(true), false).unwrap();
        runtime.record_saved_choice(true);
        assert_eq!(runtime.auto_start(), Some(true));
        assert!(!runtime.can_record_choice());
    }

    #[test]
    fn startup_consent_is_first_choice_only_and_unsupported_enable_is_denied() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agent-control-preferences.json");
        let mut runtime = Runtime::default();
        runtime.load(&path);
        assert!(runtime.can_record_choice());
        assert!(check_enable_supported(true, false).is_err());
        assert!(check_enable_supported(false, false).is_ok());
        save(&path, Some(true), false).unwrap();
        runtime.record_saved_choice(true);
        assert!(!runtime.can_record_choice());
        assert!(!runtime.begin_initialization(false));
        assert_eq!(read(&path).unwrap().auto_start, Some(true));
    }
}
