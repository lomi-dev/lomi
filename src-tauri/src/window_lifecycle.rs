use tauri::{Manager, Window};

#[tauri::command]
pub fn hide_main_window(window: Window) -> Result<(), String> {
    crate::files::main_window(&window)?;
    window
        .hide()
        .map_err(|_| "Could not hide the workspace.".into())
}

#[tauri::command]
pub fn request_quit(window: Window) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Untrusted caller.".into());
    }
    window.app_handle().exit(0);
    Ok(())
}

#[tauri::command]
pub fn reopen_main_window(window: Window) -> Result<(), String> {
    crate::files::main_window(&window)?;
    let main = window
        .app_handle()
        .get_window("main")
        .ok_or("Workspace unavailable.")?;
    main.unminimize().map_err(|_| "Workspace unavailable.")?;
    main.show().map_err(|_| "Workspace unavailable.")?;
    main.set_focus()
        .map_err(|_| "Workspace unavailable.".into())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn install_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::{
        menu::{Menu, MenuItem},
        tray::TrayIconBuilder,
    };
    let show = MenuItem::with_id(app, "show-workspace", "Show Lomi", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Lomi", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut builder = TrayIconBuilder::with_id("lomi-background")
        .tooltip("Lomi")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show-workspace" => {
                if let Some(main) = app.get_window("main") {
                    let _ = main.unminimize();
                    let _ = main.show();
                    let _ = main.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    app.manage(builder.build(app)?);
    Ok(())
}
