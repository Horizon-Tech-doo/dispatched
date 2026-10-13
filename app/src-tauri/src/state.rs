use dispatched_core::Explorer;
use std::sync::Arc;
use tokio::sync::RwLock;

/// App-wide state: the active namespace explorer, if connected.
#[derive(Default)]
pub struct AppState {
    pub explorer: RwLock<Option<Arc<Explorer>>>,
}

impl AppState {
    pub async fn explorer(&self) -> Result<Arc<Explorer>, crate::error::CommandError> {
        self.explorer
            .read()
            .await
            .clone()
            .ok_or_else(|| crate::error::CommandError {
                kind: "not_connected",
                message: "connect to a namespace first".into(),
            })
    }
}
