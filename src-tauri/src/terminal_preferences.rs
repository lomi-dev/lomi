use serde_json::Value;
use std::{fs, io::Read, path::Path, sync::Mutex};
use tauri::{Emitter, Manager, State, Window};

const LIMIT: u64 = 16 * 1024;

#[derive(Default)]
pub struct TerminalPreferencesFile(pub Mutex<()>);

type TerminalPreferences = Value;

fn validate(data: &Value) -> Result<(), String> {
    let valid = || -> Option<()> {
        let data = data.as_object()?;
        if data.keys().any(|key| {
            !matches!(
                key.as_str(),
                "version"
                    | "appearance"
                    | "behavior"
                    | "windowsShell"
                    | "agentNotifications"
                    | "alwaysShowTitles"
            )
        }) || data.get("version")?.as_u64()? != 1
        {
            return None;
        }
        if let Some(shell) = data.get("windowsShell") {
            if !matches!(shell.as_str()?, "powershell" | "cmd") {
                return None;
            }
        }
        if let Some(enabled) = data.get("agentNotifications") {
            enabled.as_bool()?;
        }
        if let Some(enabled) = data.get("alwaysShowTitles") {
            enabled.as_bool()?;
        }
        let appearance = data.get("appearance")?.as_object()?;
        for (key, value) in appearance {
            let range = match key.as_str() {
                "fontSize" => Some((6.0, 72.0)),
                "lineHeight" => Some((1.0, 3.0)),
                "letterSpacing" => Some((-2.0, 20.0)),
                "cursorWidth" => Some((1.0, 10.0)),
                "minimumContrastRatio" => Some((1.0, 21.0)),
                _ => None,
            };
            if let Some((min, max)) = range {
                let number = value.as_f64()?;
                if !(min..=max).contains(&number) || (key == "cursorWidth" && number.fract() != 0.0)
                {
                    return None;
                }
                continue;
            }
            match key.as_str() {
                "fontFamily" => {
                    let font = value.as_str()?;
                    if font.trim().is_empty()
                        || font.encode_utf16().count() > 500
                        || font
                            .chars()
                            .any(|c| c.is_ascii_control() || ";{}<>".contains(c))
                    {
                        return None;
                    }
                }
                "fontWeight" | "fontWeightBold" => {
                    if !matches!(value.as_str(), Some("normal" | "bold"))
                        && !(1.0..=1000.0).contains(&value.as_f64()?)
                    {
                        return None;
                    }
                }
                "cursorStyle" => {
                    if !matches!(value.as_str()?, "bar" | "block" | "underline") {
                        return None;
                    }
                }
                "cursorInactiveStyle" => {
                    if !matches!(
                        value.as_str()?,
                        "outline" | "bar" | "block" | "underline" | "none"
                    ) {
                        return None;
                    }
                }
                "cursorBlink" | "drawBoldTextInBrightColors" => {
                    value.as_bool()?;
                }
                "colors" => {
                    for (key, value) in value.as_object()? {
                        if !matches!(
                            key.as_str(),
                            "background"
                                | "foreground"
                                | "cursor"
                                | "cursorAccent"
                                | "selectionBackground"
                                | "selectionForeground"
                                | "selectionInactiveBackground"
                                | "black"
                                | "red"
                                | "green"
                                | "yellow"
                                | "blue"
                                | "magenta"
                                | "cyan"
                                | "white"
                                | "brightBlack"
                                | "brightRed"
                                | "brightGreen"
                                | "brightYellow"
                                | "brightBlue"
                                | "brightMagenta"
                                | "brightCyan"
                                | "brightWhite"
                                | "searchMatchBackground"
                                | "searchActiveMatchBackground"
                                | "searchMatchBorder"
                                | "searchActiveMatchBorder"
                        ) {
                            return None;
                        }
                        let color = value.as_str()?;
                        if !matches!(color.len(), 7 | 9)
                            || !color.starts_with('#')
                            || !color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
                        {
                            return None;
                        }
                    }
                }
                _ => return None,
            }
        }
        let behavior = data.get("behavior")?.as_object()?;
        if behavior.len() != 15 {
            return None;
        }
        for (key, min, max, integer) in [
            ("scrollback", 0.0, 100_000.0, true),
            ("scrollSensitivity", 0.1, 100.0, false),
            ("fastScrollSensitivity", 0.1, 100.0, false),
            ("smoothScrollDuration", 0.0, 1000.0, true),
            ("tabStopWidth", 1.0, 32.0, true),
        ] {
            let number = behavior.get(key)?.as_f64()?;
            if !(min..=max).contains(&number) || (integer && number.fract() != 0.0) {
                return None;
            }
        }
        for key in [
            "scrollOnUserInput",
            "scrollOnEraseInDisplay",
            "altClickMovesCursor",
            "rightClickSelectsWord",
            "macOptionIsMeta",
            "macOptionClickForcesSelection",
            "screenReaderMode",
            "customGlyphs",
            "rescaleOverlappingGlyphs",
        ] {
            behavior.get(key)?.as_bool()?;
        }
        let separators = behavior.get("wordSeparator")?.as_str()?;
        if separators.encode_utf16().count() > 200
            || separators.chars().any(|c| c.is_ascii_control())
        {
            return None;
        }
        Some(())
    };
    valid().ok_or_else(|| "Invalid or unsupported terminal settings.".into())
}

fn read(path: &Path) -> Result<Option<TerminalPreferences>, String> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let result = if bytes.len() as u64 > LIMIT {
        Err("The terminal settings file exceeds 16 KiB.".into())
    } else {
        serde_json::from_slice::<TerminalPreferences>(&bytes)
            .map_err(|error| error.to_string())
            .and_then(|data| validate(&data).map(|()| data))
    };
    result.map(Some).map_err(|error: String| {
        format!(
            "Cannot load terminal settings ({error}). The file has been left intact at {}. Retry loading or reset defaults to replace it.",
            path.display()
        )
    })
}

fn save(path: &Path, data: &TerminalPreferences) -> Result<(), String> {
    validate(data)?;
    crate::files::write_json(path, data, LIMIT as usize)
}

#[cfg(unix)]
fn agent_plan_values(
    source: Option<&[u8]>,
    patch: &lomi_control_protocol::settings::SettingsPatch,
) -> Result<
    (
        lomi_control_protocol::settings::SettingsUpdateValues,
        lomi_control_protocol::settings::SettingsUpdateValues,
        Vec<u8>,
    ),
    lomi_control_protocol::ErrorCode,
> {
    use lomi_control_protocol::{
        settings::{SettingsTerminalValues, SettingsUpdateValues},
        ErrorCode,
    };
    use serde_json::json;
    let mut stored: Value = match source {
        Some(bytes) => {
            serde_json::from_slice(bytes).map_err(|_| ErrorCode::UnsupportedCapability)?
        }
        None => json!({"version":1,"appearance":{},"behavior":{
            "scrollback":10000,"scrollSensitivity":1,"fastScrollSensitivity":5,"smoothScrollDuration":0,"tabStopWidth":8,
            "scrollOnUserInput":true,"scrollOnEraseInDisplay":false,"altClickMovesCursor":true,"rightClickSelectsWord":false,
            "macOptionIsMeta":false,"macOptionClickForcesSelection":false,"screenReaderMode":false,"customGlyphs":true,
            "rescaleOverlappingGlyphs":false,"wordSeparator":" ()[]{}',\"`"
        }}),
    };
    let project = |stored: &Value| -> Result<SettingsUpdateValues, ErrorCode> {
        validate(stored).map_err(|_| ErrorCode::UnsupportedCapability)?;
        let mut data = json!({
            "appearance": stored["appearance"], "behavior": stored["behavior"],
            "windowsShell": stored.get("windowsShell").cloned().unwrap_or(json!("powershell")),
            "agentNotifications": stored.get("agentNotifications").cloned().unwrap_or(json!(true)),
            "alwaysShowTitles": stored.get("alwaysShowTitles").cloned().unwrap_or(json!(false))
        });
        // The existing validator accepts integral JSON floats. Normalize only
        // the typed projection; untouched stored fields retain their values.
        for key in ["scrollback", "smoothScrollDuration", "tabStopWidth"] {
            data["behavior"][key] = json!(data["behavior"][key].as_f64().unwrap() as u32);
        }
        if let Some(value) = data["appearance"]["cursorWidth"].as_f64() {
            data["appearance"]["cursorWidth"] = json!(value as u8);
        }
        let current: SettingsTerminalValues =
            serde_json::from_value(data).map_err(|_| ErrorCode::UnsupportedCapability)?;
        Ok(SettingsUpdateValues::Terminal(Box::new(current)))
    };
    let before = project(&stored)?;
    patch.apply_terminal(&mut stored)?;
    validate(&stored).map_err(|_| ErrorCode::ResourceExhausted)?;
    let after = project(&stored)?;
    let bytes = serde_json::to_vec_pretty(&stored).map_err(|_| ErrorCode::ResourceExhausted)?;
    if bytes.len() as u64 > LIMIT {
        return Err(ErrorCode::ResourceExhausted);
    }
    Ok((before, after, bytes))
}

#[cfg(unix)]
pub(crate) fn agent_prepare(
    app: &tauri::AppHandle,
    patch: &lomi_control_protocol::settings::SettingsPatch,
    check: &dyn Fn() -> Result<(), lomi_control_protocol::ErrorCode>,
) -> Result<lomi_control_core::broker::SettingsPlan, lomi_control_protocol::ErrorCode> {
    use crate::settings_control::PreferenceSource;
    use lomi_control_core::{atomic_file, broker::SettingsPlan};
    use lomi_control_protocol::ErrorCode;
    use std::sync::Arc;
    let state = app.state::<TerminalPreferencesFile>();
    let _guard = state.0.lock().map_err(|_| ErrorCode::StorageUnavailable)?;
    let source = PreferenceSource::open(app, "terminal-preferences.json", LIMIT, check)?;
    let (before, after, bytes) = agent_plan_values(source.bytes.as_deref(), patch)?;
    let source_revision = source.revision.clone();
    let apply_app = app.clone();
    Ok(SettingsPlan {
        before,
        after,
        source_revision,
        apply: Arc::new(move |check| {
            let state = apply_app.state::<TerminalPreferencesFile>();
            let _guard = state.0.lock().map_err(|_| ErrorCode::StorageUnavailable)?;
            let revision = source.commit(&bytes, check)?;
            apply_app
                .emit("terminal-preferences-changed", ())
                .map_err(|_| atomic_file::ReplaceError::Uncertain)?;
            Ok(revision)
        }),
    })
}

#[tauri::command]
pub fn load_terminal_preferences(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, TerminalPreferencesFile>,
) -> Result<Option<TerminalPreferences>, String> {
    if window.label() != "main" {
        return Err("Terminal settings are only available in the main window.".into());
    }
    let _guard = state.0.lock().map_err(|error| error.to_string())?;
    read(
        &app.path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join("terminal-preferences.json"),
    )
}

#[tauri::command]
pub fn save_terminal_preferences(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, TerminalPreferencesFile>,
    data: TerminalPreferences,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Terminal preferences can only be changed in Settings.".into());
    }
    let _guard = state.0.lock().map_err(|error| error.to_string())?;
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    save(&directory.join("terminal-preferences.json"), &data)?;
    app.emit("terminal-preferences-changed", ())
        .map_err(|error| error.to_string())
}
