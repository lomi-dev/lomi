use tauri::Manager;

pub fn page(webview: &tauri::Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if webview.label() == "main"
        && matches!(payload.event(), tauri::webview::PageLoadEvent::Started)
    {
        // No frontend close listener exists yet; quitting must preserve the window.
        webview.app_handle().exit(0);
    }
    if webview.label() == "main"
        && matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
    {
        let _ = webview.eval(include_str!("notification-smoke.js"));
    }
}

pub fn result(
    app: &tauri::AppHandle,
    stage: &str,
    data: serde_json::Value,
) -> Result<serde_json::Value, String> {
    match stage {
        "notification-foreground" => {
            let window = app.get_window("main").ok_or("Missing main window")?;
            window.show().map_err(|error| error.to_string())?;
            window.set_focus().map_err(|error| error.to_string())?;
        }
        "notification-settings" => {
            let window = app
                .get_window("settings")
                .ok_or("Missing settings window")?;
            window.show().map_err(|error| error.to_string())?;
            window.set_focus().map_err(|error| error.to_string())?;
        }
        #[cfg(target_os = "macos")]
        "notification-cmd-q" => {
            app.run_on_main_thread(|| unsafe {
                use std::ffi::c_void;
                #[link(name = "CoreGraphics", kind = "framework")]
                extern "C" {
                    fn CGEventCreateKeyboardEvent(
                        source: *const c_void,
                        key: u16,
                        down: bool,
                    ) -> *const c_void;
                    fn CGEventSetFlags(event: *const c_void, flags: u64);
                }
                #[link(name = "CoreFoundation", kind = "framework")]
                extern "C" {
                    fn CFRelease(value: *const c_void);
                }
                let main = objc2::MainThreadMarker::new().unwrap();
                let application = objc2_app_kit::NSApplication::sharedApplication(main);
                for down in [true, false] {
                    let event = CGEventCreateKeyboardEvent(std::ptr::null(), 12, down);
                    assert!(!event.is_null());
                    CGEventSetFlags(event, 1 << 20);
                    let native: *mut objc2::runtime::AnyObject =
                        objc2::msg_send![objc2::class!(NSEvent), eventWithCGEvent: event];
                    assert!(!native.is_null());
                    // Route the shortcut through this application's native event dispatch,
                    // without sending keystrokes to other running applications.
                    let _: () = objc2::msg_send![&*application, sendEvent: native];
                    CFRelease(event);
                }
            })
            .map_err(|error| error.to_string())?;
        }
        #[cfg(target_os = "macos")]
        "notification-quit" => {
            app.run_on_main_thread(|| {
                let main = objc2::MainThreadMarker::new().unwrap();
                let application = objc2_app_kit::NSApplication::sharedApplication(main);
                // Activate the actual Quit item, including its native menu callback.
                unsafe {
                    let menu: *mut objc2::runtime::AnyObject =
                        objc2::msg_send![&*application, mainMenu];
                    assert!(!menu.is_null());
                    let item: *mut objc2::runtime::AnyObject =
                        objc2::msg_send![menu, itemAtIndex: 0isize];
                    assert!(!item.is_null());
                    let submenu: *mut objc2::runtime::AnyObject = objc2::msg_send![item, submenu];
                    assert!(!submenu.is_null());
                    let count: isize = objc2::msg_send![submenu, numberOfItems];
                    assert!(count > 0);
                    let _: () = objc2::msg_send![submenu, performActionForItemAtIndex: count - 1];
                }
            })
            .map_err(|error| error.to_string())?;
        }
        "notification-close" => {
            app.get_window("main")
                .ok_or("Missing main window")?
                .close()
                .map_err(|error| error.to_string())?;
        }
        "notification-toggle" => {
            app.get_webview("settings").ok_or("Missing settings view")?.eval(format!(r#"(async () => {{
                const invoke = window.__TAURI_INTERNALS__.invoke;
                const data = await invoke('load_terminal_preferences');
                const defaults = (await import('/src/terminal-preferences.ts')).defaultTerminalPreferences;
                await invoke('save_terminal_preferences', {{data: {{version: 1, ...defaults, ...data, agentNotifications: {data}}}}});
            }})();"#)).map_err(|error| error.to_string())?;
        }
        "notification-background" => {
            app.get_window("main")
                .ok_or("Missing main window")?
                .hide()
                .map_err(|error| error.to_string())?;
        }
        "passed" | "failed" => {
            let directory = std::env::var("LOMI_NOTIFICATION_SMOKE_DIRECTORY")
                .map_err(|error| error.to_string())?;
            std::fs::write(
                std::path::Path::new(&directory).join("result.json"),
                serde_json::to_vec_pretty(&serde_json::json!({"stage": stage, "data": data}))
                    .unwrap(),
            )
            .map_err(|error| error.to_string())?;
            app.exit(if stage == "passed" { 0 } else { 1 });
        }
        _ => return Err("Unknown notification smoke stage.".into()),
    }
    Ok(serde_json::Value::Null)
}
