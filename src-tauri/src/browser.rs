use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Mutex};
use tauri::{
    webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, WebviewBuilder},
    Emitter, Manager, Webview, WebviewUrl, Window,
};

#[cfg(target_os = "macos")]
pub(crate) mod agent_capture;
#[cfg(target_os = "macos")]
pub(crate) mod agent_dom;
#[cfg(target_os = "macos")]
mod agent_navigation;
#[cfg(target_os = "macos")]
pub(crate) mod agent_permissions;
#[cfg(target_os = "macos")]
pub(crate) mod native_input;
pub mod servers;

#[derive(Default)]
pub struct Browsers {
    creation: tauri::async_runtime::Mutex<()>,
    pages: Mutex<HashMap<String, Page>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    id: String,
    revision: String,
    url: String,
    title: String,
    loading: bool,
    error: String,
    download: String,
    browser_generation: Option<String>,
    profile_id: Option<String>,
    navigation_id: Option<String>,
    agent_controlled: bool,
    #[cfg(unix)]
    #[serde(skip)]
    control: Option<std::sync::Arc<lomi_control_core::browser::BrowserControl>>,
    #[serde(skip)]
    bounds: Option<Bounds>,
}

#[derive(Clone, Copy, Deserialize, PartialEq)]
pub struct Bounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Slot {
    id: String,
    #[serde(default)]
    hidden: bool,
    url: String,
    bounds: Bounds,
    automation: Option<Automation>,
    agent_ticket: Option<AgentTicket>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Automation {
    generation: String,
    profile_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTicket {
    operation_id: String,
    nonce: String,
}

pub fn trusted_app_view(view: &str, window: &str) -> bool {
    matches!(view, "main" | "settings") && view == window
}

fn label(id: &str) -> Result<String, String> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("Invalid browser panel ID.".into());
    }
    Ok(format!("browser-{id}"))
}

fn address(value: &str) -> Result<tauri::Url, String> {
    lomi_control_protocol::browser::address(value).map_err(str::to_owned)
}

fn emit(app: &tauri::AppHandle, event: &str, payload: impl Serialize + Clone) {
    let _ = app.emit_to(
        tauri::EventTarget::Webview {
            label: "main".into(),
        },
        event,
        payload,
    );
}

static PAGE_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
fn next_page_revision() -> String {
    PAGE_REVISION
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        .to_string()
}

fn update(app: &tauri::AppHandle, id: &str, change: impl FnOnce(&mut Page)) {
    let state = app.state::<Browsers>();
    let page = state.pages.lock().ok().and_then(|mut pages| {
        let page = pages.get_mut(id)?;
        change(page);
        page.revision = next_page_revision();
        #[cfg(unix)]
        {
            page.agent_controlled = page.control.as_ref().is_some_and(|c| c.authorized());
        }
        Some(page.clone())
    });
    if let Some(page) = page {
        emit(app, "browser-page", page);
    }
}

pub fn refresh_control_state(app: &tauri::AppHandle) {
    let ids: Vec<_> = app
        .state::<Browsers>()
        .pages
        .lock()
        .map(|pages| {
            pages
                .values()
                .filter(|p| p.browser_generation.is_some())
                .map(|p| p.id.clone())
                .collect()
        })
        .unwrap_or_default();
    for id in ids {
        update(app, &id, |_| {});
    }
}

fn view(app: &tauri::AppHandle, id: &str) -> Result<Webview, String> {
    app.get_webview(&label(id)?)
        .ok_or_else(|| "Browser panel is closed.".into())
}

#[cfg(target_os = "macos")]
pub fn close_controlled(
    app: &tauri::AppHandle,
    control: std::sync::Arc<lomi_control_core::browser::BrowserControl>,
) -> Result<(), lomi_control_protocol::ErrorCode> {
    use lomi_control_protocol::ErrorCode;
    if !control.authorized() {
        return Err(ErrorCode::ControlRevoked);
    }
    {
        let state = app.state::<Browsers>();
        let pages = state.pages.lock().map_err(|_| ErrorCode::AppUnavailable)?;
        let page = pages
            .get(&control.panel_id)
            .ok_or(ErrorCode::TargetNotFound)?;
        if page
            .control
            .as_ref()
            .is_none_or(|c| !std::sync::Arc::ptr_eq(c, &control))
        {
            return Err(ErrorCode::StaleGeneration);
        }
    }
    let webview = view(app, &control.panel_id).map_err(|_| ErrorCode::TargetNotFound)?;
    let target = webview.clone();
    let app = app.clone();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    app.clone()
        .run_on_main_thread(move || {
            let result = (|| {
                if std::time::Instant::now() >= deadline {
                    return Err(ErrorCode::DeadlineExceeded);
                }
                if !control.authorized() {
                    return Err(ErrorCode::ControlRevoked);
                }
                target.close().map_err(|_| ErrorCode::OutcomeUnknown)?;
                control.revoke();
                native_input::forget(&app, target.label().to_owned());
                app.state::<Browsers>()
                    .pages
                    .lock()
                    .map_err(|_| ErrorCode::OutcomeUnknown)?
                    .remove(&control.panel_id);
                Ok(())
            })();
            let _ = send.send(result);
        })
        .map_err(|_| ErrorCode::AppUnavailable)?;
    receive
        .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
        .map_err(|_| ErrorCode::OutcomeUnknown)?
}

const SCRIPT: &str = r#"(() => {
  const send = signal => window.__TAURI_INTERNALS__.invoke('plugin:browser|signal', { signal }).catch(() => {});
  addEventListener('focus', () => send('focus'));
  addEventListener('pointerdown', () => send('focus'), true);
  addEventListener('keydown', event => {
    if (!event.isTrusted || event.isComposing || event.altKey) return;
    if ((event.ctrlKey || event.metaKey) && !event.shiftKey) {
      const signal = { l: 'address', w: 'close', f: 'find' }[event.key.toLowerCase()];
      if (signal) { event.preventDefault(); send(signal); }
    }
  }, true);
  for (const name of ['pushState', 'replaceState']) {
    const original = history[name];
    history[name] = function(...args) { const result = original.apply(this, args); send('changed'); return result; };
  }
  addEventListener('popstate', () => send('changed'));
  addEventListener('hashchange', () => send('changed'));
})();"#;

async fn create(window: &Window, slot: &Slot) -> Result<Webview, String> {
    let app = window.app_handle();
    let url = address(&slot.url)?;
    if slot.hidden && slot.automation.is_none() {
        return Err("Hidden creation requires an authorized agent operation.".into());
    }
    if slot.automation.is_some() && !crate::agent_control::supported_host() {
        return Err("Browser automation is not qualified on this host.".into());
    }
    #[cfg(unix)]
    let control = if let Some(automation) = &slot.automation {
        let ticket = slot
            .agent_ticket
            .as_ref()
            .ok_or("This isolated browser session expired. Create a new agent panel.")?;
        let broker = app.state::<crate::agent_control::Control>().required()?;
        let (operation, nonce, panel, generation, profile, url) = (
            ticket.operation_id.clone(),
            ticket.nonce.clone(),
            slot.id.clone(),
            automation.generation.clone(),
            automation.profile_id.clone(),
            slot.url.clone(),
        );
        let visible = !slot.hidden;
        Some(
            tauri::async_runtime::spawn_blocking(move || {
                broker.authorize_browser_start(lomi_control_core::broker::BrowserStart {
                    operation: &operation,
                    visible,
                    nonce: &nonce,
                    panel: &panel,
                    generation: &generation,
                    profile: &profile,
                    url: &url,
                })
            })
            .await
            .map_err(|e| e.to_string())??,
        )
    } else {
        if slot.agent_ticket.is_some() {
            return Err("Browser ticket requires its isolated descriptor.".into());
        }
        None
    };
    let page = Page {
        id: slot.id.clone(),
        revision: next_page_revision(),
        url: url.to_string(),
        title: "Browser".into(),
        loading: url.as_str() != "about:blank",
        error: String::new(),
        download: String::new(),
        browser_generation: slot.automation.as_ref().map(|a| a.generation.clone()),
        profile_id: slot.automation.as_ref().map(|a| a.profile_id.clone()),
        navigation_id: None,
        agent_controlled: slot.automation.is_some(),
        #[cfg(unix)]
        control: control.clone(),
        bounds: None,
    };
    app.state::<Browsers>()
        .pages
        .lock()
        .map_err(|e| e.to_string())?
        .insert(slot.id.clone(), page);
    let id = slot.id.clone();
    let navigation_app = app.clone();
    let navigation_id = id.clone();
    let popup_app = app.clone();
    let popup_id = id.clone();
    let title_id = id.clone();
    let load_id = id.clone();
    let download_id = id.clone();
    #[cfg(unix)]
    let navigation_control = control.clone();
    #[cfg(unix)]
    let load_control = control.clone();
    let automated = slot.automation.is_some();
    let data_directory = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("browser-data");
    let initial_url = if automated {
        address("about:blank")?
    } else {
        url.clone()
    };
    let mut builder = WebviewBuilder::new(label(&id)?, WebviewUrl::External(initial_url))
        .focused(false)
        .zoom_hotkeys_enabled(true)
        .disable_drag_drop_handler()
        .data_directory(data_directory.clone())
        .data_store_identifier(*b"LomiBrowserWeb01")
        .initialization_script(SCRIPT)
        .on_navigation(move |url| {
            if address(url.as_str()).is_err() {
                return false;
            }
            #[cfg(unix)]
            if let Some(control) = &navigation_control {
                if !control.started() {
                    return false;
                }
                if !control.native_navigation(url.as_str()) {
                    update(&navigation_app, &navigation_id, |page| {
                        page.error = "Navigation blocked by agent browser permissions.".into()
                    });
                    return false;
                }
                update(&navigation_app, &navigation_id, |page| {
                    page.navigation_id = Some(control.navigation_id())
                });
            }
            update(&navigation_app, &navigation_id, |page| {
                page.url = url.to_string();
                page.error.clear();
            });
            true
        })
        .on_new_window(move |url, _| {
            if automated {
                return NewWindowResponse::Deny;
            }
            if address(url.as_str()).is_ok() {
                emit(
                    &popup_app,
                    "browser-open",
                    serde_json::json!({"id": popup_id, "url": url}),
                );
            }
            NewWindowResponse::Deny
        })
        .on_document_title_changed(move |webview, title| {
            update(webview.app_handle(), &title_id, |page| {
                page.title = title.chars().take(512).collect();
                if page.title.is_empty() {
                    page.title = "Browser".into();
                }
            });
        })
        .on_page_load(move |webview, payload| {
            #[cfg(unix)]
            if let Some(control) = &load_control {
                match payload.event() {
                    PageLoadEvent::Started => control.document_committed(payload.url().as_str()),
                    PageLoadEvent::Finished => control.document_loaded(payload.url().as_str()),
                }
            }
            update(webview.app_handle(), &load_id, |page| {
                if address(payload.url().as_str()).is_ok() {
                    page.url = payload.url().to_string();
                }
                page.loading = matches!(payload.event(), PageLoadEvent::Started);
                // The accepted navigation policy already clears automation errors.
                // A late commit callback must not erase a newer redirect denial.
                if page.loading && !automated {
                    page.error.clear();
                }
            });
        })
        .on_download(move |webview, event| {
            if automated {
                return false;
            }
            match event {
                DownloadEvent::Requested { destination, .. } => {
                    let Ok(folder) = webview.app_handle().path().download_dir() else {
                        return false;
                    };
                    let Some(name) = destination.file_name().map(|name| name.to_owned()) else {
                        return false;
                    };
                    if std::fs::create_dir_all(&folder).is_err() {
                        return false;
                    }
                    *destination = folder.join(name);
                    if destination.exists() {
                        return false;
                    }
                    update(webview.app_handle(), &download_id, |page| {
                        page.download = "Downloading…".into();
                    });
                }
                DownloadEvent::Finished { path, success, .. } => {
                    update(webview.app_handle(), &download_id, |page| {
                        page.download = if success {
                            format!(
                                "Saved to {}",
                                path.map(|p| p.display().to_string()).unwrap_or_default()
                            )
                        } else {
                            "Download failed.".into()
                        };
                    });
                }
                _ => {}
            }
            true
        });
    #[cfg(unix)]
    if let Some(control) = &control {
        if !control.authorized() {
            return Err("Browser control was revoked.".into());
        }
        builder = builder
            .data_directory(data_directory.join("automation").join(&control.profile_id))
            .data_store_identifier(control.profile_identifier());
    }
    // Creation must stay off the event thread on WebView2.
    let created = window
        .add_child(
            builder,
            tauri::LogicalPosition::new(slot.bounds.x, slot.bounds.y),
            tauri::LogicalSize::new(slot.bounds.width, slot.bounds.height),
        )
        .map_err(|e| e.to_string());
    let webview = match created {
        Ok(webview) => webview,
        Err(error) => {
            #[cfg(unix)]
            if let Some(control) = &control {
                control.revoke();
            }
            app.state::<Browsers>()
                .pages
                .lock()
                .map_err(|e| e.to_string())?
                .remove(&id);
            return Err(error);
        }
    };
    if slot.hidden {
        // Blank creation is hidden before installing delegates or starting HTTP.
        webview.hide().map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    if let Some(control) = control {
        if !control.authorized() {
            let _ = webview.close();
            return Err("Browser control was revoked during creation.".into());
        }
        #[cfg(target_os = "macos")]
        if let Err(error) = agent_permissions::install(&webview, control.profile_identifier()).await
        {
            control.revoke();
            let _ = webview.close();
            return Err(error);
        }
        #[cfg(target_os = "macos")]
        if let Err(error) = native_input::install(&webview, control.clone()).await {
            control.revoke();
            let _ = webview.close();
            return Err(error);
        }
        control.mark_started();
        if !control.permits(url.as_str()) || webview.navigate(url.clone()).is_err() {
            control.revoke();
            #[cfg(target_os = "macos")]
            native_input::forget(app, webview.label().to_owned());
            let _ = webview.close();
            return Err("Browser initial navigation did not start.".into());
        }
        update(app, &id, |page| {
            page.navigation_id = Some(control.navigation_id())
        });
    }
    #[cfg(target_os = "linux")]
    if let Err(error) = linux_attach(&webview) {
        let _ = webview.close();
        return Err(error);
    }
    Ok(webview)
}

#[tauri::command]
pub async fn sync_browsers(
    window: Window,
    retained: Vec<String>,
    slots: Vec<Slot>,
) -> Result<Vec<Page>, String> {
    crate::files::main_window(&window)?;
    for id in &retained {
        label(id)?;
    }
    for slot in &slots {
        if !retained.contains(&slot.id) {
            return Err("Unknown browser panel.".into());
        }
        address(&slot.url)?;
        let b = slot.bounds;
        if ![b.x, b.y, b.width, b.height]
            .iter()
            .all(|n| n.is_finite() && *n >= 0.0 && *n <= 100_000.0)
            || b.width < 1.0
            || b.height < 1.0
        {
            return Err("Invalid browser bounds.".into());
        }
    }
    let app = window.app_handle();
    let state = app.state::<Browsers>();
    let _creation = state.creation.lock().await;
    let ids: Vec<_> = state
        .pages
        .lock()
        .map_err(|e| e.to_string())?
        .keys()
        .cloned()
        .collect();
    for id in ids {
        if !retained.contains(&id) {
            #[cfg(target_os = "macos")]
            native_input::forget(app, label(&id)?);
            #[cfg(unix)]
            if let Some(control) = state
                .pages
                .lock()
                .ok()
                .and_then(|pages| pages.get(&id)?.control.clone())
            {
                control.revoke();
            }
            if let Ok(webview) = view(app, &id) {
                webview.close().map_err(|e| e.to_string())?;
            }
            state.pages.lock().map_err(|e| e.to_string())?.remove(&id);
        } else if !slots.iter().any(|slot| slot.id == id) {
            let visible = state
                .pages
                .lock()
                .map_err(|e| e.to_string())?
                .get_mut(&id)
                .and_then(|page| page.bounds.take())
                .is_some();
            if visible {
                view(app, &id)?.hide().map_err(|e| e.to_string())?;
            }
        }
    }
    for slot in slots {
        if let Some(page) = state.pages.lock().map_err(|e| e.to_string())?.get(&slot.id) {
            if page.browser_generation.as_deref()
                != slot.automation.as_ref().map(|a| a.generation.as_str())
                || page.profile_id.as_deref()
                    != slot.automation.as_ref().map(|a| a.profile_id.as_str())
            {
                return Err("Browser generation or profile changed.".into());
            }
        }
        let webview = match view(app, &slot.id) {
            Ok(webview) => webview,
            Err(_) => create(&window, &slot).await?,
        };
        let previous = state
            .pages
            .lock()
            .map_err(|e| e.to_string())?
            .get(&slot.id)
            .and_then(|page| page.bounds);
        if slot.hidden {
            if previous.is_some() {
                webview.hide().map_err(|e| e.to_string())?;
                if let Some(page) = state
                    .pages
                    .lock()
                    .map_err(|e| e.to_string())?
                    .get_mut(&slot.id)
                {
                    page.bounds = None;
                }
            }
        } else if previous != Some(slot.bounds) {
            set_bounds(&webview, slot.bounds)?;
            webview.show().map_err(|e| e.to_string())?;
            if let Some(page) = state
                .pages
                .lock()
                .map_err(|e| e.to_string())?
                .get_mut(&slot.id)
            {
                page.bounds = Some(slot.bounds);
            }
        }
    }
    let pages = state
        .pages
        .lock()
        .map_err(|e| e.to_string())?
        .values()
        .cloned()
        .collect();
    Ok(pages)
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Action {
    TakeControl,
    Navigate { url: String },
    Back,
    Forward,
    Reload,
    Stop,
    Focus,
    Find { text: String, backwards: bool },
}

#[tauri::command]
pub async fn agent_browser_navigate(
    window: Window,
    operation_id: String,
    nonce: String,
) -> Result<lomi_control_protocol::control::BrowserNavigationResult, String> {
    crate::files::main_window(&window)?;
    #[cfg(target_os = "macos")]
    {
        use lomi_control_protocol::ErrorCode;
        let code = |e: ErrorCode| {
            serde_json::to_value(e)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        };
        let app = window.app_handle();
        let broker = app.state::<crate::agent_control::Control>().required()?;
        let result = async {
            let op = operation_id.clone();
            let dispatch_broker = broker.clone();
            let dispatch_nonce = nonce.clone();
            let lomi_control_core::broker::BrowserNavigationDispatch {
                control,
                url,
                workspace,
                wait,
                permit,
            } = tauri::async_runtime::spawn_blocking(move || {
                dispatch_broker.authorize_browser_navigation(&op, &dispatch_nonce)
            })
            .await
            .map_err(|_| ErrorCode::AppUnavailable)??;
            let registered = app
                .state::<Browsers>()
                .pages
                .lock()
                .ok()
                .and_then(|pages| pages.get(&control.panel_id)?.control.clone());
            if registered
                .as_ref()
                .is_none_or(|native| !std::sync::Arc::ptr_eq(native, &control))
                || !control.permits(&url)
            {
                control.fail_navigation(&operation_id, ErrorCode::ControlRevoked);
                return Err(ErrorCode::ControlRevoked);
            }
            let webview = view(app, &control.panel_id).map_err(|_| ErrorCode::TargetNotFound)?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            if let Err(error) = agent_navigation::start(
                &webview,
                control.clone(),
                operation_id.clone(),
                url,
                permit.clone(),
                deadline,
            )
            .await
            {
                control.fail_navigation(&operation_id, error);
                agent_navigation::stop(&webview, control.clone(), operation_id.clone());
                return Err(error);
            }
            loop {
                let observation = permit.check().and_then(|_| {
                    if std::time::Instant::now() >= deadline {
                        return Err(ErrorCode::DeadlineExceeded);
                    }
                    control.navigation_observation(&operation_id, wait)
                });
                let observation = match observation {
                    Ok(value) => value,
                    Err(error) => {
                        control.fail_navigation(&operation_id, error);
                        agent_navigation::stop(&webview, control.clone(), operation_id.clone());
                        return Err(error);
                    }
                };
                if let Some(observed) = observation {
                    return Ok(lomi_control_protocol::control::BrowserNavigationResult {
                        workspace_id: workspace,
                        panel_id: control.panel_id.clone(),
                        browser_generation: control.generation.clone(),
                        navigation_id: observed.navigation_id,
                        url: observed.url,
                        committed: observed.committed,
                        loaded: observed.loaded,
                    });
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        }
        .await;
        let final_result = result.clone();
        tauri::async_runtime::spawn_blocking(move || {
            broker.finish_browser_navigation(&operation_id, &nonce, final_result)
        })
        .await
        .map_err(|_| code(ErrorCode::StorageUnavailable))?
        .map_err(|_| code(ErrorCode::StorageUnavailable))?;
        result.map_err(code)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (operation_id, nonce);
        Err("HOST_UNQUALIFIED".into())
    }
}

#[tauri::command]
pub async fn browser_action(window: Window, id: String, action: Action) -> Result<(), String> {
    crate::files::main_window(&window)?;
    let webview = view(window.app_handle(), &id)?;
    #[cfg(unix)]
    if !matches!(action, Action::Focus | Action::Find { .. }) {
        let control = window
            .app_handle()
            .state::<Browsers>()
            .pages
            .lock()
            .ok()
            .and_then(|pages| pages.get(&id)?.control.clone());
        if let Some(control) = control {
            control.take_over();
            #[cfg(target_os = "macos")]
            native_input::forget(window.app_handle(), webview.label().to_owned());
            update(window.app_handle(), &id, |_| {});
        }
    }
    match action {
        Action::TakeControl => Ok(()),
        Action::Navigate { url } => webview.navigate(address(&url)?),
        Action::Back => webview.eval("history.back()"),
        Action::Forward => webview.eval("history.forward()"),
        Action::Reload => webview.reload(),
        Action::Stop => {
            update(window.app_handle(), &id, |page| {
                page.loading = false;
            });
            webview.eval("window.stop()")
        }
        Action::Focus => webview.set_focus(),
        Action::Find { text, backwards } => {
            if text.len() > 4096 {
                return Err("Search text is too long.".into());
            }
            webview.eval(format!(
                "window.find({}, false, {backwards}, true)",
                serde_json::to_string(&text).map_err(|e| e.to_string())?
            ))
        }
    }
    .map_err(|e| e.to_string())
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Signal {
    Focus,
    Address,
    Close,
    Find,
    Changed,
}

#[tauri::command]
pub async fn signal(webview: Webview, signal: Signal) -> Result<(), String> {
    let id = webview
        .label()
        .strip_prefix("browser-")
        .ok_or("Not a browser panel.")?;
    let app = webview.app_handle();
    if webview.window().label() != "main"
        || !app
            .state::<Browsers>()
            .pages
            .lock()
            .map_err(|e| e.to_string())?
            .contains_key(id)
    {
        return Err("Unknown browser panel.".into());
    }
    if matches!(signal, Signal::Changed) {
        if let Ok(url) = webview.url() {
            if address(url.as_str()).is_ok() {
                update(app, id, |page| {
                    #[cfg(unix)]
                    if let Some(control) = &page.control {
                        control.document_changed(url.as_str());
                        page.navigation_id = Some(control.navigation_id());
                    }
                    page.url = url.to_string();
                });
            }
        }
    } else {
        if app
            .state::<Browsers>()
            .pages
            .lock()
            .map_err(|e| e.to_string())?
            .get(id)
            .is_none_or(|page| page.bounds.is_none())
        {
            return Ok(());
        }
        if matches!(signal, Signal::Address | Signal::Find) {
            if let Some(main) = app.get_webview("main") {
                main.set_focus().map_err(|e| e.to_string())?;
            }
        }
        emit(
            app,
            "browser-signal",
            serde_json::json!({"id": id, "signal": signal}),
        );
    }
    Ok(())
}

fn set_bounds(webview: &Webview, bounds: Bounds) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    return webview
        .with_webview(move |platform| {
            use gtk::prelude::*;
            let widget = platform.inner();
            if let Some(parent) = widget
                .parent()
                .and_then(|p| p.downcast::<gtk::Fixed>().ok())
            {
                parent.move_(&widget, bounds.x.round() as i32, bounds.y.round() as i32);
                widget.set_size_request(bounds.width.round() as i32, bounds.height.round() as i32);
                widget.size_allocate(&gtk::Allocation::new(
                    bounds.x.round() as i32,
                    bounds.y.round() as i32,
                    bounds.width.round() as i32,
                    bounds.height.round() as i32,
                ));
            }
        })
        .map_err(|e| e.to_string());
    #[cfg(not(target_os = "linux"))]
    webview
        .set_bounds(tauri::Rect {
            position: tauri::LogicalPosition::new(bounds.x, bounds.y).into(),
            size: tauri::LogicalSize::new(bounds.width, bounds.height).into(),
        })
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
fn linux_attach(webview: &Webview) -> Result<(), String> {
    let app = webview.app_handle().clone();
    let id = webview.label().trim_start_matches("browser-").to_owned();
    // Wry packs GTK children vertically and ignores their bounds outside GtkFixed.
    // Keep the app expanding underneath a fixed overlay containing only browser views.
    webview
        .with_webview(move |platform| {
            use gtk::prelude::*;
            use webkit2gtk::WebViewExt;
            let widget = platform.inner();
            widget.set_widget_name(&format!("browser-{id}"));
            let Some(container) = widget.parent().and_then(|p| p.downcast::<gtk::Box>().ok())
            else {
                return;
            };
            let overlay = container
                .children()
                .into_iter()
                .find_map(|child| child.downcast::<gtk::Overlay>().ok())
                .unwrap_or_else(|| {
                    let overlay = gtk::Overlay::new();
                    let fixed = gtk::Fixed::new();
                    fixed.set_widget_name("browser-panels");
                    let resize_app = app.clone();
                    fixed.connect_size_allocate(move |fixed, _| {
                        // GTK's preferred WebKit size can retain the previous viewport.
                        // Reapply panel bounds after the container allocates its children.
                        let state = resize_app.state::<Browsers>();
                        let bounds: HashMap<_, _> = state
                            .pages
                            .lock()
                            .map(|pages| {
                                pages
                                    .iter()
                                    .filter_map(|(id, page)| {
                                        page.bounds.map(|bounds| (format!("browser-{id}"), bounds))
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        for child in fixed.children() {
                            if let Some(bounds) = bounds.get(child.widget_name().as_str()) {
                                child.size_allocate(&gtk::Allocation::new(
                                    bounds.x.round() as i32,
                                    bounds.y.round() as i32,
                                    bounds.width.round() as i32,
                                    bounds.height.round() as i32,
                                ));
                            }
                        }
                    });
                    overlay.add_overlay(&fixed);
                    // The full-window overlay must not intercept input outside its WebKit children.
                    overlay.set_overlay_pass_through(&fixed, true);
                    for child in container.children() {
                        if child != widget {
                            container.remove(&child);
                            overlay.add(&child);
                        }
                    }
                    container.pack_start(&overlay, true, true, 0);
                    fixed.show();
                    overlay.show();
                    overlay
                });
            if let Some(fixed) = overlay
                .children()
                .into_iter()
                .find_map(|child| child.downcast::<gtk::Fixed>().ok())
            {
                container.remove(&widget);
                fixed.put(&widget, 0, 0);
            }
            widget.connect_load_failed(move |_, _, _, error| {
                if error.matches(webkit2gtk::NetworkError::Cancelled) {
                    return false;
                }
                update(&app, &id, |page| {
                    page.loading = false;
                    page.error = error.to_string();
                });
                false
            });
        })
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_cannot_cross_the_application_boundary() {
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/browser.json")).unwrap();
        let patterns: Vec<tauri::utils::acl::RemoteUrlPattern> = capability["remote"]["urls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().parse().unwrap())
            .collect();
        for url in [
            "http://localhost:3000",
            "http://127.0.0.1:18765/path",
            "https://example.com",
            "https://example.com:8443/path",
        ] {
            assert!(
                patterns
                    .iter()
                    .any(|pattern| pattern.test(&url.parse().unwrap())),
                "{url}"
            );
        }
        assert!(!patterns
            .iter()
            .any(|pattern| pattern.test(&"file:///etc/passwd".parse().unwrap())));
        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,hello",
            "tauri://localhost",
            "http://tauri.localhost",
            "http://ipc.localhost",
            "http://theme.localhost",
            "https://plugin.localhost",
            "https://PLUGIN.LOCALHOST./asset.js",
            "http://child.plugin.localhost",
            "https://user:password@example.com",
        ] {
            assert!(address(url).is_err(), "{url}");
        }
        for url in [
            "about:blank",
            "https://example.com",
            "http://localhost:3000",
            "http://127.0.0.1:1420",
        ] {
            assert!(address(url).is_ok(), "{url}");
        }
        assert!(trusted_app_view("main", "main"));
        assert!(trusted_app_view("settings", "settings"));
        assert!(!trusted_app_view("browser-main", "main"));
        assert!(!trusted_app_view("settings", "main"));
        assert!(label("../main").is_err());
    }
}

#[cfg(any(feature = "native-smoke", feature = "mcp-probe"))]
pub fn smoke_pages(app: &tauri::AppHandle) -> serde_json::Value {
    let state = app.state::<Browsers>();
    let pages = state.pages.lock().unwrap();
    serde_json::json!(pages.iter().map(|(id,page)|(id.clone(),serde_json::json!({"url":page.url,"title":page.title,"error":page.error,"visible":page.bounds.is_some(),"bounds":page.bounds.map(|b|[b.x,b.y,b.width,b.height])}))).collect::<std::collections::BTreeMap<_,_>>())
}

#[tauri::command]
pub async fn agent_browser_interact(
    window: Window,
    operation_id: String,
    nonce: String,
) -> Result<lomi_control_protocol::control::BrowserInteractionResult, String> {
    crate::files::main_window(&window)?;
    #[cfg(target_os = "macos")]
    {
        let app = window.app_handle().clone();
        let broker = app.state::<crate::agent_control::Control>().required()?;
        tauri::async_runtime::spawn_blocking(move || {
            let (control, command, permit) =
                broker.authorize_browser_interaction(&operation_id, &nonce)?;
            agent_dom::interact(&app, control, &operation_id, command, permit)
        })
        .await
        .map_err(|_| "APP_UNAVAILABLE".to_owned())?
        .map_err(|e: lomi_control_protocol::ErrorCode| {
            serde_json::to_value(e)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (operation_id, nonce);
        Err("HOST_UNQUALIFIED".into())
    }
}
