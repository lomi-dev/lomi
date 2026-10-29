mod agent_control;
mod agent_notifications;
mod agent_runtime;

/// Fixed, headless evidence helper. It performs no GUI initialization, task
/// execution, account login, or configuration mutation.
pub fn internal_native_evidence(mode: &str) -> i32 {
    use std::io::{Read, Write};
    const LIMIT: usize = 128 * 1024;
    if !matches!(mode, "capture" | "verify") {
        return 2;
    }
    #[cfg(target_os = "macos")]
    if native_evidence_parent_lifetime().is_err() {
        return 4;
    }
    let mut input = Vec::new();
    if std::io::stdin()
        .lock()
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut input)
        .is_err()
        || input.len() > LIMIT
    {
        return 2;
    }
    let Ok(output) = agent_runtime::native_evidence_helper_reply(mode, &input) else {
        return 2;
    };
    if std::io::stdout().lock().write_all(&output).is_err() {
        return 4;
    }
    0
}

/// ABI 1 reserves FD 3 for a private Unix socket whose other end belongs to
/// the host holding the profile OwnerLock. The helper only reads evidence, so
/// immediate exit on host death cannot leave an outstanding mutation behind.
#[cfg(target_os = "macos")]
fn native_evidence_parent_lifetime() -> Result<(), ()> {
    const FD: libc::c_int = 3;
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(FD, metadata.as_mut_ptr()) } != 0 {
        return Err(());
    }
    let metadata = unsafe { metadata.assume_init() };
    if metadata.st_mode & libc::S_IFMT != libc::S_IFSOCK {
        return Err(());
    }
    let mut kind: libc::c_int = 0;
    let mut size = std::mem::size_of_val(&kind) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            FD,
            libc::SOL_SOCKET,
            libc::SO_TYPE,
            (&mut kind as *mut libc::c_int).cast(),
            &mut size,
        )
    } != 0
        || kind != libc::SOCK_STREAM
    {
        return Err(());
    }
    let mut peer = std::mem::MaybeUninit::<libc::sockaddr_un>::zeroed();
    let mut peer_size = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
    if unsafe { libc::getpeername(FD, peer.as_mut_ptr().cast(), &mut peer_size) } != 0
        || unsafe { peer.assume_init() }.sun_family as libc::c_int != libc::AF_UNIX
    {
        return Err(());
    }
    let flags = unsafe { libc::fcntl(FD, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(FD, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(());
    }
    std::thread::Builder::new()
        .name("native-evidence-parent".into())
        .spawn(|| {
            let mut event = libc::pollfd {
                fd: FD,
                events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
                revents: 0,
            };
            loop {
                let ready = unsafe { libc::poll(&mut event, 1, -1) };
                if ready < 0
                    && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                {
                    continue;
                }
                // This channel has no messages. Readability, EOF, an invalid
                // descriptor, or any error means the lifetime is no longer valid.
                if ready != 0 {
                    unsafe { libc::_exit(4) };
                }
            }
        })
        .map(|_| ())
        .map_err(|_| ())
}

#[cfg(target_os = "macos")]
pub fn agent_runtime_host_worker_entry() -> Option<i32> {
    agent_runtime::host_worker_entry()
}
mod android;

mod android_protocol;
mod auth;
mod browser;
mod chat;

mod cli_catalog;
mod cli_config;
mod cli_integrations;
mod cli_launch;
mod cli_mcp;
mod cli_notifications;
mod cli_titles;
mod cli_usage;
mod credential_store;

pub fn cursor_usage_keychain_authorization_entry() -> Option<i32> {
    credential_store::cursor_authorization_entry()
}

mod notifications;
mod push;

pub use cli_titles::print_agy_title;

pub fn canonical_host_entry() -> Option<i32> {
    shared_host::canonical_host_entry(&std::env::args_os().skip(1).collect::<Vec<_>>())
}

#[path = "remote/activity.rs"]
mod activity;
mod desktop_file_effects;
mod editor_preferences;
mod external_files;
mod files;
mod git;
mod keybindings;
#[cfg(target_os = "linux")]
mod linux_instance;
#[cfg(target_os = "macos")]
mod macos;
mod native_file_effects;
mod native_git_effects;

mod plugins;
mod project_write_guard;
mod remote_v2;
#[cfg(unix)]
mod settings_control;
mod settings_window;
mod shared_host;
mod shell;
mod terminal;
mod terminal_preferences;
mod terminal_renderer;
mod themes;
mod updater;
#[cfg(windows)]
mod windows_instance;

mod window_lifecycle;

use tauri::{Manager, State, Window};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    directory: String,
    home: String,
    platform: String,
    terminal_renderer: &'static str,
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
        terminal_renderer: terminal_renderer::select(
            std::env::consts::OS,
            std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").as_deref(),
        ),
        profiles: shells.profiles.clone(),
    })
}

#[tauri::command]
fn show_ready_window(window: Window, background: [u8; 3]) -> Result<bool, String> {
    if window.label() != "main" {
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
    if !visible {
        window.show().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
    }
    #[cfg(windows)]
    window
        .state::<windows_instance::Instance>()
        .listen(window.app_handle())
        .map_err(|error| error.to_string())?;
    #[cfg(target_os = "linux")]
    linux_instance::ready(window.app_handle());
    external_files::ready(window.app_handle());
    Ok(true)
}

#[tauri::command]
fn finish_window_startup(window: Window) -> Result<(), String> {
    if window.label() != "main" {
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
    let context = tauri::generate_context!();
    #[cfg(windows)]
    let instance = match windows_instance::Instance::acquire(&context.config().identifier)
        .expect("failed to acquire the Lomi application instance")
    {
        Some(instance) => instance,
        None => return,
    };
    let builder = tauri::Builder::default().manage(external_files::Requests::default());

    #[cfg(target_os = "linux")]
    let builder = builder
        .manage(linux_instance::Reopen::default())
        .plugin(linux_instance::plugin());

    #[cfg(windows)]
    let builder = builder.manage(instance);
    // Install the guarded Quit action before Tauri can create its default macOS menu.
    #[cfg(target_os = "macos")]
    let builder = builder
        .menu(macos::menu)
        .on_menu_event(macos::handle_menu_event);
    let app = builder
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
        .manage(auth::AuthController::default())
        .manage(push::Push::default())
        .manage(agent_control::Control::default())
        .manage(android::manager::Android::default())
        .manage(android::open::Requests::default())
        .manage(browser::Browsers::default())
        .manage(terminal::Terminals::default())
        .manage(cli_titles::CliTitleConfig::default())
        .manage(cli_usage::CliUsage::default())
        .manage(agent_runtime::AgentRuntime::default())
        .manage(cli_integrations::CliIntegrations::default())
        .manage(notifications::Notifications::default())
        .manage(files::SessionFile::default())
        .manage(files::search::ProjectSearch::default())
        .manage(files::editor::EditorFiles::default())
        .manage(keybindings::KeybindingsFile::default())
        .manage(editor_preferences::EditorPreferencesFile::default())
        .manage(terminal_preferences::TerminalPreferencesFile::default())
        .manage(themes::Themes::default())
        .manage(plugins::Plugins::default())
        .manage(settings_window::SettingsTabs::default())
        .manage(shared_host::SharedHost::default())
        .manage(remote_v2::RemoteV2::default())
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    #[cfg(not(windows))]
                    let _ = window.hide();
                }
            }
            android::commands::window_event(window, event);
        })
        .on_page_load(|_view, _payload| {
            agent_control::page_load(_view, _payload);
            #[cfg(windows)]
            android::commands::page_load(_view, _payload);
        })
        .register_asynchronous_uri_scheme_protocol("theme", themes::protocol)
        .register_asynchronous_uri_scheme_protocol("plugin", plugins::protocol)
        .setup(|app| {
            external_files::enqueue(
                app.handle(),
                external_files::arguments(
                    std::env::args_os().skip(1),
                    &std::env::current_dir().unwrap_or_else(|_| shell::home()),
                ),
            );
            #[cfg(target_os = "linux")]
            linux_instance::wait_for_previous_owner(app.handle())?;
            app.state::<agent_runtime::AgentRuntime>()
                .with(app.handle(), |_| Ok(()))?;

            let integration = app.path().app_data_dir()?.join("shell-integration");
            shell::prepare(&integration).map_err(std::io::Error::other)?;
            app.state::<push::Push>().initialize(app.handle());
            let account_auth = app.path().app_data_dir()?.join("account-auth");
            app.state::<auth::AuthController>()
                .initialize(&account_auth, app.handle().clone());
            app.state::<push::Push>().start(app.handle());
            app.manage(terminal::Shells {
                profiles: shell::discover(),
                integration,
            });

            #[cfg(not(target_os = "macos"))]
            window_lifecycle::install_tray(app)?;
            agent_control::initialize_startup(app.handle().clone());
            Ok(())
        })
        .invoke_handler(|invoke| {
            // Child browser views share the main window, but never its application privileges.
            let view = invoke.message.webview();
            if !browser::trusted_app_view(view.label(), view.window().label())
                && shared_host::trusted_full_window(&view.window(), &view).is_err()
            {
                invoke
                    .resolver
                    .reject("Web pages cannot call application commands.");
                return true;
            }
            let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
                external_files::external_files_pending,
                external_files::external_files_ack,
                external_files::resolve_external_file,
                cli_integrations::inspect_cli_integrations,
                cli_integrations::dismiss_cli_integrations,
                cli_integrations::enable_cli_integration,
                cli_integrations::inspect_mcp_clients,
                cli_integrations::install_mcp_client,
                cli_launch::installed_agent_clis,
                agent_runtime::agent_legacy_migration_rollback,
                agent_runtime::agent_legacy_migration_export_open,
                agent_runtime::agent_legacy_migration_preview,
                agent_runtime::agent_legacy_migration_apply,
                agent_runtime::agent_accounts_snapshot,
                agent_runtime::agent_tasks_snapshot,
                agent_runtime::agent_task_snapshot,
                agent_runtime::agent_account_create,
                agent_runtime::agent_account_update,
                agent_runtime::agent_account_remove,
                agent_runtime::agent_account_verify,
                agent_runtime::agent_account_recover,
                agent_runtime::agent_task_create,
                agent_runtime::agent_task_send,
                agent_runtime::agent_task_transfer_preview,
                agent_runtime::agent_task_transfer_apply,
                agent_runtime::agent_task_update_history_grant,
                agent_runtime::agent_task_prepare_switch,
                agent_runtime::agent_task_commit_switch,
                agent_runtime::agent_task_stop,
                agent_runtime::agent_task_recover,
                agent_runtime::agent_task_prepare_close,
                agent_runtime::agent_task_close_release,
                agent_runtime::agent_runtime_prepare_close,
                agent_runtime::agent_runtime_cancel_close,
                agent_runtime::agent_tasks_drain,
                agent_runtime::agent_runtime_shutdown,
                agent_runtime::agent_permission_snapshot,
                agent_runtime::agent_permission_reply,
                agent_runtime::agent_permission_freeze,
                agent_runtime::agent_permission_freeze_complete,
                agent_control::agent_control_state,
                agent_control::agent_control_startup_state,
                agent_control::agent_control_startup_decide,
                agent_control::agent_control_set_auto_start,
                agent_control::agent_control_set_yolo_mode,
                agent_control::agent_control_settings_open,
                agent_control::agent_control_settings_prepare,
                agent_control::agent_control_settings_source,
                agent_control::agent_control_settings_decide,
                agent_control::agent_control_settings_read_reply,
                agent_control::agent_control_enable,
                agent_control::agent_control_approve,
                agent_control::agent_control_chat_catalog,
                agent_control::agent_control_chat_open,
                agent_control::agent_control_chat_draft,
                agent_control::agent_control_chat_send_prepare,
                agent_control::agent_control_chat_send_pending,
                agent_control::agent_control_chat_send_decide,
                agent_control::agent_control_chat_send,
                agent_control::agent_control_project_open_decide,
                agent_control::agent_control_project_open_ready,
                agent_control::agent_control_project_open_commit,
                agent_control::agent_control_reject,
                agent_control::agent_control_revoke,
                agent_control::agent_control_closing,
                agent_control::agent_control_ui_register,
                agent_control::agent_control_ui_publish,
                agent_control::agent_control_ui_claim,
                agent_control::agent_control_ui_commit_close,
                agent_control::agent_control_workspace_close_pending,
                agent_control::agent_control_terminal_screen_reply,
                agent_control::agent_control_editor_read_reply,
                agent_control::agent_control_editor_open_file,
                agent_control::agent_control_git_open,
                agent_control::agent_control_git_mutation_prepare,
                agent_control::agent_control_git_mutation_pending,
                agent_control::agent_control_git_mutation_decide,
                agent_control::agent_control_git_mutation_commit,
                agent_control::agent_control_git_read,
                agent_control::agent_control_git_release,
                agent_control::agent_control_preview_asset,
                agent_control::agent_control_preview_release,
                agent_control::agent_control_files_mutate,
                agent_control::agent_control_artifact_export,
                agent_control::agent_control_file_trash_prepare,
                agent_control::agent_control_file_trash_decide,
                agent_control::agent_control_file_trash_pending,
                agent_control::agent_control_open_recovery,
                agent_control::agent_control_editor_save_file,
                agent_control::agent_control_decide_terminal,
                agent_control::agent_control_ui_ack,
                agent_control::agent_artifact_import,
                agent_control::agent_browser_download,
                agent_control::agent_browser_upload_prepare,
                agent_control::agent_browser_upload_decide,
                agent_control::agent_control_decide_install,
                agent_control::agent_control_decide_android_management,
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
                android::commands::android_take_control,
                android::commands::android_agent_input_blur,
                android::commands::agent_android_input,
                android::commands::agent_android_runtime,
                android::commands::agent_android_launch,
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
                window_lifecycle::hide_main_window,
                window_lifecycle::request_quit,
                window_lifecycle::reopen_main_window,
                push::push_get_state,
                push::push_refresh_state,
                push::push_set_current_source_enabled,
                push::desktop_notifications_get_state,
                push::desktop_notifications_save_preferences,
                push::desktop_notifications_report_permission,
                auth::auth_get_state,
                auth::auth_begin_login,
                auth::auth_open_verification,
                auth::auth_cancel_login,
                auth::auth_refresh_state,
                auth::auth_sign_out,
                auth::auth_open_account_portal,
                browser::sync_browsers,
                browser::browser_action,
                browser::agent_browser_navigate,
                browser::agent_browser_interact,
                browser::servers::local_web_servers,
                app_info,
                settings_window::about_info,
                updater::update_environment,
                updater::check_app_update,
                updater::request_update_check,
                updater::restart_after_update,
                show_ready_window,
                finish_window_startup,
                settings_window::open_settings,
                settings_window::settings_tab_ready,
                shared_host::shared_host_status,
                shared_host::shared_host_auth_setup,
                shared_host::shared_host_auth_configure,
                shared_host::shared_host_login_status,
                shared_host::shared_host_login_start,
                shared_host::shared_host_login_presentation,
                shared_host::shared_host_logout,
                shared_host::shared_host_workspaces,
                shared_host::shared_host_workspace_bindings,
                shared_host::shared_host_set_workspace_sharing,
                shared_host::shared_host_detach_view,
                shared_host::shared_host_import_workspace,
                shared_host::shared_host_close_terminal,
                shared_host::shared_host_workspace_state,
                shared_host::shared_host_rpc,
                shared_host::shared_host_validate_workspace_frame,
                shared_host::shared_host_stream_ack,
                shared_host::shared_host_register_shell_profile,
                shared_host::shared_host_terminal_aliases,
                shared_host::shared_host_save_terminal_alias,
                remote_v2::remote_v2_request,
                remote_v2::remote_v2_account_id,
                remote_v2::remote_v2_open_view,
                remote_v2::remote_v2_authorize_consumer,
                remote_v2::remote_v2_existing_window_identity,
                remote_v2::remote_v2_register_consumer_window,
                remote_v2::remote_v2_clear_revoked_consumer_window,
                remote_v2::remote_v2_channel_open,
                remote_v2::remote_v2_channel_send,
                remote_v2::remote_v2_channel_close,
                remote_v2::remote_v2_terminal_claim,
                remote_v2::remote_v2_terminal_input,
                remote_v2::remote_v2_terminal_resize,
                remote_v2::remote_v2_terminal_release,
                remote_v2::remote_v2_terminal_focus,
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
                themes::sync_theme_webview_background,
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
                git::git_repositories,
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
                terminal::write_terminal_response,
                terminal::take_terminal_control,
                terminal::resize_terminal,
                terminal::acknowledge_terminal,
                terminal::close_terminal,
                terminal::busy_terminals,
                terminal::reset_terminals,
                terminal::quote_paths,
                terminal::clipboard::paste_terminal_clipboard,
                terminal::terminal_contexts,
                cli_titles::inspect_cli_titles,
                cli_usage::inspect_cli_usage,
                cli_usage::authorize_cursor_usage_keychain,
                agent_notifications::request_agent_notification_setup,
                agent_notifications::inspect_agent_notifications,
                agent_notifications::enable_agent_notifications,
                agent_notifications::notify_agent,
                notifications::load_notifications,
                notifications::record_app_notification,
                notifications::mark_notifications_read,
                notifications::dismiss_notification,
                notifications::clear_read_notifications,
                notifications::clear_notifications,
                cli_titles::enable_cli_titles
            ];
            handler(invoke)
        })
        .build(context)
        .expect("failed to build Lomi");
    app.run(|app, event| {
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Opened { urls } = &event {
            external_files::enqueue(
                app,
                urls.iter()
                    .filter_map(|url| url.to_file_path().ok())
                    .collect(),
            );
        }
        #[cfg(target_os = "macos")]
        macos::handle_run_event(app, &event);
        if let tauri::RunEvent::ExitRequested { api, code, .. } = &event {
            if *code != Some(tauri::RESTART_EXIT_CODE) {
                if let Some(window) = app.get_window("main") {
                    api.prevent_exit();
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                    let _ = tauri::Emitter::emit(&window, "lomi-quit-requested", ());
                }
            }
        }
        if matches!(event, tauri::RunEvent::Exit) {
            if let Err(error) = app.state::<agent_runtime::AgentRuntime>().drain_all(app) {
                eprintln!("Agent runtime exit cleanup could not confirm completion: {error}");
            }

            tauri::async_runtime::block_on(agent_control::shutdown(app));
            app.state::<auth::AuthController>().shutdown();
            if let Err(error) = tauri::async_runtime::block_on(
                app.state::<android::manager::Android>().emergency_cleanup(),
            ) {
                eprintln!("Android exit cleanup could not confirm completion: {error}");
            }

            app.state::<chat::commands::Chats>().stop();
            app.state::<terminal::Terminals>().stop_all();
            #[cfg(windows)]
            app.state::<windows_instance::Instance>().release();
            #[cfg(target_os = "linux")]
            tauri_plugin_single_instance::destroy(app);
        }
        if let tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::Destroyed,
            ..
        } = event
        {
            if label == "main" {
                app.state::<agent_runtime::AgentRuntime>().stop_all();
                app.state::<chat::commands::Chats>().stop();
                app.state::<terminal::Terminals>().stop_all();
                app.exit(0);
            }
        }
    });
}
