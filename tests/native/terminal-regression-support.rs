use serde_json::{json, Value};
use std::{path::PathBuf, time::Instant};
use tauri::{Emitter, Manager};
#[cfg(windows)]
#[path = "android-windows-input.rs"]
#[allow(dead_code)]
mod windows_input;
#[cfg(windows)]
#[path = "terminal-regression-windows-input.rs"]
mod windows_return;

fn isolated(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let identifier =
        std::env::var("LOMI_TERMINAL_REGRESSION_IDENTIFIER").map_err(|e| e.to_string())?;
    if !identifier.starts_with("dev.lomi.terminal-regression-")
        || app.config().identifier != identifier
    {
        return Err("Terminal regression requires its unique build-time fixture identity".into());
    }
    let expected = PathBuf::from(
        std::env::var("LOMI_TERMINAL_REGRESSION_APP_DATA").map_err(|e| e.to_string())?,
    );
    let actual = app.path().app_data_dir().map_err(|e| e.to_string())?;
    if std::fs::canonicalize(actual).map_err(|e| e.to_string())?
        != std::fs::canonicalize(expected).map_err(|e| e.to_string())?
    {
        return Err("Terminal regression application data directory mismatch".into());
    }
    let root = PathBuf::from(
        std::env::var("LOMI_TERMINAL_REGRESSION_DIRECTORY").map_err(|e| e.to_string())?,
    );
    if !root.is_absolute() {
        return Err("Terminal regression root must be absolute".into());
    }
    std::fs::canonicalize(root).map_err(|e| e.to_string())
}

pub fn startup_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("terminal-regression-startup")
        .js_init_script(
            r#"
if (window.top === window) {
  const clip = value => String(value ?? '').slice(0, 1000);
  let count = 0;
  let animationFrames = 0;
  const frameDeadline = Date.now() + 11000;
  const frame = () => {
    animationFrames++;
    if (Date.now() < frameDeadline && animationFrames < 2000) requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
  const send = data => {
    if (++count > 24) return;
    const attempt = remaining => {
      const invoke = window.__TAURI_INTERNALS__?.invoke;
      if (invoke) {
        invoke('plugin_smoke_result', {stage: 'regression-startup', data}).catch(() => {});
      } else if (remaining > 0) {
        setTimeout(() => attempt(remaining - 1), 20);
      }
    };
    attempt(75);
  };
  const status = event => {
    const sampled = event === 'status-1500ms' || event === 'status-10000ms';
    const root = document.querySelector('#root');
    const data = {
      event,
      readyState: document.readyState,
      visibilityState: document.visibilityState,
      hasFocus: document.hasFocus(),
      animationFrames,
      rootTextLength: root?.textContent?.length ?? 0,
      scripts: Array.from(document.scripts).slice(0, 8).map(script => script.src.slice(0, 200))
    };
    if (sampled) {
      data.rootText = clip(root?.textContent);
      const invoke = window.__TAURI_INTERNALS__?.invoke;
      if (invoke) {
        invoke('plugin_smoke_result', {stage: 'regression-window-state', data: null})
          .then(nativeWindowState => send({...data, nativeWindowState}))
          .catch(error => send({...data, nativeWindowStateError: clip(error)}));
        return;
      }
    }
    send(data);
  };
  window.addEventListener('error', event => send({
    event: 'error', message: clip(event.message), stack: clip(event.error?.stack),
    filename: clip(event.filename || event.target?.src), line: event.lineno, column: event.colno
  }), true);
  window.addEventListener('unhandledrejection', event => send({
    event: 'unhandledrejection', message: clip(event.reason), stack: clip(event.reason?.stack)
  }));
  document.addEventListener('DOMContentLoaded', () => status('DOMContentLoaded'), {once: true});
  window.addEventListener('load', () => status('load'), {once: true});
  status('script-init');
  setTimeout(() => status('status-1500ms'), 1500);
  setTimeout(() => status('status-10000ms'), 10000);
}
"#,
        )
        .build()
}

fn startup(app: &tauri::AppHandle, data: Value) -> Result<(), String> {
    use std::io::Write;
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let root = isolated(app)?;
    let phase = std::env::var("LOMI_TERMINAL_REGRESSION_PHASE").map_err(|e| e.to_string())?;
    if !matches!(phase.as_str(), "exercise" | "restore") {
        return Err("Invalid fixture phase".into());
    }
    let mut line = serde_json::to_vec(&data).map_err(|e| e.to_string())?;
    if line.len() > 4096 {
        return Err("Startup diagnostic exceeds 4 KB".into());
    }
    line.push(b'\n');
    let _guard = LOCK.lock().map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join(format!("{phase}-startup.jsonl")))
        .map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() + line.len() as u64 > 65536 {
        return Err("Startup diagnostic exceeds 64 KB".into());
    }
    file.write_all(&line).map_err(|e| e.to_string())
}

pub fn page(webview: &tauri::Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if webview.label() != "main" {
        return;
    }
    let event = match payload.event() {
        tauri::webview::PageLoadEvent::Started => "native-started",
        tauri::webview::PageLoadEvent::Finished => "native-finished",
    };
    if let Err(error) = startup(
        webview.app_handle(),
        json!({"event":event,"url":payload.url().as_str().chars().take(1000).collect::<String>()}),
    ) {
        eprintln!("{error}");
    }
}

async fn native_text(app: &tauri::AppHandle, text: String) -> Result<Value, String> {
    if text.len() > 256
        || text.is_empty()
        || !text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"# \r".contains(&b))
    {
        return Err("Native fixture input accepts bounded ASCII markers and return only".into());
    }
    let window = app.get_window("main").ok_or("Missing main window")?;
    let started = Instant::now();
    #[cfg(target_os = "macos")]
    {
        use objc2::{msg_send, runtime::AnyObject, sel};
        use objc2_foundation::{NSRange, NSString};
        let (tx, rx) = tokio::sync::oneshot::channel();
        let target = window.clone();
        window.run_on_main_thread(move || {
            let result: Result<bool, String> = (|| {
                let native = target.ns_window().map_err(|e| e.to_string())?;
                let native = unsafe { &*native.cast::<AnyObject>() };
                let key: bool = unsafe { msg_send![native, isKeyWindow] };
                let responder: *mut AnyObject = unsafe { msg_send![native, firstResponder] };
                if responder.is_null() { return Err("Missing native text responder".into()); }
                let supported: bool = unsafe { msg_send![responder, respondsToSelector: sel!(insertText:replacementRange:)] };
                let view: bool = unsafe { msg_send![responder, respondsToSelector: sel!(isDescendantOf:)] };
                if !supported || !view { return Err("Expected owned WebKit text responder".into()); }
                let content: *mut AnyObject = unsafe { msg_send![native, contentView] };
                let owned: bool = unsafe { msg_send![responder, isDescendantOf: content] };
                let owner: *mut AnyObject = unsafe { msg_send![responder, window] };
                if !owned || owner != native as *const _ as *mut _ { return Err("Native responder is outside the owned window".into()); }
                let value = NSString::from_str(&text);
                unsafe { let _: () = msg_send![responder, insertText: &*value, replacementRange: NSRange::new(isize::MAX as usize, 0)]; }
                Ok(key)
            })();
            let _ = tx.send(result);
        }).map_err(|e| e.to_string())?;
        let key = tokio::time::timeout(std::time::Duration::from_secs(4), rx)
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())??;
        return Ok(
            json!({"inputKind":"owned-AppKit-firstResponder-insertText", "keyWindow":key, "dispatchDurationMs": started.elapsed().as_secs_f64()*1000.0}),
        );
    }
    #[cfg(windows)]
    {
        let enter = text == "\r";
        if text.contains('\r') && !enter {
            return Err("Native Return must be dispatched separately".into());
        }
        tauri::async_runtime::spawn_blocking(move || {
            if enter {
                windows_return::enter(&window)
            } else {
                windows_input::text(&window, "commit", &text)
            }
        })
        .await
        .map_err(|e| e.to_string())??;
        return Ok(
            json!({"inputKind":if enter { "owned-foreground-HWND-VK_RETURN-keydown-keyup" } else { "owned-foreground-HWND-WM_CHAR" }, "dispatchDurationMs": started.elapsed().as_secs_f64()*1000.0}),
        );
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (window, started);
        Err("This regression fixture requires native macOS or Windows".into())
    }
}

pub async fn result(app: &tauri::AppHandle, stage: &str, data: Value) -> Result<Value, String> {
    let root = isolated(app)?;
    let phase = std::env::var("LOMI_TERMINAL_REGRESSION_PHASE").map_err(|e| e.to_string())?;
    if !matches!(phase.as_str(), "exercise" | "restore") {
        return Err("Invalid fixture phase".into());
    }
    match stage {
        "regression-startup" => {
            startup(app, data)?;
            Ok(Value::Null)
        }
        "regression-present" => {
            let window = app.get_window("main").ok_or("Missing fixture window")?;
            window.show().map_err(|e| e.to_string())?;
            window.set_focus().map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "regression-window-state" => {
            #[cfg(target_os = "macos")]
            {
                use objc2::{msg_send, runtime::AnyObject};
                use objc2_app_kit::NSView;
                let window = app.get_window("main").ok_or("Missing fixture window")?;
                let webview = app.get_webview("main").ok_or("Missing fixture webview")?;
                let (tx, rx) = tokio::sync::oneshot::channel();
                webview.with_webview(move |platform| {
                    let result: Result<Value, String> = (|| {
                        let native = window.ns_window().map_err(|e| e.to_string())?;
                        let native = unsafe { &*native.cast::<AnyObject>() };
                        let visible: bool = unsafe { msg_send![native, isVisible] };
                        let key: bool = unsafe { msg_send![native, isKeyWindow] };
                        let miniaturized: bool = unsafe { msg_send![native, isMiniaturized] };
                        let occlusion: usize = unsafe { msg_send![native, occlusionState] };
                        let number: isize = unsafe { msg_send![native, windowNumber] };
                        let ns_app: *mut AnyObject = unsafe { msg_send![objc2::class!(NSApplication), sharedApplication] };
                        let app_active: bool = unsafe { msg_send![ns_app, isActive] };
                        let activation_policy: isize = unsafe { msg_send![ns_app, activationPolicy] };
                        let nsview = unsafe { &*platform.inner().cast::<NSView>() };
                        let frame = nsview.frame();
                        let bounds = nsview.bounds();
                        let actual_number = nsview.window().map(|owner| owner.windowNumber());
                        Ok(json!({
                            "visible":visible,"keyWindow":key,"miniaturized":miniaturized,
                            "occlusionState":occlusion,"windowNumber":number,
                            "appActive":app_active,"activationPolicy":activation_policy,
                            "mainWebview": {
                                "hiddenOrHasHiddenAncestor":nsview.isHiddenOrHasHiddenAncestor(),
                                "frame":{"width":frame.size.width,"height":frame.size.height},
                                "bounds":{"width":bounds.size.width,"height":bounds.size.height},
                                "alphaValue":nsview.alphaValue(),
                                "actualNSWindowNumber":actual_number,
                                "matchesMainWindow":actual_number == Some(number),
                                "isAttachedToWindow":actual_number.is_some()
                            }
                        }))
                    })();
                    let _ = tx.send(result);
                }).map_err(|e| e.to_string())?;
                return tokio::time::timeout(std::time::Duration::from_secs(4), rx)
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
            }
            #[cfg(not(target_os = "macos"))]
            Ok(Value::Null)
        }
        "regression-bootstrap" => {
            #[cfg(target_os = "macos")]
            app.get_window("main")
                .ok_or("Missing fixture window")?
                .set_always_on_top(true)
                .map_err(|error| error.to_string())?;
            Ok(
                json!({"directory":dunce::simplified(&root),"phase":phase,"node":std::env::var("LOMI_TERMINAL_REGRESSION_NODE").map_err(|e| e.to_string())?,"platform":std::env::consts::OS}),
            )
        }
        "regression-inspect" => {
            let pids = app.state::<crate::terminal::Terminals>().smoke_sessions();
            for pid in pids.as_object().ok_or("Missing PTY process map")?.values() {
                let pid = pid.as_u64().ok_or("Missing PTY PID")? as u32;
                #[cfg(unix)]
                if unsafe { libc::kill(pid as i32, 0) } != 0 {
                    return Err(format!("PTY PID {pid} is not alive"));
                }
                #[cfg(windows)]
                {
                    use windows_sys::Win32::Foundation::CloseHandle;
                    use windows_sys::Win32::System::Threading::{
                        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
                    };
                    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
                    if handle.is_null() {
                        return Err(format!("Cannot verify PTY PID {pid}"));
                    }
                    let mut code = 0;
                    let read = unsafe { GetExitCodeProcess(handle, &mut code) };
                    unsafe {
                        CloseHandle(handle);
                    }
                    if read == 0 || code != 259 {
                        return Err(format!("PTY PID {pid} is not alive"));
                    }
                }
            }
            Ok(pids)
        }
        "regression-input" => native_text(app, data.as_str().ok_or("Expected text")?.into()).await,
        "regression-workload" => {
            let token = data.as_str().ok_or("Expected workload token")?;
            if !token.starts_with('w')
                || token.len() > 10
                || !token.bytes().all(|b| b.is_ascii_alphanumeric())
            {
                return Err("Invalid workload token".into());
            }
            let raw = std::fs::read(root.join("project").join(format!("{token}.json")))
                .map_err(|e| e.to_string())?;
            serde_json::from_slice(&raw).map_err(|e| e.to_string())
        }
        "regression-reopen" => {
            if !cfg!(target_os = "macos") || phase != "exercise" {
                return Err("Native reopen requires the macOS exercise phase".into());
            }
            let window = app.get_window("main").ok_or("Missing owned main window")?;
            window.show().map_err(|e| e.to_string())?;
            window.set_focus().map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "regression-checkpoint" | "ready-to-quit" | "failed" => {
            crate::native_smoke::terminal_output::write(&root, &phase)?;
            let name = format!("{phase}-{stage}.json");
            std::fs::write(
                root.join(name),
                serde_json::to_vec_pretty(&json!({"stage":stage,"data":data})).unwrap(),
            )
            .map_err(|e| e.to_string())?;
            // The frontend emits the quit event after this durable receipt. Never
            // exit here: Workbench must run its real save and close guards.
            if stage == "failed" {
                app.emit_to("main", "lomi-quit-requested", ())
                    .map_err(|e| e.to_string())?;
            }
            Ok(Value::Null)
        }
        _ => Err("Unknown terminal regression stage".into()),
    }
}
