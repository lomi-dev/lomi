//! Management qualification using only the explicitly licensed private SDK.
use super::*;

struct PreferencesFault {
    root: PathBuf,
    original: Option<Vec<u8>>,
    backup: Option<Vec<u8>>,
    restored: bool,
}
impl PreferencesFault {
    fn prepare(root: &Path, evidence: &Path) -> Result<Self, String> {
        let read = |name: &str| match std::fs::read(root.join(name)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        };
        let original = read("preferences.json")?;
        let backup = read("preferences.previous.json")?;
        std::fs::write(
            evidence.join("android-preferences-baseline.json"),
            serde_json::to_vec(&json!({"original":original,"backup":backup})).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            root: root.into(),
            original,
            backup,
            restored: false,
        })
    }
    fn restore(&mut self) -> Result<(), String> {
        for (name, bytes) in [
            ("preferences.json", &self.original),
            ("preferences.previous.json", &self.backup),
        ] {
            match bytes {
                Some(bytes) => {
                    std::fs::write(self.root.join(name), bytes).map_err(|e| e.to_string())?
                }
                None => match std::fs::remove_file(self.root.join(name)) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.to_string()),
                },
            }
        }
        self.restored = true;
        Ok(())
    }
}
impl Drop for PreferencesFault {
    fn drop(&mut self) {
        if !self.restored {
            let _ = self.restore();
        }
    }
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}
async fn inventory(wire: &mut Wire, workspace: &Value) -> Result<Value, String> {
    let response = wire
        .tool(
            "lomi_android_setup_plan",
            json!({"workspaceId":workspace,"action":{"type":"inventory"}}),
        )
        .await?;
    require(
        response["structuredContent"]["status"] == "ok",
        &format!("Inventory failed: {response}"),
    )?;
    Ok(response["structuredContent"]["data"].clone())
}
async fn decide(
    settings: &Webview,
    approve: bool,
    confirmation: Option<&str>,
) -> Result<(), String> {
    let label = if approve {
        "Approve Android operation"
    } else {
        "Deny Android operation"
    };
    wait_for(settings,"Boolean(document.querySelector('section[aria-label=\"Android setup and device requests\"] article'))").await?;
    if approve {
        evaluate(settings,"(()=>{const root=document.querySelector('section[aria-label=\"Android setup and device requests\"]');root.querySelectorAll('input[type=checkbox]').forEach(e=>{if(!e.checked)e.click();});return true;})()").await?;
        if let Some(name) = confirmation {
            require(evaluate(settings,"[...document.querySelectorAll('button')].find(b=>b.textContent==='Approve Android operation').disabled").await?==true,"Destructive approval did not require typed confirmation")?;
            evaluate(settings,&format!("(()=>{{const e=document.querySelector('section[aria-label=\"Android setup and device requests\"] input:not([type=checkbox])');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,{});e.dispatchEvent(new Event('input',{{bubbles:true}}));return true;}})()",json!(name))).await?;
        }
    }
    click(settings, label).await?;
    wait_for(settings,"!document.querySelector('section[aria-label=\"Android setup and device requests\"] article')").await?;
    Ok(())
}
async fn mutate(
    wire: &mut Wire,
    settings: &Webview,
    tool: &str,
    args: Value,
    approve: bool,
    confirmation: Option<&str>,
) -> Result<Value, String> {
    let receipt = wire.tool(tool, args).await?;
    require(
        receipt["structuredContent"]["data"]["state"] == "awaiting_user",
        &format!("Expected Android approval: {receipt}"),
    )?;
    let op = receipt["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or("Missing management operation")?;
    decide(settings, approve, confirmation).await?;
    let result = wire.settled_with_limit(op, 7200).await?;
    require(
        result["structuredContent"]["data"]["state"]
            == if approve { "succeeded" } else { "cancelled" },
        &format!("Android operation failed: {result}"),
    )?;
    Ok(result)
}
pub(super) async fn qualify(
    app: &tauri::AppHandle,
    wire: &mut Wire,
    main: &Webview,
    settings: &Webview,
    workspace: &Value,
    epoch: &Value,
    directory: &Path,
) -> Result<(), String> {
    let root = crate::android::fixture::directory()?.ok_or("Missing isolated SDK")?;
    let original: Value = serde_json::from_slice(
        &std::fs::read(root.join("devices.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let consent: Value = serde_json::from_slice(
        &std::fs::read(root.join("licenses.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut checks = vec![];
    let initial = inventory(wire, workspace).await?;
    require(
        initial["devices"].as_array().is_some_and(|d| d.len() == 1)
            && initial["profiles"]
                .as_array()
                .is_some_and(|p| !p.is_empty()),
        "Inventory omitted the selected device or installed phone profiles",
    )?;
    checks.push("selected inventory and installed profiles");
    let catalog=wire.tool("lomi_android_setup_plan",json!({"workspaceId":workspace,"action":{"type":"catalog","offset":0,"limit":32,"expectedCatalogRevision":null}})).await?;
    let catalog = &catalog["structuredContent"]["data"];
    let package = catalog["packages"]
        .as_array()
        .ok_or_else(|| format!("Catalog failed: {catalog}"))?
        .iter()
        .find(|p| p["id"] == "platform-tools")
        .ok_or("Catalog has no platform-tools in its first page")?
        .clone();
    checks.push("bounded real provider catalog");
    let stale=wire.tool("lomi_android_setup_plan",json!({"workspaceId":workspace,"action":{"type":"catalog","offset":1,"limit":1,"expectedCatalogRevision":"stale"}})).await?;
    require(
        stale["structuredContent"]["code"] == "REVISION_CONFLICT",
        "Changed catalog was accepted",
    )?;
    checks.push("catalog revision conflict");
    let prepare_args = json!({"workspaceId":workspace,"action":{"type":"prepare","catalogRevision":catalog["catalogRevision"],"packages":[{"id":package["id"],"revision":package["revision"]}],"prepareTools":false}});
    let plan = wire
        .tool("lomi_android_setup_plan", prepare_args.clone())
        .await?;
    let plan = plan["structuredContent"]["data"].clone();
    require(
        plan["planId"].is_string() && plan["downloads"][0]["id"] == "platform-tools",
        "Plan did not bind the selected package",
    )?;
    for license in plan["licenses"].as_array().ok_or("Missing plan terms")? {
        require(
            consent["accepted"][license["id"].as_str().ok_or("Invalid license ID")?]
                == license["digest"],
            "Fixture refuses provider terms not already accepted for this isolated SDK",
        )?;
    }
    checks.push("exact plan and previously accepted license digests");
    let mut apply = json!({"workspaceId":workspace,"planId":plan["planId"],"planRevision":"stale","retryEpoch":epoch,"requestKey":"setup-apply"});
    let bad = wire.tool("lomi_android_setup_apply", apply.clone()).await?;
    require(
        bad["structuredContent"]["code"] == "REVISION_CONFLICT",
        "Wrong plan revision was accepted",
    )?;
    checks.push("stale plan refused");
    apply["planRevision"] = plan["revision"].clone();
    let receipt = wire.tool("lomi_android_setup_apply", apply.clone()).await?;
    require(
        receipt["structuredContent"]["data"]["state"] == "awaiting_user",
        "Installation did not await human terms",
    )?;
    let op = receipt["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or("Missing apply operation")?;
    let refused=javascript(main,&format!("try{{await window.__TAURI_INTERNALS__.invoke('agent_control_decide_android_management',{{operationId:{},revision:{},approve:true,accepted:[],confirmation:null}});return false;}}catch{{return true;}}",json!(op),plan["revision"])).await?;
    require(refused == true, "Main can approve Android provider terms")?;
    checks.push("main caller cannot approve");
    wait_for(settings,"Boolean(document.querySelector('section[aria-label=\"Android setup and device requests\"] article'))").await?;
    require(evaluate(settings,"[...document.querySelectorAll('button')].find(b=>b.textContent==='Approve Android operation').disabled").await?==true,"Provider approval was enabled without accepting terms")?;
    evaluate(settings,"document.querySelector('section[aria-label=\"Android setup and device requests\"]').scrollIntoView({block:'start'});true").await?;
    screenshot(settings, directory.join("android-setup-terms.png")).await?;
    decide(settings, true, None).await?;
    let installed = wire.settled_with_limit(op, 7200).await?;
    require(
        installed["structuredContent"]["data"]["state"] == "succeeded",
        &format!("Native SDK install failed: {installed}"),
    )?;
    checks.push("real private SDK installation after Settings consent");
    let replay = wire.tool("lomi_android_setup_apply", apply).await?;
    require(
        replay["structuredContent"]["data"] == installed["structuredContent"]["data"],
        "SDK retry redispatched",
    )?;
    checks.push("durable installation replay");
    let current = inventory(wire, workspace).await?;
    let source = &initial["devices"][0];
    let name = format!(
        "MCP setup {}",
        directory.file_name().unwrap().to_string_lossy()
    );
    let mut create = json!({"workspaceId":workspace,"retryEpoch":epoch,"requestKey":"setup-create-denied","action":{"type":"create","expectedDevicesRevision":current["devicesRevision"],"name":name,"image":source["image"],"profile":source["profile"],"hardware":{"ramMib":2560,"cpuCount":2,"dataGib":4,"gpu":"host","quickBoot":false}}});
    mutate(
        wire,
        settings,
        "lomi_android_device_manage",
        create.clone(),
        false,
        None,
    )
    .await?;
    require(
        inventory(wire, workspace).await?["devicesRevision"] == current["devicesRevision"],
        "Denied create changed metadata",
    )?;
    checks.push("declined create has no effects");
    create["requestKey"] = json!("setup-create");
    let created = mutate(
        wire,
        settings,
        "lomi_android_device_manage",
        create.clone(),
        true,
        None,
    )
    .await?;
    let device = created["structuredContent"]["data"]["result"]["deviceId"]
        .as_str()
        .ok_or_else(|| format!("Created device ID missing: {created}"))?
        .to_string();
    std::fs::write(
        directory.join("android-setup-created.json"),
        serde_json::to_vec(&json!({"deviceId":device,"name":name})).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let avd = root.join("avd").join(format!("sb_{device}.avd"));
    require(avd.is_dir(), "Native AVD was not created")?;
    checks.push("real AVD creation and creator grant");
    require(
        wire.tool("lomi_android_device_manage", create).await?["structuredContent"]["data"]
            == created["structuredContent"]["data"],
        "Create retry allocated another AVD",
    )?;
    checks.push("durable device create replay");
    let current = inventory(wire, workspace).await?;
    require(
        current["devices"].as_array().is_some_and(|d| d.len() == 2),
        "Creator cannot observe its new AVD",
    )?;
    let modify = json!({"workspaceId":workspace,"retryEpoch":epoch,"requestKey":"setup-modify","action":{"type":"modify","expectedDevicesRevision":current["devicesRevision"],"deviceId":device,"generation":null,"name":name,"profile":source["profile"],"hardware":{"ramMib":3072,"cpuCount":2,"dataGib":4,"gpu":"host","quickBoot":false}}});
    mutate(
        wire,
        settings,
        "lomi_android_device_manage",
        modify,
        true,
        None,
    )
    .await?;
    let current = inventory(wire, workspace).await?;
    require(
        current["devices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["deviceId"] == device && d["hardware"]["ramMib"] == 3072),
        "Modified desired hardware was not saved",
    )?;
    checks.push("native desired hardware modification");
    std::fs::write(avd.join("mcp-wipe-sentinel"), b"only the new fixture AVD")
        .map_err(|e| e.to_string())?;
    let mut wipe = json!({"workspaceId":workspace,"retryEpoch":epoch,"requestKey":"setup-wipe","action":{"type":"wipe","expectedDevicesRevision":current["devicesRevision"],"deviceId":device,"generation":null,"confirmation":"wrong"}});
    require(
        wire.tool("lomi_android_device_manage", wipe.clone())
            .await?["structuredContent"]["code"]
            == "SCOPE_DENIED",
        "Wrong wipe name was accepted",
    )?;
    checks.push("exact destructive target name");
    wipe["action"]["confirmation"] = json!(name);
    mutate(
        wire,
        settings,
        "lomi_android_device_manage",
        wipe,
        true,
        Some(&name),
    )
    .await?;
    require(
        !avd.join("mcp-wipe-sentinel").exists() && avd.is_dir(),
        "Native wipe did not replace only the fixture AVD",
    )?;
    checks.push("native wipe with typed Settings confirmation");
    let current = inventory(wire, workspace).await?;
    let delete = json!({"workspaceId":workspace,"retryEpoch":epoch,"requestKey":"setup-delete","action":{"type":"delete","expectedDevicesRevision":current["devicesRevision"],"deviceId":device,"generation":null,"confirmation":name}});
    mutate(
        wire,
        settings,
        "lomi_android_device_manage",
        delete,
        true,
        Some(&name),
    )
    .await?;
    require(!avd.exists(), "Native device deletion retained the AVD")?;
    checks.push("native delete and directory cleanup");
    let current = inventory(wire, workspace).await?;
    let used=wire.tool("lomi_android_device_manage",json!({"workspaceId":workspace,"retryEpoch":epoch,"requestKey":"setup-remove-used-image","action":{"type":"remove_package","expectedManifestRevision":current["manifestRevision"],"packageId":source["image"]}})).await?;
    require(
        used["structuredContent"]["code"] == "TARGET_BUSY",
        "An image referenced by another AVD could be removed",
    )?;
    checks.push("referenced image removal refused");
    let rollback = json!({"workspaceId":workspace,"retryEpoch":epoch,"requestKey":"setup-rollback","action":{"type":"rollback_package","expectedManifestRevision":current["manifestRevision"],"packageId":"platform-tools"}});
    mutate(
        wire,
        settings,
        "lomi_android_device_manage",
        rollback,
        true,
        None,
    )
    .await?;
    checks.push("native previous tool revision rollback");
    let current = inventory(wire, workspace).await?;
    mutate(wire,settings,"lomi_android_device_manage",json!({"workspaceId":workspace,"retryEpoch":epoch,"requestKey":"setup-remove-tools","action":{"type":"remove_package","expectedManifestRevision":current["manifestRevision"],"packageId":"platform-tools"}}),true,None).await?;
    require(
        !inventory(wire, workspace).await?["installed"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "platform-tools"),
        "Removed SDK package is still installed",
    )?;
    checks.push("native removal of unused SDK tools");
    let restore_plan = wire.tool("lomi_android_setup_plan", prepare_args).await?;
    let restore_plan = &restore_plan["structuredContent"]["data"];
    mutate(wire,settings,"lomi_android_setup_apply",json!({"workspaceId":workspace,"retryEpoch":epoch,"requestKey":"setup-restore-tools","planId":restore_plan["planId"],"planRevision":restore_plan["revision"]}),true,None).await?;
    require(
        inventory(wire, workspace).await?["installed"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "platform-tools" && p["revision"] == package["revision"]),
        "SDK tools were not restored",
    )?;
    checks.push("real SDK reinstall after explicit removal");
    let mut fault = PreferencesFault::prepare(&root, directory)?;
    std::fs::write(
        root.join("preferences.previous.json"),
        serde_json::to_vec_pretty(
            &json!({"version":1,"revision":7,"defaultDeviceId":source["deviceId"]}),
        )
        .unwrap(),
    )
    .map_err(|e| e.to_string())?;
    for (reset, key) in [
        (false, "setup-restore-metadata"),
        (true, "setup-reset-preferences"),
    ] {
        let corrupt = format!("owned metadata fault: {} {reset}", directory.display());
        std::fs::write(root.join("preferences.json"), corrupt.as_bytes())
            .map_err(|e| e.to_string())?;
        let current = inventory(wire, workspace).await?;
        let recovery = current["recovery"]
            .as_array()
            .ok_or("No recovery choices")?
            .iter()
            .find(|r| r["file"] == "preferences")
            .ok_or("Corrupt preferences were not detected")?;
        mutate(wire,settings,"lomi_android_device_manage",json!({"workspaceId":workspace,"retryEpoch":epoch,"requestKey":key,"action":{"type":"restore_metadata","file":"preferences","digest":recovery["digest"],"reset":reset}}),true,reset.then_some("RESET PREFERENCES")).await?;
        let recovered: Value = serde_json::from_slice(
            &std::fs::read(root.join("preferences.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        require(
            if reset {
                recovered["defaultDeviceId"].is_null() && recovered["revision"] == 1
            } else {
                recovered["defaultDeviceId"] == source["deviceId"] && recovered["revision"] == 8
            },
            "Metadata recovery did not honor the approved choice",
        )?;
        require(
            std::fs::read(root.join("recovery").join(format!(
                "preferences-{}.json",
                recovery["digest"].as_str().unwrap()
            )))
            .map_err(|e| e.to_string())?
                == corrupt.as_bytes(),
            "Recovery did not preserve the corrupt original",
        )?;
        checks.push(if reset {
            "native preference reset with typed confirmation"
        } else {
            "native metadata backup restoration and preserved corruption"
        });
    }
    fault.restore()?;
    for (kind, key) in [("recover", "setup-recover"), ("cleanup", "setup-cleanup")] {
        mutate(wire,settings,"lomi_android_device_manage",json!({"workspaceId":workspace,"retryEpoch":epoch,"requestKey":key,"action":{"type":kind}}),true,None).await?;
    }
    checks.push("native recovery and owned-cache cleanup");
    let final_devices: Value = serde_json::from_slice(
        &std::fs::read(root.join("devices.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    require(
        final_devices["devices"] == original["devices"],
        "Management changed the original qualification device",
    )?;
    checks.push("original device preserved");
    screenshot(settings, directory.join("android-setup-complete.png")).await?;
    std::fs::write(directory.join("android-setup.json"),serde_json::to_vec_pretty(&json!({"checks":checks,"originalDevicePreserved":true,"fixtureDeviceRemoved":true,"plan":plan,"installed":installed,"created":created})).unwrap()).map_err(|e|e.to_string())?;
    let _ = app;
    Ok(())
}
