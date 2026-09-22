use tauri::Manager;

pub fn page(webview: &tauri::Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if webview.label() == "main"
        && matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
    {
        let directory = std::env::var("LOMI_IMAGE_SMOKE_DIRECTORY").unwrap();
        let script = include_str!("image-smoke.js").replace(
            "SMOKE_DIRECTORY",
            &serde_json::to_string(&directory).unwrap(),
        );
        let _ = webview.eval(script);
    }
}

pub fn result(
    app: &tauri::AppHandle,
    stage: &str,
    data: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let directory = std::env::var("LOMI_IMAGE_SMOKE_DIRECTORY").map_err(|e| e.to_string())?;
    if stage == "image-settings" {
        let script = format!(
            r#"(async()=>{{const invoke=window.__TAURI_INTERNALS__.invoke;let denied=false;try{{await invoke('read_image_file',{{root:{},relative:'picture.png'}});}}catch{{denied=true;}}await invoke('plugin_smoke_result',{{stage:denied?'passed':'failed',data:denied?'Native image reads, rendering, zoom, errors and settings isolation passed':'Settings could read project images'}});}})();"#,
            serde_json::to_string(&std::path::Path::new(&directory).join("project")).unwrap()
        );
        app.get_webview("settings")
            .ok_or("Missing settings webview")?
            .eval(script)
            .map_err(|e| e.to_string())?;
    } else if matches!(stage, "passed" | "failed") {
        std::fs::write(
            std::path::Path::new(&directory).join("result.json"),
            serde_json::to_vec_pretty(&serde_json::json!({"stage":stage,"data":data})).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        app.exit(if stage == "passed" { 0 } else { 1 });
    } else {
        return Err("Unknown image smoke stage.".into());
    }
    Ok(serde_json::Value::Null)
}
