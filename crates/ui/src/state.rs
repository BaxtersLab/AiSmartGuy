use std::sync::{Arc, Mutex};
use crate::types::UiState;

/// Thread-safe handle to the global UI application state.
pub type SharedUiState = Arc<Mutex<UiState>>;

/// Create a new default shared state.
pub fn new_shared_state() -> SharedUiState {
    Arc::new(Mutex::new(UiState::default()))
}
