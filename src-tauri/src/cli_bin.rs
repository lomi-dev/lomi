//! Locates CLI binaries already installed on the system. Never spawns.

use std::path::{Path, PathBuf};

fn executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    let mode_ok = std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o111 != 0;
    #[cfg(not(unix))]
    let mode_ok = true;
    meta.is_file() && mode_ok
}

fn resolve_in(cli: &str, mut dirs: Vec<PathBuf>, home: Option<&Path>) -> Option<PathBuf> {
    if cli.is_empty() || cli.contains(['/', '\\', '\0']) {
        return None;
    }
    if let Some(home) = home {
        dirs.push(home.join(".local/bin"));
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    if let (Some(home), "kimi") = (home, cli) {
        dirs.push(home.join(".kimi-code/bin"));
    }
    dirs.into_iter()
        .map(|dir| dir.join(cli))
        .find(|path| executable(path))
}

pub(crate) fn resolve(cli: &str) -> Option<PathBuf> {
    let dirs = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    resolve_in(cli, dirs, home.as_deref())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn finds_executables_only_in_known_directories() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".kimi-code/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let kimi = bin.join("kimi");
        std::fs::write(&kimi, "#!/bin/sh\n").unwrap();
        assert_eq!(resolve_in("kimi", vec![], Some(home.path())), None);
        std::fs::set_permissions(&kimi, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(resolve_in("kimi", vec![], Some(home.path())), Some(kimi));
        assert_eq!(resolve_in("codex", vec![], Some(home.path())), None);
        assert_eq!(resolve_in("../kimi", vec![], Some(home.path())), None);
    }
}
