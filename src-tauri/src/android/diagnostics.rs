use super::{installation, manager::Manager, storage};
use std::{fs, io::Read, path::Path, sync::Arc};

pub async fn collect(manager: Arc<Manager>) -> Result<String, String> {
    let mut logs = Vec::new();
    for (id, runtime) in manager.diagnostic_runtimes()? {
        logs.push((format!("{id} (live)"), runtime.diagnostic_log().await?));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let directory = manager.directory.lock().map_err(|_| "Android directory failed")?;
        let mut text = format!("Lomi Android diagnostics\nHost: {} / {}\nLogs are limited to 64 KiB each. Authentication lines and managed/home paths are removed.\n\n", std::env::consts::OS, std::env::consts::ARCH);
        let path = installation::checked_path(&directory.root, Path::new("logs"))?;
        if path.exists() {
            for (index, entry) in fs::read_dir(path).map_err(|e| e.to_string())?.enumerate() {
                if index >= 4096 { return Err("Android logs exceed the inspection limit. Use Maintenance cleanup.".into()); }
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if name != "installer.log" && !name.strip_suffix(".log").is_some_and(storage::valid_id) { continue; }
                let metadata = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
                if metadata.file_type().is_symlink() || !metadata.is_file() { continue; }
                let mut bytes = vec![];
                fs::File::open(entry.path()).map_err(|e| e.to_string())?.take(65536).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
                logs.push((name, String::from_utf8_lossy(&bytes).into_owned()));
                if logs.len() >= 64 { break; }
            }
        }
        let mut paths = vec![directory.root.to_string_lossy().into_owned()];
        for variable in ["HOME", "USERPROFILE"] {
            if let Ok(path) = std::env::var(variable) { if !path.is_empty() { paths.push(path); } }
        }
        for (name, log) in logs {
            text.push_str(&format!("--- {name} ---\n"));
            text.push_str(&redact(&log, &paths));
            text.push('\n');
            if text.len() > 4 * 1024 * 1024 { return Err("Android diagnostics exceed the export limit".into()); }
        }
        Ok(text)
    }).await.map_err(|e| e.to_string())?
}

fn redact(text: &str, paths: &[String]) -> String {
    let mut output = String::new();
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if [
            "token",
            "authorization",
            "bearer",
            "jwt",
            "jwk",
            "private key",
            "generation_key",
            "generationkey",
            "password",
            "secret",
        ]
        .iter()
        .any(|word| lower.contains(word))
        {
            output.push_str("[authentication details removed]\n");
            continue;
        }
        let mut line = line.to_string();
        for path in paths {
            line = line.replace(path, "[private path]");
        }
        output.extend(line.chars().filter(|c| !c.is_control() || *c == '\t'));
        output.push('\n');
    }
    output
}

#[cfg(test)]
mod tests {
    #[test]
    fn exported_logs_omit_credentials_private_paths_and_terminal_controls() {
        let result = super::redact("ready at /private/android/sdk\nauthorization: secret\nJWT eyJsecret\nerror\u{1b}[31m\n", &["/private/android".into()]);
        assert!(!result.contains("secret"));
        assert!(!result.contains("/private/android"));
        assert!(!result.contains('\u{1b}'));
        assert!(result.contains("[private path]/sdk"));
    }
}
