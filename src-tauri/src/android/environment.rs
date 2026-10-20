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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn external_windows_paths_preserve_identity_without_verbatim_cli_arguments() {
        let temp = tempfile::tempdir().unwrap();
        let native_root = temp.path().join("private Android żółć");
        std::fs::create_dir(&native_root).unwrap();
        let root = native_root.canonicalize().unwrap();
        for directory in ["user", "sdk", "private-java/bin"] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        let program = root.join("android-cli.exe");
        std::fs::write(&program, b"fixture").unwrap();
        let sdk = root.join("sdk");
        let java = root.join("private-java");
        let process = command(&program, &root, &sdk, Some(&java), 5037).unwrap();
        let environment: std::collections::BTreeMap<_, _> = process.get_envs().collect();
        assert!(!process.get_program().to_string_lossy().starts_with(r"\\?\"));
        assert_eq!(
            std::fs::canonicalize(process.get_program()).unwrap(),
            program
        );
        assert_eq!(
            process.get_current_dir().unwrap().canonicalize().unwrap(),
            root.join("user")
        );
        for name in [
            "ANDROID_HOME",
            "ANDROID_SDK_ROOT",
            "ANDROID_USER_HOME",
            "ANDROID_EMULATOR_HOME",
            "ANDROID_AVD_HOME",
            "TMPDIR",
            "TMP",
            "TEMP",
            "JAVA_HOME",
        ] {
            let value = environment[std::ffi::OsStr::new(name)].unwrap();
            assert!(!value.to_string_lossy().starts_with(r"\\?\"), "{name}");
        }
        assert_eq!(
            Path::new(environment[std::ffi::OsStr::new("JAVA_HOME")].unwrap())
                .canonicalize()
                .unwrap(),
            java
        );
        assert!(!environment[std::ffi::OsStr::new("JAVA_TOOL_OPTIONS")]
            .unwrap()
            .to_string_lossy()
            .contains(r"\\?\"));
        let path = environment[std::ffi::OsStr::new("PATH")].unwrap();
        assert_eq!(
            std::env::split_paths(path)
                .next()
                .unwrap()
                .canonicalize()
                .unwrap(),
            java.join("bin")
        );
        assert_eq!(external_path(&root).canonicalize().unwrap(), root);
        let unc = Path::new(r"\\server\share\private Android");
        assert_eq!(external_path(unc), unc);
        // dunce deliberately retains prefixes whose removal may change meaning.
        let verbatim_unc = Path::new(r"\\?\UNC\server\share\private Android");
        assert_eq!(external_path(verbatim_unc), verbatim_unc);
    }

    #[cfg(unix)]
    #[test]
    fn external_unix_paths_preserve_non_unicode_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let bytes = b"/tmp/private-\xff Android";
        let path = Path::new(std::ffi::OsStr::from_bytes(bytes));
        assert_eq!(external_path(path).as_os_str().as_bytes(), bytes);
    }

    #[cfg(windows)]
    #[test]
    fn private_java_bin_precedes_all_inherited_windows_path_entries() {
        let java = Path::new(r"C:\Lomi trial\private Java");
        let inherited_entries = [
            Path::new(r"C:\Windows\System32"),
            Path::new(r"D:\tools with spaces"),
            Path::new(r"D:\żółć\tools"),
        ];
        let inherited = std::env::join_paths(inherited_entries).unwrap();
        let path = private_java_path(java, Some(&inherited)).unwrap();
        let actual: Vec<_> = std::env::split_paths(&path).collect();
        let expected: Vec<_> = std::iter::once(java.join("bin"))
            .chain(inherited_entries.into_iter().map(Path::to_path_buf))
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(
            std::env::split_paths(&private_java_path(java, None).unwrap()).collect::<Vec<_>>(),
            vec![java.join("bin")]
        );
        assert!(private_java_path(Path::new("C:\\invalid\"java"), None)
            .unwrap_err()
            .contains("private Android Java PATH"));
    }

    #[test]
    fn private_java_configuration_is_scoped_to_the_child_command() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let java = root.join("private-java");
        let inherited = std::env::var_os("PATH");
        let process = command(
            &root.join("tool"),
            root,
            &root.join("sdk"),
            Some(&java),
            5037,
        )
        .unwrap();
        let environment: std::collections::BTreeMap<_, _> = process.get_envs().collect();
        assert_eq!(
            environment[std::ffi::OsStr::new("JAVA_HOME")],
            Some(java.as_os_str())
        );
        #[cfg(windows)]
        {
            let path = environment[std::ffi::OsStr::new("PATH")].unwrap();
            let expected: Vec<_> = std::iter::once(java.join("bin"))
                .chain(
                    inherited
                        .as_deref()
                        .into_iter()
                        .flat_map(std::env::split_paths),
                )
                .collect();
            assert_eq!(std::env::split_paths(path).collect::<Vec<_>>(), expected);
        }
        #[cfg(not(windows))]
        assert!(!environment.contains_key(std::ffi::OsStr::new("PATH")));
        assert_eq!(std::env::var_os("PATH"), inherited);
    }

    #[test]
    fn children_cannot_inherit_foreign_sdk_adb_or_jvm_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let mut process = command(&root.join("tool"), root, &root.join("sdk"), None, 5037).unwrap();
        let dangerous = [
            "ANDROID_SDK_HOME",
            "ADB_SERVER_PORT",
            "JDK_JAVA_OPTIONS",
            "_JAVA_OPTIONS",
            "REPO_OS_OVERRIDE",
            "CLASSPATH",
            "DYLD_INSERT_LIBRARIES",
        ];
        sanitize(&mut process, dangerous.into_iter().map(OsString::from));
        let environment: std::collections::BTreeMap<_, _> = process.get_envs().collect();
        for name in dangerous {
            assert_eq!(environment.get(std::ffi::OsStr::new(name)), Some(&None));
        }
        assert!(!environment.contains_key(std::ffi::OsStr::new("HOME")));
        assert!(!environment.contains_key(std::ffi::OsStr::new("PATH")));
        assert_eq!(
            environment[std::ffi::OsStr::new("ADB_SERVER_SOCKET")],
            Some(std::ffi::OsStr::new("tcp:127.0.0.1:5037"))
        );
        assert_eq!(process.get_current_dir(), Some(root.join("user").as_path()));
        assert_eq!(
            java_home_option("/data/Bob's \"phone\""),
            "-Duser.home='/data/Bob'\"'\"'s \"phone\"'"
        );
    }
}
