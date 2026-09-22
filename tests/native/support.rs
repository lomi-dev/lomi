use tauri::{Manager, Window};
#[path = "image-support.rs"]
mod image_smoke;
#[path = "notification-support.rs"]
mod notification_smoke;
#[path = "terminal-clipboard-support.rs"]
mod terminal_clipboard_smoke;
#[path = "theme-support.rs"]
mod theme_smoke;
fn entry_script(source: &str, data: &serde_json::Value) -> String {
    source.replace("SMOKE_ENTRY", &serde_json::to_string(data).unwrap())
}
pub fn page(webview: &tauri::Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if std::env::var_os("LOMI_CLIPBOARD_SMOKE_DIRECTORY").is_some() {
        terminal_clipboard_smoke::page(webview, payload);
        return;
    }
    #[cfg(feature = "android-probe")]
    if std::env::var_os("LOMI_ANDROID_PROBE_DIRECTORY").is_some() {
        crate::android_probe::page(webview, payload);
        return;
    }
    if std::env::var_os("LOMI_NOTIFICATION_SMOKE_DIRECTORY").is_some() {
        notification_smoke::page(webview, payload);
        return;
    }
    if std::env::var_os("LOMI_IMAGE_SMOKE_DIRECTORY").is_some() {
        image_smoke::page(webview, payload);
        return;
    }
    if std::env::var_os("LOMI_THEME_SMOKE_DIRECTORY").is_some() {
        theme_smoke::page(webview, payload);
        return;
    }
    if webview.label() != "settings"
        || !matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
    {
        return;
    }
    let Ok(path) = std::env::var("LOMI_PLUGIN_SMOKE_PACKAGE") else {
        return;
    };
    if crate::plugins::safe_mode() {
        if let Some(main) = webview.app_handle().get_webview("main") {
            let _ = main.eval(r#"(async()=>{const invoke=window.__TAURI_INTERNALS__.invoke;try{const catalog=await invoke('list_plugins');if(!catalog.safeMode||!catalog.entries.some(e=>e.enabled)||catalog.entries.some(e=>e.evaluated))throw Error('Safe startup evaluated a plugin');const entry=catalog.entries.find(e=>e.enabled);let blocked=false;try{await invoke('prepare_plugin',{id:entry.id,expected:entry.revision});}catch{blocked=true;}if(!blocked)throw Error('Safe startup allowed activation');await invoke('plugin_smoke_result',{stage:'passed',data:{checks:['safe startup preserves enabled metadata and rejects executable preparation']}});}catch(error){await invoke('plugin_smoke_result',{stage:'failed',data:String(error)});}})();"#);
        }
        return;
    }
    let script = format!(
        r#"(async()=>{{try{{const invoke=window.__TAURI_INTERNALS__.invoke;const id=await invoke('import_plugin',{{path:{}}});const catalog=await invoke('list_plugins');await invoke('plugin_smoke_result',{{stage:'imported',data:catalog.entries.find(e=>e.id===id)}});}}catch(error){{await window.__TAURI_INTERNALS__.invoke('plugin_smoke_result',{{stage:'failed',data:String(error)}});}}}})();"#,
        serde_json::to_string(&path).unwrap()
    );
    let _ = webview.eval(&script);
}
#[tauri::command]
pub fn plugin_smoke_result(
    window: Window,
    app: tauri::AppHandle,
    stage: String,
    data: serde_json::Value,
) -> Result<serde_json::Value, String> {
    if !matches!(window.label(), "main" | "settings") {
        return Err("Unknown test caller.".into());
    }
    if std::env::var_os("LOMI_CLIPBOARD_SMOKE_DIRECTORY").is_some() {
        return terminal_clipboard_smoke::result(&app, &stage, data);
    }
    #[cfg(feature = "android-probe")]
    if std::env::var_os("LOMI_ANDROID_PRODUCT_DIRECTORY").is_some() {
        return crate::android_product::result(&window, &stage, data);
    }
    if std::env::var_os("LOMI_NOTIFICATION_SMOKE_DIRECTORY").is_some() {
        return notification_smoke::result(&app, &stage, data);
    }
    if std::env::var_os("LOMI_IMAGE_SMOKE_DIRECTORY").is_some() {
        return image_smoke::result(&app, &stage, data);
    }
    if std::env::var_os("LOMI_THEME_SMOKE_DIRECTORY").is_some() {
        return theme_smoke::result(&app, &stage, data);
    }
    if std::env::var_os("LOMI_PLUGIN_SMOKE_PACKAGE").is_none() {
        return Err("The smoke environment is not active.".into());
    }
    if stage == "inspect" {
        return Ok(
            serde_json::json!({"terminals":app.state::<crate::terminal::Terminals>().smoke_sessions(),"browsers":crate::browser::smoke_pages(&app)}),
        );
    }
    if stage == "browser-probe" || stage == "browser-retained" {
        let script = if stage == "browser-probe" {
            r#"(async()=>{let denied=false;try{await window.__TAURI_INTERNALS__.invoke('load_session');}catch{denied=true;}const field=document.querySelector('#input');field.focus();field.value='retained input';field.dispatchEvent(new Event('input',{bubbles:true}));document.title=denied&&document.activeElement===field?'Native browser isolated':'Native browser probe failed';})();"#
        } else {
            r#"document.title=document.querySelector('#input').value==='retained input'?'Native browser retained':'Native browser lost state';"#
        };
        app.get_webview("browser-native-browser")
            .ok_or("Missing child browser")?
            .eval(script)
            .map_err(|e| e.to_string())?;
        return Ok(serde_json::Value::Null);
    }
    if stage == "theme-write" {
        let folder = app
            .path()
            .app_data_dir()
            .map_err(|e| e.to_string())?
            .join("themes/theme-copy");
        let raw = serde_json::json!({"version":2,"name":"Native watch","appearance":"light","common":{"tokens":{"--radius-control":"11px"},"terminal":{"fontSize":16},"editor":{"syntax":{"keyword":{"color":"#123456"}}}}});
        std::fs::write(
            folder.join("theme.jsonc"),
            serde_json::to_vec_pretty(&raw).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        return Ok(serde_json::Value::Null);
    }
    let result=match stage.as_str() {
        "imported" => app
            .get_webview("main")
            .ok_or("Missing main view")?
            .eval(entry_script(
                include_str!("plugin-smoke.js"),
                &data,
            ))
            .map_err(|e| e.to_string()),
        "untrusted" => {
            let script = format!(
                r#"(async()=>{{try{{const invoke=window.__TAURI_INTERNALS__.invoke;const entry={};await invoke('enable_plugin',{{id:entry.id,expected:entry.revision}});await invoke('plugin_smoke_result',{{stage:'enabled',data:entry}});}}catch(error){{await window.__TAURI_INTERNALS__.invoke('plugin_smoke_result',{{stage:'failed',data:String(error)}});}}}})();"#,
                serde_json::to_string(&data).unwrap()
            );
            app.get_webview("settings")
                .ok_or("Missing settings view")?
                .eval(&script)
                .map_err(|e| e.to_string())
        }
        "enabled" => app
            .get_webview("main")
            .ok_or("Missing main view")?
            .eval(entry_script(
                include_str!("plugin-smoke-enabled.js"),
                &data,
            ))
            .map_err(|e| e.to_string()),
        "theme-ready"=>app.get_webview("main").ok_or("Missing main view")?.eval(include_str!("workbench-smoke.js")).map_err(|e|e.to_string()),
        "check-settings"=>app.get_webview("settings").ok_or("Missing settings")?.eval(entry_script(r#"(async()=>{const invoke=window.__TAURI_INTERNALS__.invoke;try{for(let i=0;i<150;i++){if(getComputedStyle(document.documentElement).getPropertyValue('--radius-control').trim()==='11px'&&document.documentElement.dataset.appearance==='light'&&document.documentElement.dataset.themeSourceRevision===SMOKE_ENTRY){await invoke('plugin_smoke_result',{stage:'settings-synced',data:{appearance:document.documentElement.dataset.appearance,radius:'11px',sourceRevision:document.documentElement.dataset.themeSourceRevision}});return;}await new Promise(r=>setTimeout(r,100));}throw Error('Settings did not follow the theme watch');}catch(error){await invoke('plugin_smoke_result',{stage:'failed',data:String(error)});}})();"#, &data)).map_err(|e|e.to_string()),
        "settings-synced"=>app.get_webview("main").ok_or("Missing main view")?.eval(format!("globalThis.__lomiSmokeSettings={}",serde_json::to_string(&data).unwrap())).map_err(|e|e.to_string()),
        "workbench-ready"=>app.get_webview("settings").ok_or("Missing settings")?.eval(r#"(async()=>{const invoke=window.__TAURI_INTERNALS__.invoke;try{const id=await invoke('duplicate_theme',{id:null});await invoke('save_theme_preferences',{data:{version:1,active:id,appearance:'dark'}});await invoke('plugin_smoke_result',{stage:'theme-ready',data:{id}});}catch(error){await invoke('plugin_smoke_result',{stage:'failed',data:String(error)});}})();"#).map_err(|e|e.to_string()),
        "passed" | "failed" => {
            let path =
                std::env::var("LOMI_PLUGIN_SMOKE_REPORT").map_err(|e| e.to_string())?;
            std::fs::write(
                path,
                serde_json::to_vec_pretty(&serde_json::json!({"stage":stage,"data":data})).unwrap(),
            )
            .map_err(|e| e.to_string())?;
            app.exit(if stage == "passed" { 0 } else { 1 });
            Ok(())
        }
        _ => Err("Unknown test stage.".into()),
    };
    result.map(|()| serde_json::Value::Null)
}
