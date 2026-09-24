//! Native WKContentWorld downloads, opaque artifacts and source-scoped export.
use super::*;
use sha2::{Digest, Sha256};
fn require(ok: bool, message: &str) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err(message.into())
    }
}
async fn settled(wire: &mut Wire, mut args: Value) -> Result<(Value, Value), String> {
    args["expectedRevision"] = wire.tool("lomi_workspace_list", json!({})).await?
        ["structuredContent"]["data"]["domainRevision"]
        .clone();
    let value = wire.tool("lomi_browser_download", args.clone()).await?;
    let id = value["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| value.to_string())?;
    Ok((args, wire.settled_with_limit(id, 900).await?))
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
    let origin = fixture["origin"]
        .as_str()
        .ok_or("Missing download origin")?;
    let (_,opened) = layout_call(wire,"lomi_browser_open",json!({"workspaceId":workspace,"url":origin,"retryEpoch":epoch,"requestKey":"download-open"})).await?;
    let target = &opened["structuredContent"]["data"]["result"];
    let browser = app
        .get_webview(&format!(
            "browser-{}",
            target["panelId"].as_str().ok_or("Missing download panel")?
        ))
        .ok_or("Missing download browser")?;
    wait_for(
        &browser,
        "document.title==='Download fixture' && document.readyState==='complete'",
    )
    .await?;
    evaluate(
        &browser,
        "globalThis.fetch=()=>Promise.resolve(new Response('forged-page-fetch'));true",
    )
    .await?;
    let snapshot = wire.tool("lomi_browser_snapshot",json!({"workspaceId":workspace,"panelId":target["panelId"],"browserGeneration":target["browserGeneration"],"maxNodes":20,"maxBytes":4096})).await?;
    require(
        snapshot["structuredContent"]["status"] == "ok",
        &format!("Download snapshot failed: {snapshot}"),
    )?;
    let base = json!({"workspaceId":workspace,"panelId":target["panelId"],"browserGeneration":target["browserGeneration"],"navigationId":snapshot["structuredContent"]["data"]["navigationId"],"maxBytes":4*1024*1024,"retryEpoch":epoch});
    let mut proof = Vec::new();
    let mut counted = None;
    for (name, bytes) in [
        ("binary", vec![0, 255, 65, 13, 10]),
        ("empty", vec![]),
        ("large", vec![165; 4 * 1024 * 1024]),
        ("counted", b"count:1".to_vec()),
    ] {
        let mut args = base.clone();
        args["url"] = json!(format!("{origin}/download/{name}"));
        args["requestKey"] = json!(format!("download-{name}"));
        let (args, downloaded) = settled(wire, args).await?;
        let result = &downloaded["structuredContent"]["data"]["result"];
        require(
            downloaded["structuredContent"]["data"]["state"] == "succeeded"
                && result["byteLength"] == bytes.len()
                && result["sha256"] == format!("{:x}", Sha256::digest(&bytes))
                && result["mediaType"] == "application/octet-stream",
            &format!("Native {name} download mismatch: {downloaded}"),
        )?;
        let read = wire
            .tool(
                "lomi_artifact_read",
                json!({"workspaceId":workspace,"artifactId":result["id"]}),
            )
            .await?;
        require(
            read["structuredContent"]["status"] == "ok"
                && read["structuredContent"]["data"]["image"].is_null(),
            "Opaque download was not metadata-only",
        )?;
        let parent = wire
            .tool(
                "lomi_files_list",
                json!({"workspaceId":workspace,"relativeDirectory":"","limit":100}),
            )
            .await?;
        let (_,exported) = layout_call(wire,"lomi_artifact_export",json!({"workspaceId":workspace,"artifactId":result["id"],"expectedSha256":result["sha256"],"relativePath":format!("download-{name}.bin"),"expectedParentRevision":parent["structuredContent"]["data"]["directoryRevision"],"retryEpoch":epoch,"requestKey":format!("export-download-{name}")})).await?;
        require(
            std::fs::read(directory.join(format!("project/download-{name}.bin")))
                .map_err(|e| e.to_string())?
                == bytes,
            "Downloaded export changed bytes",
        )?;
        let replay = wire.tool("lomi_browser_download", args.clone()).await?;
        require(
            replay["structuredContent"]["data"]["operationId"]
                == downloaded["structuredContent"]["data"]["operationId"],
            "Download retry repeated GET",
        )?;
        if name == "counted" {
            counted = Some((args, downloaded.clone(), exported.clone()));
        }
        proof.push(json!({"name":name,"downloaded":downloaded,"exported":exported}));
    }
    for (name, code) in [
        ("redirect", "UNSUPPORTED_CAPABILITY"),
        ("overflow", "ARTIFACT_TOO_LARGE"),
        ("stall", "DEADLINE_EXCEEDED"),
    ] {
        let mut args = base.clone();
        args["url"] = json!(format!("{origin}/download/{name}"));
        args["requestKey"] = json!(format!("download-{name}"));
        args["maxBytes"] = json!(8192);
        let (args, result) = settled(wire, args).await?;
        require(
            result["structuredContent"]["data"]["state"] == "outcome_unknown"
                && result["structuredContent"]["data"]["effectState"] == "unknown"
                && result["structuredContent"]["data"]["result"]["code"] == code,
            &format!("Failed transfer concealed GET uncertainty: {result}"),
        )?;
        let replay = wire.tool("lomi_browser_download", args).await?;
        require(
            replay["structuredContent"]["data"]["operationId"]
                == result["structuredContent"]["data"]["operationId"],
            "Failed download retry repeated GET",
        )?;
        proof.push(result);
    }
    let (args, downloaded, exported) = counted.ok_or("Missing counted download")?;
    for (field, value, code) in [
        ("url", json!("http://127.0.0.1:1/foreign"), "SCOPE_DENIED"),
        (
            "workspaceId",
            json!("foreign-workspace"),
            "TARGET_NOT_FOUND",
        ),
        ("navigationId", json!("stale"), "STALE_SNAPSHOT"),
    ] {
        let mut denied = args.clone();
        denied[field] = value;
        denied["requestKey"] = json!(format!("denied-{field}"));
        let result = wire.tool("lomi_browser_download", denied).await?;
        require(
            result["structuredContent"]["code"] == code,
            &format!("Download scope denial failed: {result}"),
        )?;
    }
    let mut cancel = base.clone();
    cancel["url"] = json!(format!("{origin}/download/cancel"));
    cancel["requestKey"] = json!("cancel-download");
    cancel["expectedRevision"] = wire.tool("lomi_workspace_list", json!({})).await?
        ["structuredContent"]["data"]["domainRevision"]
        .clone();
    let accepted = wire.tool("lomi_browser_download", cancel.clone()).await?;
    let id = accepted["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or("Missing cancel operation")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    wire.tool("lomi_operation_cancel", json!({"operationId":id}))
        .await?;
    let cancelled = wire.settled_with_limit(id, 900).await?;
    require(
        ["cancelled", "outcome_unknown"].contains(
            &cancelled["structuredContent"]["data"]["state"]
                .as_str()
                .unwrap_or(""),
        ),
        &format!("Cancelled download published success: {cancelled}"),
    )?;
    require(
        wire.tool("lomi_browser_download", cancel).await?["structuredContent"]["data"]
            ["operationId"]
            == id,
        "Cancelled download replayed",
    )?;
    proof.push(cancelled);
    let (_,closed)=layout_call(wire,"lomi_panel_close",json!({"workspaceId":workspace,"panelId":target["panelId"],"browserGeneration":target["browserGeneration"],"retryEpoch":epoch,"requestKey":"download-close"})).await?;
    let read=wire.tool("lomi_artifact_read",json!({"workspaceId":workspace,"artifactId":downloaded["structuredContent"]["data"]["result"]["id"]})).await?;
    require(
        read["structuredContent"]["status"] == "error",
        "Closed source disclosed its download",
    )?;
    for prior in [downloaded, exported] {
        let read = wire
            .tool(
                "lomi_operation_get",
                json!({"operationId":prior["structuredContent"]["data"]["operationId"]}),
            )
            .await?;
        require(
            read["structuredContent"]["status"] == "error",
            "Closed source disclosed a stored download/export receipt",
        )?;
    }
    screenshot(main, directory.join("browser-download-cleanup.png")).await?;
    std::fs::write(directory.join("browser-downloads.json"),serde_json::to_vec_pretty(&json!({"checks":["same-origin profile credentials","page-world fetch replacement cannot change native bytes","binary and empty downloads","full 4 MiB download and export","opaque metadata-only artifact","exact retry sends one GET","redirect refused before target request","streamed overflow bounded","deadline abort reports uncertainty","foreign origin/workspace and stale document denied","cancellation has no successful artifact or replay","closed source blocks artifacts and stored receipts"],"proof":proof,"closed":closed})).unwrap()).map_err(|e|e.to_string())?;
    Ok(())
}
