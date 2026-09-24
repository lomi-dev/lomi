//! Scoped MCP metadata over the existing managed-device owner. Never starts ADB.
use lomi_control_protocol::{android::*, ErrorCode};
use tauri::Manager as _;

pub(crate) fn snapshot(
    app: &tauri::AppHandle,
    control: std::sync::Arc<lomi_control_core::android::AndroidControl>,
    input: AndroidSnapshotInput,
    snapshot_id: String,
    deadline: std::time::Instant,
) -> Result<AndroidSnapshot, ErrorCode> {
    use std::sync::Arc;
    control.check_generation(&input.generation)?;
    let window = app.get_window("main").ok_or(ErrorCode::UiNotReady)?;
    let manager = super::commands::backend(&window, &app.state::<super::manager::Android>())
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let runtime = manager
        .agent_runtime(&control, &input.generation)
        .map_err(|_| ErrorCode::StaleGeneration)?;
    let hardware = runtime.status().display.ok_or(ErrorCode::TargetBusy)?;
    let token = control.clone();
    let generation = input.generation.clone();
    let guard: super::runtime::DispatchGuard = Arc::new(move || {
        token
            .check_generation(&generation)
            .map_err(|_| "Android observation authority ended".to_string())?;
        if std::time::Instant::now() >= deadline {
            return Err("Android observation expired".into());
        }
        Ok(())
    });
    let guest = tauri::async_runtime::block_on(async {
        tokio::time::timeout_at(
            deadline.into(),
            runtime.observation_guest(input.generation.clone(), guard.clone()),
        )
        .await
        .map_err(|_| ErrorCode::DeadlineExceeded)?
        .map_err(|_| ErrorCode::StaleGeneration)
    })?;
    let xml = guest.hierarchy(deadline, guard);
    control.check_generation(&input.generation)?;
    if std::time::Instant::now() >= deadline {
        return Err(ErrorCode::DeadlineExceeded);
    }
    let xml = xml.map_err(|_| ErrorCode::UnsupportedCapability)?;
    super::hierarchy::parse(&xml, &input, snapshot_id, [hardware.0, hardware.1])
}

pub(crate) fn list(
    app: &tauri::AppHandle,
    allowed: &[String],
) -> Result<AndroidDevices, ErrorCode> {
    list_checked(app, allowed, &|| Ok(()))
}

pub(crate) fn list_all(
    app: &tauri::AppHandle,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<AndroidDevices, ErrorCode> {
    check()?;
    let window = app.get_window("main").ok_or(ErrorCode::UiNotReady)?;
    let manager = super::commands::backend(&window, &app.state::<super::manager::Android>())
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let devices = manager
        .directory
        .lock()
        .map_err(|_| ErrorCode::StorageUnavailable)?
        .devices()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let mut allowed = Vec::new();
    for device in devices.devices.into_iter().take(16) {
        check()?;
        allowed.push(device.id);
    }
    check()?;
    list_checked(app, &allowed, check)
}

fn list_checked(
    app: &tauri::AppHandle,
    allowed: &[String],
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<AndroidDevices, ErrorCode> {
    if allowed.len() > 16 || allowed.iter().any(|id| !valid_device_id(id)) {
        return Err(ErrorCode::ScopeDenied);
    }
    check()?;
    let window = app.get_window("main").ok_or(ErrorCode::UiNotReady)?;
    let manager = super::commands::backend(&window, &app.state::<super::manager::Android>())
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let devices = manager
        .directory
        .lock()
        .map_err(|_| ErrorCode::StorageUnavailable)?
        .devices()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let statuses = manager
        .statuses()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let items = devices
        .devices
        .into_iter()
        .filter_map(|d| {
            if !allowed.contains(&d.id) {
                return None;
            }
            Some(d)
        })
        .map(|device| {
            check()?;
            let status = statuses.iter().find(|s| s.device_id == device.id);
            use super::runtime::Phase;
            Ok(AndroidDevice {
                device_id: device.id,
                name: device.name,
                generation: status.and_then(|s| s.generation.clone()),
                phase: match status.map(|s| s.phase).unwrap_or(Phase::Stopped) {
                    Phase::Stopped => AndroidPhase::Stopped,
                    Phase::Starting => AndroidPhase::Starting,
                    Phase::Booting => AndroidPhase::Booting,
                    Phase::Running => AndroidPhase::Running,
                    Phase::Stopping => AndroidPhase::Stopping,
                    Phase::Failed => AndroidPhase::Failed,
                },
                process_alive: status.is_some_and(|s| s.process_alive),
                display: status.and_then(|s| s.display.map(|(w, h)| [w, h])),
            })
        })
        .collect::<Result<Vec<_>, ErrorCode>>()?;
    check()?;
    Ok(AndroidDevices {
        devices_revision: devices.revision.to_string(),
        host_qualified: super::host::require_qualification().is_ok(),
        items,
    })
}

pub(crate) fn logcat(
    app: &tauri::AppHandle,
    control: std::sync::Arc<lomi_control_core::android::AndroidControl>,
    input: AndroidLogcatInput,
    deadline: std::time::Instant,
) -> Result<lomi_control_core::broker::AndroidLogBatch, ErrorCode> {
    control.check_generation(&input.generation)?;
    let window = app.get_window("main").ok_or(ErrorCode::UiNotReady)?;
    let manager = super::commands::backend(&window, &app.state::<super::manager::Android>())
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let runtime = manager.agent_runtime(&control, &input.generation)?;
    let token = control.clone();
    let generation = input.generation.clone();
    let guard: super::runtime::DispatchGuard = std::sync::Arc::new(move || {
        token
            .check_generation(&generation)
            .map_err(|_| "Android logs authority ended".to_string())?;
        if std::time::Instant::now() >= deadline {
            return Err("Android logs expired".into());
        }
        Ok(())
    });
    let guest = tauri::async_runtime::block_on(async {
        tokio::time::timeout_at(
            deadline.into(),
            runtime.observation_guest(input.generation.clone(), guard.clone()),
        )
        .await
        .map_err(|_| ErrorCode::DeadlineExceeded)?
        .map_err(|_| ErrorCode::StaleGeneration)
    })?;
    let result = guest.app_logs(&input.package_name, input.min_priority, deadline, guard);
    control.check_generation(&input.generation)?;
    if std::time::Instant::now() >= deadline {
        return Err(ErrorCode::DeadlineExceeded);
    }
    result.map_err(|_| ErrorCode::UnsupportedCapability)
}
