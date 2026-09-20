mod agent_notifications;
mod android;
#[cfg(feature = "android-probe")]
#[path = "../../tests/native/android-support.rs"]
mod android_probe;
#[cfg(feature = "android-probe")]
#[path = "../../tests/native/android-product-support.rs"]
mod android_product;
mod android_protocol;
mod browser;
mod chat;
#[cfg(feature = "chat-probe")]
#[path = "../../tests/native/chat-support.rs"]
mod chat_probe;
mod cli_config;
mod cli_titles;
pub use cli_titles::print_agy_title;
mod editor_preferences;
mod files;
mod git;
mod keybindings;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(feature = "native-smoke")]
#[path = "../../tests/native/support.rs"]
mod native_smoke;
mod plugins;
mod settings_window;
mod shell;
mod terminal;
mod terminal_preferences;
mod themes;
mod updater;

use tauri::{Manager, State, Window};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    directory: String,
    home: String,
    platform: String,
    profiles: Vec<shell::Profile>,
}

#[tauri::command]
fn app_info(window: Window, shells: State<'_, terminal::Shells>) -> Result<AppInfo, String> {
    files::main_window(&window)?;
    let mut directory = std::env::current_dir().unwrap_or_else(|_| shell::home());
    if cfg!(debug_assertions)
        && directory
            .file_name()
            .is_some_and(|name| name == "src-tauri")
    {
        directory.pop();
    }
    Ok(AppInfo {
        directory: directory.to_string_lossy().into_owned(),
        home: shell::home().to_string_lossy().into_owned(),
        platform: std::env::consts::OS.into(),
        profiles: shells.profiles.clone(),
    })
}

#[tauri::command]
fn show_ready_window(window: Window, background: [u8; 3]) -> Result<bool, String> {
    if !matches!(window.label(), "main" | "settings") {
        return Err("Unknown application window.".into());
    }
    let visible = window.is_visible().map_err(|error| error.to_string())?;
    if !visible {
        #[cfg(target_os = "linux")]
        {
            use gtk::prelude::*;
            // Tao's transparent draw path does not normalize byte RGB values.
            // GTK paints the startup color correctly through its CSS provider.
            window
                .gtk_window()
                .map_err(|error| error.to_string())?
                .set_app_paintable(false);
        }
        window
            .set_background_color(Some(tauri::window::Color(
                background[0],
                background[1],
                background[2],
                255,
            )))
            .map_err(|error| error.to_string())?;
    }
    if window.label() == "settings" {
        return settings_window::ready(&window);
    }
    if !visible {
        window.show().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
    }
    Ok(true)
}

#[tauri::command]
fn finish_window_startup(window: Window) -> Result<(), String> {
    if !matches!(window.label(), "main" | "settings") {
        return Err("Unknown application window.".into());
    }
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;
        window
            .gtk_window()
            .map_err(|error| error.to_string())?
            .set_app_paintable(true);
    }
    window
        .set_background_color(Some(tauri::window::Color(0, 0, 0, 0)))
        .map_err(|error| error.to_string())
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(
            tauri::plugin::Builder::<tauri::Wry, ()>::new("browser")
                .invoke_handler(tauri::generate_handler![browser::signal])
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(chat::commands::Chats::default())
        .manage(android::manager::Android::default())
        .manage(android::open::Requests::default())
        .manage(browser::Browsers::default())
        .manage(terminal::Terminals::default())
        .manage(cli_titles::CliTitleConfig::default())
        .manage(files::SessionFile::default())
        .manage(files::search::ProjectSearch::default())
        .manage(files::editor::EditorFiles::default())
        .manage(keybindings::KeybindingsFile::default())
        .manage(editor_preferences::EditorPreferencesFile::default())
        .manage(terminal_preferences::TerminalPreferencesFile::default())
        .manage(themes::Themes::default())
        .manage(plugins::Plugins::default())
        .manage(settings_window::SettingsWindow::default())
        .on_window_event(|window, event| {
            settings_window::on_window_event(window, event);
            android::commands::window_event(window, event);
        })
        .on_page_load(|_view, _payload| {
            #[cfg(feature = "native-smoke")]
            native_smoke::page(_view, _payload);
            #[cfg(feature = "chat-probe")]
            chat_probe::page(_view, _payload);
        })
        .register_asynchronous_uri_scheme_protocol("theme", themes::protocol)
        .register_asynchronous_uri_scheme_protocol("plugin", plugins::protocol)
        .setup(|app| {
            #[cfg(target_os = "macos")]
            macos::setup_menu(app)?;
            let integration = app.path().app_data_dir()?.join("shell-integration");
            shell::prepare(&integration).map_err(std::io::Error::other)?;
            app.manage(terminal::Shells {
                profiles: shell::discover(),
                integration,
            });
            #[cfg(feature = "chat-probe")]
            app.manage(chat_probe::Probe::default());
            let handle = app.handle().clone();
            #[cfg(feature = "android-probe")]
            app.manage(android_probe::Probe::default());
            #[cfg(feature = "android-probe")]
            android_probe::watch_native_control(app.handle().clone());
            #[cfg(feature = "android-probe")]
            android_product::watch(app.handle().clone());
            // Load settings alongside the workspace so its first click can
            // reuse the prepared view. Window creation stays off the GUI thread.
            tauri::async_runtime::spawn(async move {
                if let Err(error) = settings_window::prepare(&handle).await {
                    eprintln!("Cannot prepare settings window: {error}");
                }
            });
            Ok(())
        })
        .invoke_handler(|invoke| {
            // Child browser views share the main window, but never its application privileges.
            let view = invoke.message.webview();
            if !browser::trusted_app_view(view.label(), view.window().label()) {
                invoke
                    .resolver
                    .reject("Web pages cannot call application commands.");
                return true;
            }
            #[cfg(feature = "native-smoke")]
            if invoke.message.command() == "plugin_smoke_result" {
                let smoke: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool =
                    tauri::generate_handler![native_smoke::plugin_smoke_result];
                return smoke(invoke);
            }
            #[cfg(feature = "chat-probe")]
            if invoke.message.command().starts_with("chat_probe_") {
                let probe: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
                    chat_probe::chat_probe_start,
                    chat_probe::chat_probe_backend,
                    chat_probe::chat_probe_cancel,
                    chat_probe::chat_probe_result
                ];
                return probe(invoke);
            }
            #[cfg(feature = "android-probe")]
            if invoke.message.command().starts_with("android_probe_") {
                let probe: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
                    android_probe::android_probe_control,
                    android_probe::android_probe_subscribe,
                    android_probe::android_probe_ack,
                    android_probe::android_probe_input,
                    android_probe::android_probe_native_text,
                    android_probe::android_probe_native_pointer,
                    android_product::android_probe_product_guest,
                    android_probe::android_probe_report
                ];
                return probe(invoke);
            }
            let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
                android::commands::android_state,
                android::commands::android_prepare_setup,
                android::commands::android_setup_context,
                android::commands::android_request_open,
                android::commands::android_open_result,
                android::commands::android_catalog,
                android::commands::android_storage,
                android::commands::android_export_diagnostics,
                android::commands::android_install_plan,
                android::commands::android_install,
                android::commands::android_cancel_operation,
                android::commands::android_manage_device,
                android::commands::android_maintenance,
                android::commands::save_android_preferences,
                android::commands::android_start,
                android::commands::android_stop,
                android::commands::android_exit,
                android::commands::android_subscribe_frames,
                android::commands::android_ack_frame,
                android::commands::android_unsubscribe_frames,
                android::commands::android_input,
                android::commands::android_install_apk,
                android::commands::android_save_screenshot,
                chat::commands::chat_connection_action,
                chat::commands::chat_preview_models,
                chat::commands::chat_main,
                chat::commands::chat_preferences,
                chat::commands::chat_preferences_save,
                chat::commands::chat_generate,
                chat::commands::chat_cancel,
                chat::commands::chat_subscribe,
                chat::commands::chat_ack,
                chat::commands::chat_close,
                chat::commands::chat_flush,
                chat::commands::chat_retain,
                chat::commands::chat_export,
                chat::commands::chat_discard,
                chat::commands::chat_recover,
                browser::sync_browsers,
                browser::browser_action,
                browser::servers::local_web_servers,
                app_info,
                updater::update_environment,
                updater::check_app_update,
                updater::request_update_check,
                updater::restart_after_update,
                show_ready_window,
                finish_window_startup,
                settings_window::open_settings,
                files::list_directory,
                files::watch::watch_explorer_directories,
                files::search::search_project,
                files::search::cancel_project_search,
                files::operations::file_operation,
                files::operations::resolve_project_entry,
                files::operations::open_project_item,
                files::operations::ignore_project_item,
                files::validate_directory,
                files::preview_file,
                files::markdown::read_markdown_image,
                files::images::read_image_file,
                files::editor::resolve_editor_file,
                files::editor::read_editor_file,
                files::editor::save_editor_file,
                files::editor::save_new_editor_file,
                files::editor::watch_editor_files,
                files::load_session,
                files::save_session,
                keybindings::load_keybindings,
                keybindings::save_keybindings,
                editor_preferences::load_editor_preferences,
                editor_preferences::save_editor_preferences,
                terminal_preferences::load_terminal_preferences,
                terminal_preferences::save_terminal_preferences,
                themes::load_theme_preferences,
                themes::load_theme,
                themes::list_themes,
                themes::save_theme_preferences,
                themes::save_theme_manifest,
                themes::refresh_themes,
                themes::open_themes_folder,
                themes::import_theme,
                themes::vscode::import_vscode_themes,
                themes::vscode::export_vscode_theme,
                themes::vscode::export_vscode_icon_theme,
                themes::create_theme,
                themes::duplicate_theme,
                themes::sync_theme_window,
                plugins::list_plugins,
                plugins::import_plugin,
                plugins::enable_plugin,
                plugins::prepare_plugin,
                plugins::request_plugin_removal,
                plugins::finish_plugin_removal,
                plugins::request_plugin_restart,
                plugins::restart_plugins,
                plugins::report_plugin_status,
                git::git_status,
                git::git_fetch,
                git::git_remotes,
                git::git_push,
                files::operations::git_pull,
                git::git_stage,
                files::operations::git_discard,
                git::git_diff,
                git::git_commit,
                git::history::git_history,
                git::history::git_commit_details,
                git::history::git_commit_diff,
                terminal::start_terminal,
                terminal::write_terminal,
                terminal::resize_terminal,
                terminal::acknowledge_terminal,
                terminal::close_terminal,
                terminal::busy_terminals,
                terminal::reset_terminals,
                terminal::quote_paths,
                terminal::terminal_contexts,
                cli_titles::inspect_cli_titles,
                agent_notifications::request_agent_notification_setup,
                agent_notifications::inspect_agent_notifications,
                agent_notifications::enable_agent_notifications,
                agent_notifications::notify_agent,
                cli_titles::enable_cli_titles
            ];
            handler(invoke)
        })
        .build(tauri::generate_context!())
        .expect("failed to build SimpleBench");
    app.run(|app, event| {
        #[cfg(target_os = "macos")]
        macos::handle_run_event(app, &event);
        if matches!(event, tauri::RunEvent::Exit) {
            if let Err(error) = tauri::async_runtime::block_on(
                app.state::<android::manager::Android>().emergency_cleanup(),
            ) {
                eprintln!("Android exit cleanup could not confirm completion: {error}");
            }
            #[cfg(feature = "android-probe")]
            app.state::<android_probe::Probe>().cleanup();
            app.state::<chat::commands::Chats>().stop();
            app.state::<terminal::Terminals>().stop_all();
        }
        if let tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::Destroyed,
            ..
        } = event
        {
            if label == "main" {
                app.state::<chat::commands::Chats>().stop();
                app.state::<terminal::Terminals>().stop_all();
                app.exit(0);
            }
        }
    });
}
