#![cfg(windows)]

#[path = "../src/windows_instance.rs"]
mod windows_instance;

#[path = "../src/external_files.rs"]
mod external_files;

mod files {
    pub(crate) fn main_window(window: &tauri::Window) -> Result<(), String> {
        if window.label() == "main" {
            Ok(())
        } else {
            Err("Unknown application window.".into())
        }
    }
}
