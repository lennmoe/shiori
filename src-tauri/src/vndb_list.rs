use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use serde_json::{json, Map, Value};

use crate::state::Shared;
use crate::vndb::{push_list_status, ListDates};

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

    let dates = list_dates(&shared.config.lock().unwrap().game(key));
    let shared = Arc::clone(shared);
    let (key, vn_id, status) = (key.to_string(), vn_id.to_string(), status.to_string());
    std::thread::spawn(move || {
        let result = push_list_status(&shared.vndb, &token, &vn_id, &status, keep_existing, &dates);
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

fn list_dates(entry: &Map<String, Value>) -> ListDates {
    let started = entry
        .get("daily")
        .and_then(Value::as_object)
        .and_then(|days| days.iter().filter(|(_, s)| s.as_i64().unwrap_or(0) > 0).map(|(day, _)| day.clone()).min());
    let finished = entry
        .get("finished_at")
        .and_then(Value::as_i64)
        .filter(|at| *at > 0)
        .and_then(|at| chrono::DateTime::from_timestamp(at, 0))
        .map(|at| at.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string());
    ListDates { started, finished }
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

pub(crate) fn finished_at(date: &str) -> i64 {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()
        .and_then(|day| day.and_hms_opt(12, 0, 0))
        .and_then(|time| time.and_local_timezone(chrono::Local).single())
        .map(|time| time.timestamp())
        .unwrap_or(0)
}

#[derive(Clone)]
struct Pulled {
    key: Option<String>,
    id: String,
    title: String,
    image_url: String,
    nsfw: bool,
    status: String,
    vote: i64,
    finished_at: i64,
    seconds: i64,
}

static PULLED: Mutex<Vec<Pulled>> = Mutex::new(Vec::new());

fn list_status(item: &Value) -> String {
    let ids: Vec<i64> = item["labels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|label| label["id"].as_i64())
        .collect();
    crate::vndb::LIST_LABELS
        .iter()
        .find(|(_, label)| ids.contains(label))
        .map(|(name, _)| name.to_string())
        .unwrap_or_default()
}

fn num(entry: &Map<String, Value>, field: &str) -> i64 {
    entry.get(field).and_then(Value::as_i64).unwrap_or(0)
}

fn entry_vn(entry: &Map<String, Value>) -> String {
    ["vndb_id", "matched_vndb_id"]
        .iter()
        .filter_map(|field| entry.get(*field).and_then(Value::as_str))
        .find(|id| !id.is_empty())
        .unwrap_or("")
        .to_string()
}

pub(crate) fn time_merge(playtime: i64, before: i64, vndb_seconds: i64) -> Option<i64> {
    let tracked = (playtime - before).max(0);
    let wanted = (vndb_seconds - tracked).max(0);
    (vndb_seconds > tracked + 60 && wanted != before).then_some(wanted)
}

fn change(field: &str, shiori: Value, vndb: Value, checked: bool) -> Value {
    json!({ "field": field, "shiori": shiori, "vndb": vndb, "checked": checked })
}

fn diff(pulled: &Pulled, entry: Option<&Map<String, Value>>) -> Vec<Value> {
    let Some(entry) = entry else {
        if pulled.status.is_empty() {
            return Vec::new();
        }
        return vec![json!({
            "field": "add",
            "vndb": { "status": pulled.status, "vote": pulled.vote, "finished_at": pulled.finished_at, "seconds": pulled.seconds },
            "checked": true,
        })];
    };
    let mut out = Vec::new();
    let status = entry.get("status").and_then(Value::as_str).unwrap_or("");
    if !pulled.status.is_empty() && pulled.status != status {
        out.push(change("status", json!(status), json!(pulled.status), status.is_empty()));
    }
    let vote = num(entry, "vndb_vote");
    if pulled.vote > 0 && pulled.vote != vote {
        out.push(change("vote", json!(vote), json!(pulled.vote), vote == 0));
    }
    if pulled.finished_at > 0 && num(entry, "finished_at") == 0 {
        out.push(change("finished", json!(0), json!(pulled.finished_at), true));
    }
    let playtime = num(entry, "playtime_seconds");
    if let Some(before) = time_merge(playtime, num(entry, "before_seconds"), pulled.seconds) {
        let mut item = change("time", json!(playtime), json!(pulled.seconds), true);
        item["before"] = json!(before);
        item["tracked"] = json!((playtime - num(entry, "before_seconds")).max(0));
        out.push(item);
    }
    out
}

pub fn pull_preview(shared: &Arc<Shared>) -> Result<Value, String> {
    let token = token(shared);
    if token.is_empty() {
        return Err("add your VNDB token in Settings first".into());
    }
    let list = shared.vndb.whole_list(&token)?;
    let uid = shared.vndb.token_user(&token)?["id"].as_str().unwrap_or("").to_string();
    let lengths = shared.vndb.length_votes(&uid).unwrap_or_default();

    let cfg = shared.config.lock().unwrap();
    let mut by_vn: HashMap<String, (String, Map<String, Value>)> = HashMap::new();
    for (key, entry) in cfg.all_games() {
        let id = entry_vn(entry);
        if !id.is_empty() {
            by_vn.entry(id).or_insert_with(|| (key.clone(), entry.clone()));
        }
    }
    let ignored: HashSet<String> = cfg
        .get("vndb_import_ignored")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    drop(cfg);

    let mut pulled: Vec<Pulled> = Vec::new();
    let mut seen = HashSet::new();
    for (vn, item) in &list {
        seen.insert(vn.id.clone());
        pulled.push(Pulled {
            key: by_vn.get(&vn.id).map(|(key, _)| key.clone()),
            id: vn.id.clone(),
            title: vn.title.clone(),
            image_url: vn.image_url.clone(),
            nsfw: vn.nsfw,
            status: list_status(item),
            vote: item["vote"].as_i64().unwrap_or(0),
            finished_at: finished_at(item["finished"].as_str().unwrap_or("")),
            seconds: lengths.get(&vn.id).copied().unwrap_or(0) * 60,
        });
    }
    for (id, minutes) in &lengths {
        if seen.contains(id) {
            continue;
        }
        if let Some((key, entry)) = by_vn.get(id) {
            pulled.push(Pulled {
                key: Some(key.clone()),
                id: id.clone(),
                title: entry.get("title").and_then(Value::as_str).unwrap_or(id).to_string(),
                image_url: String::new(),
                nsfw: false,
                status: String::new(),
                vote: 0,
                finished_at: 0,
                seconds: minutes * 60,
            });
        }
    }

    let mut items = Vec::new();
    let mut kept = Vec::new();
    for item in pulled {
        if item.key.is_none() && ignored.contains(&item.id) {
            continue;
        }
        let entry = item.key.as_ref().and_then(|key| by_vn.get(&item.id).filter(|(k, _)| k == key)).map(|(_, e)| e);
        let changes = diff(&item, entry);
        if changes.is_empty() {
            continue;
        }
        let title = entry
            .and_then(|e| e.get("title").and_then(Value::as_str))
            .unwrap_or(&item.title)
            .to_string();
        items.push(json!({
            "id": item.id,
            "key": item.key,
            "title": title,
            "cover": (!item.image_url.is_empty()).then(|| item.image_url.clone()),
            "nsfw": item.nsfw,
            "changes": changes,
        }));
        kept.push(item);
    }
    *PULLED.lock().unwrap() = kept;
    Ok(json!({ "items": items, "list": list.len(), "lengths": lengths.len() }))
}

pub fn pull_apply(shared: &Arc<Shared>, picks: &Value) -> Result<usize, String> {
    let pulled = PULLED.lock().unwrap().clone();
    let stamp = chrono::Utc::now().timestamp();
    let mut applied = 0;
    let mut covers = Vec::new();
    {
        let mut cfg = shared.config.lock().unwrap();
        for pick in picks.as_array().into_iter().flatten() {
            let id = pick["id"].as_str().unwrap_or("");
            let Some(item) = pulled.iter().find(|p| p.id == id) else { continue };
            let fields: HashSet<&str> = pick["fields"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            match &item.key {
                None if fields.contains("add") && !item.status.is_empty() => {
                    let key = format!("{IMPORTED_PREFIX}{}", item.id);
                    if cfg.all_games().contains_key(&key) {
                        continue;
                    }
                    let mut entry = Map::from_iter([
                        ("title".into(), json!(item.title)),
                        ("vndb_id".into(), json!(item.id)),
                        ("match_checked".into(), json!(true)),
                        ("status".into(), json!(item.status)),
                        ("vndb_vote".into(), json!(item.vote)),
                        ("cover_nsfw".into(), json!(item.nsfw)),
                        ("vndb_list_only".into(), json!(true)),
                        ("vndb_pulled_at".into(), json!(stamp)),
                    ]);
                    if item.finished_at > 0 {
                        entry.insert("finished_at".into(), json!(item.finished_at));
                    }
                    if item.seconds > 0 {
                        entry.insert("playtime_seconds".into(), json!(item.seconds));
                        entry.insert("before_seconds".into(), json!(item.seconds));
                    }
                    cfg.set_game(&key, entry);
                    if !item.image_url.is_empty() {
                        covers.push((item.id.clone(), item.image_url.clone()));
                    }
                    applied += 1;
                }
                Some(key) => {
                    let entry = cfg.game(key);
                    let mut changes = Map::new();
                    if fields.contains("status") && !item.status.is_empty() {
                        changes.insert("status".into(), json!(item.status));
                        if item.status == "finished" && item.finished_at > 0 {
                            changes.insert("finished_at".into(), json!(item.finished_at));
                        }
                    }
                    if fields.contains("vote") && item.vote > 0 {
                        changes.insert("vndb_vote".into(), json!(item.vote));
                    }
                    if fields.contains("finished") && item.finished_at > 0 && num(&entry, "finished_at") == 0 {
                        changes.insert("finished_at".into(), json!(item.finished_at));
                    }
                    if fields.contains("time") {
                        let playtime = num(&entry, "playtime_seconds");
                        let before = num(&entry, "before_seconds");
                        if let Some(next) = time_merge(playtime, before, item.seconds) {
                            changes.insert("playtime_seconds".into(), json!((playtime - before).max(0) + next));
                            changes.insert("before_seconds".into(), json!(next));
                        }
                    }
                    if !changes.is_empty() {
                        changes.insert("vndb_pulled_at".into(), json!(stamp));
                        cfg.set_game(key, changes);
                        applied += 1;
                    }
                }
                None => {}
            }
        }
    }
    shared.emit("library-changed", json!({}));
    if !covers.is_empty() {
        let shared = Arc::clone(shared);
        std::thread::spawn(move || {
            for (id, url) in covers {
                shared.vndb.cover_path(&id, &url);
            }
            shared.emit("library-changed", json!({}));
        });
    }
    Ok(applied)
}

pub fn remove_before_time(shared: &Shared, key: &str) -> Result<(), String> {
    let mut cfg = shared.config.lock().unwrap();
    let entry = cfg.game(key);
    let before = num(&entry, "before_seconds");
    if before <= 0 {
        return Ok(());
    }
    let playtime = (num(&entry, "playtime_seconds") - before).max(0);
    cfg.set_game(key, Map::from_iter([
        ("playtime_seconds".into(), json!(playtime)),
        ("before_seconds".into(), json!(0)),
    ]));
    drop(cfg);
    shared.emit("library-changed", json!({ "key": key }));
    Ok(())
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
    fn merges_vndb_time_as_time_before_shiori() {
        assert_eq!(time_merge(12 * 3600, 0, 40 * 3600), Some(28 * 3600));
        assert_eq!(time_merge(40 * 3600 + 28 * 3600, 28 * 3600, 40 * 3600), None);
        assert_eq!(time_merge(50 * 3600, 0, 40 * 3600), None);
        assert_eq!(time_merge(20 * 3600 + 28 * 3600, 28 * 3600, 40 * 3600), Some(20 * 3600));
    }

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
