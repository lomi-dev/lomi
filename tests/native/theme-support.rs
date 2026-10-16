use tauri::Manager;
fn diagnostic(name: &str, data: serde_json::Value) {
    let Ok(directory) = std::env::var("LOMI_THEME_SMOKE_DIRECTORY") else {
        return;
    };
    if let Ok(bytes) = serde_json::to_vec_pretty(&data) {
        let _ = std::fs::write(std::path::Path::new(&directory).join(name), bytes);
    }
}

pub fn page(webview: &tauri::Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if !matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
        || webview.label() != "main"
    {
        return;
    }
    let app = webview.app_handle();
    diagnostic(
        "page-load.json",
        serde_json::json!({"phase":"page-load-finished"}),
    );
    // This opt-in fixture must paint before waiting for renderer readiness:
    // a hidden WKWebView can suspend the startup frames and timer callbacks.
    let presented = (|| -> Result<(), String> {
        let window = app
            .get_window("main")
            .ok_or("Missing fixture main window")?;
        window.unminimize().map_err(|error| error.to_string())?;
        window.show().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        #[cfg(target_os = "macos")]
        app.show().map_err(|error| error.to_string())?;
        Ok(())
    })();
    if let Err(error) = presented {
        diagnostic(
            "eval-error.json",
            serde_json::json!({"phase":"fixture-presentation","error":error}),
        );
        let _ = result(
            app,
            "failed",
            serde_json::json!({"phase":"fixture-presentation","error":error}),
        );
        return;
    }
    diagnostic(
        "page-load.json",
        serde_json::json!({"phase":"fixture-presented"}),
    );
    // Load test JavaScript through this fixture's Vite server so assertion edits
    // can be rerun with the same compiled native binary and private identity.
    let script = r#"import('/tests/native/theme-smoke-main.js?native-smoke=' + Date.now()).catch(error => window.__TAURI_INTERNALS__.invoke('plugin_smoke_result', {stage:'failed', data:{phase:'smoke-import',error:String(error),stack:String(error?.stack ?? '')}}));"#;
    if let Err(error) = webview.eval(script) {
        diagnostic(
            "eval-error.json",
            serde_json::json!({"phase":"smoke-eval","error":error.to_string()}),
        );
        let _ = result(
            app,
            "failed",
            serde_json::json!({"phase":"smoke-eval","error":error.to_string()}),
        );
    }
}
pub fn result(
    app: &tauri::AppHandle,
    stage: &str,
    data: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let directory = std::env::var("LOMI_THEME_SMOKE_DIRECTORY").map_err(|e| e.to_string())?;
    match stage {
        "theme-checkpoint" => {
            let encoded = serde_json::to_vec(&data).map_err(|error| error.to_string())?;
            if encoded.len() > 16_384 {
                return Err("Theme smoke checkpoint exceeds its diagnostic limit.".into());
            }
            eprintln!("Theme smoke checkpoint: {data}");
            diagnostic("checkpoint.json", data);
        }
        "theme-inspect" => {
            return Ok(serde_json::json!({
                "windows": app.windows().keys().cloned().collect::<Vec<_>>(),
                "mainVisible": app.get_window("main").ok_or("Missing main window")?
                    .is_visible().map_err(|error| error.to_string())?,
                "terminals": app.state::<crate::terminal::Terminals>().smoke_sessions(),
            }));
        }
        "theme-main-ready" => {
            let source = std::env::var("LOMI_THEME_SMOKE_SOURCE")
                .unwrap_or_else(|_| "supplied-vscode".into());
            let script = include_str!("theme-smoke-settings.js")
                .replace(
                    "SMOKE_DIRECTORY",
                    &serde_json::to_string(&directory).unwrap(),
                )
                .replace(
                    "SMOKE_THEME_SOURCE",
                    &serde_json::to_string(&source).unwrap(),
                );
            app.get_webview("main")
                .ok_or("Missing main settings host")?
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
