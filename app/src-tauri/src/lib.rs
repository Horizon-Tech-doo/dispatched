mod commands;
mod error;
mod events;
mod ops;
mod state;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
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
            commands::connect,
            commands::disconnect,
            commands::list_entities,
            commands::list_subscriptions,
            commands::peek,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Dispatched");
}
