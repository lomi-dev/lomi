use tauri::{Manager, Window};
#[cfg(target_os = "linux")]
#[path = "android-linux-input.rs"]
mod linux_input;

pub const IDENTIFIER: &str = "dev.lomi.terminal-latency-smoke";
fn directory(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let directory = std::path::PathBuf::from(
        std::env::var("LOMI_TERMINAL_LATENCY_DIRECTORY").map_err(|e| e.to_string())?,
    );
    if app.config().identifier != IDENTIFIER || !directory.is_absolute() {
        return Err("Terminal latency fixture requires its isolated application identity".into());
    }
    let root = std::fs::canonicalize(&directory).map_err(|e| e.to_string())?;
    let data = std::fs::canonicalize(app.path().app_data_dir().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if !data.starts_with(root.join("xdg-data"))
        || std::env::var_os("LOMI_TERMINAL_LATENCY_TOKEN").is_none()
    {
        return Err("Terminal latency profile is outside the owned fixture".into());
    }
    Ok(root)
}
pub fn page(webview: &tauri::Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if webview.label() == "main"
        && matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
    {
        if let Err(error) = directory(webview.app_handle()) {
            eprintln!("{error}");
            webview.app_handle().exit(1);
        }
    }
}
pub fn result(
    app: &tauri::AppHandle,
    stage: &str,
    data: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let root = directory(app)?;
    match stage {
        "latency-inspect" => return Ok(app.state::<crate::terminal::Terminals>().smoke_sessions()),
        "latency-bootstrap" => {
            return Ok(
                serde_json::json!({"directory":root,"token":std::env::var("LOMI_TERMINAL_LATENCY_TOKEN").unwrap(), "rendererMode": if std::env::var("LOMI_TERMINAL_LATENCY_RENDERER").as_deref() == Ok("dom") { "dom" } else { "baseline" }}),
            )
        }
        "latency-checkpoint" | "passed" | "failed" => {
            let name = if stage == "latency-checkpoint" {
                "checkpoint.json"
            } else {
                "result.json"
            };
            let temporary = root.join(format!("{name}.tmp"));
            std::fs::write(
                &temporary,
                serde_json::to_vec_pretty(&serde_json::json!({"stage":stage,"data":data})).unwrap(),
            )
            .map_err(|e| e.to_string())?;
            std::fs::rename(temporary, root.join(name)).map_err(|e| e.to_string())?;
            if stage != "latency-checkpoint" {
                app.exit(if stage == "passed" { 0 } else { 1 });
            }
        }
        _ => return Err("Unknown terminal latency stage".into()),
    }
    Ok(serde_json::Value::Null)
}
#[tauri::command]
pub async fn terminal_latency_input(
    window: Window,
    token: String,
    text: String,
    mode: Option<String>,
) -> Result<serde_json::Value, String> {
    directory(window.app_handle())?;
    if window.label() != "main"
        || Some(token) != std::env::var("LOMI_TERMINAL_LATENCY_TOKEN").ok()
        || text.len() > 4096
        || text.contains('\0')
    {
        return Err("Invalid terminal latency fixture input".into());
    }
    let mode = mode.as_deref().unwrap_or("keys");
    if !matches!(mode, "keys" | "ime")
        || (mode == "keys"
            && !text
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == 0x15))
    {
        return Err("Native key fixture accepts alphanumeric markers and Ctrl+U only".into());
    }
    let ime = mode == "ime";
    #[cfg(target_os = "linux")]
    {
        use gtk::{gdk, glib::translate::*, prelude::*};
        use webkit2gtk::WebViewExt;
        linux_input::with_main(&window, move |widget| {
            if !widget.has_focus() { return Err("Main WebKit widget lacks keyboard focus".into()); }
            let before = gtk::glib::monotonic_time();
            if ime {
                let context = widget.input_method_context().ok_or("Missing WebKit IM context")?;
                context.emit_by_name::<()>("committed", &[&text]);
            } else {
                let native = widget.window().ok_or("WebKit widget has no GDK window")?;
                let display = native.display();
                let keyboard = display.default_seat().and_then(|seat| seat.keyboard()).ok_or("Missing native keyboard device")?;
                let keymap = gdk::Keymap::for_display(&display).ok_or("Missing native GDK keymap")?;
                for character in text.chars() {
                    let control = character == '\u{15}';
                    let key = gdk::keys::Key::from_unicode(if control { 'u' } else { character });
                    let mut entries = keymap.entries_for_keyval(*key);
                    entries.sort_by_key(|entry| (entry.group() != 0, entry.level()));
                    let mapping = entries.into_iter().find(|entry| (0..=1).contains(&entry.level())).ok_or("Marker key is absent from the simple native keymap")?;
                    let mut state = if control { gdk::ModifierType::CONTROL_MASK } else { gdk::ModifierType::empty() };
                    if mapping.level() == 1 { state |= gdk::ModifierType::SHIFT_MASK; }
                    let content = character.to_string();
                    for kind in [gdk::EventType::KeyPress, gdk::EventType::KeyRelease] {
                        let mut event = gdk::Event::new(kind);
                        event.set_device(Some(&keyboard));
                        // The allocated event variant owns its retained window and GLib string.
                        unsafe {
                            let raw: *mut gdk::ffi::GdkEvent = event.to_glib_none_mut().0;
                            let value = &mut (*raw).key;
                            value.window = native.to_glib_full();
                            value.send_event = 1;
                            value.time = (gtk::glib::monotonic_time() / 1000) as u32;
                            value.state = state.into_glib();
                            value.keyval = *key;
                            value.length = content.len() as i32;
                            value.string = content.as_str().to_glib_full();
                            value.hardware_keycode = u16::try_from(mapping.keycode()).map_err(|_| "Native keycode exceeds u16")?;
                            value.group = u8::try_from(mapping.group()).map_err(|_| "Native keyboard group exceeds u8")?;
                            value.is_modifier = 0;
                        }
                        // Normal GTK routing for this owned widget only, never the global queue.
                        gtk::main_do_event(&mut event);
                    }
                }
            }
            let after = gtk::glib::monotonic_time();
            let frames = widget.frame_clock().map(|clock| {
                let last = clock.frame_counter();
                ((last - 8).max(0)..=last).filter_map(|i| clock.timings(i)).map(|timing| serde_json::json!({
                    "frame":timing.frame_counter(), "presentationTimeUs":timing.presentation_time().map(|n|n.get())
                })).collect::<Vec<_>>()
            }).unwrap_or_default();
            Ok(serde_json::json!({"dispatchStartUs":before,"dispatchEndUs":after,"gtkRecentFrames":frames,"inputKind":if ime { "native-widget-IME-commit" } else { "native-widget-GDK-keypress-release" }}))
        }).await
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = ime;
        Err("Terminal latency native input requires Linux WebKitGTK".into())
    }
}

/// Native WebKit snapshot, not a compositor screenshot or input-to-photon probe.
#[tauri::command]
pub async fn terminal_latency_snapshot(
    window: Window,
    token: String,
    checkpoint: String,
) -> Result<serde_json::Value, String> {
    let root = directory(window.app_handle())?;
    if window.label() != "main"
        || Some(token) != std::env::var("LOMI_TERMINAL_LATENCY_TOKEN").ok()
        || !matches!(
            checkpoint.as_str(),
            "first-input-next-frame"
                | "first-input-two-frames"
                | "first-input-100ms"
                | "first-input-500ms"
                | "first-steady"
                | "after-visible-8"
        )
    {
        return Err("Invalid terminal latency snapshot checkpoint".into());
    }
    #[cfg(target_os = "linux")]
    {
        use webkit2gtk::WebViewExt;
        let path = root.join(format!("snapshot-{checkpoint}.png"));
        let (send, receive) = tokio::sync::oneshot::channel();
        linux_input::with_main(&window, move |widget| {
            let started = gtk::glib::monotonic_time();
            widget.snapshot(
                webkit2gtk::SnapshotRegion::Visible,
                webkit2gtk::SnapshotOptions::NONE,
                None::<&gtk::gio::Cancellable>,
                move |surface| {
                    let captured = gtk::glib::monotonic_time();
                    let result = (|| {
                        let surface = surface.map_err(|e| e.to_string())?;
                        let image = gtk::cairo::ImageSurface::try_from(surface).map_err(|_| {
                            "WebKit snapshot did not return a Cairo image surface".to_string()
                        })?;
                        let width = image.width();
                        let height = image.height();
                        let pixels = gtk::gdk::pixbuf_get_from_surface(&image, 0, 0, width, height)
                            .ok_or("Could not read native WebKit snapshot surface")?;
                        pixels.savev(&path, "png", &[]).map_err(|e| e.to_string())?;
                        let saved = gtk::glib::monotonic_time();
                        Ok(serde_json::json!({
                            "checkpoint":checkpoint, "path":path, "width":width, "height":height,
                            "nativeSnapshotStartUs":started, "nativeSnapshotCallbackUs":captured,
                            "nativePngSavedUs":saved,
                            "nativeSnapshotDurationMs":(captured-started) as f64 / 1000.0,
                            "nativePngDurationMs":(saved-captured) as f64 / 1000.0,
                            "captureKind":"WebKitGTK-visible-snapshot",
                            "physicalCompositorPresentationVerified":false
                        }))
                    })();
                    let _ = send.send(result);
                },
            );
            Ok(())
        })
        .await?;
        tokio::time::timeout(std::time::Duration::from_secs(20), receive)
            .await
            .map_err(|_| "Native WebKit snapshot timed out after 20 seconds".to_string())?
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = root;
        Err("Terminal latency snapshots require Linux WebKitGTK".into())
    }
}

/// Hold only this fixture's real session mutex, keeping the GTK event loop free.
#[tauri::command]
pub async fn terminal_latency_hold_session(
    window: Window,
    token: String,
) -> Result<serde_json::Value, String> {
    let root = directory(window.app_handle())?;
    if window.label() != "main" || Some(token) != std::env::var("LOMI_TERMINAL_LATENCY_TOKEN").ok()
    {
        return Err("Invalid terminal latency session-lock caller".into());
    }
    let app = window.app_handle().clone();
    let (ready, receive) = tokio::sync::oneshot::channel();
    // Tokio JoinHandle drop detaches rather than cancels this blocking task.
    // Completion is independently recorded in the owned artifact directory.
    let _worker = tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::files::SessionFile>();
        let guard = match state.0.lock() {
            Ok(guard) => guard,
            Err(error) => {
                let _ = ready.send(Err(error.to_string()));
                return;
            }
        };
        let acquired = std::time::Instant::now();
        let acquired_unix_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64;
        if ready
            .send(Ok(
                serde_json::json!({"holdMs":3000,"acquiredUnixUs":acquired_unix_us}),
            ))
            .is_err()
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_secs(3));
        let held_ms = acquired.elapsed().as_secs_f64() * 1000.0;
        drop(guard);
        let _ = std::fs::write(
            root.join("session-lock-release.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "acquiredUnixUs":acquired_unix_us,"requestedHoldMs":3000,
                "heldMs":held_ms,"released":true
            }))
            .unwrap(),
        );
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), receive)
        .await
        .map_err(|_| "Timed out waiting for the fixture session mutex".to_string())?
        .map_err(|error| error.to_string())?
}
