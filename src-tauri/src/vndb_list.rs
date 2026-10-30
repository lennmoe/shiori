use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use serde_json::{json, Map, Value};

use crate::state::Shared;
use crate::vndb::push_list_status;

static SYNCING: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn token(shared: &Shared) -> String {
    shared
        .config
        .lock()
        .unwrap()
        .str("vndb_token")
        .trim()
        .to_string()
}

pub fn sync_status(
    shared: &Arc<Shared>,
    key: &str,
    vn_id: &str,
    status: &str,
    keep_existing: bool,
) {
    let token = token(shared);
    let enabled = shared.config.lock().unwrap().bool("vndb_sync");
    if !enabled || token.is_empty() {
        return;
    }

    {
        let mut syncing = SYNCING.lock().unwrap();
        let syncing = syncing.get_or_insert_with(HashSet::new);
        if !syncing.insert(vn_id.to_string()) {
            return;
        }
    }

    let shared = Arc::clone(shared);
    let (key, vn_id, status) = (key.to_string(), vn_id.to_string(), status.to_string());
    std::thread::spawn(move || {
        let result = push_list_status(&shared.vndb, &token, &vn_id, &status, keep_existing);
        SYNCING
            .lock()
            .unwrap()
            .get_or_insert_with(HashSet::new)
            .remove(&vn_id);

        let now = match result {
            Ok(now) => now,
            Err(e) => {
                shared.vndb_message(false, format!("VNDB list sync failed: {e}"));
                return;
            }
        };

        let mut fields = Map::new();
        fields.insert("vndb_synced".into(), json!(vn_id));
        if now != status {
            fields.insert("status".into(), json!(now));
        }
        shared.config.lock().unwrap().set_game(&key, fields);

        let shown = if now.is_empty() {
            "no status".to_string()
        } else {
            capitalize(&now)
        };
        shared.vndb_message(true, format!("VNDB list: {vn_id} → {shown}"));
        shared.emit("library-changed", json!({ "key": key }));
    });
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn rating_target(shared: &Shared, key: &str) -> Result<(String, String), String> {
    let token = token(shared);
    let vn_id = shared
        .config
        .lock()
        .unwrap()
        .game(key)
        .get("vndb_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if token.is_empty() {
        return Err("add your VNDB token in Settings first".into());
    }
    if vn_id.is_empty() {
        return Err("confirm this VN's VNDB entry first".into());
    }
    Ok((token, vn_id))
}

pub fn vote(shared: &Shared, key: &str) -> Result<Option<i64>, String> {
    let (token, vn_id) = rating_target(shared, key)?;
    let entry = shared.vndb.list_entry(&token, &vn_id)?;
    let vote = entry.and_then(|e| e["vote"].as_i64()).filter(|v| *v > 0);

    let fields = Map::from_iter([("vndb_vote".into(), json!(vote.unwrap_or(0)))]);
    shared.config.lock().unwrap().set_game(key, fields);
    Ok(vote)
}

pub fn set_vote(shared: &Shared, key: &str, vote: Option<i64>) -> Result<(), String> {
    let (token, vn_id) = rating_target(shared, key)?;
    if let Some(v) = vote {
        if !(10..=100).contains(&v) {
            return Err(format!("a VNDB rating is 10-100, not {v}"));
        }
    }

    shared
        .vndb
        .update_list_entry(&token, &vn_id, &json!({ "vote": vote }))?;
    let fields = Map::from_iter([("vndb_vote".into(), json!(vote.unwrap_or(0)))]);
    shared.config.lock().unwrap().set_game(key, fields);
    Ok(())
}

pub const IMPORTED_PREFIX: &str = "vndb@";

fn finished_at(date: &str) -> i64 {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()
        .and_then(|day| day.and_hms_opt(12, 0, 0))
        .and_then(|time| time.and_local_timezone(chrono::Local).single())
        .map(|time| time.timestamp())
        .unwrap_or(0)
}

pub fn import_finished(shared: &Arc<Shared>) -> Result<usize, String> {
    let token = token(shared);
    if token.is_empty() {
        return Err("add your VNDB token in Settings first".into());
    }
    let list = shared.vndb.finished_list(&token)?;

    let added: Vec<_> = {
        let mut cfg = shared.config.lock().unwrap();
        let mut known: HashSet<String> = cfg
            .all_games()
            .values()
            .flat_map(|entry| {
                ["vndb_id", "matched_vndb_id"].map(|field| {
                    entry
                        .get(field)
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string()
                })
            })
            .filter(|id| !id.is_empty())
            .collect();
        known.extend(
            cfg.get("vndb_import_ignored")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string),
        );

        let fresh: Vec<_> = list
            .into_iter()
            .filter(|(vn, _)| !known.contains(&vn.id))
            .collect();
        for (vn, item) in &fresh {
            let fields = Map::from_iter([
                ("title".into(), json!(vn.title)),
                ("vndb_id".into(), json!(vn.id)),
                ("match_checked".into(), json!(true)),
                ("status".into(), json!("finished")),
                (
                    "finished_at".into(),
                    json!(finished_at(item["finished"].as_str().unwrap_or(""))),
                ),
                (
                    "vndb_vote".into(),
                    json!(item["vote"].as_i64().unwrap_or(0)),
                ),
                ("cover_nsfw".into(), json!(vn.nsfw)),
                ("vndb_list_only".into(), json!(true)),
            ]);
            cfg.set_game(&format!("{IMPORTED_PREFIX}{}", vn.id), fields);
        }
        fresh.into_iter().map(|(vn, _)| vn).collect()
    };

    let count = added.len();
    shared.emit("library-changed", json!({}));
    let shared = Arc::clone(shared);
    std::thread::spawn(move || {
        for vn in added.iter().filter(|vn| !vn.image_url.is_empty()) {
            shared.vndb.cover_path(&vn.id, &vn.image_url);
        }
        shared.emit("library-changed", json!({}));
    });
    Ok(count)
}

pub fn absorb_import(cfg: &mut crate::config::Config, key: &str, vn_id: &str) -> bool {
    let imported = format!("{IMPORTED_PREFIX}{vn_id}");
    if vn_id.is_empty() || key == imported || !cfg.all_games().contains_key(&imported) {
        return false;
    }
    cfg.merge_games(&imported, key);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn reads_the_vndb_finish_date() {
        let noon = chrono::NaiveDate::from_ymd_opt(2024, 3, 9)
            .and_then(|day| day.and_hms_opt(12, 0, 0))
            .and_then(|time| time.and_local_timezone(chrono::Local).single())
            .map(|time| time.timestamp());
        assert_eq!(Some(finished_at("2024-03-09")), noon);
        assert_eq!(finished_at(""), 0);
    }

    #[test]
    fn a_detected_game_takes_over_its_imported_entry() {
        let dir = std::env::temp_dir().join(format!("vnrpc-test-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cfg = Config::load(&dir);
        let imported = Map::from_iter([
            ("vndb_id".into(), json!("v16044")),
            ("status".into(), json!("finished")),
            ("finished_at".into(), json!(1_700_000_000)),
            ("vndb_list_only".into(), json!(true)),
        ]);
        cfg.set_game("vndb@v16044", imported);
        cfg.set_game(
            "sabbatofthewitch",
            Map::from_iter([
                ("path".into(), json!(r"C:\vn\SabbatOfTheWitch.exe")),
                ("vndb_id".into(), json!("v16044")),
            ]),
        );

        assert!(!absorb_import(&mut cfg, "sabbatofthewitch", "v3144"));
        assert!(absorb_import(&mut cfg, "sabbatofthewitch", "v16044"));
        assert!(!cfg.all_games().contains_key("vndb@v16044"));
        let game = cfg.game("sabbatofthewitch");
        assert_eq!(game["status"], json!("finished"));
        assert_eq!(game["finished_at"], json!(1_700_000_000));
    }
}
