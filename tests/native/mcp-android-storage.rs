//! Real guest-space exhaustion with the approved immutable APK installer.
use super::*;

pub(super) async fn qualify(
    wire: &mut Wire,
    settings: &Webview,
    arguments: &Value,
    before_package: &str,
    directory: &Path,
) -> Result<(), String> {
    let root = crate::android::fixture::directory()?.ok_or("Missing licensed fixture")?;
    let guest = crate::android::fixture::guest(
        &root,
        arguments["deviceId"].as_str().ok_or("Missing device")?,
    )?;
    let owned_guest = guest.clone();
    let (mut storage, before_free, filled_free, fill_blocks) =
        tauri::async_runtime::spawn_blocking(move || {
            let storage = owned_guest.native_apk_storage_fixture()?;
            let before = storage.free_bytes()?;
            require(
                (256 * 1024 * 1024..=6 * 1024 * 1024 * 1024).contains(&before),
                "Unexpected isolated guest capacity",
            )?;
            let blocks = before / (1024 * 1024) - 32;
            storage.fill(blocks)?;
            let after = storage.free_bytes()?;
            require(
                after < 40 * 1024 * 1024,
                "Guest fill did not reach the bounded low-space condition",
            )?;
            Ok::<_, String>((storage, before, after, blocks))
        })
        .await
        .map_err(|e| e.to_string())??;
    std::fs::write(directory.join("android-storage-filled.json"), serde_json::to_vec_pretty(&json!({"beforeFreeBytes":before_free,"filledFreeBytes":filled_free,"fillMiB":fill_blocks,"deviceId":arguments["deviceId"],"generation":arguments["generation"]})).unwrap()).map_err(|e|e.to_string())?;
    let result = async {
        let accepted = wire
            .tool("lomi_android_install_apk", arguments.clone())
            .await?;
        require(
            data(&accepted)["state"] == "awaiting_user",
            "Storage test skipped exact install approval",
        )?;
        let operation = data(&accepted)["operationId"]
            .as_str()
            .ok_or("Missing storage receipt")?;
        wait_for(
            settings,
            &format!(
                "document.body.textContent.includes({})&&document.body.textContent.includes({})",
                arguments["sha256"], arguments["generation"]
            ),
        )
        .await?;
        click(settings, "Install this APK").await?;
        wait_for(
            settings,
            "![...document.querySelectorAll('button')].some(e=>e.textContent==='Install this APK')",
        )
        .await?;
        let rejected = wire.settled_with_limit(operation, 7200).await?;
        std::fs::write(
            directory.join("android-storage-install.json"),
            serde_json::to_vec_pretty(&rejected).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        require(
            data(&rejected)["state"] == "failed"
                && data(&rejected)["effectState"] == "none"
                && data(&rejected)["result"]["installed"] == false
                && data(&rejected)["result"]["installerFailure"]
                    == "INSTALL_FAILED_INSUFFICIENT_STORAGE",
            format!("Guest did not report the expected definitive storage refusal: {rejected}"),
        )?;
        let retry = wire
            .tool("lomi_android_install_apk", arguments.clone())
            .await?;
        require(
            data(&retry)["operationId"] == operation && data(&retry)["state"] == "failed",
            "Storage retry repeated installation",
        )?;
        Ok::<_, String>((rejected, retry))
    }
    .await;
    // Restore space even when the installer or assertion fails. Drop is a second
    // bounded cleanup attempt for early exits; it only touches our own directory.
    let restored = tauri::async_runtime::spawn_blocking(move || {
        storage.clear()?;
        storage.free_bytes()
    })
    .await
    .map_err(|e| e.to_string())??;
    let after_guest = guest.clone();
    let after_package =
        tauri::async_runtime::spawn_blocking(move || after_guest.native_apk_fixture_hash())
            .await
            .map_err(|e| e.to_string())??;
    std::fs::write(directory.join("android-storage-restored.json"),serde_json::to_vec_pretty(&json!({"restoredFreeBytes":restored,"beforePackage":before_package,"afterPackage":after_package})).unwrap()).map_err(|e|e.to_string())?;
    require(
        restored > before_free.saturating_sub(256 * 1024 * 1024),
        "Guest fixture did not restore free space",
    )?;
    require(
        before_package == after_package,
        "Storage refusal altered the installed application",
    )?;
    let (rejected, retry) = result?;
    std::fs::write(directory.join("android-storage.json"),serde_json::to_vec_pretty(&json!({"passed":true,"deviceId":arguments["deviceId"],"generation":arguments["generation"],"beforeFreeBytes":before_free,"filledFreeBytes":filled_free,"restoredFreeBytes":restored,"fillMiB":fill_blocks,"beforePackage":before_package,"afterPackage":after_package,"rejected":rejected,"retry":retry})).unwrap()).map_err(|e|e.to_string())?;
    Ok(())
}
