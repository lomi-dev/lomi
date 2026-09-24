//! Real helper, Settings approval and isolated WKContentWorld file attachment.
use super::*;
use sha2::{Digest, Sha256};
fn require(ok: bool, message: &str) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err(message.into())
    }
}
async fn snapshot(wire: &mut Wire, workspace: &Value, target: &Value) -> Result<Value, String> {
    let value=wire.tool("lomi_browser_snapshot",json!({"workspaceId":workspace,"panelId":target["panelId"],"browserGeneration":target["browserGeneration"],"maxNodes":100,"maxBytes":16384})).await?;
    require(
        value["structuredContent"]["status"] == "ok",
        &format!("Upload snapshot: {value}"),
    )?;
    Ok(value["structuredContent"]["data"].clone())
}
async fn submit(wire: &mut Wire, mut args: Value) -> Result<(Value, String), String> {
    args["expectedRevision"] = wire.tool("lomi_workspace_list", json!({})).await?
        ["structuredContent"]["data"]["domainRevision"]
        .clone();
    let result = wire.tool("lomi_browser_upload", args.clone()).await?;
    let id = result["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| format!("Upload admission: {result}"))?
        .to_owned();
    Ok((args, id))
}
async fn complete(wire: &mut Wire, id: &str) -> Result<Value, String> {
    for _ in 0..1200 {
        let value = wire
            .tool("lomi_operation_get", json!({"operationId":id}))
            .await?;
        if !matches!(
            value["structuredContent"]["data"]["state"].as_str(),
            Some("queued" | "awaiting_user" | "running" | "cancelling")
        ) {
            return Ok(value);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Err(format!("Upload did not complete after decision: {id}"))
}
async fn approve(settings: &Webview, approve: bool) -> Result<(), String> {
    let label = if approve {
        "Upload this file"
    } else {
        "Deny upload"
    };
    wait_for(settings,&format!("[...document.querySelectorAll('button')].some(b=>b.textContent==={label:?}&&!b.disabled)")).await?;
    evaluate(settings,&format!("[...document.querySelectorAll('button')].find(b=>b.textContent==={label:?}).click();true")).await?;
    Ok(())
}
pub(super) async fn qualify(
    app: &tauri::AppHandle,
    wire: &mut Wire,
    main: &Webview,
    workspace: &Value,
    epoch: &Value,
    directory: &Path,
    fixture: &Value,
) -> Result<(), String> {
    activate_main(app).await?;
    let (_,opened)=layout_call(wire,"lomi_browser_open",json!({"workspaceId":workspace,"url":fixture["origin"],"retryEpoch":epoch,"requestKey":"upload-open"})).await?;
    let target = opened["structuredContent"]["data"]["result"].clone();
    let browser = app
        .get_webview(&format!(
            "browser-{}",
            target["panelId"].as_str().ok_or("Missing upload panel")?
        ))
        .ok_or("Missing upload browser")?;
    let settings = app
        .get_webview("settings")
        .ok_or("Missing upload Settings")?;
    wait_for(&browser,"document.title==='Upload fixture'&&!!document.querySelector('iframe').contentDocument?.querySelector('input')").await?;
    evaluate(&browser,"(()=>{for(const w of [window,document.querySelector('iframe').contentWindow]){w.DataTransfer=function(){throw Error('page replacement')};w.File=function(){throw Error('page replacement')};}return true;})()").await?;
    let mut checks = vec![
        "real owned browser and same-origin child ready",
        "page-world File/DataTransfer replacement cannot select native constructors",
    ];
    let mut uploads = vec![];
    let mut final_args = Value::Null;
    let mut last_artifact = Value::Null;
    for (index, bytes) in [
        vec![0, 255, 65, 13, 10],
        vec![],
        vec![165; 4 * 1024 * 1024],
        b"Zaz\xcc\x87o\xcc\x81lc\xcc\x81 \xf0\x9f\x99\x82".to_vec(),
    ]
    .into_iter()
    .enumerate()
    {
        activate_main(app).await?;
        let path = format!("upload-source-{index}.bin");
        std::fs::write(directory.join("project").join(&path), &bytes).map_err(|e| e.to_string())?;
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let (_,imported)=layout_call(wire,"lomi_artifact_import",json!({"workspaceId":workspace,"relativePath":path,"kind":"file","expectedByteLength":bytes.len(),"expectedSha256":hash,"retryEpoch":epoch,"requestKey":format!("upload-import-{index}")})).await?;
        let artifact = imported["structuredContent"]["data"]["result"].clone();
        require(
            artifact["sha256"] == hash,
            &format!("Import failed: {imported}"),
        )?;
        std::fs::write(
            directory.join("project").join(&path),
            b"source changed after staging",
        )
        .map_err(|e| e.to_string())?;
        let view = snapshot(wire, workspace, &target).await?;
        let frame = if index % 2 == 0 { "main" } else { "f1" };
        let reference = view["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["role"] == "file_input" && e["frameId"] == frame)
            .ok_or("Missing file input ref")?;
        let args = json!({"workspaceId":workspace,"panelId":target["panelId"],"browserGeneration":target["browserGeneration"],"leaseId":target["leaseId"],"navigationId":view["navigationId"],"snapshotId":view["snapshotId"],"elementRef":reference["elementRef"],"frameId":frame,"artifactId":artifact["artifactId"],"expectedSha256":hash,"fileName":format!("Zażółć-{index}.bin"),"retryEpoch":epoch,"requestKey":format!("upload-{index}")});
        let (args, id) = submit(wire, args).await?;
        wait_for(
            &settings,
            "document.querySelector('#control-uploads-heading')!==null",
        )
        .await?;
        let pending = wire
            .tool("lomi_operation_get", json!({"operationId":id}))
            .await?;
        require(
            pending["structuredContent"]["data"]["state"] == "awaiting_user",
            &format!("Upload bypassed consent: {pending}"),
        )?;
        if index == 0 {
            evaluate(&settings,"document.querySelector('#control-uploads-heading').scrollIntoView({block:'start'});true").await?;
            screenshot(&settings, directory.join("browser-upload-approval.png")).await?;
        }
        approve(&settings, true).await?;
        let result = complete(wire, &id).await?;
        require(
            result["structuredContent"]["data"]["state"] == "succeeded"
                && result["structuredContent"]["data"]["result"]["artifact"]["sha256"] == hash,
            &format!("Upload failed: {result}"),
        )?;
        let source = if frame == "main" {
            "window"
        } else {
            "document.querySelector('iframe').contentWindow"
        };
        wait_for(
            &browser,
            &format!(
                "{source}.uploads.some(u=>u.sha256==={hash:?}&&u.byteLength==={})",
                bytes.len()
            ),
        )
        .await?;
        let events = evaluate(&browser, &format!("{source}.events")).await?;
        require(
            events
                .as_array()
                .is_some_and(|events| events.iter().all(|e| e[1] == false)),
            "Upload forged trusted input",
        )?;
        let retry = wire.tool("lomi_browser_upload", args.clone()).await?;
        require(
            retry["structuredContent"]["data"]["operationId"] == id,
            "Upload retry changed operation",
        )?;
        uploads.push(
            json!({"sha256":hash,"byteLength":bytes.len(),"frameId":frame,"operation":result}),
        );
        final_args = args;
        last_artifact = artifact;
    }
    checks.extend([
        "binary, empty, full4MiB and Unicode files reach page/server with exact hashes",
        "source edits never change staged upload bytes",
        "native Settings approval precedes each transfer",
        "main and child input/change events remain synthetic",
        "exact retry attaches once",
    ]);
    for case in ["deny", "cancel", "stale"] {
        activate_main(app).await?;
        let view = snapshot(wire, workspace, &target).await?;
        let reference = view["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["role"] == "file_input" && e["frameId"] == "main")
            .unwrap();
        let mut args = final_args.clone();
        args["snapshotId"] = view["snapshotId"].clone();
        args["elementRef"] = reference["elementRef"].clone();
        args["frameId"] = json!("main");
        args["requestKey"] = json!(format!("upload-{case}"));
        let (args, id) = submit(wire, args).await?;
        wait_for(
            &settings,
            "document.querySelector('#control-uploads-heading')!==null",
        )
        .await?;
        if case == "deny" {
            approve(&settings, false).await?;
        } else if case == "cancel" {
            wire.tool("lomi_operation_cancel", json!({"operationId":id}))
                .await?;
        } else {
            evaluate(&browser,"(()=>{const e=document.querySelector('input');e.replaceWith(e.cloneNode());return true;})()").await?;
            approve(&settings, true).await?;
        }
        let result = complete(wire, &id).await?;
        require(
            result["structuredContent"]["data"]["state"]
                == if case == "stale" {
                    "failed"
                } else {
                    "cancelled"
                },
            &format!("{case} receipt: {result}"),
        )?;
        let retry = wire.tool("lomi_browser_upload", args).await?;
        require(
            retry["structuredContent"]["data"]["operationId"] == id,
            "Denied upload replay changed operation",
        )?;
        wait_for(
            &settings,
            "document.querySelector('#control-uploads-heading')===null",
        )
        .await?;
    }
    checks.push("deny, MCP cancel and replaced input do not transmit bytes");
    activate_main(app).await?;
    let view = snapshot(wire, workspace, &target).await?;
    require(
        !view.to_string().contains("Zażółć-"),
        "Snapshot exposed selected file names",
    )?;
    checks.push("selected file names and bytes remain absent from snapshots");
    for (field, value) in [
        ("workspaceId", json!("foreign-workspace")),
        ("expectedSha256", json!("0".repeat(64))),
        ("fileName", json!("../escape")),
    ] {
        let mut args = final_args.clone();
        args[field] = value;
        args["snapshotId"] = view["snapshotId"].clone();
        args["elementRef"] = view["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["role"] == "file_input" && e["frameId"] == "f1")
            .unwrap()["elementRef"]
            .clone();
        args["requestKey"] = json!(format!("upload-invalid-{field}"));
        args["expectedRevision"] = wire.tool("lomi_workspace_list", json!({})).await?
            ["structuredContent"]["data"]["domainRevision"]
            .clone();
        let denied = wire.tool("lomi_browser_upload", args).await?;
        require(
            denied["structuredContent"]["status"] == "error",
            &format!("Invalid upload accepted: {denied}"),
        )?;
    }
    checks.push("foreign workspace, wrong artifact hash and path filename denied");
    let (_,closed)=layout_call(wire,"lomi_panel_close",json!({"workspaceId":workspace,"panelId":target["panelId"],"browserGeneration":target["browserGeneration"],"retryEpoch":epoch,"requestKey":"upload-close"})).await?;
    require(
        closed["structuredContent"]["data"]["state"] == "succeeded",
        &format!("Upload browser close: {closed}"),
    )?;
    let denied = wire.tool("lomi_browser_upload", final_args).await?;
    require(
        denied["structuredContent"]["status"] == "error",
        "Closed browser upload result disclosed",
    )?;
    checks.push("closed destination blocks upload receipt disclosure");
    require(
        last_artifact["artifactId"].is_string(),
        "No immutable artifact retained",
    )?;
    checks.push("owned browser closes normally after bounded transfers");
    screenshot(main, directory.join("browser-upload-cleanup.png")).await?;
    std::fs::write(
        directory.join("browser-uploads.json"),
        serde_json::to_vec_pretty(&json!({"checks":checks,"uploads":uploads,"closed":closed}))
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
