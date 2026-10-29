// Hide the extra Windows console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
mod linux_graphics;

fn main() {
    if let Err(error) = lomi_platform::installation::check_current_executable() {
        eprintln!("{error}");
        std::process::exit(4);
    }
    #[cfg(feature = "remote-host")]
    if let Some(code) = lomi_host::internal_native_entry() {
        std::process::exit(code);
    }
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--internal-native-evidence"))
    {
        let mut args = std::env::args_os().skip(2);
        let (Some(mode), None) = (args.next(), args.next()) else {
            std::process::exit(2);
        };
        let Some(mode) = mode.to_str() else {
            std::process::exit(2);
        };
        std::process::exit(lomi_lib::internal_native_evidence(mode));
    }
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--internal-pty-coalition-protocol"))
    {
        #[cfg(all(target_os = "macos", feature = "experimental-macos-coalition-cleanup"))]
        {
            let mut args = std::env::args_os().skip(2);
            if args.next().as_deref() != Some(std::ffi::OsStr::new("3")) || args.next().is_some() {
                std::process::exit(2);
            }
            if lomi_platform::process::initialize_child_ownership().is_err() {
                std::process::exit(4);
            }
            std::process::exit(lomi_terminal::internal_pty_coalition_protocol(3));
        }
        #[cfg(not(all(target_os = "macos", feature = "experimental-macos-coalition-cleanup")))]
        std::process::exit(2);
    }
    #[cfg(all(target_os = "macos", feature = "experimental-macos-coalition-cleanup"))]
    let coalition_mode = std::env::args_os().nth(1);
    #[cfg(all(target_os = "macos", feature = "experimental-macos-coalition-cleanup"))]
    if coalition_mode.as_deref().is_some_and(|mode| {
        mode == std::ffi::OsStr::new("--internal-pty-coalition")
            || mode == std::ffi::OsStr::new("--internal-pty-coalition-recover")
    }) {
        let mut args = std::env::args_os().skip(2);
        let Some(directory) = args.next() else {
            std::process::exit(2);
        };
        if args.next().is_some() {
            std::process::exit(2);
        }
        if lomi_platform::process::initialize_child_ownership().is_err() {
            std::process::exit(4);
        }
        let directory = std::path::Path::new(&directory);
        if coalition_mode.as_deref() == Some(std::ffi::OsStr::new("--internal-pty-coalition")) {
            std::process::exit(lomi_terminal::internal_pty_coalition(directory));
        }
        std::process::exit(
            if lomi_terminal::internal_pty_coalition_recover(directory).is_ok() {
                0
            } else {
                4
            },
        );
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--internal-host-status"))
    {
        #[cfg(all(target_os = "macos", feature = "remote-host"))]
        {
            let mut args = std::env::args_os().skip(2);
            let (Some(profile), Some(expected_pid), None) = (args.next(), args.next(), args.next())
            else {
                std::process::exit(2);
            };
            let Some(expected_pid) = expected_pid
                .to_str()
                .and_then(|value| {
                    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
                        .then(|| value.parse::<u32>().ok())
                        .flatten()
                })
                .filter(|value| *value > 0 && *value <= i32::MAX as u32)
            else {
                std::process::exit(2);
            };
            std::process::exit(lomi_platform::service_macos::internal_host_status(
                std::path::Path::new(&profile),
                expected_pid,
            ));
        }
        #[cfg(not(all(target_os = "macos", feature = "remote-host")))]
        std::process::exit(2);
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--internal-git-worker"))
    {
        #[cfg(not(feature = "remote-host"))]
        std::process::exit(2);
        #[cfg(feature = "remote-host")]
        std::process::exit(if std::env::args_os().nth(2).is_none() {
            lomi_host::internal_git_worker()
        } else {
            2
        });
    }
    if let Some(code) = lomi_lib::canonical_host_entry() {
        std::process::exit(code);
    }
    if let Some(code) = lomi_lib::cursor_usage_keychain_authorization_entry() {
        std::process::exit(code);
    }
    #[cfg(target_os = "macos")]
    if let Some(code) = lomi_lib::agent_runtime_host_worker_entry() {
        std::process::exit(code);
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--mcp")) {
        if let Err(error) = lomi_mcp::run(std::env::args().skip(2).collect()) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--agy-terminal-title")) {
        if let Err(error) = lomi_lib::print_agy_title() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }

    #[cfg(target_os = "linux")]
    linux_graphics::configure();

    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::path::Path::new("/sys/module/nvidia").exists()
        && std::env::var_os("__NV_DISABLE_EXPLICIT_SYNC").is_none()
    {
        // Avoid WebKitGTK's NVIDIA Wayland Error 71 before Tauri starts GUI threads.
        std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");
    }

    lomi_lib::run()
}
