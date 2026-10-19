mod clipboard;
mod commands;
mod error;
mod events;
mod jobs;
mod ops;
mod safety;
mod state;

/// The macOS menu bar. Tauri's default one has File > Close Window on Cmd+W, which would close the
/// window instead of letting the webview close the active tab, so the app builds its own without it.
/// Edit keeps the standard items so copy and paste work in text fields.
#[cfg(target_os = "macos")]
fn app_menu<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{AboutMetadata, Menu, PredefinedMenuItem, Submenu, WINDOW_SUBMENU_ID};
    let info = app.package_info();
    let about = AboutMetadata {
        name: Some(info.name.clone()),
        version: Some(info.version.to_string()),
        copyright: app.config().bundle.copyright.clone(),
        ..Default::default()
    };
    let app_submenu = Submenu::with_items(
        app,
        info.name.clone(),
        true,
        &[
            &PredefinedMenuItem::about(app, None, Some(about))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    // The window menu id lets macOS list the open windows in it.
    let window = Submenu::with_id_and_items(
        app,
        WINDOW_SUBMENU_ID,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
        ],
    )?;
    Menu::with_items(app, &[&app_submenu, &edit, &window])
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    // Windows and Linux have no menu bar by default; only macOS gets one.
    #[cfg(target_os = "macos")]
    let builder = builder.menu(app_menu);
    builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppState::open())
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::settings_get,
            commands::settings_set,
            commands::profiles_list,
            commands::profile_save,
            commands::profile_delete,
            commands::profile_test,
            commands::profile_test_draft,
            commands::copy_entity_connection_string,
            commands::account_current,
            commands::sign_in,
            commands::sign_in_cancel,
            commands::sign_ins_list,
            commands::sign_out,
            commands::open_sign_in_page,
            commands::list_tenants,
            commands::discover_namespaces,
            commands::connect,
            commands::disconnect,
            commands::list_entities,
            commands::list_subscriptions,
            commands::peek_page,
            commands::message_detail,
            commands::check_filter,
            commands::entity_details,
            commands::list_rules,
            commands::list_sessions,
            commands::dashboard,
            commands::pick_export_path,
            commands::export_start,
            commands::job_cancel,
            commands::confirm_answer,
            commands::op_send,
            commands::op_import,
            commands::op_cancel_scheduled,
            commands::op_receive,
            commands::op_settle_deferred,
            commands::op_delete,
            commands::op_move,
            commands::op_resubmit,
            commands::purge_plan,
            commands::op_purge,
            commands::dlq_summary,
            commands::session_state_get,
            commands::op_session_state_set,
            commands::resolve_kind,
            commands::save_session_state,
            commands::pick_open_paths,
            commands::job_report,
            commands::pick_report_path,
            commands::job_report_save,
            commands::activity_list,
            commands::activity_since,
            commands::pick_activity_export_path,
            commands::activity_export,
            commands::job_reveal,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Dispatched")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                // The process exit skips destructors, so a running export would leave its hidden temp
                // file in the user's folder: cancel the exports and give them a moment to remove it.
                use tauri::Manager;
                let jobs = app.state::<state::AppState>().jobs.clone();
                if jobs.any_running() {
                    tauri::async_runtime::block_on(jobs.stop_all_and_wait(EXIT_WAIT));
                }
            }
        });
}

/// Longest the app waits at exit for cancelled exports to remove their temp files.
const EXIT_WAIT: std::time::Duration = std::time::Duration::from_secs(2);
