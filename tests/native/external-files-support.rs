use serde_json::{json, Value};
use std::path::PathBuf;
use tauri::Manager;

const MARKER: &str = "external-files-native-smoke.json";

fn directory(app: &tauri::AppHandle) -> Result<Option<PathBuf>, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let marker = app_data.join(MARKER);
    let contents = match std::fs::read(&marker) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let config: Value = serde_json::from_slice(&contents).map_err(|error| error.to_string())?;
    let identifier = config
        .get("identifier")
        .and_then(Value::as_str)
        .ok_or("External file smoke marker has no application identifier.")?;
    if !identifier.starts_with("dev.lomi.external-files-smoke-")
        || app.config().identifier != identifier
    {
        return Err("External file smoke application identifier is not isolated.".into());
    }
    let directory = config
        .get("directory")
        .and_then(Value::as_str)
        .ok_or("External file smoke marker has no artifact directory.")?;
    Ok(Some(PathBuf::from(directory)))
}

pub fn handles(app: &tauri::AppHandle) -> Result<bool, String> {
    directory(app).map(|directory| directory.is_some())
}

pub fn page(webview: &tauri::Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if webview.label() != "main"
        || !matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
    {
        return;
    }
    let app = webview.app_handle();
    let result = directory(app).and_then(|directory| {
        let Some(directory) = directory else {
            return Ok(());
        };
        std::fs::write(
            directory.join("page-loaded.json"),
            serde_json::to_vec_pretty(&json!({
                "pid": std::process::id(),
                "webview": webview.label(),
            }))
            .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        let config = json!({
            "directory": directory,
            "root": directory.join("project with spaces — żółć"),
            "files": {
                "spaced": "name with spaces — żółć.txt",
                "unknown": "unknown extension.odd",
                "dotenv": ".env",
                "extensionless": "README",
                "image": "tiny image.png",
                "second": "nested/second file.txt",
                "dirty": "name with spaces — żółć.txt",
                "warm": "warm open.txt",
                "binary": "binary.dat",
                "missing": "missing external file.txt",
                "directory": "folder",
            },
        });
        webview
            .eval(
                &include_str!("external-files-smoke.js")
                    .replace("SMOKE_CONFIG", &config.to_string()),
            )
            .map_err(|error| error.to_string())
    });
    if let Err(error) = result {
        eprintln!("External file smoke isolation failed: {error}");
        app.exit(1);
    }
}

pub fn result(app: &tauri::AppHandle, stage: &str, data: Value) -> Result<Value, String> {
    let directory = directory(app)?.ok_or("External file smoke is not active.")?;
    if stage == "window-visible" {
        let window = app.get_window("main").ok_or("Missing main window")?;
        return window
            .is_visible()
            .map(|visible| json!(visible))
            .map_err(|error| error.to_string());
    }
    if !matches!(
        stage,
        "script-started" | "cold-ready" | "warm-ready" | "passed" | "failed"
    ) {
        return Err("Unknown external file smoke stage.".into());
    }
    let result = json!({
        "stage": stage,
        "pid": std::process::id(),
        "data": data,
    });
    let name = if matches!(stage, "passed" | "failed") {
        "result.json"
    } else {
        match stage {
            "script-started" => "script-started.json",
            "cold-ready" => "cold-ready.json",
            "warm-ready" => "warm-ready.json",
            _ => unreachable!(),
        }
    };
    std::fs::write(
        directory.join(name),
        serde_json::to_vec_pretty(&result).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    if matches!(stage, "passed" | "failed") {
        app.exit(if stage == "passed" { 0 } else { 1 });
    }
    Ok(Value::Null)
}
