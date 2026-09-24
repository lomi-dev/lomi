//! Real helper and native immutable import/non-overwriting export qualification.
use super::*;
use sha2::{Digest, Sha256};
fn require(ok: bool, message: &str) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err(message.into())
    }
}
async fn parent(wire: &mut Wire, workspace: &Value) -> Result<Value, String> {
    let value = wire
        .tool(
            "lomi_files_list",
            json!({"workspaceId":workspace,"relativeDirectory":"","limit":100}),
        )
        .await?;
    require(
        value["structuredContent"]["status"] == "ok",
        &format!("Cannot list parent: {value}"),
    )?;
    Ok(value["structuredContent"]["data"]["directoryRevision"].clone())
}
async fn denied(wire: &mut Wire, tool: &str, mut args: Value, code: &str) -> Result<Value, String> {
    let current = wire.tool("lomi_workspace_list", json!({})).await?;
    args["expectedRevision"] = current["structuredContent"]["data"]["domainRevision"].clone();
    let value = wire.tool(tool, args).await?;
    let final_value = if let Some(id) = value["structuredContent"]["data"]["operationId"].as_str() {
        wire.settled_with_limit(id, 900).await?
    } else {
        value
    };
    require(
        final_value["structuredContent"]["code"] == code
            || final_value["structuredContent"]["data"]["result"]["code"] == code,
        &format!("Expected {code} from {tool}: {final_value}"),
    )?;
    Ok(final_value)
}
pub(super) async fn qualify(
    app: &tauri::AppHandle,
    wire: &mut Wire,
    main: &Webview,
    workspace: &Value,
    epoch: &Value,
    directory: &Path,
) -> Result<(), String> {
    activate_main(app).await?;
    let project = directory.join("project");
    let contents = b"Opaque\0 Za\xc5\xbc\xc3\xb3\xc5\x82\xc4\x87 \xf0\x9f\x99\x82\r\n";
    let mut evidence = Vec::new();
    for (index, bytes) in [&contents[..], &b""[..], &b"not an APK"[..]]
        .into_iter()
        .enumerate()
    {
        let name = if index == 2 {
            "opaque.apk"
        } else {
            "source.bin"
        };
        std::fs::write(project.join(name), bytes).map_err(|e| e.to_string())?;
        let hash = format!("{:x}", Sha256::digest(bytes));
        let (args, imported) = layout_call(wire, "lomi_artifact_import", json!({"workspaceId":workspace,"relativePath":name,"kind":"file","expectedByteLength":bytes.len(),"expectedSha256":hash,"retryEpoch":epoch,"requestKey":format!("file-import-{index}")})).await?;
        let imported_result = &imported["structuredContent"]["data"]["result"];
        require(
            imported_result["sha256"] == hash && imported_result["byteLength"] == bytes.len(),
            "Imported metadata differs from exact bytes",
        )?;
        let read = wire
            .tool(
                "lomi_artifact_read",
                json!({"workspaceId":workspace,"artifactId":imported_result["artifactId"]}),
            )
            .await?;
        require(
            read["structuredContent"]["status"] == "ok"
                && read["structuredContent"]["data"]["artifact"]["mediaType"]
                    == "application/octet-stream",
            &format!("Opaque classification missing: {read}"),
        )?;
        std::fs::write(project.join(name), b"changed after immutable staging")
            .map_err(|e| e.to_string())?;
        let replay = wire.tool("lomi_artifact_import", args).await?;
        require(
            replay["structuredContent"]["data"]["operationId"]
                == imported["structuredContent"]["data"]["operationId"],
            "Import retry copied again",
        )?;
        let export = json!({"workspaceId":workspace,"artifactId":imported_result["artifactId"],"expectedSha256":hash,"relativePath":format!("export-{index}.bin"),"expectedParentRevision":parent(wire,workspace).await?,"retryEpoch":epoch,"requestKey":format!("file-export-{index}")});
        let (export_args, exported) = layout_call(wire, "lomi_artifact_export", export).await?;
        require(
            std::fs::read(project.join(format!("export-{index}.bin")))
                .map_err(|e| e.to_string())?
                == bytes,
            "Export did not preserve staged bytes",
        )?;
        let replay = wire
            .tool("lomi_artifact_export", export_args.clone())
            .await?;
        require(
            replay["structuredContent"]["data"]["operationId"]
                == exported["structuredContent"]["data"]["operationId"],
            "Export retry wrote again",
        )?;
        if index == 0 {
            let mut changed = export_args.clone();
            changed["relativePath"] = json!("changed.bin");
            denied(
                wire,
                "lomi_artifact_export",
                changed,
                "IDEMPOTENCY_CONFLICT",
            )
            .await?;
            for case in ["collision", "symlink", "stale", "foreign", "secret", "hash"] {
                let mut request = export_args.clone();
                request["requestKey"] = json!(format!("export-denied-{case}"));
                request["relativePath"] = json!(format!("{case}.bin"));
                if case == "collision" {
                    std::fs::write(project.join("collision.bin"), b"preserve")
                        .map_err(|e| e.to_string())?;
                }
                if case == "symlink" {
                    std::os::unix::fs::symlink(
                        project.join("export-0.bin"),
                        project.join("symlink.bin"),
                    )
                    .map_err(|e| e.to_string())?;
                }
                request["expectedParentRevision"] = parent(wire, workspace).await?;
                if case == "stale" {
                    std::fs::write(project.join("human-created.bin"), b"human")
                        .map_err(|e| e.to_string())?;
                }
                if case == "foreign" {
                    request["workspaceId"] = json!("foreign-workspace");
                }
                if case == "secret" {
                    request["relativePath"] = json!(".env");
                }
                if case == "hash" {
                    request["expectedSha256"] = json!("0".repeat(64));
                }
                let code = match case {
                    "foreign" => "TARGET_NOT_FOUND",
                    "secret" => "SCOPE_DENIED",
                    _ => "REVISION_CONFLICT",
                };
                evidence.push(denied(wire, "lomi_artifact_export", request, code).await?);
            }
            require(
                std::fs::read(project.join("collision.bin")).map_err(|e| e.to_string())?
                    == b"preserve",
                "Collision replaced user bytes",
            )?;
            require(
                project
                    .join("symlink.bin")
                    .symlink_metadata()
                    .map_err(|e| e.to_string())?
                    .file_type()
                    .is_symlink(),
                "Export replaced a symlink",
            )?;
            require(
                !project.join("stale.bin").exists() && !project.join(".env").exists(),
                "Denied export created a destination",
            )?;
        }
        evidence.push(json!({"imported":imported,"read":read,"exported":exported}));
    }
    for (name, length, code) in [
        (".env.fixture", 27, "SCOPE_DENIED"),
        ("oversize.bin", 4 * 1024 * 1024 + 1, "RESOURCE_EXHAUSTED"),
    ] {
        evidence.push(denied(wire,"lomi_artifact_import",json!({"workspaceId":workspace,"relativePath":name,"kind":"file","expectedByteLength":length,"expectedSha256":"0".repeat(64),"retryEpoch":epoch,"requestKey":format!("denied-{length}")}),code).await?);
    }
    screenshot(main, directory.join("artifact-export-cleanup.png")).await?;
    std::fs::write(directory.join("artifact-files.json"),serde_json::to_vec_pretty(&json!({"checks":["binary Unicode and exact CRLF bytes","empty file","opaque APK extension cannot relabel kind","source rewrite cannot change private copy","import and export retry never repeat","changed retry conflicts","existing destination preserved","symlink preserved","stale parent denied","foreign workspace denied","secret paths denied","oversize import denied"],"evidence":evidence})).unwrap()).map_err(|e|e.to_string())?;
    Ok(())
}
