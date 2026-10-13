mod commands;
mod error;
mod state;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(state::AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::connection::connect_connection_string,
            commands::connection::disconnect,
            commands::browse::list_queues,
            commands::browse::peek,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Dispatched");
}
