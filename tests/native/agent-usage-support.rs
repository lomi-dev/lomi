use tauri::Manager;

pub fn page(webview: &tauri::Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if webview.label() == "main"
        && matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
    {
        let directory = std::env::var("LOMI_USAGE_SMOKE_DIRECTORY").unwrap();
        let script = include_str!("agent-usage-smoke.js")
            .replace(
                "SMOKE_DIRECTORY",
                &serde_json::to_string(&directory).unwrap(),
            )
            .replace(
                "SMOKE_AGY_EXECUTABLE",
                &serde_json::to_string(&std::env::var("LOMI_USAGE_SMOKE_AGY_EXECUTABLE").unwrap())
                    .unwrap(),
            )
            .replace(
                "SMOKE_AGY_HOME",
                &serde_json::to_string(&std::env::var("LOMI_USAGE_SMOKE_AGY_HOME").unwrap())
                    .unwrap(),
            )
            .replace(
                "SMOKE_LIVE_HOME",
                &serde_json::to_string(&std::env::var("LOMI_USAGE_SMOKE_LIVE_HOME").ok()).unwrap(),
            );
        let script = script.replace(
            "SMOKE_LIVE_MODE",
            &serde_json::to_string(&std::env::var("LOMI_USAGE_SMOKE_LIVE_MODE").ok()).unwrap(),
        );
        let script = script.replace(
            "SMOKE_LIVE_KIMI_HOME",
            &serde_json::to_string(&std::env::var("LOMI_USAGE_SMOKE_LIVE_KIMI_HOME").ok()).unwrap(),
        );
        let script = script.replace(
            "SMOKE_LIVE_AGY_EXECUTABLE",
            &serde_json::to_string(&std::env::var("LOMI_USAGE_SMOKE_LIVE_AGY_EXECUTABLE").ok())
                .unwrap(),
        );
        let script = script.replace(
            "SMOKE_LIVE_AGY_HOME",
            &serde_json::to_string(&std::env::var("LOMI_USAGE_SMOKE_LIVE_AGY_HOME").ok()).unwrap(),
        );
        let _ = webview.eval(script);
    }
}

pub fn result(
    app: &tauri::AppHandle,
    stage: &str,
    data: serde_json::Value,
) -> Result<serde_json::Value, String> {
    match stage {
        "agy-fixture-decline" => {
            let directory =
                std::env::var("LOMI_USAGE_SMOKE_DIRECTORY").map_err(|error| error.to_string())?;
            let fixture = std::env::var("LOMI_USAGE_SMOKE_AGY_FIXTURE_DATA")
                .map_err(|error| error.to_string())?;
            let directory = std::fs::canonicalize(directory).map_err(|error| error.to_string())?;
            let fixture = std::path::PathBuf::from(fixture);
            if fixture
                .file_name()
                .is_none_or(|name| name != "agy-usage.json")
                || fixture
                    .parent()
                    .and_then(|path| path.canonicalize().ok())
                    .as_deref()
                    != Some(directory.as_path())
            {
                return Err("The Antigravity fixture path is outside the smoke directory.".into());
            }
            let declined = serde_json::json!({
                "status": "SUCCESS",
                "num_turns": 0,
                "conversation_id": "",
                "duration_seconds": 0,
                "response": "",
                "usage": {
                    "input_tokens": 0,
                    "output_tokens": 0,
                    "thinking_tokens": 0,
                    "cache_read_tokens": 0,
                    "total_tokens": 0,
                },
                "command": {
                    "name": "usage",
                    "data": {
                        "description": "Account quota",
                        "groups": [{
                            "name": "Plan",
                            "description": "",
                            "buckets": [
                                {
                                    "id": "short-window",
                                    "name": "Short window",
                                    "window": "5h",
                                    "remaining_fraction": 0.12,
                                    "reset_time": "2099-01-01T00:00:00Z",
                                    "description": "",
                                },
                                {
                                    "id": "long-window",
                                    "name": "Long window",
                                    "window": "weekly",
                                    "remaining_fraction": 0.07,
                                    "reset_time": "2099-01-01T00:00:00Z",
                                    "description": "",
                                },
                            ],
                        }],
                    },
                },
            });
            std::fs::write(
                fixture,
                serde_json::to_vec(&declined).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
        }
        "usage-settings" => {
            let script = format!(
                r#"(async()=>{{const invoke=window.__TAURI_INTERNALS__.invoke;const data={};try{{await invoke('inspect_cli_usage',{{targets:[],force:false}});await invoke('plugin_smoke_result',{{stage:'passed',data:{{...data,settingsUsesMainHost:true}}}});}}catch(error){{await invoke('plugin_smoke_result',{{stage:'failed',data:{{...data,error:String(error)}}}});}}}})();"#,
                serde_json::to_string(&data).unwrap(),
            );
            app.get_webview("main")
                .ok_or("Missing main settings host")?
                .eval(script)
                .map_err(|error| error.to_string())?;
        }
        "passed" | "failed" => {
            let directory =
                std::env::var("LOMI_USAGE_SMOKE_DIRECTORY").map_err(|error| error.to_string())?;
            std::fs::write(
                std::path::Path::new(&directory).join("result.json"),
                serde_json::to_vec_pretty(&serde_json::json!({"stage":stage,"data":data})).unwrap(),
            )
            .map_err(|error| error.to_string())?;
            app.state::<crate::terminal::Terminals>().stop_all();
            app.exit(if stage == "passed" { 0 } else { 1 });
        }
        _ => return Err("Unknown agent usage smoke stage.".into()),
    }
    Ok(serde_json::Value::Null)
}
