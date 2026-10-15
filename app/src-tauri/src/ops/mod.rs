//! Plain functions behind the Tauri commands. Commands only unwrap Tauri's
//! state and handle, so everything here is testable without a window.

pub mod account;
pub mod connection;
pub mod discovery;
pub mod profiles;
pub mod settings;
pub mod tree;

#[cfg(test)]
pub(crate) mod test_util {
    use crate::state::AppState;
    use dispatched_core::profiles::ProfileStore;
    use horizon_azure_kit::secrets::MemoryStore;
    use std::sync::Arc;

    pub const CS: &str = "Endpoint=sb://myns.servicebus.windows.net/;SharedAccessKeyName=reader;SharedAccessKey=c2VjcmV0LWtleQ==";
    pub const SECRET_PART: &str = "c2VjcmV0LWtleQ";

    pub fn state() -> AppState {
        AppState::new(
            ProfileStore::open_in_memory().unwrap(),
            Arc::new(MemoryStore::new()),
        )
    }
}
