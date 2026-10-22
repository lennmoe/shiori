use std::sync::Arc;

use serde_json::Value;

use super::{blocking, SharedState};

#[tauri::command]
pub async fn vn_characters(shared: SharedState<'_>, vn_id: String) -> Result<Value, String> {
    let shared = Arc::clone(&shared);
    blocking(move || shared.vndb.characters(&vn_id)).await
}

#[tauri::command]
pub async fn vn_description(shared: SharedState<'_>, vn_id: String) -> Result<String, String> {
    let shared = Arc::clone(&shared);
    blocking(move || shared.vndb.description(&vn_id)).await
}
