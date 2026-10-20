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

#[tauri::command(async)]
pub fn ui_language(shared: SharedState) -> &'static str {
    crate::lang::resolve(&shared.config.lock().unwrap().str("language"))
}

#[tauri::command(async)]
pub fn languages() -> Vec<crate::lang::Language> {
    crate::lang::list()
}

#[tauri::command(async)]
pub fn language_strings(code: String) -> serde_json::Value {
    crate::lang::data(&code)["ui"].clone()
}

#[tauri::command(async)]
pub fn mascot_personalities(shared: SharedState) -> Vec<crate::personalities::Choice> {
    let language = crate::lang::resolve(&shared.config.lock().unwrap().str("language"));
    crate::personalities::choices(language)
}

#[tauri::command(async)]
pub fn set_mascot_personality(shared: SharedState, id: String, personality: String) {
    mascot::set_personality(&shared, &id, &personality);
}

#[tauri::command(async)]
pub fn personality_lines(shared: SharedState, personality: String) -> crate::personalities::Lines {
    let language = crate::lang::resolve(&shared.config.lock().unwrap().str("language"));
    crate::personalities::lines(&personality, language)
}

#[tauri::command(async)]
pub fn set_mascot_custom_lines(shared: SharedState, id: String, lines: serde_json::Value) {
    mascot::set_custom_lines(&shared, &id, lines);
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
