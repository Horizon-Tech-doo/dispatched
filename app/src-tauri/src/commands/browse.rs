use crate::error::CommandError;
use crate::state::AppState;
use dispatched_core::{EntityPath, PeekedMessage, QueueInfo, SubQueue};
use tauri::State;

#[tauri::command]
pub async fn list_queues(state: State<'_, AppState>) -> Result<Vec<QueueInfo>, CommandError> {
    Ok(state.explorer().await?.list_queues().await?)
}

#[tauri::command]
pub async fn peek(
    state: State<'_, AppState>,
    entity: String,
    sub_queue: SubQueue,
    from_sequence: i64,
    max: u32,
) -> Result<Vec<PeekedMessage>, CommandError> {
    let path = EntityPath::parse(&entity)?;
    Ok(state
        .explorer()
        .await?
        .peek(&path, sub_queue, from_sequence, max.min(500))
        .await?)
}
