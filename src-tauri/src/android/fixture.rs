//! Isolated production-path native test configuration; excluded from normal builds.
use std::path::PathBuf;

pub fn preset_dialog<R: tauri::Runtime>(
    dialog: tauri_plugin_dialog::FileDialogBuilder<R>,
    screenshot: bool,
) -> Result<tauri_plugin_dialog::FileDialogBuilder<R>, String> {
    let Some(root) = directory()? else {
        return Ok(dialog);
    };
    let trial = root.parent().ok_or("Missing fixture trial")?;
    let (folder, name) = if screenshot {
        (trial.join("product"), "screenshot.png")
    } else {
        (trial.join("product/apk-selection"), "input-test.apk")
    };
    // Only the initial native picker location differs. Selection, cancellation,
    // file opening and installation still use the real product command.
    // rfd 0.16 appends a non-empty name to directoryURL on macOS. Keep the
    // fixture location a directory; the native driver fills the name afterward.
    Ok(dialog
        .set_directory(folder)
        .set_file_name(if screenshot { "" } else { name }))
}

pub fn guest(root: &std::path::Path, device: &str) -> Result<super::adb::Guest, String> {
    super::runtime::fixture_guest(root, device)
}

pub fn directory() -> Result<Option<PathBuf>, String> {
    let Some(value) = std::env::var_os("LOMI_ANDROID_PRODUCT_DIRECTORY") else {
        return Ok(None);
    };
    let root = PathBuf::from(value)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let trial = root.parent().ok_or("Missing Android fixture parent")?;
    if !trial
        .file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with("lomi-android-stage0-"))
        || !root
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("native-managed-"))
    {
        return Err(
            "Production Android probes require the isolated, licensed native trial directory"
                .into(),
        );
    }
    let consent: serde_json::Value = super::storage::read(&trial.join("evidence/consent.json"))?
        .ok_or("Missing native trial consent")?;
    if consent["accepted"] != true {
        return Err("The native trial SDK license has not been accepted".into());
    }
    Ok(Some(root))
}

pub fn adb_port() -> u16 {
    if std::env::var_os("LOMI_ANDROID_PRODUCT_DIRECTORY").is_some() {
        15047
    } else {
        5037
    }
}
