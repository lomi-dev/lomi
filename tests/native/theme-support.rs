use tauri::Manager;
pub fn page(webview: &tauri::Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if !matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
        || webview.label() != "main"
    {
        return;
    }
    let _ = webview.eval(include_str!("theme-smoke-main.js"));
}
pub fn result(
    app: &tauri::AppHandle,
    stage: &str,
    data: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let directory = std::env::var("LOMI_THEME_SMOKE_DIRECTORY").map_err(|e| e.to_string())?;
    match stage {
        "theme-main-ready" => {
            let script = include_str!("theme-smoke-settings.js").replace(
                "SMOKE_DIRECTORY",
                &serde_json::to_string(&directory).unwrap(),
            );
            app.get_webview("settings")
                .ok_or("Missing settings view")?
                .eval(script)
                .map_err(|e| e.to_string())?;
        }
        "theme-exported" => {
            std::fs::write(
                std::path::Path::new(&directory).join("exports.json"),
                serde_json::to_vec_pretty(&data).unwrap(),
            )
            .map_err(|e| e.to_string())?;
            app.get_webview("main")
                .ok_or("Missing main view")?
                .eval(format!("window.__themeSmokeExport = {}", data))
                .map_err(|e| e.to_string())?;
        }
        "passed" | "failed" => {
            std::fs::write(
                std::path::Path::new(&directory).join("result.json"),
                serde_json::to_vec_pretty(&serde_json::json!({"stage":stage,"data":data})).unwrap(),
            )
            .map_err(|e| e.to_string())?;
            app.exit(if stage == "passed" { 0 } else { 1 });
        }
        _ => return Err("Unknown theme smoke stage.".into()),
    }
    Ok(serde_json::Value::Null)
}
