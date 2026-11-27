//! Onboarding completion and the chosen work mode. They used to live only in
//! webview storage, which can be lost (WebView2 on Windows), and onboarding then
//! reappeared on every launch; this file is the durable copy.
use serde::{Deserialize, Serialize};
use std::{fs, io::Read, path::Path, sync::Mutex};
use tauri::{Manager, Window};

const FILE: &str = "onboarding.json";
const LIMIT: u64 = 4 * 1024;
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OnboardingState {
    #[serde(default)]
    done: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    work_mode: Option<String>,
}

fn validate(state: &OnboardingState) -> Result<(), String> {
    match state.work_mode.as_deref() {
        None | Some("cli" | "chat") => Ok(()),
        Some(_) => Err("Unknown work mode.".into()),
    }
}

/// A missing, oversized or invalid file reads as "not done", like a first launch.
fn read(path: &Path) -> OnboardingState {
    let mut bytes = Vec::new();
    let parsed = fs::File::open(path)
        .and_then(|file| file.take(LIMIT + 1).read_to_end(&mut bytes))
        .ok()
        .filter(|&n| n as u64 <= LIMIT)
        .and_then(|_| serde_json::from_slice::<OnboardingState>(&bytes).ok());
    parsed
        .filter(|state| validate(state).is_ok())
        .unwrap_or_default()
}

fn main_only(window: &Window) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("Onboarding state is only available in the main window.".into())
    }
}

#[tauri::command]
pub fn load_onboarding_state(
    window: Window,
    app: tauri::AppHandle,
) -> Result<OnboardingState, String> {
    main_only(&window)?;
    let _guard = LOCK.lock().map_err(|error| error.to_string())?;
    let directory = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(read(&directory.join(FILE)))
}

#[tauri::command]
pub fn save_onboarding_state(
    window: Window,
    app: tauri::AppHandle,
    state: OnboardingState,
) -> Result<(), String> {
    main_only(&window)?;
    validate(&state)?;
    let _guard = LOCK.lock().map_err(|error| error.to_string())?;
    let directory = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    crate::files::write_json(&directory.join(FILE), &state, LIMIT as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_saved_state_and_treats_bad_files_as_first_launch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        assert_eq!(read(&path), OnboardingState::default());
        let saved = OnboardingState {
            done: true,
            work_mode: Some("chat".into()),
        };
        crate::files::write_json(&path, &saved, LIMIT as usize).unwrap();
        assert_eq!(read(&path), saved);
        for bad in [
            r#"{"done":true,"workMode":"other"}"#,
            r#"{"done":"yes"}"#,
            "not json",
        ] {
            fs::write(&path, bad).unwrap();
            assert_eq!(read(&path), OnboardingState::default(), "{bad}");
        }
    }
}
