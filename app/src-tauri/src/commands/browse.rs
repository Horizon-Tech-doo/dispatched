use crate::error::CommandError;
use crate::state::AppState;
use dispatched_core::{EntityPath, PeekOptions, PeekView, PeekedMessage, QueueInfo, SubQueue};
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
    let view = match sub_queue {
        SubQueue::Active => PeekView::Active,
        SubQueue::DeadLetter => PeekView::DeadLetter,
        SubQueue::TransferDeadLetter => PeekView::TransferDeadLetter,
    };
    let opts = PeekOptions::new(max.min(500)).from_sequence(from_sequence);
    let page = state.explorer().await?.peek(&path, view, &opts).await?;
    Ok(page.messages)
}
