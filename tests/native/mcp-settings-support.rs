use super::*;

fn same_scalar(actual: &Value, expected: &Value) -> bool {
    match (actual.as_f64(), expected.as_f64()) {
        (Some(actual), Some(expected)) => actual == expected,
        _ => actual == expected,
    }
}

pub(super) async fn qualify_terminal_preferences(
    wire: &mut Wire,
    main: &Webview,
    settings: &Webview,
    directory: &Path,
    context: Value,
) -> Result<(), String> {
    let data = main
        .app_handle()
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    let path = data.join("terminal-preferences.json");
    if path.exists() {
        return Err("Terminal preference creation fixture must begin absent".into());
    }
    let mut terminals = Vec::new();
    for index in 0..2 {
        let (_, created) = layout_call(wire,"lomi_terminal_create",json!({"workspaceId":context["anchor"],"cwdRelative":".","title":format!("Settings terminal {index}"),"retryEpoch":context["retryEpoch"],"requestKey":format!("settings-terminal-{index}")})).await?;
        let target = created["structuredContent"]["data"]["result"].clone();
        let mut ready = false;
        for _ in 0..120 {
            let read = wire.tool("lomi_terminal_read",json!({"workspaceId":context["anchor"],"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"]})).await?;
            if read["structuredContent"]["data"]["prompt"] == "ready" {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if !ready {
            return Err("Settings terminal prompt not ready".into());
        }
        let run = wire.tool("lomi_terminal_run",json!({"workspaceId":context["anchor"],"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"leaseId":target["leaseId"],"command":format!("printf 'SETTINGS_RETAINED_{index}\\n'"),"retryEpoch":context["retryEpoch"],"requestKey":format!("settings-terminal-output-{index}")})).await?;
        let run = wire
            .settled(
                run["structuredContent"]["data"]["operationId"]
                    .as_str()
                    .ok_or("Terminal fixture command rejected")?,
            )
            .await?;
        if run["structuredContent"]["data"]["state"] != "succeeded" {
            return Err(format!("Terminal fixture command: {run}"));
        }
        terminals.push(target);
    }
    javascript(main,&format!("const m=await import('/src/terminal-runtime.ts');window.__settingsTerminals={}.map(t=>m.runningTerminal(t.panelId));return window.__settingsTerminals.length;",json!(terminals))).await?;
    let native_before = javascript(
        main,
        "return await window.__TAURI_INTERNALS__.invoke('terminal_contexts');",
    )
    .await?;
    if std::env::var_os("LOMI_MCP_SETTINGS_KEYBINDS_ONLY").is_some() {
        qualify_keybindings(wire, main, settings, directory, context.clone()).await?;
        layout_call(wire, "lomi_panel_focus", json!({"workspaceId":context["anchor"],"panelId":terminals[1]["panelId"],"terminalSessionId":terminals[1]["terminalSessionId"],"retryEpoch":context["retryEpoch"],"requestKey":"terminal-focus-after-shortcuts"})).await?;
    }
    if std::env::var_os("LOMI_MCP_SETTINGS_THEMES_ONLY").is_some() {
        super::theme_probe::qualify_themes(wire, main, settings, directory, context.clone())
            .await?;
        layout_call(wire, "lomi_panel_focus", json!({"workspaceId":context["anchor"],"panelId":terminals[1]["panelId"],"terminalSessionId":terminals[1]["terminalSessionId"],"retryEpoch":context["retryEpoch"],"requestKey":"terminal-focus-after-themes"})).await?;
    }
    let mut evidence = Vec::new();
    for (case, field, value) in [
        ("create", "appearance.fontSize", json!(19)),
        ("color", "appearance.colors.red", json!("#123456")),
        ("inherit", "appearance.fontSize", Value::Null),
        ("behavior", "behavior.tabStopWidth", json!(4)),
        ("invalid", "appearance.fontSize", json!(1)),
        (
            "unicode-font",
            "appearance.fontFamily",
            json!("🙂".repeat(251)),
        ),
        (
            "unicode-separator",
            "behavior.wordSeparator",
            json!("🙂".repeat(101)),
        ),
        ("stale", "alwaysShowTitles", json!(true)),
        ("recovery", "appearance.fontSize", json!(20)),
    ] {
        if case == "recovery" {
            std::fs::write(&path, b"PRIVATE_FIXTURE malformed terminal settings")
                .map_err(|e| e.to_string())?;
            main.app_handle()
                .emit("terminal-preferences-changed", ())
                .map_err(|e| e.to_string())?;
        }
        let mut snapshot = Value::Null;
        for _ in 0..80 {
            snapshot = wire
                .tool(
                    "lomi_settings_read",
                    json!({"workspaceId":context["anchor"],"section":"terminal"}),
                )
                .await?;
            if snapshot["structuredContent"]["data"]["readiness"]
                == if case == "recovery" {
                    "recovery_required"
                } else {
                    "ready"
                }
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let domain = wire.tool("lomi_workspace_list", json!({})).await?;
        let args = json!({"workspaceId":context["anchor"],"patch":{"type":"terminal_field","field":field,"value":value},"expectedSettingsRevision":snapshot["structuredContent"]["data"]["revision"],"expectedRevision":domain["structuredContent"]["data"]["domainRevision"],"retryEpoch":context["retryEpoch"],"requestKey":format!("terminal-preference-{case}")});
        let queued = wire.tool("lomi_settings_update", args.clone()).await?;
        let operation = queued["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| queued.to_string())?;
        let mut expected_unchanged = std::fs::read(&path).ok();
        if !matches!(
            case,
            "invalid" | "unicode-font" | "unicode-separator" | "recovery"
        ) {
            let selector = format!("[data-settings-operation='{operation}']");
            wait_for(
                settings,
                &format!("!!document.querySelector({})", json!(selector)),
            )
            .await?;
            if std::fs::read(&path).ok() != expected_unchanged {
                return Err("Terminal preferences changed before exact approval".into());
            }
            if case == "create" {
                evaluate(
                    settings,
                    &format!(
                        "document.querySelector({}).scrollIntoView({{block:'center'}});true",
                        json!(selector)
                    ),
                )
                .await?;
                screenshot(settings, directory.join("settings-terminal-approval.png")).await?;
            }
            if case == "stale" {
                let mut stored: Value =
                    serde_json::from_slice(expected_unchanged.as_ref().unwrap())
                        .map_err(|e| e.to_string())?;
                stored["agentNotifications"] = json!(false);
                let bytes = serde_json::to_vec_pretty(&stored).unwrap();
                std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
                expected_unchanged = Some(bytes);
                main.app_handle()
                    .emit("terminal-preferences-changed", ())
                    .map_err(|e| e.to_string())?;
            }
            click(settings, "Apply change").await?;
        }
        let mut settled = Value::Null;
        for _ in 0..160 {
            settled = wire
                .tool("lomi_operation_get", json!({"operationId":operation}))
                .await?;
            if !matches!(
                settled["structuredContent"]["data"]["state"].as_str(),
                Some("queued" | "running" | "awaiting_user" | "cancelling")
            ) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let success = !matches!(
            case,
            "invalid" | "unicode-font" | "unicode-separator" | "stale" | "recovery"
        );
        let result = &settled["structuredContent"]["data"];
        if result["state"] != if success { "succeeded" } else { "failed" }
            || result["effectState"] != if success { "complete" } else { "none" }
        {
            return Err(format!("Terminal preference {case}: {settled}"));
        }
        if success {
            if result["result"]["section"] != "terminal" {
                return Err("Terminal write receipt used wrong section".into());
            }
            let stored: Value =
                serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let pointer = format!("/{}", field.replace('.', "/"));
            if stored.pointer(&pointer).unwrap_or(&Value::Null) != &value {
                return Err(format!("Terminal field write differs: {case}"));
            }
            let mut observed = Value::Null;
            for _ in 0..100 {
                observed = wire
                    .tool(
                        "lomi_settings_read",
                        json!({"workspaceId":context["anchor"],"section":"terminal"}),
                    )
                    .await?;
                let pointer = pointer.replace("/appearance/", "/appearanceOverrides/");
                if same_scalar(
                    observed["structuredContent"]["data"]["values"]
                        .pointer(&pointer)
                        .unwrap_or(&Value::Null),
                    &value,
                ) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            let pointer = pointer.replace("/appearance/", "/appearanceOverrides/");
            if !same_scalar(
                observed["structuredContent"]["data"]["values"]
                    .pointer(&pointer)
                    .unwrap_or(&Value::Null),
                &value,
            ) {
                return Err(format!(
                    "Retained terminal provider did not observe {case} at {pointer}: {observed}"
                ));
            }
            let runtime_check = match case {
                "create" => "r.terminal.options.fontSize===19",
                "color" => "r.terminal.options.theme.red==='#123456ff'",
                "inherit" => "r.terminal.options.fontSize!==19",
                _ => "r.terminal.options.tabStopWidth===4",
            };
            wait_for(
                main,
                &format!("window.__settingsTerminals.every(r=>{runtime_check})"),
            )
            .await?;
        } else if std::fs::read(&path).ok() != expected_unchanged {
            return Err(format!("Rejected terminal {case} changed stored data"));
        }
        let retry = wire.tool("lomi_settings_update", args).await?;
        if retry["structuredContent"] != settled["structuredContent"] {
            return Err("Terminal write retry differed".into());
        }
        evidence.push(json!({"case":case,"result":settled,"retry":retry}));
    }
    let retained=javascript(main,&format!("const m=await import('/src/terminal-runtime.ts');return {}.map((t,i)=>{{const r=m.runningTerminal(t.panelId);const b=r.terminal.buffer.active;const lines=Array.from({{length:b.length}},(_,n)=>b.getLine(n)?.translateToString(true)??'').join('\\n');return {{same:r===window.__settingsTerminals[i],sameSession:r.sessionId===t.terminalSessionId,output:lines.includes('SETTINGS_RETAINED_'+i),running:r.getSnapshot().status==='running',visible:!!r.host.offsetParent}};}});",json!(terminals))).await?;
    if retained.as_array().is_none_or(|a| {
        a.len() != 2
            || a.iter().any(|v| {
                v["same"] != true
                    || v["sameSession"] != true
                    || v["output"] != true
                    || v["running"] != true
            })
    }) {
        return Err(format!(
            "Terminal preferences replaced retained runtime/output: {retained}"
        ));
    }
    if retained[0]["visible"] != false || retained[1]["visible"] != true {
        return Err(format!(
            "Hidden/visible terminal fixture mismatch: {retained}"
        ));
    }
    let native_after = javascript(
        main,
        "return await window.__TAURI_INTERNALS__.invoke('terminal_contexts');",
    )
    .await?;
    if native_after != native_before {
        return Err("Terminal preference writes changed native process contexts".into());
    }
    std::fs::write(
        directory.join("settings-terminal.json"),
        serde_json::to_vec_pretty(
            &json!({"cases":evidence,"retained":retained,"nativeContextsUnchanged":true}),
        )
        .unwrap(),
    )
    .map_err(|e| e.to_string())?;
    // Restore only this isolated fixture, then use ordinary owned-panel closure.
    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    main.app_handle()
        .emit("terminal-preferences-changed", ())
        .map_err(|e| e.to_string())?;
    for (i, target) in terminals.iter().enumerate() {
        layout_call(wire,"lomi_panel_close",json!({"workspaceId":context["anchor"],"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"retryEpoch":context["retryEpoch"],"requestKey":format!("settings-terminal-close-{i}")})).await?;
    }
    Ok(())
}

async fn qualify_keybindings(
    wire: &mut Wire,
    main: &Webview,
    settings: &Webview,
    directory: &Path,
    context: Value,
) -> Result<(), String> {
    let editor_args = json!({"workspaceId":context["anchor"],"panelId":"mcp-control-fixture","relativePath":"fixture.txt"});
    let original_document = wire.tool("lomi_editor_read", editor_args.clone()).await?;
    javascript(main, "const m=await import('/src/editor-runtime.ts');const d=m.documents().find(d=>d.location.relative==='fixture.txt');window.__shortcutDocument=d;d.dispatch({changes:{from:0,to:d.state.doc.length,insert:'Shortcut retained draft 🙂\\n'}});return true;").await?;
    let data = main
        .app_handle()
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    let path = data.join("keybindings.json");
    if path.exists() {
        return Err("Shortcut fixture must begin absent".into());
    }
    let plugin_preferences = data.join("plugins/installed.json");
    let plugin_original = std::fs::read(&plugin_preferences).ok();
    let mut evidence = Vec::new();
    let mut last_revision: Option<String> = None;
    for (case, patch) in [
        (
            "create",
            json!({"type":"keybinding_set","action":"saveFile","shortcut":"Ctrl+Alt+F20"}),
        ),
        (
            "disable",
            json!({"type":"keybinding_set","action":"saveFile","shortcut":null}),
        ),
        (
            "reset",
            json!({"type":"keybinding_reset","action":"saveFile"}),
        ),
        (
            "focus",
            json!({"type":"keybinds_focus_follows_pointer","value":true}),
        ),
        (
            "conflict",
            json!({"type":"keybinding_set","action":"chatNew","shortcut":"Meta+KeyS"}),
        ),
        (
            "invalid",
            json!({"type":"keybinding_set","action":"saveFile","shortcut":"KeyA"}),
        ),
        (
            "unavailable",
            json!({"type":"keybinding_set","action":"missing.plugin","shortcut":null}),
        ),
        (
            "reject",
            json!({"type":"keybinding_set","action":"saveFile","shortcut":"Ctrl+Alt+F21"}),
        ),
        (
            "cancel",
            json!({"type":"keybinding_set","action":"saveFile","shortcut":"Ctrl+Alt+F21"}),
        ),
        (
            "stale",
            json!({"type":"keybinding_set","action":"saveFile","shortcut":"Ctrl+Alt+F21"}),
        ),
        (
            "definitions",
            json!({"type":"keybinding_set","action":"saveFile","shortcut":"Ctrl+Alt+F21"}),
        ),
        (
            "recovery",
            json!({"type":"keybinding_set","action":"saveFile","shortcut":"Ctrl+Alt+F21"}),
        ),
    ] {
        if case == "recovery" {
            std::fs::write(&path, b"PRIVATE_FIXTURE malformed shortcuts")
                .map_err(|e| e.to_string())?;
            main.app_handle()
                .emit("keybindings-changed", ())
                .map_err(|e| e.to_string())?;
        }
        let mut snapshot = Value::Null;
        for _ in 0..100 {
            snapshot = wire
                .tool(
                    "lomi_settings_read",
                    json!({"workspaceId":context["anchor"],"section":"keybinds","limit":200}),
                )
                .await?;
            let s = &snapshot["structuredContent"]["data"];
            if s["readiness"]
                == if case == "recovery" {
                    "recovery_required"
                } else {
                    "ready"
                }
                && last_revision.as_ref().is_none_or(|r| s["revision"] != *r)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let snapshot = &snapshot["structuredContent"]["data"];
        last_revision = None;
        let domain = wire.tool("lomi_workspace_list", json!({})).await?;
        let args = json!({"workspaceId":context["anchor"],"patch":patch,"expectedSettingsRevision":snapshot["revision"],"expectedRevision":domain["structuredContent"]["data"]["domainRevision"],"retryEpoch":context["retryEpoch"],"requestKey":format!("settings-keybinds-{case}")});
        let mut unchanged = std::fs::read(&path).ok();
        let queued = wire.tool("lomi_settings_update", args.clone()).await?;
        let operation = queued["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| format!("Shortcut queue {case}: {queued}"))?;
        if !matches!(case, "conflict" | "invalid" | "unavailable" | "recovery") {
            let selector = format!("[data-settings-operation='{operation}']");
            wait_for(
                settings,
                &format!("!!document.querySelector({})", json!(selector)),
            )
            .await?;
            if std::fs::read(&path).ok() != unchanged {
                return Err("Shortcut changed before approval".into());
            }
            if case == "reset" {
                evaluate(
                    settings,
                    &format!(
                        "document.querySelector({}).scrollIntoView({{block:'center'}});true",
                        json!(selector)
                    ),
                )
                .await?;
                screenshot(
                    settings,
                    directory.join("settings-keybindings-approval.png"),
                )
                .await?;
            }
            if case == "stale" {
                let mut stored: Value =
                    serde_json::from_slice(unchanged.as_ref().unwrap()).unwrap();
                stored["opaque"] = json!({"preserved":true});
                stored["bindings"]["missing.plugin"] = Value::Null;
                let bytes = serde_json::to_vec_pretty(&stored).unwrap();
                std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
                unchanged = Some(bytes);
                last_revision = snapshot["revision"].as_str().map(str::to_owned);
                main.app_handle()
                    .emit("keybindings-changed", ())
                    .map_err(|e| e.to_string())?;
            }
            if case == "definitions" {
                let fixture = directory.join("shortcut-package");
                std::fs::create_dir(&fixture).map_err(|e| e.to_string())?;
                std::fs::write(fixture.join("plugin.json"), serde_json::to_vec(&json!({"schemaVersion":1,"id":"fixture.shortcuts","name":"Shortcut fixture","version":"1.0.0","description":"Disabled metadata fixture","hostApi":1,"entry":"index.js","activation":"startup","contributes":{"commands":[{"id":"fixture.shortcuts.open","label":"Fixture action","description":"Never evaluated"}],"keybindings":[{"command":"fixture.shortcuts.open","shortcut":"Ctrl+Alt+F22"}]}})).unwrap()).map_err(|e| e.to_string())?;
                std::fs::write(
                    fixture.join("index.js"),
                    b"throw new Error('Disabled shortcut fixture must not execute');",
                )
                .map_err(|e| e.to_string())?;
                javascript(settings, &format!("return await window.__TAURI_INTERNALS__.invoke('import_plugin',{{path:{}}});", json!(fixture.to_string_lossy()))).await?;
            }
            if case == "cancel" {
                wire.tool("lomi_operation_cancel", json!({"operationId":operation}))
                    .await?;
            } else {
                click(
                    settings,
                    if case == "reject" {
                        "Reject change"
                    } else {
                        "Apply change"
                    },
                )
                .await?;
            }
        }
        let mut settled = Value::Null;
        for _ in 0..160 {
            settled = wire
                .tool("lomi_operation_get", json!({"operationId":operation}))
                .await?;
            if !matches!(
                settled["structuredContent"]["data"]["state"].as_str(),
                Some("queued" | "running" | "awaiting_user" | "cancelling")
            ) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let success = matches!(case, "create" | "disable" | "reset" | "focus");
        let state = if success {
            "succeeded"
        } else if matches!(case, "reject" | "cancel") {
            "cancelled"
        } else {
            "failed"
        };
        let result = &settled["structuredContent"]["data"];
        if result["state"] != state
            || result["effectState"] != if success { "complete" } else { "none" }
        {
            return Err(format!("Shortcut {case}: {settled}"));
        }
        if success {
            if result["result"]["section"] != "keybinds" {
                return Err("Wrong shortcut receipt section".into());
            }
            let mut expected: Value = unchanged
                .as_deref()
                .map(serde_json::from_slice)
                .transpose()
                .unwrap()
                .unwrap_or(json!({"version":1,"bindings":{}}));
            match patch["type"].as_str().unwrap() {
                "keybinding_set" => {
                    expected["bindings"][patch["action"].as_str().unwrap()] =
                        patch["shortcut"].clone();
                }
                "keybinding_reset" => {
                    expected["bindings"]
                        .as_object_mut()
                        .unwrap()
                        .remove(patch["action"].as_str().unwrap());
                }
                _ => expected["focusFollowsPointer"] = patch["value"].clone(),
            }
            let stored: Value =
                serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?).unwrap();
            if stored != expected {
                return Err(format!("Shortcut {case} changed unrelated data"));
            }
            last_revision = snapshot["revision"].as_str().map(str::to_owned);
        } else if std::fs::read(&path).ok() != unchanged {
            return Err(format!("Rejected shortcut {case} changed bytes"));
        }
        if case == "definitions" {
            let catalog = javascript(
                settings,
                "return await window.__TAURI_INTERNALS__.invoke('list_plugins');",
            )
            .await?;
            let plugin = catalog["entries"]
                .as_array()
                .and_then(|a| a.iter().find(|e| e["id"] == "fixture.shortcuts"))
                .ok_or("Fixture plugin missing")?;
            if plugin["enabled"] != false || plugin["evaluated"] != false {
                return Err("Shortcut preflight enabled/evaluated a plugin".into());
            }
            if let Some(bytes) = &plugin_original {
                std::fs::write(&plugin_preferences, bytes).map_err(|e| e.to_string())?;
            } else {
                std::fs::remove_file(&plugin_preferences).map_err(|e| e.to_string())?;
            }
            main.app_handle()
                .emit("plugins-changed", ())
                .map_err(|e| e.to_string())?;
        }
        let retry = wire.tool("lomi_settings_update", args).await?;
        if retry["structuredContent"] != settled["structuredContent"] {
            return Err(format!("Shortcut retry differs: {case}"));
        }
        evidence.push(json!({"case":case,"result":settled,"retry":retry}));
    }
    let dirty = wire.tool("lomi_editor_read", editor_args.clone()).await?;
    if dirty["structuredContent"]["data"]["content"] != "Shortcut retained draft 🙂\n"
        || dirty["structuredContent"]["data"]["dirty"] != true
        || dirty["structuredContent"]["data"]["documentId"] != original_document["structuredContent"]["data"]["documentId"]
        || javascript(main, "const m=await import('/src/editor-runtime.ts');return m.documents().find(d=>d.location.relative==='fixture.txt')===window.__shortcutDocument;").await? != true {
        return Err("Shortcut updates replaced the existing dirty document".into());
    }
    layout_call(wire, "lomi_panel_focus", json!({"workspaceId":context["anchor"],"panelId":"mcp-control-fixture","retryEpoch":context["retryEpoch"],"requestKey":"shortcut-undo-focus"})).await?;
    wait_for(main, "Boolean(window.__shortcutDocument?.view?.dom.isConnected && window.__shortcutDocument.view.dom.getClientRects().length)").await?;
    javascript(
        main,
        "window.__shortcutDocument.command('undo');return true;",
    )
    .await?;
    let undone = wire.tool("lomi_editor_read", editor_args).await?;
    if undone["structuredContent"]["data"]["content"]
        != original_document["structuredContent"]["data"]["content"]
    {
        return Err("Shortcut updates discarded existing Undo history".into());
    }
    std::fs::write(
        directory.join("settings-keybindings.json"),
        serde_json::to_vec_pretty(
            &json!({"cases":evidence,"pluginNeverEnabled":true,"exactStoredPatches":true,"retainedDocument":true,"dirty":dirty,"undo":undone}),
        )
        .unwrap(),
    )
    .map_err(|e| e.to_string())?;
    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    main.app_handle()
        .emit("keybindings-changed", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}
