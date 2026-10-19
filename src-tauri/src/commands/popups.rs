use std::path::Path;

use tauri::{AppHandle, WebviewWindow};

use super::SharedState;
use crate::mascot::{self, GalleryItem, MascotInfo};
use crate::ui_windows::{self, Toast};

#[tauri::command]
pub fn mascot_info(window: WebviewWindow) -> Option<MascotInfo> {
    mascot::info(window.label())
}

#[tauri::command]
pub fn mascot_poke(app: AppHandle, window: WebviewWindow) {
    mascot::poke(&app, window.label());
}

#[tauri::command]
pub fn mascot_open_app(app: AppHandle) {
    ui_windows::show_main(&app);
}

#[tauri::command]
pub fn mascot_menu(app: AppHandle, window: WebviewWindow) {
    mascot::show_menu(&app, window.label());
}

#[tauri::command(async)]
pub fn list_mascots(shared: SharedState) -> Vec<GalleryItem> {
    mascot::gallery(&shared)
}

#[tauri::command(async)]
pub fn add_mascot(app: AppHandle, shared: SharedState, path: String) -> Result<(), String> {
    let id = mascot::add_to_gallery(&shared, Path::new(&path))?;
    mascot::set_shown(&shared, &id, true);
    mascot::refresh(&app);
    Ok(())
}

#[tauri::command(async)]
pub fn remove_mascot(app: AppHandle, shared: SharedState, id: String) -> Result<(), String> {
    mascot::remove_from_gallery(&shared, &id)?;
    mascot::refresh(&app);
    Ok(())
}

#[tauri::command]
pub async fn set_mascot_size(app: AppHandle, id: String, height: u32) -> Result<(), String> {
    super::blocking(move || mascot::resize(&app, &id, height)).await
}

#[tauri::command(async)]
pub fn show_mascot(app: AppHandle, shared: SharedState, id: String, shown: bool) {
    mascot::set_shown(&shared, &id, shown);
    mascot::refresh(&app);
}

#[tauri::command]
pub fn balloon_text() -> String {
    ui_windows::balloon_text()
}

#[tauri::command]
pub fn place_balloon(app: AppHandle, width: u32, height: u32) {
    ui_windows::place_balloon(&app, width, height);
}

#[tauri::command]
pub fn hide_balloon(app: AppHandle) {
    ui_windows::hide_balloon(&app);
}

#[tauri::command]
pub fn toast_content() -> Option<Toast> {
    ui_windows::current_toast()
}
