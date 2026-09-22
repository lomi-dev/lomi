//! Development-only driver for the real Android Settings and workspace UI.
use std::{fs, path::PathBuf, time::Duration};
use tauri::{Manager, Window};

fn root() -> Result<PathBuf, String> {
    let managed = PathBuf::from(
        std::env::var_os("LOMI_ANDROID_PRODUCT_DIRECTORY").ok_or("Product probe is disabled")?,
    )
    .canonicalize()
    .map_err(|e| e.to_string())?;
    let trial = managed.parent().ok_or("Missing native trial")?;
    let consent: serde_json::Value = serde_json::from_slice(
        &fs::read(trial.join("evidence/consent.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if consent["accepted"] != true {
        return Err("Missing isolated SDK consent".into());
    }
    let root = trial.join("product");
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    Ok(root)
}
pub fn watch(app: tauri::AppHandle) {
    let Ok(root) = root() else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let mut last = String::new();
        loop {
            tokio::time::sleep(Duration::from_millis(150)).await;
            let Ok(bytes) = fs::read(root.join("instruction.json")) else {
                continue;
            };
            if bytes.len() > 128 * 1024 {
                continue;
            }
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            if value["processId"]
                .as_u64()
                .is_some_and(|pid| pid != u64::from(std::process::id()))
            {
                continue;
            }
            let Some(id) = value["id"].as_str() else {
                continue;
            };
            if id == last || !valid(id) {
                continue;
            }
            let label = value["window"].as_str().unwrap_or("main");
            if !matches!(label, "main" | "settings") {
                continue;
            }
            if let Some(action) = value["action"].as_str() {
                last = id.into();
                let result = control(&app, label, action, &value).await;
                let report = match result {
                    Ok(data) => serde_json::json!({"ok":true,"data":data}),
                    Err(error) => serde_json::json!({"ok":false,"error":error}),
                };
                let _ = fs::write(root.join(format!("{id}.json")), report.to_string());
                continue;
            }
            let Some(script) = value["script"].as_str() else {
                continue;
            };
            let Some(view) = app.get_webview(label) else {
                continue;
            };
            last = id.into();
            let id = serde_json::to_string(id).unwrap();
            let script = format!("(async()=>{{const invoke=window.__TAURI_INTERNALS__.invoke;const sleep=ms=>new Promise(r=>setTimeout(r,ms));const wait=async(test,ms=20000)=>{{const end=Date.now()+ms;for(;;){{const value=await test();if(value)return value;if(Date.now()>end)throw Error('Native UI condition timed out');await sleep(100);}}}};const click=(name)=>{{const el=[...document.querySelectorAll('button')].find(el=>el.getAttribute('aria-label')===name||el.textContent.trim()===name);if(!el||el.disabled)throw Error('Unavailable UI button: '+name);el.click();return el;}};try{{const data=await(async()=>{{{script}}})();await invoke('plugin_smoke_result',{{stage:'product-report',data:{{id:{id},ok:true,data}}}});}}catch(error){{await invoke('plugin_smoke_result',{{stage:'product-report',data:{{id:{id},ok:false,error:String(error),stack:error?.stack}}}});}}}})()");
            if let Err(error) = view.eval(script) {
                let _ = fs::write(root.join("eval-error.txt"), error.to_string());
            }
        }
    });
}
async fn control(
    app: &tauri::AppHandle,
    label: &str,
    action: &str,
    value: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let window = app.get_window(label).ok_or("Missing product test window")?;
    match action {
        "present" | "move-to-current-space" => {
            for other in ["main", "settings"] {
                if other != label {
                    if let Some(other) = app.get_window(other) {
                        other.hide().map_err(|e| e.to_string())?;
                    }
                }
            }
            window.unminimize().map_err(|e| e.to_string())?;
            window.show().map_err(|e| e.to_string())?;
            window.set_always_on_top(true).map_err(|e| e.to_string())?;
            #[cfg(not(target_os = "macos"))]
            window
                .set_visible_on_all_workspaces(true)
                .map_err(|e| e.to_string())?;
            #[cfg(target_os = "macos")]
            {
                crate::android_probe::present_on_spaces(&window).await?;
                app.show().map_err(|e| e.to_string())?;
                let target = window.clone();
                let move_to_current_space = action == "move-to-current-space";
                let (send, receive) = tokio::sync::oneshot::channel();
                window
                    .run_on_main_thread(move || {
                        use objc2::{class, msg_send, runtime::AnyObject};
                        use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior as Behavior};
                        let result = target.ns_window().map_err(|e|e.to_string()).and_then(|pointer| objc2::exception::catch(|| (|| -> Result<(), String> {
                            let native = unsafe { &*pointer.cast::<AnyObject>() };
                            if move_to_current_space {
                                let native = unsafe { &*pointer.cast::<NSWindow>() };
                                let mut behavior = native.collectionBehavior();
                                behavior.remove(Behavior::CanJoinAllSpaces);
                                behavior.insert(Behavior::MoveToActiveSpace);
                                native.setCollectionBehavior(behavior);
                            }
                            unsafe {
                                let _: () = msg_send![native, setLevel: 25isize];
                                let application: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
                                let _: () = msg_send![application, activateIgnoringOtherApps: true];
                                let _: () = msg_send![native, makeKeyAndOrderFront: std::ptr::null::<AnyObject>()];
                                let _: () = msg_send![native, orderFrontRegardless];
                            }
                            Ok(())
                        })()).map_err(|error|format!("Native presentation failed: {error:?}")).and_then(|result|result));
                        let _ = send.send(result);
                    })
                    .map_err(|e| e.to_string())?;
                receive.await.map_err(|e| e.to_string())??;
            }
            #[cfg(not(target_os = "macos"))]
            window.set_focus().map_err(|e| e.to_string())?;
        }
        "minimize" => window.minimize().map_err(|e| e.to_string())?,
        "hide" => window.hide().map_err(|e| e.to_string())?,
        #[cfg(target_os = "macos")]
        "dialog-location"
        | "dialog-type-apk"
        | "dialog-type-screenshot"
        | "dialog-confirm"
        | "dialog-cancel"
        | "dialog-state"
        | "dialog-directory-apk"
        | "dialog-directory-screenshot"
        | "dialog-select-apk" => {
            let trial = root()?.parent().ok_or("Missing trial")?.to_path_buf();
            let path = if matches!(
                action,
                "dialog-type-apk" | "dialog-directory-apk" | "dialog-select-apk"
            ) {
                trial.join("product/apk-selection/input-test.apk")
            } else {
                trial.join("product/screenshot.png")
            };
            let action = action.to_owned();
            let (send, receive) = tokio::sync::oneshot::channel();
            window.run_on_main_thread(move || {
                use objc2::{class, msg_send, runtime::AnyObject};
                use objc2_foundation::{NSString, NSPoint, NSURL};
                let result = objc2::exception::catch(|| unsafe { (|| -> Result<serde_json::Value, String> {
                    let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
                    let windows: *mut AnyObject = msg_send![app, windows];
                    let count: usize = msg_send![windows, count];
                    let mut panel: *mut AnyObject = std::ptr::null_mut();
                    for index in 0..count {
                        let candidate: *mut AnyObject = msg_send![windows, objectAtIndex: index];
                        let is_panel: bool = msg_send![candidate, isKindOfClass: class!(NSSavePanel)];
                        let visible: bool = msg_send![candidate, isVisible];
                        if is_panel && visible { panel = candidate; break; }
                    }
                    if panel.is_null() { return Err("No visible native file panel in the test application".into()); }
                    if action == "dialog-state" {
                        let name: *const NSString = msg_send![panel, nameFieldStringValue];
                        let url: *const NSURL = msg_send![panel, directoryURL];
                        let location: *const NSString = msg_send![url, absoluteString];
                        return Ok(serde_json::json!({"name":(*name).to_string(),"directory":if location.is_null(){String::new()}else{(*location).to_string()}}));
                    }
                    let _: () = msg_send![app, activateIgnoringOtherApps: true];
                    let _: () = msg_send![panel, setLevel: 26isize];
                    let _: () = msg_send![panel, makeKeyAndOrderFront: std::ptr::null::<AnyObject>()];
                    if matches!(action.as_str(), "dialog-directory-apk" | "dialog-directory-screenshot") {
                        let url = NSURL::fileURLWithPath(&NSString::from_str(path.parent().unwrap().to_str().unwrap()));
                        let _: () = msg_send![panel, setDirectoryURL: &*url];
                        if action == "dialog-directory-screenshot" {
                            let _: () = msg_send![panel, setNameFieldStringValue: &*NSString::from_str("screenshot.png")];
                        }
                        return Ok(serde_json::Value::Null);
                    }
                    if action == "dialog-cancel" {
                        let _: () = msg_send![panel, cancel: std::ptr::null::<AnyObject>()];
                        return Ok(serde_json::Value::Null);
                    }
                    let number: isize = msg_send![panel, windowNumber];
                    let info: *const AnyObject = msg_send![class!(NSProcessInfo), processInfo];
                    let time: f64 = msg_send![info, systemUptime];
                    let send_key = |text: &str, modifiers: usize, code: u16| {
                        let text = NSString::from_str(text);
                        for kind in [10usize,11] {
                            let event: *mut AnyObject = msg_send![class!(NSEvent), keyEventWithType: kind, location: NSPoint::new(0.0,0.0), modifierFlags: modifiers, timestamp: time, windowNumber: number, context: std::ptr::null::<AnyObject>(), characters: &*text, charactersIgnoringModifiers: &*text, isARepeat: false, keyCode: code];
                            // Native file panels route their default button through key
                            // equivalents before forwarding text events to the XPC view.
                            let handled = kind == 10 && action == "dialog-confirm" && {
                                let handled: bool = msg_send![panel, performKeyEquivalent: event];
                                handled
                            };
                            if !handled {
                                let _: () = msg_send![app, sendEvent: event];
                            }
                        }
                    };
                    match action.as_str() {
                        "dialog-location" => send_key("G", (1usize<<20)|(1usize<<17), 5),
                        "dialog-confirm" => send_key("\r", 0, 36),
                        "dialog-select-apk" => {
                            let url: *const NSURL = msg_send![panel, directoryURL];
                            let directory: *const NSString = msg_send![url, path];
                            if directory.is_null() || PathBuf::from((*directory).to_string()).canonicalize().ok() != path.parent().and_then(|p|p.canonicalize().ok()) {
                                return Err("APK picker is outside the isolated single-file selection directory".into());
                            }
                            send_key("\u{f701}", 1usize<<23, 125);
                        },
                        _ => {
                            // Modern file panels forward events to an XPC remote view;
                            // they do not expose NSTextInputClient on firstResponder.
                            send_key("a", 1usize<<20, 0);
                            for character in path.to_str().unwrap().chars() {
                                send_key(&character.to_string(), 0, 0);
                            }
                        }
                    }
                    Ok(serde_json::Value::Null)
                })() }).map_err(|exception|format!("Native dialog test exception: {exception:?}")).and_then(|result|result);
                let _ = send.send(result);
            }).map_err(|e|e.to_string())?;
            return receive.await.map_err(|e| e.to_string())?;
        }
        "resize" => {
            let width = value["width"]
                .as_f64()
                .filter(|x| (560.0..=1600.0).contains(x))
                .ok_or("Invalid fixture width")?;
            let height = value["height"]
                .as_f64()
                .filter(|x| (420.0..=1200.0).contains(x))
                .ok_or("Invalid fixture height")?;
            window
                .set_size(tauri::LogicalSize::new(width, height))
                .map_err(|e| e.to_string())?;
        }
        #[cfg(target_os = "macos")]
        "process-info" => return crate::android_probe::webkit_processes(app).await,
        _ => return Err("Unknown native product control".into()),
    }
    Ok(serde_json::Value::Null)
}

#[tauri::command]
pub async fn android_probe_product_guest(
    window: Window,
    device_id: String,
    action: String,
) -> Result<String, String> {
    crate::files::main_window(&window)?;
    let root = crate::android::fixture::directory()?.ok_or("Product probe is disabled")?;
    let manager = window
        .app_handle()
        .state::<crate::android::manager::Android>()
        .loaded()
        .ok_or("Android was not opened")?;
    let status = manager
        .statuses()?
        .into_iter()
        .find(|s| s.device_id == device_id)
        .ok_or("Missing fixture phone")?;
    let generation = status.generation.ok_or("Fixture phone stopped")?;
    manager.runtime(&device_id, &generation)?;
    let guest = crate::android::fixture::guest(&root, &device_id)?;
    tauri::async_runtime::spawn_blocking(move || match action.as_str() {
        "screen" => guest.native_input_fixture(false),
        "launch" => guest.native_input_fixture(true),
        "marker-write" => guest.native_runtime_marker(Some("00000000-0000-0000-0000-000000000019")),
        "marker-read" => guest.native_runtime_marker(None),
        _ => Err("Unknown guest fixture action".into()),
    })
    .await
    .map_err(|e| e.to_string())?
}

fn valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
pub fn result(
    window: &Window,
    stage: &str,
    data: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let root = root()?;
    if stage == "product-report" {
        let id = data["id"]
            .as_str()
            .filter(|id| valid(id))
            .ok_or("Invalid report identity")?;
        let bytes = serde_json::to_vec_pretty(&data).map_err(|e| e.to_string())?;
        if bytes.len() > 1024 * 1024 {
            return Err("Native report exceeds its limit".into());
        }
        fs::write(root.join(format!("{id}.json")), bytes).map_err(|e| e.to_string())?;
    } else if stage == "product-screenshot" {
        let id = data
            .as_str()
            .filter(|id| valid(id))
            .ok_or("Invalid screenshot identity")?;
        let path = root.join(format!("{id}.png"));
        let target = window.clone();
        #[cfg(target_os = "macos")]
        window.run_on_main_thread(move || {
            use objc2::{class, msg_send, runtime::AnyObject};
            use objc2_foundation::{NSRect, NSString};
            let result = (|| -> Result<(), String> {
                let native = target.ns_window().map_err(|e| e.to_string())?;
                let native = unsafe { &*native.cast::<AnyObject>() };
                unsafe {
                    let frame: *mut AnyObject = msg_send![native, contentView];
                    let bounds: NSRect = msg_send![frame, bounds];
                    let bitmap: *mut AnyObject = msg_send![frame, bitmapImageRepForCachingDisplayInRect: bounds];
                    if bitmap.is_null() { return Err("No native bitmap".into()); }
                    let _: () = msg_send![frame, cacheDisplayInRect: bounds, toBitmapImageRep: bitmap];
                    let properties: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
                    let png: *mut AnyObject = msg_send![bitmap, representationUsingType: 4usize, properties: properties];
                    if png.is_null() { return Err("No native PNG".into()); }
                    let saved: bool = msg_send![png, writeToFile: &*NSString::from_str(path.to_str().unwrap()), atomically: true];
                    if !saved { return Err("Could not save native screenshot".into()); }
                }
                Ok(())
            })();
            if let Err(error) = result { let _ = fs::write(path.with_extension("error.txt"), error); }
        }).map_err(|e| e.to_string())?;
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (target, path);
            return Err("Native screenshots are only qualified on macOS".into());
        }
    } else {
        return Err("Unknown product probe action".into());
    }
    Ok(serde_json::Value::Null)
}
