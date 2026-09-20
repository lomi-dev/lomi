use tauri::Manager;

pub fn page(webview: &tauri::Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if webview.label() == "main"
        && matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
    {
        let directory = std::env::var("SIMPLEBENCH_CLIPBOARD_SMOKE_DIRECTORY").unwrap();
        let _ = webview.eval(
            include_str!("terminal-clipboard-smoke.js")
                .replace(
                    "SMOKE_AGENTS",
                    if std::env::var_os("SIMPLEBENCH_CLIPBOARD_SMOKE_AGENTS").is_some() {
                        "true"
                    } else {
                        "false"
                    },
                )
                .replace(
                    "SMOKE_DIRECTORY",
                    &serde_json::to_string(&directory).unwrap(),
                )
                .replace(
                    "SMOKE_REPOSITORY",
                    &serde_json::to_string(
                        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                            .parent()
                            .unwrap(),
                    )
                    .unwrap(),
                ),
        );
    }
}

pub fn result(
    app: &tauri::AppHandle,
    stage: &str,
    data: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let directory = std::path::PathBuf::from(
        std::env::var("SIMPLEBENCH_CLIPBOARD_SMOKE_DIRECTORY").map_err(|e| e.to_string())?,
    );
    match stage {
        #[cfg(target_os = "macos")]
        "clipboard-menu" => {
            app.run_on_main_thread(|| {
                let main = objc2::MainThreadMarker::new().unwrap();
                let application = objc2_app_kit::NSApplication::sharedApplication(main);
                unsafe {
                    let menu: *mut objc2::runtime::AnyObject =
                        objc2::msg_send![&*application, mainMenu];
                    let item: *mut objc2::runtime::AnyObject =
                        objc2::msg_send![menu, itemAtIndex: 1isize];
                    let submenu: *mut objc2::runtime::AnyObject = objc2::msg_send![item, submenu];
                    let _: () = objc2::msg_send![submenu, performActionForItemAtIndex: 5isize];
                }
            })
            .map_err(|e| e.to_string())?;
        }
        #[cfg(target_os = "macos")]
        "clipboard-screenshot" => {
            let name = data
                .as_str()
                .filter(|name| matches!(*name, "codex" | "agy"))
                .ok_or("Unknown screenshot")?;
            let path = directory.join(format!("{name}.png"));
            let window = app.get_window("main").ok_or("Missing main window")?;
            app.run_on_main_thread(move || {
                let native = unsafe {
                    &*window
                        .ns_window()
                        .unwrap()
                        .cast::<objc2_app_kit::NSWindow>()
                };
                let _ = std::process::Command::new("screencapture")
                    .args(["-x", "-l", &native.windowNumber().to_string()])
                    .arg(path)
                    .status();
            })
            .map_err(|e| e.to_string())?;
        }
        "clipboard-image" | "clipboard-image-only" | "clipboard-text" => {
            let status = std::process::Command::new(directory.join("clipboard"))
                .arg(stage.strip_prefix("clipboard-").unwrap())
                .arg(&directory)
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err("Could not set fixture clipboard.".into());
            }
        }
        "clipboard-settings" => {
            app.get_webview("settings").ok_or("Missing settings")?.eval(format!(
                r#"(async()=>{{let denied=false;try{{await window.__TAURI_INTERNALS__.invoke('paste_terminal_clipboard',{{id:{data}}});}}catch{{denied=true;}}await window.__TAURI_INTERNALS__.invoke('plugin_smoke_result',{{stage:denied?'passed':'failed',data:denied?'Native PNG and text clipboard, Cmd+V, AppKit Paste, PTY and settings isolation passed':'Settings read the clipboard'}});}})();"#
            )).map_err(|e| e.to_string())?;
        }
        "passed" | "failed" => {
            std::fs::write(
                directory.join("result.json"),
                serde_json::to_vec_pretty(&serde_json::json!({"stage":stage,"data":data})).unwrap(),
            )
            .map_err(|e| e.to_string())?;
            app.exit(if stage == "passed" { 0 } else { 1 });
        }
        _ => return Err("Unknown clipboard smoke stage.".into()),
    }
    Ok(serde_json::Value::Null)
}
