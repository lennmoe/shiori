use std::sync::Arc;

use serde_json::Value;
use tauri::AppHandle;

use super::{blocking, SharedState};
use crate::cloud;
use crate::state::{Cmd, Shared};

#[tauri::command]
pub async fn cloud_status(shared: SharedState<'_>) -> Result<Value, String> {
    let shared = Arc::clone(&shared);
    blocking(move || cloud::status(&shared)).await
}

#[tauri::command]
pub async fn cloud_login(shared: SharedState<'_>, app: AppHandle) -> Result<Value, String> {
    let shared = Arc::clone(&shared);
    blocking(move || {
        cloud::login(&shared)?;
        let status = cloud::status(&shared);
        cloud::check_unread(&app, &shared);
        status
    })
    .await
}

#[tauri::command]
pub fn site_unread() -> u64 {
    cloud::unread()
}

#[tauri::command]
pub fn clear_site_unread(app: AppHandle) {
    cloud::clear_unread(&app);
}

#[tauri::command]
pub async fn cloud_save(shared: SharedState<'_>) -> Result<Value, String> {
    let shared = Arc::clone(&shared);
    blocking(move || cloud::upload(&shared, 60)).await
}

fn reloaded(shared: &Shared, app: &AppHandle) {
    shared.send(Cmd::Reload);
    super::app::allow_screenshot_folder(shared);
    crate::mascot::apply_settings(app);
    let settings: Value = shared.config.lock().unwrap().data().clone().into();
    shared.emit("settings-changed", settings);
    shared.emit("library-changed", serde_json::json!({}));
}

#[tauri::command]
pub async fn cloud_preview(shared: SharedState<'_>) -> Result<Value, String> {
    let shared = Arc::clone(&shared);
    blocking(move || {
        let file = cloud::fetch_backup(&shared)?;
        let cfg = shared.config.lock().unwrap();
        crate::backup::preview(&cfg, &file, super::library::in_library(&cfg))
    })
    .await
}

#[tauri::command]
pub async fn cloud_load(shared: SharedState<'_>, app: AppHandle) -> Result<usize, String> {
    let shared = Arc::clone(&shared);
    let count = {
        let shared = Arc::clone(&shared);
        blocking(move || cloud::download(&shared)).await?
    };
    reloaded(&shared, &app);
    Ok(count)
}

#[tauri::command]
pub async fn cloud_delete(shared: SharedState<'_>) -> Result<(), String> {
    let shared = Arc::clone(&shared);
    blocking(move || cloud::delete(&shared)).await
}

#[tauri::command]
pub async fn cloud_profile(shared: SharedState<'_>, public: bool) -> Result<Value, String> {
    let shared = Arc::clone(&shared);
    blocking(move || cloud::set_public(&shared, public)).await
}

#[tauri::command]
pub async fn cloud_logout(shared: SharedState<'_>, app: AppHandle) -> Result<(), String> {
    let shared = Arc::clone(&shared);
    blocking(move || {
        cloud::logout(&shared);
        cloud::clear_unread(&app);
        Ok(())
    })
    .await
}
