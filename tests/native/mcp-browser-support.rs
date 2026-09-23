//! Native P0 fixture. No command or listener is included in normal builds.
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{Manager, Webview};

pub(crate) async fn evaluate(view: &Webview, script: &str) -> Result<Value, String> {
    let (send, receive) = tokio::sync::oneshot::channel();
    let send = Mutex::new(Some(send));
    view.eval_with_callback(script, move |result| {
        if let Some(send) = send.lock().unwrap().take() {
            let value = if result.len() > 65536 {
                Err("Probe JS result exceeded limit".into())
            } else {
                serde_json::from_str(&result)
                    .map_err(|_| "Probe JS did not return JSON".to_string())
            };
            let _ = send.send(value);
        }
    })
    .map_err(|e| e.to_string())?;
    tokio::time::timeout(Duration::from_secs(5), receive)
        .await
        .map_err(|_| "Probe JS timed out")?
        .map_err(|_| "Probe callback dropped")?
}

pub(crate) async fn wait_for(view: &Webview, script: &str) -> Result<Value, String> {
    for _ in 0..100 {
        let value = evaluate(view, script).await.unwrap_or(Value::Null);
        if value == true {
            return Ok(value);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(format!("Fixture condition did not hold: {script}"))
}

async fn isolated(view: &Webview, script: &str) -> Result<Value, String> {
    let (send, receive) = tokio::sync::oneshot::channel();
    let send = Mutex::new(Some(send));
    let script = script.to_owned();
    view.with_webview(move |platform| {
        use objc2::{runtime::AnyObject, MainThreadMarker};
        use objc2_foundation::{NSError, NSString};
        use objc2_web_kit::{WKContentWorld, WKWebView};
        let native = unsafe { &*platform.inner().cast::<WKWebView>() };
        let main = MainThreadMarker::new().unwrap();
        let world = unsafe {
            WKContentWorld::worldWithName(&NSString::from_str("LomiMcpQualification"), main)
        };
        let callback = block2::RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
            let result = if !error.is_null() || value.is_null() {
                Err("Isolated JS failed".into())
            } else {
                unsafe { &*value }
                    .downcast_ref::<NSString>()
                    .ok_or("Expected a JSON string".into())
                    .and_then(|text| {
                        if text.length() > 65536 {
                            return Err("Isolated JS exceeded limit".into());
                        }
                        serde_json::from_str(&text.to_string())
                            .map_err(|_| "Invalid isolated JSON".into())
                    })
            };
            if let Some(send) = send.lock().unwrap().take() {
                let _ = send.send(result);
            }
        });
        unsafe {
            native.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
                &NSString::from_str(&script),
                None,
                None,
                &world,
                Some(&callback),
            );
        }
    })
    .map_err(|e| e.to_string())?;
    tokio::time::timeout(Duration::from_secs(5), receive)
        .await
        .map_err(|_| "Isolated JS timed out")?
        .map_err(|_| "Isolated JS callback dropped")?
}

async fn profile_and_navigation(app: &tauri::AppHandle, view: &Webview) -> Result<Value, String> {
    let origin = view.url().map_err(|e| e.to_string())?;
    let scope = origin.origin();
    let blocked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let denied = blocked.clone();
    let mut identifier = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut identifier)
        .map_err(|_| "Cannot generate isolated profile")?;
    let builder = tauri::webview::WebviewBuilder::new(
        "browser-mcp-isolated",
        tauri::WebviewUrl::External(origin.clone()),
    )
    .data_store_identifier(identifier)
    .data_directory(
        app.path()
            .app_data_dir()
            .map_err(|e| e.to_string())?
            .join("probe-profile"),
    )
    .on_navigation(move |url| {
        let allowed = url.origin() == scope && url.path() != "/blocked";
        if !allowed {
            denied.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        allowed
    })
    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
    .on_download(|_, _| false);
    let other = app
        .get_window("main")
        .ok_or("Missing main")?
        .add_child(
            builder,
            tauri::LogicalPosition::new(300., 150.),
            tauri::LogicalSize::new(500., 360.),
        )
        .map_err(|e| e.to_string())?;
    wait_for(&other, "Boolean(document.querySelector('#save'))").await?;
    let before = isolated(&other, "return JSON.stringify({cookie:document.cookie, storage:localStorage.getItem('lomi-mcp-p0'),workers:(await navigator.serviceWorker.getRegistrations()).length,caches:(await caches.keys()).length});").await?;
    if before["cookie"] != ""
        || !before["storage"].is_null()
        || before["workers"] != 0
        || before["caches"] != 0
    {
        return Err("New profile contains another profile's state".into());
    }
    isolated(view, "document.cookie='lomi-mcp-p0=source; SameSite=Strict'; localStorage.setItem('lomi-mcp-p0','source'); await caches.open('lomi-mcp-p0'); await navigator.serviceWorker.register('/worker.js'); return JSON.stringify({ok:true});").await?;
    let source = isolated(view, "return JSON.stringify({cookie:document.cookie, storage:localStorage.getItem('lomi-mcp-p0'),workers:(await navigator.serviceWorker.getRegistrations()).length,caches:(await caches.keys()).length});").await?;
    let after = isolated(&other, "return JSON.stringify({cookie:document.cookie, storage:localStorage.getItem('lomi-mcp-p0'),workers:(await navigator.serviceWorker.getRegistrations()).length,caches:(await caches.keys()).length});").await?;
    if before != after || source["workers"] != 1 || source["caches"] != 1 {
        return Err("Browser profiles are not isolated".into());
    }
    let target = origin.join("/redirect").map_err(|e| e.to_string())?;
    other.navigate(target).map_err(|e| e.to_string())?;
    for _ in 0..50 {
        if blocked.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if blocked.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        return Err("Redirect was not intercepted before navigation".into());
    }
    isolated(view, "document.cookie='lomi-mcp-p0=; Max-Age=0'; localStorage.removeItem('lomi-mcp-p0'); await caches.delete('lomi-mcp-p0'); for (const r of await navigator.serviceWorker.getRegistrations()) { if(r.active?.scriptURL.endsWith('/worker.js')) await r.unregister(); } return JSON.stringify({cleaned:true});").await?;
    other.close().map_err(|e| e.to_string())?;
    Ok(
        json!({"before":before,"source":source,"other":after,"deniedNavigations":blocked.load(std::sync::atomic::Ordering::SeqCst)}),
    )
}

pub(crate) async fn screenshot(view: &Webview, path: PathBuf) -> Result<Value, String> {
    let (send, receive) = tokio::sync::oneshot::channel();
    let send = Arc::new(Mutex::new(Some(send)));
    view.with_webview(move |platform| {
        use objc2_app_kit::NSImage;
        use objc2_foundation::NSError;
        use objc2_web_kit::WKWebView;
        // The handle is retained by Tauri and is accessed on its GUI thread.
        let native = unsafe { &*platform.inner().cast::<WKWebView>() };
        let callback = block2::RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
            let result = if !error.is_null() || image.is_null() {
                Err("WKWebView snapshot failed".into())
            } else {
                let image = unsafe { &*image };
                image
                    .TIFFRepresentation()
                    .ok_or("Missing native image data".to_string())
                    .and_then(|data| {
                        let bytes = unsafe { data.as_bytes_unchecked() };
                        if bytes.len() > 32 * 1024 * 1024 {
                            return Err("Snapshot too large".into());
                        }
                        let image =
                            image::load_from_memory_with_format(bytes, image::ImageFormat::Tiff)
                                .map_err(|e| e.to_string())?;
                        let dimensions = json!({"width": image.width(), "height": image.height()});
                        image.save(&path).map_err(|e| e.to_string())?;
                        Ok(dimensions)
                    })
            };
            if let Some(send) = send.lock().unwrap().take() {
                let _ = send.send(result);
            }
        });
        unsafe {
            native.takeSnapshotWithConfiguration_completionHandler(None, &callback);
        }
    })
    .map_err(|e| e.to_string())?;
    tokio::time::timeout(Duration::from_secs(5), receive)
        .await
        .map_err(|_| "Snapshot timeout")?
        .map_err(|_| "Snapshot callback dropped")?
}

async fn run(app: &tauri::AppHandle, directory: &std::path::Path) -> Result<Value, String> {
    let mut view = None;
    for _ in 0..200 {
        view = app.get_webview("browser-mcp-fixture");
        if view.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let view = view.ok_or("Workbench did not create the fixture child")?;
    wait_for(
        &view,
        "Boolean(window.fixtureReady && document.querySelector('#save'))",
    )
    .await?;
    view.set_focus().map_err(|e| e.to_string())?;
    let initial = evaluate(&view, "({title:document.title,url:location.href,cssWidth:innerWidth,cssHeight:innerHeight,dpr:devicePixelRatio})").await?;
    evaluate(&view, "document.querySelector('#save').click(); true").await?;
    wait_for(
        &view,
        "document.querySelector('#result').textContent === 'Name is required'",
    )
    .await?;
    evaluate(&view, r#"(() => { const input=document.querySelector('#name'); input.focus(); Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'Zażółć 🙂'); input.dispatchEvent(new Event('input',{bubbles:true})); return document.activeElement===input; })()"#).await?;
    evaluate(&view, "document.querySelector('#save').click(); true").await?;
    wait_for(
        &view,
        "document.querySelector('#result').textContent === 'Saved: Zażółć 🙂'",
    )
    .await?;
    let controls = evaluate(&view, r#"(() => { const select=document.querySelector('#choice'); select.value='b'; select.dispatchEvent(new Event('change',{bubbles:true})); const edit=document.querySelector('#editable'); edit.textContent='Unicode 🙂'; edit.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:'Unicode 🙂'})); document.querySelector('#canvas').getContext('2d').fillRect(4,4,120,60); return {select:select.value,editable:edit.textContent,focused:document.activeElement.id,isTrusted:window.fixtureTrusted}; })()"#).await?;
    if controls["select"] != "b"
        || controls["editable"] != "Unicode 🙂"
        || controls["isTrusted"] != false
    {
        return Err("Unexpected control semantics".into());
    }
    evaluate(&view, "window.fixtureDenied=null; window.__TAURI_INTERNALS__.invoke('load_session').then(()=>window.fixtureDenied=false,()=>window.fixtureDenied=true); true").await?;
    wait_for(&view, "window.fixtureDenied === true").await?;
    evaluate(&view, "document.querySelector('#spa').click(); true").await?;
    wait_for(&view, "location.hash === '#saved'").await?;
    let image = screenshot(&view, directory.join("browser.png")).await?;
    let async_result = match evaluate(&view, "Promise.resolve({asynchronous:true})").await {
        Ok(value) => json!({"result":value}),
        Err(error) => json!({"unsupported":error}),
    };
    let exception = evaluate(
        &view,
        "(() => { try { throw new Error('fixture'); } catch { return {caught:true}; } })()",
    )
    .await?;
    if exception["caught"] != true {
        return Err("JS exception wrapper failed".into());
    }
    let isolated_result = isolated(&view, "await Promise.resolve(); return JSON.stringify({asynchronous:true,pageGlobal:typeof window.fixtureReady,result:document.querySelector('#result').textContent});").await?;
    if isolated_result["asynchronous"] != true
        || isolated_result["pageGlobal"] != "undefined"
        || isolated_result["result"] != "Saved: Zażółć 🙂"
    {
        return Err("Isolated async content world failed".into());
    }
    let profiles = profile_and_navigation(app, &view).await?;
    Ok(
        json!({"panelId":"mcp-fixture","webviewLabel":view.label(),"engine":"WKWebView","initial":initial,"controls":controls,"snapshot":image,"asyncResult":async_result,"isolatedAsync":isolated_result,"profiles":profiles,"inputMode":"synthetic_js","checks":["native callback","controlled React validation and save","Unicode","DOM focus","select","contenteditable","canvas screenshot","SPA","browser child denied application command","caught JS exception","isolated async JS","cookies storage caches and service workers isolation","redirect interception"],"limitations":["Native trusted input not qualified","Frames, complete navigation policy and gesture-dependent controls still require qualification","networkIsolation: none"]}),
    )
}

pub fn start(app: tauri::AppHandle) {
    let Some(directory) = std::env::var_os("LOMI_MCP_PROBE_DIRECTORY").map(PathBuf::from) else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let result = run(&app, &directory).await;
        let failed = result.is_err();
        let report = match result {
            Ok(data) => json!({"stage":"passed","data":data}),
            Err(error) => json!({"stage":"failed","error":error}),
        };
        let _ = std::fs::write(
            directory.join("result.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        );
        app.exit(if failed { 1 } else { 0 });
    });
}
