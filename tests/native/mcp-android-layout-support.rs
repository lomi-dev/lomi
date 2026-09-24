//! Retained Android layout and last-view closure in the licensed private fixture.
use super::*;

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}
async fn mutate(wire: &mut Wire, tool: &str, args: Value) -> Result<Value, String> {
    let (args, result) = layout_call(wire, tool, args).await?;
    let repeated = wire.tool(tool, args).await?;
    require(
        repeated["structuredContent"]["data"] == result["structuredContent"]["data"],
        "Android layout retry changed its receipt",
    )?;
    Ok(result["structuredContent"]["data"]["result"].clone())
}
async fn open(
    wire: &mut Wire,
    workspace: &Value,
    device: &Value,
    epoch: &Value,
    key: &str,
) -> Result<Value, String> {
    let result = mutate(
        wire,
        "lomi_android_open",
        json!({"workspaceId":workspace,"deviceId":device,"retryEpoch":epoch,"requestKey":key}),
    )
    .await?;
    result
        .get("panelId")
        .cloned()
        .ok_or_else(|| format!("Missing Android view: {result}"))
}
async fn start(
    wire: &mut Wire,
    workspace: &Value,
    panel: &Value,
    device: &Value,
    epoch: &Value,
    key: &str,
) -> Result<Value, String> {
    let result = mutate(wire, "lomi_android_start", json!({"workspaceId":workspace,"panelId":panel,"deviceId":device,"retryEpoch":epoch,"requestKey":key})).await?;
    require(
        result["ready"] == true && result["stopped"] == false,
        "Android did not become ready",
    )?;
    Ok(result["generation"].clone())
}
async fn running(wire: &mut Wire, workspace: &Value, generation: &Value) -> Result<(), String> {
    let list = wire
        .tool("lomi_android_list", json!({"workspaceId":workspace}))
        .await?;
    let device = &list["structuredContent"]["data"]["devices"]["items"][0];
    require(
        device["phase"] == "running"
            && device["processAlive"] == true
            && device["generation"] == *generation,
        &format!("Android generation changed during layout: {list}"),
    )
}
fn stopped(app: &tauri::AppHandle, device: &Value) -> Result<(), String> {
    let manager = app
        .state::<crate::android::manager::Android>()
        .loaded()
        .ok_or("Missing Android manager")?;
    require(
        manager
            .current_statuses()?
            .iter()
            .find(|s| json!(s.device_id) == *device)
            .is_some_and(|s| {
                !s.process_alive && serde_json::to_value(s.phase).ok() == Some(json!("stopped"))
            }),
        "Last-view close did not confirm Android exit",
    )
}

pub(super) async fn qualify(
    app: &tauri::AppHandle,
    wire: &mut Wire,
    main: &Webview,
    workspace: &Value,
    epoch: &Value,
    directory: &Path,
) -> Result<(), String> {
    let fixture: Value = serde_json::from_slice(
        &std::fs::read(directory.join("android-fixture.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let device = &fixture["deviceId"];
    let mut checks = Vec::new();
    activate_main(app).await?;
    let panel = open(wire, workspace, device, epoch, "layout-open").await?;
    let generation = start(wire, workspace, &panel, device, epoch, "layout-start").await?;
    let shared = open(wire, workspace, device, epoch, "layout-shared").await?;
    running(wire, workspace, &generation).await?;
    checks.push("second view preserves one native generation");
    mutate(wire, "lomi_panel_focus", json!({"workspaceId":workspace,"panelId":panel,"retryEpoch":epoch,"requestKey":"phone-focus"})).await?;
    wait_for(
        main,
        &format!(
            "Boolean(document.querySelector('[data-android-pane-id=\"{}\"] canvas'))",
            panel.as_str().unwrap()
        ),
    )
    .await?;
    javascript(main, &format!("const r=await import('/src/android/runtime.ts');r.setZoom({},125.0);return r.viewZoom({});",panel,panel)).await?.as_f64().filter(|v| *v == 125.0).ok_or("Cannot prepare Android zoom")?;
    checks.push("existing view focus without restart");
    let terminal = mutate(wire, "lomi_terminal_create", json!({"workspaceId":workspace,"cwdRelative":".","title":"Android layout fixture","retryEpoch":epoch,"requestKey":"layout-terminal"})).await?;
    let terminal_read = json!({"workspaceId":workspace,"panelId":terminal["panelId"],"terminalSessionId":terminal["terminalSessionId"]});
    let mut idle = false;
    for _ in 0..100 {
        if wire
            .tool("lomi_terminal_read", terminal_read.clone())
            .await?["structuredContent"]["data"]["prompt"]
            == "ready"
        {
            idle = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    require(idle, "Layout PTY did not reach a prompt")?;
    let panels = wire
        .tool("lomi_panel_list", json!({"workspaceId":workspace}))
        .await?;
    let tab = panels["structuredContent"]["data"]["items"]
        .as_array()
        .and_then(|items| items.iter().find(|p| p["id"] == terminal["panelId"]))
        .ok_or("Missing PTY panel")?["tabId"]
        .clone();
    for (key, movement) in [
        (
            "dock",
            json!({"type":"dock_tab","tabId":panel,"targetTabId":tab,"side":"right"}),
        ),
        (
            "move",
            json!({"type":"move_pane","panelId":panel,"targetPanelId":terminal["panelId"],"side":"left"}),
        ),
        (
            "reorder",
            json!({"type":"reorder_tab","tabId":tab,"beforeTabId":null}),
        ),
    ] {
        mutate(wire,"lomi_panel_move",json!({"workspaceId":workspace,"movement":movement,"retryEpoch":epoch,"requestKey":format!("phone-{key}")})).await?;
        running(wire, workspace, &generation).await?;
        checks.push(key);
    }
    let zoom = javascript(
        main,
        &format!(
            "return (await import('/src/android/runtime.ts')).viewZoom({});",
            panel
        ),
    )
    .await?;
    require(zoom == 125.0, "Docking lost retained Android zoom")?;
    screenshot(main, directory.join("android-mixed-layout.png")).await?;
    checks.push("mixed native Android and PTY with retained zoom");
    let destination = json!("foreign-workspace");
    mutate(wire,"lomi_panel_move",json!({"workspaceId":workspace,"movement":{"type":"transfer_tab","tabId":tab,"targetWorkspaceId":destination,"beforeTabId":null},"retryEpoch":epoch,"requestKey":"phone-transfer"})).await?;
    mutate(wire,"lomi_panel_focus",json!({"workspaceId":destination,"panelId":panel,"retryEpoch":epoch,"requestKey":"phone-destination-focus"})).await?;
    running(wire, &destination, &generation).await?;
    require(
        javascript(
            main,
            &format!(
                "return (await import('/src/android/runtime.ts')).viewZoom({});",
                panel
            ),
        )
        .await?
            == zoom,
        "Workspace transfer lost Android zoom",
    )?;
    let transferred = wire.tool("lomi_terminal_read",json!({"workspaceId":destination,"panelId":terminal["panelId"],"terminalSessionId":terminal["terminalSessionId"]})).await?;
    require(
        transferred["structuredContent"]["status"] == "ok",
        "Mixed workspace transfer lost PTY ownership",
    )?;
    checks.push("workspace transfer retains generation PTY and viewport");
    mutate(wire,"lomi_panel_close",json!({"workspaceId":workspace,"panelId":shared,"retryEpoch":epoch,"requestKey":"close-shared-view"})).await?;
    running(wire, &destination, &generation).await?;
    checks.push("closing a shared view leaves Android running");
    mutate(wire,"lomi_workspace_update",json!({"workspaceId":destination,"action":"close","retryEpoch":epoch,"requestKey":"close-phone-workspace"})).await?;
    stopped(app, device)?;
    checks.push("workspace closure confirms last-view Stop");
    let panel = open(wire, workspace, device, epoch, "panel-close-open").await?;
    let next = start(wire, workspace, &panel, device, epoch, "panel-close-start").await?;
    require(next != generation, "Restart reused Android generation")?;
    mutate(wire,"lomi_panel_close",json!({"workspaceId":workspace,"panelId":panel,"retryEpoch":epoch,"requestKey":"close-last-panel"})).await?;
    stopped(app, device)?;
    checks.push("panel closure confirms Stop and does not replay");
    let panel = open(wire, workspace, device, epoch, "project-close-open").await?;
    let generation = start(
        wire,
        workspace,
        &panel,
        device,
        epoch,
        "project-close-start",
    )
    .await?;
    mutate(wire,"lomi_panel_focus",json!({"workspaceId":workspace,"panelId":"mcp-control-fixture","retryEpoch":epoch,"requestKey":"close-editor-focus"})).await?;
    javascript(main,"const m=await import('/src/editor-runtime.ts');const d=m.documents().find(d=>d.location.relative==='fixture.txt');d.dispatch({changes:{from:d.state.doc.length,insert:'Android close guard 日本語 🙂'}});return true;").await?;
    let domain = wire.tool("lomi_workspace_list", json!({})).await?;
    let project = domain["structuredContent"]["data"]["items"]
        .as_array()
        .and_then(|items| items.iter().find(|w| w["id"] == *workspace))
        .ok_or("Missing project")?["projectId"]
        .clone();
    let mut outcomes = Vec::new();
    for (key, label) in [("cancel", "Cancel"), ("discard", "Discard changes")] {
        let domain = wire.tool("lomi_workspace_list", json!({})).await?;
        let args = json!({"workspaceId":workspace,"projectId":project,"expectedRevision":domain["structuredContent"]["data"]["domainRevision"],"retryEpoch":epoch,"requestKey":format!("android-project-{key}")});
        let queued = wire.tool("lomi_project_close", args.clone()).await?;
        let operation = queued["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| queued.to_string())?;
        wait_for(main, "Boolean(document.querySelector('dialog[open]'))").await?;
        running(wire, workspace, &generation).await?;
        if key == "cancel" {
            screenshot(main, directory.join("android-close-guard.png")).await?;
        }
        click(main, label).await?;
        let result = wire.settled_with_limit(operation, 7200).await?;
        if key == "cancel" {
            require(
                result["structuredContent"]["data"]["state"] == "failed",
                "Cancelled dirty guard did not fail without closing",
            )?;
            running(wire, workspace, &generation).await?;
            checks.push("dirty editor cancellation precedes Android Stop");
        } else {
            require(
                result["structuredContent"]["data"]["state"] == "succeeded",
                "Project closure failed",
            )?;
            stopped(app, device)?;
            checks.push("project closure stops Android after dirty guard");
        }
        let replay = wire.tool("lomi_project_close", args).await?;
        require(
            replay["structuredContent"]["data"] == result["structuredContent"]["data"],
            "Ancestor close replay changed receipt",
        )?;
        outcomes.push(result);
    }
    let contexts = javascript(
        main,
        "return await window.__TAURI_INTERNALS__.invoke('terminal_contexts');",
    )
    .await?;
    require(
        contexts
            .as_object()
            .is_some_and(|c| !c.contains_key(terminal["terminalSessionId"].as_str().unwrap())),
        "Closed mixed PTY survived",
    )?;
    checks.push("native terminal cleanup follows ancestor closure");
    let manager = app
        .state::<crate::android::manager::Android>()
        .loaded()
        .ok_or("Missing manager")?;
    let devices = manager
        .directory
        .lock()
        .map_err(|_| "Metadata lock")?
        .devices()?;
    let preserved = devices
        .devices
        .iter()
        .any(|d| json!(d.id) == *device && d.name == "MCP qualification");
    require(preserved, "Layout changed fixture device metadata")?;
    checks.push("device data remains configured after all closes");
    std::fs::write(directory.join("android-layout.json"),serde_json::to_vec_pretty(&json!({"checks":checks,"outcomes":outcomes,"terminalContexts":contexts,"stopped":true,"originalDevicePreserved":preserved})).unwrap()).map_err(|e|e.to_string())?;
    Ok(())
}
