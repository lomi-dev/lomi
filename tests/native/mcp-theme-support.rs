use super::*;

pub(super) async fn qualify_themes(
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
    let path = data.join("theme-settings.json");
    if path.exists() {
        return Err("Theme fixture must begin absent".into());
    }
    let editor_args = json!({"workspaceId":context["anchor"],"panelId":"mcp-control-fixture","relativePath":"fixture.txt"});
    let original = wire.tool("lomi_editor_read", editor_args.clone()).await?;
    javascript(main, "const m=await import('/src/editor-runtime.ts');const d=m.documents().find(d=>d.location.relative==='fixture.txt');window.__themeDocument=d;d.dispatch({changes:{from:0,to:d.state.doc.length,insert:'Theme retained draft 🙂\\n'}});return true;").await?;
    let mut evidence = Vec::new();
    for (case, patch) in [
        ("create", json!({"type":"theme_builtin","value":"deepmono"})),
        ("light", json!({"type":"theme_appearance","value":"light"})),
        ("dark", json!({"type":"theme_appearance","value":"dark"})),
        (
            "system",
            json!({"type":"theme_appearance","value":"system"}),
        ),
        ("lomi", json!({"type":"theme_builtin","value":"lomi"})),
        ("reject", json!({"type":"theme_builtin","value":"deepmono"})),
        ("cancel", json!({"type":"theme_builtin","value":"deepmono"})),
        ("stale", json!({"type":"theme_builtin","value":"deepmono"})),
        (
            "recovery",
            json!({"type":"theme_builtin","value":"deepmono"}),
        ),
    ] {
        if case == "recovery" {
            std::fs::write(&path, b"PRIVATE_FIXTURE corrupt theme preferences")
                .map_err(|e| e.to_string())?;
            main.app_handle()
                .emit("theme-changed", ())
                .map_err(|e| e.to_string())?;
        }
        let mut snapshot = Value::Null;
        for _ in 0..100 {
            snapshot = wire
                .tool(
                    "lomi_settings_read",
                    json!({"workspaceId":context["anchor"],"section":"themes"}),
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
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let domain = wire.tool("lomi_workspace_list", json!({})).await?;
        let args = json!({"workspaceId":context["anchor"],"patch":patch,"expectedSettingsRevision":snapshot["structuredContent"]["data"]["revision"],"expectedRevision":domain["structuredContent"]["data"]["domainRevision"],"retryEpoch":context["retryEpoch"],"requestKey":format!("settings-theme-{case}")});
        let mut unchanged = std::fs::read(&path).ok();
        let queued = wire.tool("lomi_settings_update", args.clone()).await?;
        let operation = queued["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| format!("Theme {case} queue: {queued}"))?;
        if case != "recovery" {
            let selector = format!("[data-settings-operation='{operation}']");
            wait_for(
                settings,
                &format!("!!document.querySelector({})", json!(selector)),
            )
            .await?;
            if std::fs::read(&path).ok() != unchanged {
                return Err("Theme changed before exact approval".into());
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
                screenshot(settings, directory.join("settings-theme-approval.png")).await?;
            }
            if case == "stale" {
                let mut stored: Value =
                    serde_json::from_slice(unchanged.as_ref().unwrap()).unwrap();
                stored["customCss"] = json!(true);
                let bytes = serde_json::to_vec_pretty(&stored).unwrap();
                std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
                unchanged = Some(bytes);
                main.app_handle()
                    .emit("theme-changed", ())
                    .map_err(|e| e.to_string())?;
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
        let success = matches!(case, "create" | "light" | "dark" | "system" | "lomi");
        let expected_state = if success {
            "succeeded"
        } else if matches!(case, "reject" | "cancel") {
            "cancelled"
        } else {
            "failed"
        };
        let result = &settled["structuredContent"]["data"];
        if result["state"] != expected_state
            || result["effectState"] != if success { "complete" } else { "none" }
        {
            return Err(format!("Theme {case}: {settled}"));
        }
        if success {
            if result["result"]["section"] != "themes" {
                return Err("Theme receipt used wrong section".into());
            }
            let mut expected: Value = unchanged
                .as_deref()
                .map(serde_json::from_slice)
                .transpose()
                .unwrap()
                .unwrap_or(json!({"version":1,"active":null}));
            let key = if patch["type"] == "theme_builtin" {
                "active"
            } else {
                "appearance"
            };
            let value = if key == "active" {
                if patch["value"] == "lomi" {
                    Value::Null
                } else {
                    json!("@builtin-deepmono")
                }
            } else {
                patch["value"].clone()
            };
            expected[key] = value.clone();
            let stored: Value =
                serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?).unwrap();
            if stored != expected {
                return Err(format!("Theme {case} changed unrelated fields"));
            }
            let mut observed = Value::Null;
            for _ in 0..100 {
                observed = wire
                    .tool(
                        "lomi_settings_read",
                        json!({"workspaceId":context["anchor"],"section":"themes"}),
                    )
                    .await?;
                if observed["structuredContent"]["data"]["values"][key] == value {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            if observed["structuredContent"]["data"]["values"][key] != value {
                return Err(format!("Theme provider did not observe {case}"));
            }
            let mut runtimes = Value::Null;
            for _ in 0..100 {
                runtimes = javascript(main, "const t=await import('/src/theme/runtime.ts');const current=t.terminalAppearance();return window.__settingsTerminals.every(r=>r.terminal.options.theme.background===current.theme.background && r.terminal.options.fontFamily===current.fontFamily);").await?;
                if runtimes == true {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            if runtimes != true {
                return Err("Retained terminals did not apply computed theme".into());
            }
            if matches!(case, "light" | "dark") {
                wait_for(
                    main,
                    &format!(
                        "document.documentElement.dataset.appearance==={}",
                        json!(case)
                    ),
                )
                .await?;
                screenshot(main, directory.join(format!("settings-theme-{case}.png"))).await?;
            }
        } else if std::fs::read(&path).ok() != unchanged {
            return Err(format!("Rejected theme {case} changed bytes"));
        }
        let retry = wire.tool("lomi_settings_update", args).await?;
        if retry["structuredContent"] != settled["structuredContent"] {
            return Err("Theme retry differs".into());
        }
        evidence.push(json!({"case":case,"result":settled,"retry":retry}));
    }
    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    main.app_handle()
        .emit("theme-changed", ())
        .map_err(|e| e.to_string())?;
    let protection = qualify_protected_controls(wire, main, settings, directory, &context).await?;
    let dirty = wire.tool("lomi_editor_read", editor_args.clone()).await?;
    if dirty["structuredContent"]["data"]["content"] != "Theme retained draft 🙂\n"
        || dirty["structuredContent"]["data"]["dirty"] != true
        || dirty["structuredContent"]["data"]["documentId"] != original["structuredContent"]["data"]["documentId"]
        || javascript(main, "const m=await import('/src/editor-runtime.ts');return m.documents().find(d=>d.location.relative==='fixture.txt')===window.__themeDocument;").await? != true { return Err("Theme updates replaced the dirty document".into()); }
    layout_call(wire, "lomi_panel_focus", json!({"workspaceId":context["anchor"],"panelId":"mcp-control-fixture","retryEpoch":context["retryEpoch"],"requestKey":"theme-undo-focus"})).await?;
    wait_for(main, "Boolean(window.__themeDocument?.view?.dom.isConnected && window.__themeDocument.view.dom.getClientRects().length)").await?;
    javascript(main, "window.__themeDocument.command('undo');return true;").await?;
    let undone = wire.tool("lomi_editor_read", editor_args).await?;
    if undone["structuredContent"]["data"]["content"]
        != original["structuredContent"]["data"]["content"]
    {
        return Err("Theme update destroyed Undo".into());
    }
    std::fs::write(directory.join("settings-themes.json"), serde_json::to_vec_pretty(&json!({"cases":evidence,"protection":protection,"retainedDocument":true,"dirty":dirty,"undo":undone,"computedTerminalThemes":true})).unwrap()).map_err(|e| e.to_string())?;
    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    main.app_handle()
        .emit("theme-changed", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn qualify_protected_controls(
    wire: &mut Wire,
    main: &Webview,
    settings: &Webview,
    directory: &Path,
    context: &Value,
) -> Result<Value, String> {
    let source = directory.join("hostile-theme-source");
    std::fs::create_dir(&source).map_err(|e| e.to_string())?;
    let manifest = json!({"version":2,"name":"Hostile style fixture","common":{"styles":{"html, body":{"opacity":"0","pointer-events":"none"}}},"resources":{"stylesheets":["hide.css"]}});
    std::fs::write(
        source.join("theme.jsonc"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        source.join("hide.css"),
        "button, dialog { display: none !important; }",
    )
    .map_err(|e| e.to_string())?;
    let id = javascript(settings, &format!("const id=await window.__TAURI_INTERNALS__.invoke('import_theme',{{path:{}}});await window.__TAURI_INTERNALS__.invoke('save_theme_preferences',{{data:{{version:1,active:id}}}});return id;", json!(source.to_string_lossy()))).await?;
    for view in [main, settings] {
        wait_for(view, &format!("document.documentElement.dataset.theme === {} && !!document.querySelector('link[data-theme-layer=css]')", id)).await?;
    }
    wait_for(main, "getComputedStyle(document.body).opacity==='0'").await?;
    let visible = "getComputedStyle(document.body).opacity==='1' && Array.from(document.querySelectorAll('button')).some(b=>b.getClientRects().length && getComputedStyle(b).display!=='none')";
    wait_for(settings, visible).await?;
    let path = main
        .app_handle()
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("theme-settings.json");
    let selected_bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    javascript(main, "const fixture=await import('/tests/ui/protected-theme-fixture.tsx');fixture.mountProtectedModal();return true;").await?;
    wait_for(
        main,
        &format!("({visible}) && !!document.querySelector('dialog[open]')"),
    )
    .await?;
    for view in [main, settings] {
        evaluate(
            view,
            "window.__protectedThemeLayer=document.querySelector('[data-theme-layer=tokens]');true",
        )
        .await?;
    }
    main.app_handle()
        .emit("theme-changed", ())
        .map_err(|e| e.to_string())?;
    for view in [main, settings] {
        wait_for(
            view,
            "document.querySelector('[data-theme-layer=tokens]')!==window.__protectedThemeLayer",
        )
        .await?;
        wait_for(view, visible).await?;
    }
    screenshot(main, directory.join("theme-protected-dialog.png")).await?;
    screenshot(settings, directory.join("theme-protected-settings.png")).await?;
    click(main, "Reject fixture action").await?;
    wait_for(
        main,
        "getComputedStyle(document.body).opacity==='0' && !document.querySelector('dialog[open]')",
    )
    .await?;
    click(settings, "Themes").await?;
    wait_for(settings, "getComputedStyle(document.body).opacity==='0'").await?;
    crate::macos::handle_menu_event(
        main.app_handle(),
        tauri::menu::MenuEvent {
            id: "open-agent-control".into(),
        },
    );
    wait_for(
        settings,
        &format!("({visible}) && !!document.querySelector('.agent-control-page')"),
    )
    .await?;
    if std::fs::read(&path).map_err(|e| e.to_string())? != selected_bytes {
        return Err("Control protection changed selected theme preferences".into());
    }
    let snapshot = wire
        .tool(
            "lomi_settings_read",
            json!({"workspaceId":context["anchor"],"section":"themes"}),
        )
        .await?;
    if snapshot["structuredContent"]["data"]["values"]["active"] != id {
        return Err("Protected controls changed effective theme selection".into());
    }
    let domain = wire.tool("lomi_workspace_list", json!({})).await?;
    let args = json!({"workspaceId":context["anchor"],"patch":{"type":"theme_builtin","value":"lomi"},"expectedSettingsRevision":snapshot["structuredContent"]["data"]["revision"],"expectedRevision":domain["structuredContent"]["data"]["domainRevision"],"retryEpoch":context["retryEpoch"],"requestKey":"settings-theme-hostile-restore"});
    let queued = wire.tool("lomi_settings_update", args.clone()).await?;
    let operation = queued["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| format!("Theme protection restore queue: {queued}"))?;
    wait_for(
        settings,
        &format!("!!document.querySelector('[data-settings-operation=\"{operation}\"]')"),
    )
    .await?;
    wait_for(settings, visible).await?;
    if std::fs::read(&path).map_err(|e| e.to_string())? != selected_bytes {
        return Err("Theme restore bypassed exact approval".into());
    }
    click(settings, "Apply change").await?;
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
    if settled["structuredContent"]["data"]["state"] != "succeeded"
        || settled["structuredContent"]["data"]["effectState"] != "complete"
    {
        return Err(format!("Theme protection restore: {settled}"));
    }
    for view in [main, settings] {
        wait_for(
            view,
            &format!("({visible}) && document.documentElement.dataset.theme==='lomi'"),
        )
        .await?;
    }
    let retry = wire.tool("lomi_settings_update", args).await?;
    if retry["structuredContent"] != settled["structuredContent"] {
        return Err("Protected theme restoration retry differs".into());
    }
    Ok(
        json!({"jsonStyles":true,"externalStylesheet":true,"lateReload":true,"nativeMenuHandler":true,"modalRestore":true,"preferencesUnchanged":true,"restore":settled,"retry":retry}),
    )
}
