use tauri::Manager;

pub fn page(webview: &tauri::Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if webview.label() == "main"
        && matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
    {
        let _ = webview.eval(include_str!("agent-launch-smoke.js"));
    }
}

pub fn result(
    app: &tauri::AppHandle,
    stage: &str,
    data: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let directory = std::path::PathBuf::from(
        std::env::var("LOMI_AGENT_LAUNCH_SMOKE_DIRECTORY").map_err(|e| e.to_string())?,
    );
    match stage {
        "inspect" => {
            return Ok(serde_json::json!({
                "terminals": app.state::<crate::terminal::Terminals>().smoke_sessions(),
                "launches": std::fs::read_to_string(directory.join("launches.txt")).unwrap_or_default(),
                "streamed": std::fs::read_to_string(directory.join("streamed.txt")).unwrap_or_default(),
            }));
        }
        "check-settings" => {
            let script = format!(
                r#"(async()=>{{
                    const invoke=window.__TAURI_INTERNALS__.invoke;
                    try{{
                        await invoke('open_settings',{{page:'about'}});
                        await invoke('installed_agent_clis',{{profileId:'local:zsh',cwd:{}}});
                        const clients=await invoke('inspect_mcp_clients');
                        if(clients.length!==1||clients[0].cli!=='cursor')throw Error('MCP settings did not list only the installed Cursor fixture');
                        await invoke('plugin_smoke_result',{{stage:'passed',data:{{...{},settingsUsesMainHost:true,mcpClients:clients.map(client=>client.cli)}}}});
                    }}catch(error){{
                        await invoke('plugin_smoke_result',{{stage:'failed',data:{{error:String(error)}}}});
                    }}
                }})();"#,
                serde_json::to_string(&directory.join("project")).unwrap(),
                serde_json::to_string(&data).unwrap(),
            );
            app.get_webview("main")
                .ok_or("Missing main settings host")?
                .eval(script)
                .map_err(|e| e.to_string())?;
        }
        "passed" | "failed" => {
            std::fs::write(
                directory.join("result.json"),
                serde_json::to_vec_pretty(&serde_json::json!({"stage":stage,"data":data})).unwrap(),
            )
            .map_err(|e| e.to_string())?;
            app.state::<crate::terminal::Terminals>().stop_all();
            app.exit(if stage == "passed" { 0 } else { 1 });
        }
        _ => return Err("Unknown agent launch smoke stage.".into()),
    }
    Ok(serde_json::Value::Null)
}
