use tauri::Manager;

pub fn page(webview: &tauri::Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if webview.label() == "main"
        && matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
    {
        if let Ok(directory) = std::env::var("LOMI_AGENT_RUNTIME_SMOKE_DIRECTORY") {
            let _ = std::fs::write(std::path::Path::new(&directory).join("page-loaded"), "main");
        }
        let _ = webview.eval(&include_str!("agent-runtime-smoke.js").replace(
            "SMOKE_FAILURE",
            if std::env::var_os("LOMI_AGENT_RUNTIME_SMOKE_FAILURE").is_some() {
                "true"
            } else {
                "false"
            },
        ));
    }
}

pub fn result(
    app: &tauri::AppHandle,
    stage: &str,
    data: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let directory = std::path::PathBuf::from(
        std::env::var("LOMI_AGENT_RUNTIME_SMOKE_DIRECTORY").map_err(|e| e.to_string())?,
    );
    match stage {
        #[cfg(target_os = "macos")]
        "owned-production-entry" => {
            let evidence = crate::agent_runtime::host_smoke::run(app, &directory)?;
            std::fs::write(
                directory.join("owned-production-entry.json"),
                serde_json::to_vec_pretty(&evidence).unwrap(),
            )
            .map_err(|e| e.to_string())?;
            Ok(evidence)
        }
        "inspect" => Ok(serde_json::json!({
            "pid": std::process::id(),
            "identifier": app.config().identifier,
            "appData": app.path().app_data_dir().map_err(|e| e.to_string())?,
            "restored": directory.join("restored.json").exists(),
        })),
        "restored" | "ready-to-quit" | "retention-passed" | "failed" | "call" => {
            if stage == "call" {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(directory.join("calls.jsonl"))
                    .map_err(|e| e.to_string())?;
                writeln!(file, "{}", data).map_err(|e| e.to_string())?;
            } else {
                std::fs::write(
                    directory.join(format!("{stage}.json")),
                    serde_json::to_vec_pretty(&data).unwrap(),
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(serde_json::Value::Null)
        }
        #[cfg(target_os = "macos")]
        "screenshot" => {
            let name = data
                .as_str()
                .filter(|n| matches!(*n, "restored" | "docked" | "final-close"))
                .ok_or("Unknown screenshot stage")?;
            let path = directory.join(format!("{name}.png"));
            let window = app.get_window("main").ok_or("Missing main window")?;
            app.run_on_main_thread(move || {
                let native = unsafe {
                    &*window
                        .ns_window()
                        .unwrap()
                        .cast::<objc2_app_kit::NSWindow>()
                };
                let _ = std::process::Command::new("screencapture")
                    .args(["-x", "-l", &native.windowNumber().to_string()])
                    .arg(path)
                    .status();
            })
            .map_err(|e| e.to_string())?;
            Ok(serde_json::Value::Null)
        }
        _ => Err("Unknown agent runtime smoke stage".into()),
    }
}
