//! Linux-only, owned-process native fixture helpers. No global input or shell tools.
use gtk::prelude::*;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use tauri::Window;

/// Explicit diagnostic on the actual Tauri-owned view; never a startup default.
pub async fn webview_software_policy(window: &Window) -> Result<serde_json::Value, String> {
    use webkit2gtk::{HardwareAccelerationPolicy, SettingsExt, WebViewExt};

    if window.label() != "main" {
        return Err("Software-policy diagnostic requires the main window".into());
    }
    super::linux_input::with_main(window, |widget| {
        let settings = WebViewExt::settings(&widget).ok_or("Main WebKit view has no settings")?;
        let before = settings.hardware_acceleration_policy();
        settings.set_hardware_acceleration_policy(HardwareAccelerationPolicy::Never);
        let after = settings.hardware_acceleration_policy();
        let expected_never = after == HardwareAccelerationPolicy::Never;
        if !expected_never {
            return Err(format!(
                "Main WebKit software policy readback failed: {after:?}"
            ));
        }
        Ok(serde_json::json!({
            "label": "main",
            "beforePolicy": format!("{before:?}"),
            "afterPolicy": format!("{after:?}"),
            "expectedNever": expected_never,
            "diagnosticOnly": true,
            "qualification": false,
        }))
    })
    .await
}

pub async fn dialog(
    window: &Window,
    action: String,
    product: PathBuf,
) -> Result<serde_json::Value, String> {
    let owner = window.clone();
    let (send, receive) = tokio::sync::oneshot::channel();
    window.run_on_main_thread(move || {
        let result = (|| {
            let main = owner.gtk_window().map_err(|e| e.to_string())?;
            let mut matches = Vec::new();
            for widget in gtk::Window::list_toplevels() {
                let Ok(dialog) = widget.downcast::<gtk::FileChooserDialog>() else { continue };
                if !dialog.is_visible() { continue; }
                let title = dialog.title().unwrap_or_default();
                let qualified = (title == "Install APK on Android" && dialog.action() == gtk::FileChooserAction::Open)
                    || (title == "Save Android screenshot" && dialog.action() == gtk::FileChooserAction::Save);
                // rfd's GTK backend creates a parentless GtkFileChooserNative. Only its
                // exact production title/action in this process qualifies that fallback.
                if qualified && dialog.transient_for().is_none_or(|parent| parent == main.clone().upcast::<gtk::Window>()) {
                    matches.push(dialog);
                }
            }
            if matches.len() != 1 {
                return Err(format!("Expected one owned GTK Android file chooser, found {}; portal-only chooser automation is unsupported", matches.len()));
            }
            let dialog = &matches[0];
            let save = dialog.action() == gtk::FileChooserAction::Save;
            let path = if save { product.join("screenshot.png") } else { product.join("apk-selection/input-test.apk") };
            let wants_apk = matches!(action.as_str(), "dialog-type-apk" | "dialog-directory-apk" | "dialog-select-apk");
            if !matches!(action.as_str(), "dialog-state" | "dialog-confirm" | "dialog-cancel" | "dialog-location") && wants_apk == save {
                return Err("Native file chooser action does not match fixture destination".into());
            }
            match action.as_str() {
                "dialog-state" => return Ok(serde_json::json!({
                    "name":dialog.current_name().map(|s|s.to_string()),
                    "directory":dialog.current_folder(),"filename":dialog.filename(),
                    "title":dialog.title().map(|s| s.to_string()),"backend":"gtk-file-chooser-native-widget",
                    "isModal":dialog.is_modal(),"isActive":dialog.is_active(),
                    "ownerActive":main.is_active(),
                    "transientOwnerMatches":dialog.transient_for().map(|parent|parent == main.clone().upcast::<gtk::Window>())
                })),
                "dialog-cancel" => dialog.response(gtk::ResponseType::Cancel),
                "dialog-confirm" => {
                    // Never accept an unexpected path, even if the user changed the picker.
                    let selected = dialog.filename().ok_or("Native file chooser has no selected path")?;
                    let same_file = match (selected.canonicalize(), path.canonicalize()) {
                        (Ok(selected), Ok(expected)) => selected == expected,
                        _ => false,
                    };
                    if selected != path && !same_file {
                        return Err("Native file chooser selection is outside its isolated destination".into());
                    }
                    dialog.response(gtk::ResponseType::Accept);
                }
                "dialog-directory-apk" | "dialog-directory-screenshot" => {
                    if !dialog.set_current_folder(path.parent().ok_or("Missing fixture directory")?) {
                        return Err("GTK rejected the isolated fixture directory".into());
                    }
                    if save { dialog.set_current_name("screenshot.png"); }
                }
                "dialog-type-apk" | "dialog-select-apk" => {
                    if !dialog.set_filename(&path) { return Err("GTK rejected the isolated APK selection".into()); }
                }
                "dialog-type-screenshot" => {
                    if !dialog.set_current_folder(path.parent().ok_or("Missing fixture directory")?) {
                        return Err("GTK rejected the isolated screenshot directory".into());
                    }
                    dialog.set_current_name("screenshot.png");
                }
                "dialog-location" => return Err("GTK fixture selects the isolated file through native chooser APIs; location shortcut is unsupported".into()),
                _ => return Err("Unknown Linux native dialog action".into()),
            }
            Ok(serde_json::Value::Null)
        })();
        let _ = send.send(result);
    }).map_err(|e|e.to_string())?;
    receive.await.map_err(|e| e.to_string())?
}

pub fn capture(window: &Window, path: PathBuf) -> Result<(), String> {
    let target = window.clone();
    window.run_on_main_thread(move || {
        let result = (|| -> Result<(), String> {
            let main = target.gtk_window().map_err(|e|e.to_string())?;
            let native = main.window().ok_or("Owned GTK window is not realized")?;
            let backend = native.display().type_().name();
            if backend.contains("Wayland") {
                return Err("Unsupported native window capture on Wayland; no compositor/portal capture grant exists".into());
            }
            if !backend.contains("X11") || !main.is_mapped() {
                return Err(format!("Unsupported native window capture backend or unmapped window: {backend}"));
            }
            let bitmap = native.pixbuf(0, 0, native.width(), native.height()).ok_or("Owned GDK window capture is unavailable")?;
            bitmap.savev(&path, "png", &[]).map_err(|e|e.to_string())
        })();
        if let Err(error) = result { let _ = fs::write(path.with_extension("error.txt"), error); }
    }).map_err(|e|e.to_string())
}

fn identity(pid: u32) -> Option<(u32, u64, String)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (parent, started) = stat_identity(&stat)?;
    let executable = fs::read_link(format!("/proc/{pid}/exe"))
        .ok()?
        .to_string_lossy()
        .into_owned();
    Some((parent, started, executable))
}
fn stat_identity(stat: &str) -> Option<(u32, u64)> {
    // comm can contain spaces and parentheses. Fields after the final ')' begin at 3.
    let fields: Vec<_> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    Some((fields.get(1)?.parse().ok()?, fields.get(19)?.parse().ok()?))
}
pub fn processes() -> Result<serde_json::Value, String> {
    let application = std::process::id();
    let mut identities = BTreeMap::new();
    for entry in fs::read_dir("/proc").map_err(|e| e.to_string())?.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        if let Some(value) = identity(pid) {
            identities.insert(pid, value);
        }
    }
    let mut owned = vec![application];
    for _ in 0..32 {
        let next: Vec<_> = identities
            .iter()
            .filter(|(pid, (parent, _, _))| owned.contains(parent) && !owned.contains(pid))
            .map(|(pid, _)| *pid)
            .collect();
        if next.is_empty() {
            break;
        }
        owned.extend(next);
    }
    let rows: Vec<_> = owned.into_iter().filter_map(|pid| {
        let (parent, started, executable) = identities.get(&pid)?;
        // Recheck start-time, parent and executable to reject a reused PID.
        if identity(pid).as_ref() != Some(&(*parent, *started, executable.clone())) { return None; }
        let name = Path::new(executable).file_name()?.to_str()?;
        let role = if name.contains("WebKitWebProcess") { "webContent" }
            else if name.contains("WebKitNetworkProcess") { "networking" }
            else if name.contains("WebKitGPUProcess") { "gpu" } else { "descendant" };
        Some(serde_json::json!({"pid":pid,"parent":parent,"startTimeTicks":started,"executable":executable,"role":role}))
    }).collect();
    Ok(
        serde_json::json!({"application":application,"processes":rows,"identitySource":"procfs-descendant-start-time","webviewAttribution":"unsupported: WebKitGTK has no public per-view process identifier"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proc_stat_handles_parentheses_in_process_names() {
        let stat = "42 (WebKit (test) process) S 7 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 1234 0";
        assert_eq!(stat_identity(stat), Some((7, 1234)));
        assert_eq!(stat_identity("42 (broken) S 7"), None);
    }
}
