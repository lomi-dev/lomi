use serde_json::{json, Value};
use std::path::PathBuf;
use tauri::Manager;

fn isolated(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let expected = std::env::var("LOMI_DESKTOP_SMOKE_IDENTIFIER").map_err(|e| e.to_string())?;
    if !expected.starts_with("dev.lomi.desktop-smoke-") || app.config().identifier != expected {
        return Err(
            "Desktop smoke requires its build-time isolated application identifier.".into(),
        );
    }
    let expected_data =
        PathBuf::from(std::env::var("LOMI_DESKTOP_SMOKE_APP_DATA").map_err(|e| e.to_string())?);
    let actual = app.path().app_data_dir().map_err(|e| e.to_string())?;
    if std::fs::canonicalize(&actual).map_err(|e| e.to_string())?
        != std::fs::canonicalize(&expected_data).map_err(|e| e.to_string())?
    {
        return Err("Desktop smoke application data directory is not isolated.".into());
    }
    Ok(PathBuf::from(
        std::env::var("LOMI_DESKTOP_SMOKE_DIRECTORY").map_err(|e| e.to_string())?,
    ))
}

fn project(directory: &std::path::Path) -> PathBuf {
    directory.join(if cfg!(windows) {
        "project with spaces"
    } else {
        "project with spaces \\ literal"
    })
}

pub fn page(webview: &tauri::Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if webview.label() != "main"
        || !matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
    {
        return;
    }
    let app = webview.app_handle();
    let result = isolated(app).and_then(|directory| {
        let idle_ms = std::env::var("LOMI_DESKTOP_SMOKE_IDLE_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| (1..=30_000).contains(value))
            .unwrap_or(0);
        let config = json!({"root":project(&directory),"phase":std::env::var("LOMI_DESKTOP_SMOKE_PHASE").unwrap_or_else(|_|"exercise".into()),"idleMs":idle_ms});
        webview.eval(&include_str!("desktop-smoke.js").replace("SMOKE_CONFIG", &config.to_string())).map_err(|e| e.to_string())
    });
    if let Err(error) = result {
        eprintln!("Desktop smoke isolation failed: {error}");
        app.exit(1);
    }
}

pub fn result(app: &tauri::AppHandle, stage: &str, data: Value) -> Result<Value, String> {
    let directory = isolated(app)?;
    if stage.starts_with("idle-") {
        let enabled = std::env::var("LOMI_DESKTOP_SMOKE_IDLE_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|value| (1..=30_000).contains(&value));
        if !enabled {
            return Err("Desktop idle probe requires an opt-in fixture.".into());
        }
        let main = app.get_window("main").ok_or("Missing main window")?;
        if stage == "idle-close" {
            #[cfg(windows)]
            main.minimize().map_err(|error| error.to_string())?;
            #[cfg(not(windows))]
            main.close().map_err(|error| error.to_string())?;
            return Ok(Value::Null);
        }
        if stage == "idle-reopen" {
            #[cfg(windows)]
            main.unminimize().map_err(|error| error.to_string())?;
            main.show().map_err(|error| error.to_string())?;
            return Ok(Value::Null);
        }
        let minimized = if cfg!(windows) {
            main.is_minimized().map_err(|error| error.to_string())?
        } else {
            false
        };
        let visible = main.is_visible().map_err(|error| error.to_string())? && !minimized;
        if stage == "idle-state" {
            return Ok(json!(visible));
        }
        if !matches!(stage, "idle-visible" | "idle-hidden" | "idle-reopened") {
            return Err("Unknown desktop idle stage.".into());
        }
        if visible != (stage != "idle-hidden") {
            return Err("Desktop idle window visibility did not match its phase.".into());
        }
        std::fs::write(
            directory.join(format!("{stage}.json")),
            serde_json::to_vec_pretty(
                &json!({"stage":stage,"visible":visible,"minimized":minimized,"pid":std::process::id()}),
            )
            .unwrap(),
        )
        .map_err(|error| error.to_string())?;
        return Ok(json!(visible));
    }
    if stage == "external-write" {
        // Deliberately bypass editor commands to exercise real OS watcher and
        // stale-revision handling. The target is fixed inside this fixture.
        std::fs::write(
            project(&directory).join("conflict.txt"),
            "external change\r\n",
        )
        .map_err(|e| e.to_string())?;
        return Ok(Value::Null);
    }
    if matches!(stage, "passed" | "failed") {
        let phase = std::env::var("LOMI_DESKTOP_SMOKE_PHASE").unwrap_or_else(|_| "exercise".into());
        std::fs::write(
            directory.join(format!("{phase}-result.json")),
            serde_json::to_vec_pretty(&json!({"stage":stage,"data":data})).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        app.exit(if stage == "passed" { 0 } else { 1 });
        return Ok(Value::Null);
    }
    Err("Unknown desktop smoke stage.".into())
}
