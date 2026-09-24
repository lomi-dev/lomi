//! Independently observe the two workspaces chosen by a model client.
use super::*;

pub(super) async fn verify(
    app: &tauri::AppHandle,
    directory: &Path,
    report: &Value,
) -> Result<Value, String> {
    let main = app.get_webview("main").ok_or("Missing main")?;
    let calls = report["items"].as_array().ok_or("Missing calls")?;
    let created = calls
        .iter()
        .filter(|i| i["type"] == "mcpToolCall" && i["server"] == "lomi_probe")
        .map(|i| &i["result"]["structuredContent"]["data"]["result"]);
    let mut terminals = std::collections::BTreeMap::new();
    let mut second_workspace = None;
    for resource in created {
        if resource["terminalSessionId"].is_string() {
            terminals.insert(
                resource["panelId"]
                    .as_str()
                    .ok_or("Missing panel")?
                    .to_string(),
                resource.clone(),
            );
        }
        if resource["name"] == "Routing second" && resource["workspaceId"].is_string() {
            second_workspace = resource["workspaceId"].as_str().map(String::from);
        }
    }
    let second_workspace = second_workspace.ok_or("Model did not create Routing second")?;
    if terminals.len() != 2
        || terminals
            .values()
            .map(|t| &t["workspaceId"])
            .collect::<std::collections::HashSet<_>>()
            .len()
            != 2
    {
        return Err("Model did not create one distinct terminal per workspace".into());
    }
    let mut evidence = Vec::new();
    for resource in terminals.values() {
        let expected = if resource["workspaceId"] == second_workspace {
            "ROUTING_SECOND"
        } else {
            "ROUTING_FIRST"
        };
        let foreign = if expected == "ROUTING_SECOND" {
            "ROUTING_FIRST"
        } else {
            "ROUTING_SECOND"
        };
        let observed = javascript(&main, &format!("const m=await import('/src/terminal-runtime.ts');const r=m.runningTerminal({});if(!r||r.sessionId!=={})throw Error('Lost retained PTY');return {{status:r.getSnapshot().status,text:[...Array(r.terminal.buffer.active.length)].map((_,i)=>r.terminal.buffer.active.getLine(i)?.translateToString()).join('\\n')}};",resource["panelId"],resource["terminalSessionId"])).await?;
        if observed["status"] != "running"
            || !observed["text"]
                .as_str()
                .unwrap_or("")
                .lines()
                .any(|line| line.trim() == expected)
        {
            return Err(format!(
                "Actual terminal does not contain its workspace output: {expected}"
            ));
        }
        if observed["text"]
            .as_str()
            .unwrap_or("")
            .lines()
            .any(|line| line.trim() == foreign)
        {
            return Err("Command output crossed workspace terminals".into());
        }
        let read = calls.iter().any(|i| {
            i["tool"] == "lomi_terminal_read"
                && i["arguments"]["panelId"] == resource["panelId"]
                && i["result"]["structuredContent"]["status"] == "ok"
                && i["result"]["structuredContent"]["data"]["text"]
                    .as_str()
                    .is_some_and(|s| s.contains(expected))
        });
        if !read {
            return Err("Model did not read both workspace terminal outputs".into());
        }
        evidence.push(json!({"target":resource,"native":observed}));
    }
    evaluate(&main,"(()=>{if(!document.querySelector('.workspace-list'))document.querySelector('button[title^=\"Toggle workspaces\"]').click();return true;})()").await?;
    wait_for(
        &main,
        "Boolean(document.querySelector('.workspace-list-item[aria-current=true]'))",
    )
    .await?;
    let selected = evaluate(
        &main,
        "document.querySelector('.workspace-list-item[aria-current=true]').getAttribute('title')",
    )
    .await?;
    if !selected
        .as_str()
        .unwrap_or("")
        .starts_with("Routing second\n")
    {
        return Err("Native workbench did not retain the model-selected second workspace".into());
    }
    screenshot(&main, directory.join("routing-workspaces.png")).await?;
    Ok(
        json!({"passed":true,"case":"two-workspaces","secondWorkspaceId":second_workspace,"terminals":evidence,"selected":selected}),
    )
}
