use crate::error::CommandError;
use crate::state::AppState;
use dispatched_core::{ConnectionString, Explorer, Namespace};
use serde::Serialize;
use std::sync::Arc;
use tauri::State;

#[derive(Serialize)]
pub struct ConnectionInfo {
    host: String,
    emulator: bool,
    auth: String,
}

#[tauri::command]
pub async fn connect_connection_string(
    state: State<'_, AppState>,
    connection_string: String,
) -> Result<ConnectionInfo, CommandError> {
    let ns = Namespace::from_connection_string(&ConnectionString::parse(&connection_string)?)?;
    let info = ConnectionInfo {
        host: ns.host().to_string(),
        emulator: ns.is_emulator(),
        auth: ns.tokens().describe(),
    };
    let previous = state
        .explorer
        .write()
        .await
        .replace(Arc::new(Explorer::new(ns)?));
    if let Some(old) = previous {
        old.close().await;
    }
    Ok(info)
}

#[tauri::command]
pub async fn disconnect(state: State<'_, AppState>) -> Result<(), CommandError> {
    // Take it out first so the write lock is not held while closing.
    let old = state.explorer.write().await.take();
    if let Some(old) = old {
        old.close().await;
    }
    Ok(())
}
