use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io::Read, path::Path, sync::Mutex};
use tauri::{Emitter, Manager, State, Window};

const LIMIT: u64 = 256 * 1024;

pub(crate) fn valid_shortcut(value: &str) -> bool {
    let mut parts: Vec<_> = value.split('+').collect();
    let code = parts.pop().unwrap_or("");
    let function = code
        .strip_prefix('F')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n) && code == format!("F{n}"));
    let ordinary = (code.len() == 4
        && code.starts_with("Key")
        && code.as_bytes()[3].is_ascii_uppercase())
        || (code.len() == 6 && code.starts_with("Digit") && code.as_bytes()[5].is_ascii_digit())
        || [
            "Comma",
            "Period",
            "Slash",
            "Backslash",
            "Semicolon",
            "Quote",
            "BracketLeft",
            "BracketRight",
            "Minus",
            "Equal",
            "NumpadAdd",
            "NumpadSubtract",
            "Backquote",
            "Space",
            "Tab",
            "Enter",
            "Escape",
            "Backspace",
            "Delete",
            "Insert",
            "Home",
            "End",
            "PageUp",
            "PageDown",
            "ArrowLeft",
            "ArrowRight",
            "ArrowUp",
            "ArrowDown",
        ]
        .contains(&code);
    let canonical: Vec<_> = ["Ctrl", "Alt", "Meta", "Shift"]
        .into_iter()
        .filter(|p| parts.contains(p))
        .collect();
    (function || ordinary)
        && parts == canonical
        && (function || parts.iter().any(|p| *p != "Shift"))
}

#[derive(Default)]
pub struct KeybindingsFile(pub Mutex<()>);

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Keybindings {
    version: u32,
    bindings: BTreeMap<String, Option<String>>,
    #[serde(default)]
    focus_follows_pointer: bool,
}

fn read(path: &Path) -> Result<Option<serde_json::Value>, String> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("The keybindings file exceeds 256 KiB and has been left intact.".into());
    }
    serde_json::from_slice(&bytes).map(Some).map_err(|error| {
        format!(
            "Cannot load keybindings ({error}). The file has been left intact at {}.",
            path.display()
        )
    })
}

fn save(path: &Path, data: &Keybindings) -> Result<(), String> {
    validate(data)?;
    crate::files::write_json(path, data, LIMIT as usize)
}

fn validate(data: &Keybindings) -> Result<(), String> {
    if data.version != 1
        || data.bindings.len() > 2048
        || data.bindings.iter().any(|(id, value)| {
            id.is_empty()
                || id.len() > 160
                || value.as_ref().is_some_and(|value| !valid_shortcut(value))
        })
    {
        return Err("Invalid keybindings settings.".into());
    }
    Ok(())
}

#[cfg(unix)]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSource {
    data: Option<serde_json::Value>,
    source_revision: Option<String>,
    definitions_revision: String,
    contributions: Vec<crate::plugins::ShortcutContribution>,
}

#[cfg(unix)]
pub(crate) fn agent_source(
    app: &tauri::AppHandle,
    check: &dyn Fn() -> Result<(), lomi_control_protocol::ErrorCode>,
) -> Result<AgentSource, lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    let state = app.state::<KeybindingsFile>();
    let _guard = state.0.lock().map_err(|_| ErrorCode::StorageUnavailable)?;
    let source =
        crate::settings_control::PreferenceSource::open(app, "keybindings.json", LIMIT, check)?;
    let data = source
        .bytes
        .as_deref()
        .map(serde_json::from_slice)
        .transpose()
        .map_err(|_| ErrorCode::UnsupportedCapability)?;
    crate::plugins::with_shortcut_definitions(app, check, |definitions_revision, contributions| {
        Ok(AgentSource {
            data,
            source_revision: source.revision,
            definitions_revision,
            contributions,
        })
    })
}

#[cfg(unix)]
fn agent_patch(
    source: Option<&[u8]>,
    patch: &lomi_control_protocol::settings::SettingsPatch,
) -> Result<Vec<u8>, lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::{settings::SettingsPatch, ErrorCode};
    let mut data = match source {
        Some(bytes) => serde_json::from_slice::<serde_json::Value>(bytes)
            .map_err(|_| ErrorCode::UnsupportedCapability)?,
        None => serde_json::json!({"version":1,"bindings":{}}),
    };
    let typed = serde_json::from_value::<Keybindings>(data.clone())
        .map_err(|_| ErrorCode::UnsupportedCapability)?;
    validate(&typed).map_err(|_| ErrorCode::UnsupportedCapability)?;
    match patch {
        SettingsPatch::KeybindingSet { action, shortcut } => {
            data["bindings"]
                .as_object_mut()
                .ok_or(ErrorCode::UnsupportedCapability)?
                .insert(action.clone(), serde_json::json!(shortcut));
        }
        SettingsPatch::KeybindingReset { action } => {
            data["bindings"]
                .as_object_mut()
                .ok_or(ErrorCode::UnsupportedCapability)?
                .remove(action);
        }
        SettingsPatch::KeybindsFocusFollowsPointer { value } => {
            data["focusFollowsPointer"] = serde_json::json!(value)
        }
        _ => return Err(ErrorCode::UnsupportedCapability),
    }
    let typed = serde_json::from_value::<Keybindings>(data.clone())
        .map_err(|_| ErrorCode::ResourceExhausted)?;
    validate(&typed).map_err(|_| ErrorCode::ResourceExhausted)?;
    let bytes = serde_json::to_vec_pretty(&data).map_err(|_| ErrorCode::ResourceExhausted)?;
    if bytes.len() as u64 > LIMIT {
        return Err(ErrorCode::ResourceExhausted);
    }
    Ok(bytes)
}

#[cfg(unix)]
pub(crate) fn agent_prepare(
    app: &tauri::AppHandle,
    patch: &lomi_control_protocol::settings::SettingsPatch,
    current: &lomi_control_protocol::settings::SettingsUpdateValues,
    check: &dyn Fn() -> Result<(), lomi_control_protocol::ErrorCode>,
) -> Result<lomi_control_core::broker::SettingsPlan, lomi_control_protocol::ErrorCode> {
    use crate::settings_control::PreferenceSource;
    use lomi_control_core::{atomic_file, broker::SettingsPlan};
    use lomi_control_protocol::{settings::SettingsUpdateValues, ErrorCode};
    use std::sync::Arc;
    let SettingsUpdateValues::Keybinds(values) = current else {
        return Err(ErrorCode::UnsupportedCapability);
    };
    if values.action.as_ref().is_some_and(|a| {
        a.id.is_empty()
            || a.id.len() > 160
            || a.label.is_empty()
            || a.label.len() > 640
            || [&a.shortcut, &a.default_shortcut]
                .iter()
                .any(|s| s.as_ref().is_some_and(|s| !valid_shortcut(s)))
    }) {
        return Err(ErrorCode::ResourceExhausted);
    }
    let state = app.state::<KeybindingsFile>();
    let _guard = state.0.lock().map_err(|_| ErrorCode::StorageUnavailable)?;
    let source = PreferenceSource::open(app, "keybindings.json", LIMIT, check)?;
    if source.revision != values.source_revision {
        return Err(ErrorCode::RevisionConflict);
    }
    let bytes = agent_patch(source.bytes.as_deref(), patch)?;
    crate::plugins::with_shortcut_definitions(app, check, |revision, _| {
        if revision != values.definitions_revision {
            return Err(ErrorCode::RevisionConflict);
        }
        Ok(())
    })?;
    let definitions_revision = values.definitions_revision.clone();
    let mut after = current.clone();
    patch.apply_values(&mut after)?;
    let apply_app = app.clone();
    Ok(SettingsPlan {
        before: current.clone(),
        after,
        source_revision: source.revision.clone(),
        apply: Arc::new(move |check| {
            let state = apply_app.state::<KeybindingsFile>();
            let _guard = state.0.lock().map_err(|_| ErrorCode::StorageUnavailable)?;
            crate::plugins::with_shortcut_definitions(&apply_app, check, |revision, _| {
                if revision != definitions_revision {
                    return Err(ErrorCode::RevisionConflict.into());
                }
                let revision = source.commit(&bytes, check)?;
                apply_app
                    .emit("keybindings-changed", ())
                    .map_err(|_| atomic_file::ReplaceError::Uncertain)?;
                Ok(revision)
            })
        }),
    })
}

#[tauri::command]
pub fn load_keybindings(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, KeybindingsFile>,
) -> Result<Option<serde_json::Value>, String> {
    if !matches!(window.label(), "main" | "settings") {
        return Err("Keybindings are only available in the main and settings windows.".into());
    }
    let _guard = state.0.lock().map_err(|error| error.to_string())?;
    read(
        &app.path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join("keybindings.json"),
    )
}

#[tauri::command]
pub fn save_keybindings(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, KeybindingsFile>,
    data: Keybindings,
) -> Result<(), String> {
    if window.label() != "settings" {
        return Err("Keybindings can only be changed in the settings window.".into());
    }
    let _guard = state.0.lock().map_err(|error| error.to_string())?;
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    save(&directory.join("keybindings.json"), &data)?;
    app.emit("keybindings-changed", ())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn agent_changes_one_stored_field_and_preserves_unavailable_descriptors() {
        use lomi_control_protocol::settings::SettingsPatch;
        let source = br#"{"version":1,"bindings":{"saveFile":"Meta+KeyS","missing.plugin":null},"opaque":{"preserved":true}}"#;
        for patch in [
            SettingsPatch::KeybindingSet {
                action: "saveFile".into(),
                shortcut: None,
            },
            SettingsPatch::KeybindingReset {
                action: "saveFile".into(),
            },
            SettingsPatch::KeybindsFocusFollowsPointer { value: true },
        ] {
            let output: serde_json::Value =
                serde_json::from_slice(&agent_patch(Some(source), &patch).unwrap()).unwrap();
            assert_eq!(output["opaque"]["preserved"], true);
            assert!(output["bindings"]
                .as_object()
                .unwrap()
                .contains_key("missing.plugin"));
            match patch {
                SettingsPatch::KeybindingSet { .. } => {
                    assert!(output["bindings"]["saveFile"].is_null())
                }
                SettingsPatch::KeybindingReset { .. } => assert!(!output["bindings"]
                    .as_object()
                    .unwrap()
                    .contains_key("saveFile")),
                _ => assert_eq!(output["focusFollowsPointer"], true),
            }
        }
        let invalid = SettingsPatch::KeybindingSet {
            action: "saveFile".into(),
            shortcut: Some("KeyA".into()),
        };
        assert!(agent_patch(Some(source), &invalid).is_err());
        assert!(agent_patch(
            Some(b"invalid"),
            &SettingsPatch::KeybindsFocusFollowsPointer { value: true }
        )
        .is_err());
    }

    #[test]
    fn saves_overrides_and_disabled_bindings_and_preserves_invalid_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("keybindings.json");
        assert!(read(&path).unwrap().is_none());
        let data = Keybindings {
            version: 1,
            focus_follows_pointer: true,
            bindings: BTreeMap::from([
                ("newTerminal".into(), Some("Ctrl+KeyK".into())),
                ("closeTerminal".into(), None),
            ]),
        };
        save(&path, &data).unwrap();
        let saved = read(&path).unwrap().unwrap();
        assert_eq!(saved["bindings"]["newTerminal"], "Ctrl+KeyK");
        assert!(saved["bindings"]["closeTerminal"].is_null());
        assert_eq!(saved["focusFollowsPointer"], true);
        fs::write(&path, "broken json").unwrap();
        assert!(read(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "broken json");
        assert!(save(
            &path,
            &Keybindings {
                version: 2,
                focus_follows_pointer: false,
                bindings: BTreeMap::new()
            }
        )
        .is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "broken json");
    }

    #[test]
    fn pointer_focus_defaults_to_click_and_round_trips_both_modes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("keybindings.json");
        let mut data: Keybindings = serde_json::from_str(r#"{"version":1,"bindings":{}}"#).unwrap();
        assert!(!data.focus_follows_pointer);
        for enabled in [true, false] {
            data.focus_follows_pointer = enabled;
            save(&path, &data).unwrap();
            assert_eq!(
                read(&path).unwrap().unwrap()["focusFollowsPointer"],
                enabled
            );
        }
        for invalid in ["null", "1", "\"false\""] {
            let json =
                format!(r#"{{"version":1,"bindings":{{}},"focusFollowsPointer":{invalid}}}"#);
            assert!(serde_json::from_str::<Keybindings>(&json).is_err());
        }
    }
}
