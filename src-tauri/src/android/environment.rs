use std::{ffi::OsString, path::Path, process::Command};

/// The parent environment is never mutated. Every Android child receives one owned
/// SDK, private user directories and an explicit loopback ADB endpoint.
pub fn command(
    program: &Path,
    root: &Path,
    sdk: &Path,
    java: Option<&Path>,
    adb_port: u16,
) -> Result<Command, String> {
    if !program.is_absolute() || !root.is_absolute() || !sdk.is_absolute() || adb_port == 0 {
        return Err("Invalid managed Android process configuration.".into());
    }
    // External Android/Java tools may concatenate paths using forward slashes,
    // which Windows rejects with verbatim prefixes. Keep canonical paths inside
    // Lomi and use their equivalent compatible form only at this boundary.
    let program = external_path(program);
    let root = external_path(root);
    let sdk = external_path(sdk);
    let java = java.map(external_path);
    let mut command = Command::new(program);
    sanitize(&mut command, std::env::vars_os().map(|(name, _)| name));
    let cli_home = root.join("user/cli-home");
    let cli_home = cli_home
        .to_str()
        .ok_or("Android CLI requires a Unicode data-directory path.")?;
    command
        .current_dir(root.join("user"))
        .env("ANDROID_HOME", sdk)
        .env("ANDROID_SDK_ROOT", sdk)
        .env("ANDROID_USER_HOME", root.join("user"))
        .env("ANDROID_EMULATOR_HOME", root.join("emulator-home"))
        .env("ANDROID_AVD_HOME", root.join("avd"))
        .env("ANDROID_ADB_SERVER_PORT", adb_port.to_string())
        .env("ADB_SERVER_SOCKET", format!("tcp:127.0.0.1:{adb_port}"))
        .env("TMPDIR", root.join("tmp"))
        .env("TMP", root.join("tmp"))
        .env("TEMP", root.join("tmp"))
        // Android CLI reads .androidrc using Java's user.home, independently of SDK flags.
        .env("JAVA_TOOL_OPTIONS", java_home_option(cli_home));
    if let Some(java) = java {
        command.env("JAVA_HOME", java);
        #[cfg(windows)]
        command.env(
            "PATH",
            private_java_path(java, std::env::var_os("PATH").as_deref())?,
        );
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    Ok(command)
}

/// Convert a verified internal path only when handing it to an external tool.
/// This performs no I/O and does not replace canonical ownership comparisons.
pub(super) fn external_path(path: &Path) -> &Path {
    #[cfg(windows)]
    {
        dunce::simplified(path)
    }
    #[cfg(not(windows))]
    {
        path
    }
}

#[cfg(windows)]
fn private_java_path(java: &Path, inherited: Option<&std::ffi::OsStr>) -> Result<OsString, String> {
    // The verified private JRE also supplies the runtime DLLs needed by the
    // native Android CLI. Its Java subprocesses inherit this same child PATH.
    std::env::join_paths(
        std::iter::once(external_path(java).join("bin"))
            .chain(inherited.into_iter().flat_map(std::env::split_paths)),
    )
    .map_err(|error| format!("Cannot configure the private Android Java PATH: {error}"))
}

fn sanitize(command: &mut Command, names: impl IntoIterator<Item = OsString>) {
    for name in names {
        let upper = name.to_string_lossy().to_ascii_uppercase();
        if [
            "ANDROID_",
            "ADB_",
            "JAVA_",
            "JDK_",
            "_JAVA_",
            "REPO_",
            "QT_",
            "SDKMANAGER_",
            "AVDMANAGER_",
            "DYLD_",
        ]
        .iter()
        .any(|prefix| upper.starts_with(prefix))
            || matches!(
                upper.as_str(),
                "LD_PRELOAD" | "LD_LIBRARY_PATH" | "CLASSPATH"
            )
        {
            command.env_remove(name);
        }
    }
}

fn java_home_option(path: &str) -> String {
    // HotSpot's environment-option parser concatenates adjacent quoted fragments.
    // Backslash-escaping quotes corrupts paths; verified against the private JRE.
    format!("-Duser.home='{}'", path.replace('\'', "'\"'\"'"))
}
