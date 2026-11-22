use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::SharedState;
use crate::config::Config;
use crate::screenshots;
use crate::state::{Cmd, Shared};

const MAX_NAME: usize = 40;

fn folders(cfg: &mut Config) -> HashMap<String, (PathBuf, bool)> {
    let keys: Vec<String> = cfg.all_games().keys().cloned().collect();
    keys.into_iter()
        .filter_map(|key| {
            let own = screenshots::own_folder(&cfg.game(&key)).is_some();
            screenshots::folder_for(cfg, &key, false).map(|path| (key, (path, own)))
        })
        .collect()
}

fn save_list(cfg: &mut Config, list: Vec<Map<String, Value>>) -> Result<(), String> {
    cfg.update(Map::from_iter([("collections".into(), json!(list))]))
        .map_err(|e| e.to_string())
}

fn change(shared: &Shared, edit: impl FnOnce(&mut Config) -> Result<(), String>) -> Result<(), String> {
    let moves: Vec<(String, PathBuf, PathBuf)> = {
        let mut cfg = shared.config.lock().unwrap();
        let before = folders(&mut cfg);
        edit(&mut cfg)?;
        let after = folders(&mut cfg);
        before
            .into_iter()
            .filter_map(|(key, (from, own))| {
                let (to, _) = after.get(&key)?;
                (!own && &from != to).then(|| (key, from, to.clone()))
            })
            .collect()
    };

    let mut failed = None;
    for (key, from, to) in &moves {
        if let Err(e) = screenshots::move_shots(from, to) {
            failed.get_or_insert(e);
        }
        shared.emit("screenshots-changed", json!({ "key": key }));
    }

    super::app::allow_screenshot_folder(shared);
    shared.send(Cmd::Reload);
    let settings = shared.config.lock().unwrap().data().clone();
    shared.emit("settings-changed", settings);
    shared.emit("library-changed", json!({}));
    match failed {
        Some(e) => Err(format!("Some screenshots couldn't be moved: {e}")),
        None => Ok(()),
    }
}

fn clean_name(cfg: &Config, id: &str, name: &str) -> Result<String, String> {
    let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err("Give the collection a name.".into());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!("Keep the name under {MAX_NAME} characters."));
    }
    let taken = screenshots::collection_list(cfg).iter().any(|c| {
        c.get("id").and_then(Value::as_str) != Some(id)
            && c.get("name").and_then(Value::as_str).is_some_and(|n| n.to_lowercase() == name.to_lowercase())
    });
    if taken {
        return Err("You already have a collection with that name.".into());
    }
    Ok(name)
}

fn folder_value(folder: &str) -> Result<String, String> {
    let folder = folder.trim();
    if !folder.is_empty() && !Path::new(folder).is_absolute() {
        return Err("Pick a full folder path.".into());
    }
    Ok(folder.to_string())
}

fn new_id(cfg: &Config) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let ids: Vec<String> = screenshots::collection_list(cfg)
        .iter()
        .filter_map(|c| c.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    let mut n = nanos;
    loop {
        let id = format!("c{:x}", n % 0xffff_ffff_ffff);
        if !ids.contains(&id) {
            return id;
        }
        n += 1;
    }
}

#[tauri::command(async)]
pub fn save_collection(shared: SharedState, id: Option<String>, name: String) -> Result<String, String> {
    let mut saved = String::new();
    change(&shared, |cfg| {
        let id = id.filter(|id| !id.is_empty()).unwrap_or_else(|| new_id(cfg));
        let name = clean_name(cfg, &id, &name)?;
        let mut list = screenshots::collection_list(cfg);
        match list.iter_mut().find(|c| c.get("id").and_then(Value::as_str) == Some(id.as_str())) {
            Some(found) => {
                found.insert("name".into(), json!(name));
            }
            None => list.push(Map::from_iter([
                ("id".into(), json!(id)),
                ("name".into(), json!(name)),
                ("folder".into(), json!("")),
            ])),
        }
        saved = id;
        save_list(cfg, list)
    })?;
    Ok(saved)
}

#[tauri::command(async)]
pub fn set_collection_folder(shared: SharedState, id: String, folder: String) -> Result<(), String> {
    let folder = folder_value(&folder)?;
    change(&shared, |cfg| {
        let mut list = screenshots::collection_list(cfg);
        let found = list
            .iter_mut()
            .find(|c| c.get("id").and_then(Value::as_str) == Some(id.as_str()))
            .ok_or("That collection doesn't exist anymore.")?;
        found.insert("folder".into(), json!(folder));
        save_list(cfg, list)
    })
}

#[tauri::command(async)]
pub fn move_collection(shared: SharedState, id: String, to: usize) -> Result<(), String> {
    change(&shared, |cfg| {
        let mut list = screenshots::collection_list(cfg);
        let from = list
            .iter()
            .position(|c| c.get("id").and_then(Value::as_str) == Some(id.as_str()))
            .ok_or("That collection doesn't exist anymore.")?;
        let item = list.remove(from);
        list.insert(to.min(list.len()), item);
        save_list(cfg, list)
    })
}

#[tauri::command(async)]
pub fn delete_collection(shared: SharedState, id: String) -> Result<(), String> {
    change(&shared, |cfg| {
        let list: Vec<Map<String, Value>> = screenshots::collection_list(cfg)
            .into_iter()
            .filter(|c| c.get("id").and_then(Value::as_str) != Some(id.as_str()))
            .collect();
        let members: Vec<(String, Vec<String>)> = cfg
            .all_games()
            .iter()
            .map(|(key, entry)| (key.clone(), screenshots::collections_of(entry)))
            .filter(|(_, ids)| ids.contains(&id))
            .collect();
        for (key, ids) in members {
            let kept: Vec<String> = ids.into_iter().filter(|other| other != &id).collect();
            cfg.set_game(&key, Map::from_iter([("collections".into(), json!(kept))]));
        }
        save_list(cfg, list)
    })
}

#[tauri::command(async)]
pub fn set_game_collections(shared: SharedState, key: String, ids: Vec<String>) -> Result<(), String> {
    change(&shared, |cfg| {
        let known: Vec<String> = screenshots::collection_list(cfg)
            .iter()
            .filter_map(|c| c.get("id").and_then(Value::as_str).map(str::to_string))
            .collect();
        let mut kept: Vec<String> = known.into_iter().filter(|id| ids.contains(id)).collect();
        kept.dedup();
        cfg.set_game(&key, Map::from_iter([("collections".into(), json!(kept))]));
        Ok(())
    })
}

#[tauri::command(async)]
pub fn set_collection_games(shared: SharedState, id: String, keys: Vec<String>) -> Result<(), String> {
    change(&shared, |cfg| {
        if !screenshots::collection_list(cfg)
            .iter()
            .any(|c| c.get("id").and_then(Value::as_str) == Some(id.as_str()))
        {
            return Err("That collection doesn't exist anymore.".into());
        }
        let changes: Vec<(String, Vec<String>)> = cfg
            .all_games()
            .iter()
            .filter_map(|(key, entry)| {
                let ids = screenshots::collections_of(entry);
                let want = keys.contains(key);
                (ids.contains(&id) != want).then(|| {
                    let mut ids: Vec<String> = ids.into_iter().filter(|other| other != &id).collect();
                    if want {
                        ids.push(id.clone());
                    }
                    (key.clone(), ids)
                })
            })
            .collect();
        for (key, ids) in changes {
            cfg.set_game(&key, Map::from_iter([("collections".into(), json!(ids))]));
        }
        Ok(())
    })
}

#[tauri::command(async)]
pub fn set_game_folder(shared: SharedState, key: String, folder: String) -> Result<(), String> {
    let folder = folder_value(&folder)?;
    change(&shared, |cfg| {
        cfg.set_game(&key, Map::from_iter([("screenshot_path".into(), json!(folder))]));
        Ok(())
    })
}
