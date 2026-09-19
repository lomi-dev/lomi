use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Mutex};
use tauri::{
    webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, WebviewBuilder},
    Emitter, Manager, Webview, WebviewUrl, Window,
};

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
    url: String,
    title: String,
    loading: bool,
    error: String,
    download: String,
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
pub struct Slot {
    id: String,
    url: String,
    bounds: Bounds,
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
    if value.len() > 16_384 {
        return Err("The address is too long.".into());
    }
    let url = tauri::Url::parse(value).map_err(|_| "Invalid web address.")?;
    let allowed = value == "about:blank"
        || (matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && !matches!(
                url.host_str(),
                Some("tauri.localhost" | "ipc.localhost" | "theme.localhost")
            )
            && url.username().is_empty()
            && url.password().is_none());
    if !allowed {
        return Err("Only HTTP and HTTPS pages can be opened.".into());
    }
    Ok(url)
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

fn update(app: &tauri::AppHandle, id: &str, change: impl FnOnce(&mut Page)) {
    let state = app.state::<Browsers>();
    let page = state.pages.lock().ok().and_then(|mut pages| {
        let page = pages.get_mut(id)?;
        change(page);
        Some(page.clone())
    });
    if let Some(page) = page {
        emit(app, "browser-page", page);
    }
}

fn view(app: &tauri::AppHandle, id: &str) -> Result<Webview, String> {
    app.get_webview(&label(id)?)
        .ok_or_else(|| "Browser panel is closed.".into())
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
    let page = Page {
        id: slot.id.clone(),
        url: url.to_string(),
        title: "Browser".into(),
        loading: url.as_str() != "about:blank",
        error: String::new(),
        download: String::new(),
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
    let builder = WebviewBuilder::new(label(&id)?, WebviewUrl::External(url))
        .focused(false)
        .zoom_hotkeys_enabled(true)
        .disable_drag_drop_handler()
        .data_directory(
            app.path()
                .app_data_dir()
                .map_err(|e| e.to_string())?
                .join("browser-data"),
        )
        .data_store_identifier(*b"SimpleBenchWeb01")
        .initialization_script(SCRIPT)
        .on_navigation(move |url| {
            if address(url.as_str()).is_err() {
                return false;
            }
            update(&navigation_app, &navigation_id, |page| {
                page.url = url.to_string();
                page.error.clear();
            });
            true
        })
        .on_new_window(move |url, _| {
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
            update(webview.app_handle(), &load_id, |page| {
                if address(payload.url().as_str()).is_ok() {
                    page.url = payload.url().to_string();
                }
                page.loading = matches!(payload.event(), PageLoadEvent::Started);
                if page.loading {
                    page.error.clear();
                }
            });
        })
        .on_download(move |webview, event| {
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
            app.state::<Browsers>()
                .pages
                .lock()
                .map_err(|e| e.to_string())?
                .remove(&id);
            return Err(error);
        }
    };
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
        if previous != Some(slot.bounds) {
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
    Navigate { url: String },
    Back,
    Forward,
    Reload,
    Stop,
    Focus,
    Find { text: String, backwards: bool },
}

#[tauri::command]
pub async fn browser_action(window: Window, id: String, action: Action) -> Result<(), String> {
    crate::files::main_window(&window)?;
    let webview = view(window.app_handle(), &id)?;
    match action {
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

#[cfg(feature = "native-smoke")]
pub fn smoke_pages(app: &tauri::AppHandle) -> serde_json::Value {
    let state = app.state::<Browsers>();
    let pages = state.pages.lock().unwrap();
    serde_json::json!(pages.iter().map(|(id,page)|(id.clone(),serde_json::json!({"url":page.url,"title":page.title,"visible":page.bounds.is_some(),"bounds":page.bounds.map(|b|[b.x,b.y,b.width,b.height])}))).collect::<std::collections::BTreeMap<_,_>>())
}
