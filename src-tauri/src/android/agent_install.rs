use lomi_control_core::broker::AndroidInstallRequest;
use lomi_control_protocol::{android::AndroidInstallResult, ErrorCode};
use std::{
    io::{Seek, SeekFrom},
    sync::Arc,
    time::{Duration, Instant},
};
use tauri::Manager as _;

pub(crate) fn install(
    app: &tauri::AppHandle,
    mut request: AndroidInstallRequest,
) -> Result<AndroidInstallResult, ErrorCode> {
    let _producer = lomi_control_core::artifacts::ProducerPermit::acquire()?;
    request.check()?;
    request.project_directory.check()?;
    let permit = request.permit.clone();
    let control = request.control.clone();
    let generation = request.input.generation.clone();
    let check = || {
        permit.check()?;
        control.check_generation(&generation)
    };
    request.file.verify(check)?;
    let metadata = super::agent_apk::metadata(&request.file.file, &check)?;
    // Manifest inspection shares the read-only descriptor offset. The installer
    // consumes the same file from byte zero, held and quota-pinned to completion.
    request
        .file
        .file
        .seek(SeekFrom::Start(0))
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let window = app.get_window("main").ok_or(ErrorCode::UiNotReady)?;
    let manager = super::commands::backend(&window, &app.state::<super::manager::Android>())
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let runtime = manager.agent_runtime(&request.control, &request.input.generation)?;
    let deadline = Instant::now() + Duration::from_secs(100);
    let guard: super::runtime::DispatchGuard = Arc::new(move || {
        permit
            .check()
            .and_then(|_| control.check_generation(&generation))
            .map_err(|_| "APK installation authority ended".to_string())?;
        if Instant::now() >= deadline {
            return Err("APK installation expired".into());
        }
        Ok(())
    });
    let guest = tauri::async_runtime::block_on(async {
        tokio::time::timeout(
            Duration::from_secs(5),
            runtime.observation_guest(request.input.generation.clone(), guard.clone()),
        )
        .await
        .map_err(|_| ErrorCode::DeadlineExceeded)?
        .map_err(|_| ErrorCode::StaleGeneration)
    })?;
    let previous = guest
        .package_version(
            &metadata.package,
            Instant::now() + Duration::from_secs(5),
            guard.clone(),
        )
        .ok()
        .flatten();
    request.mark_dispatching()?;
    let file = request
        .file
        .file
        .try_clone()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let installed = tauri::async_runtime::block_on(runtime.install_apk_guarded(
        request.input.generation.clone(),
        file,
        Some(guard.clone()),
    ));
    request.check()?;
    let installer_failure = match installed {
        Ok(()) => None,
        Err(message) => Some(installer_failure(&message).ok_or(ErrorCode::OutcomeUnknown)?),
    };
    let new_version = if installer_failure.is_none() {
        guest
            .package_version(
                &metadata.package,
                Instant::now() + Duration::from_secs(5),
                guard,
            )
            .ok()
            .flatten()
    } else {
        None
    };
    request.check()?;
    if installer_failure.is_none()
        && new_version
            .as_ref()
            .zip(metadata.version.as_ref())
            .is_some_and(|(observed, expected)| observed != expected)
    {
        return Err(ErrorCode::OutcomeUnknown);
    }
    Ok(AndroidInstallResult {
        workspace_id: request.input.workspace_id,
        device_id: request.input.device_id,
        generation: request.input.generation,
        artifact_id: request.input.artifact_id,
        sha256: request.input.sha256,
        installed: installer_failure.is_none(),
        package_name: Some(metadata.package),
        previous_version: previous,
        new_version,
        installer_failure,
    })
}
fn installer_failure(message: &str) -> Option<String> {
    let code = message
        .strip_prefix("APK installation failed: ")?
        .trim()
        .strip_prefix("Failure [")?
        .split([':', ']'])
        .next()?;
    (code.starts_with("INSTALL_")
        && code.len() <= 128
        && code
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'))
    .then(|| code.to_owned())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installer_failure_keeps_only_definitive_bounded_machine_code() {
        assert_eq!(installer_failure("APK installation failed: Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: private description]"),Some("INSTALL_FAILED_UPDATE_INCOMPATIBLE".into()));
        assert_eq!(installer_failure("APK installation failed: Failure [INSTALL_FAILED_INSUFFICIENT_STORAGE: bounded test]"), Some("INSTALL_FAILED_INSUFFICIENT_STORAGE".into()));
        for message in [
            "connection lost",
            "APK installation failed: Failure [INSTALL_private]",
            "APK installation failed: private text",
            "APK installation failed: Failure [Success]",
        ] {
            assert!(installer_failure(message).is_none());
        }
    }
}
