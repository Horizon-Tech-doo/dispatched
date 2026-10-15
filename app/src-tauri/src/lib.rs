mod commands;
mod error;
mod events;
mod jobs;
mod ops;
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
            commands::account_current,
            commands::sign_in,
            commands::sign_in_cancel,
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
            commands::job_reveal,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Dispatched");
}
