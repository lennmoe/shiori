use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::backup;
use crate::state::Shared;

const SITE: &str = "https://shiori.maia.moe";
const TOKEN: &str = "sync_token";
const USER: &str = "sync_user";
const AUTO_SAVE: &str = "sync_auto_save";
const MAX_BACKUP: usize = 25 * 1024 * 1024;
const LOGIN_WAIT: Duration = Duration::from_secs(300);
const USER_AGENT: &str = concat!("ShioriRPC/", env!("CARGO_PKG_VERSION"));

fn site() -> String {
    let site = std::env::var("SHIORI_SYNC_SITE").unwrap_or_else(|_| SITE.to_string());
    site.trim_end_matches('/').to_string()
}

fn api() -> String {
    format!("{}/api/sync", site())
}

fn client(timeout: u64) -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(timeout))
        .build()
        .map_err(|e| e.to_string())
}

fn token(shared: &Shared) -> Option<String> {
    Some(shared.config.lock().unwrap().str(TOKEN)).filter(|t| !t.is_empty())
}

fn remember(shared: &Shared, token: &str, name: &str) {
    let changes = Map::from_iter([(TOKEN.to_string(), json!(token)), (USER.to_string(), json!(name))]);
    let _ = shared.config.lock().unwrap().update(changes);
}

fn signed_out(shared: &Shared) {
    remember(shared, "", "");
    let _ = shared
        .config
        .lock()
        .unwrap()
        .update(Map::from_iter([(PROFILE.to_string(), json!(false))]));
}

fn nonce() -> String {
    use std::hash::{BuildHasher, RandomState};
    let mut hasher = Sha256::new();
    for n in 0..4u64 {
        hasher.update(RandomState::new().hash_one(n).to_le_bytes());
    }
    hasher.update(format!("{:?}{}", Instant::now(), std::process::id()));
    hasher.finalize().iter().take(16).map(|b| format!("{b:02x}")).collect()
}

fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let digit = |b: u8| (b as char).to_digit(16);
                match (digit(bytes[i + 1]), digit(bytes[i + 2])) {
                    (Some(high), Some(low)) => {
                        out.push((high * 16 + low) as u8);
                        i += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            byte => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query(path: &str) -> Map<String, Value> {
    let Some((_, query)) = path.split_once('?') else {
        return Map::new();
    };
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (unescape(k), json!(unescape(v))))
        .collect()
}

fn reply(stream: &mut TcpStream, status: &str, title: &str, text: &str) {
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>{title}</title>\
         <body style=\"font-family:system-ui,sans-serif;background:#0b0816;color:#f2f3f5;display:grid;\
         place-items:center;min-height:100vh;margin:0\"><div style=\"text-align:center\">\
         <h1 style=\"font-size:22px\">{title}</h1><p style=\"color:#a3a7b0\">{text}</p></div>"
    );
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}

fn read_path(stream: &mut TcpStream) -> Option<String> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut buffer = [0u8; 8192];
    let mut head = Vec::new();
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < buffer.len() {
        let n = stream.read(&mut buffer).ok()?;
        if n == 0 {
            break;
        }
        head.extend_from_slice(&buffer[..n]);
    }
    let line = String::from_utf8_lossy(&head).lines().next()?.to_string();
    let mut parts = line.split_whitespace();
    (parts.next()? == "GET").then(|| parts.next().map(str::to_string))?
}

pub fn login(shared: &Shared) -> Result<String, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let state = nonce();
    crate::shell::open(&format!("{}/login?port={port}&state={state}", api()))?;
    let (token, name) = wait_for_login(&listener, &state, LOGIN_WAIT, exchange)?;
    remember(shared, &token, &name);
    Ok(name)
}

fn exchange(code: &str) -> Result<(String, String), String> {
    let answer: Value = client(15)?
        .post(format!("{}/exchange", api()))
        .json(&json!({ "code": code }))
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json())
        .map_err(|e| e.to_string())?;
    let text = |key: &str| answer[key].as_str().unwrap_or("").to_string();
    if text("token").is_empty() {
        return Err("The login expired. Try again.".into());
    }
    Ok((text("token"), text("name")))
}

fn send_to(stream: &mut TcpStream, location: &str) {
    let head = format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _ = stream.write_all(head.as_bytes());
}

fn wait_for_login<T>(
    listener: &TcpListener,
    state: &str,
    wait: Duration,
    finish: impl Fn(&str) -> Result<T, String>,
) -> Result<T, String> {
    let started = Instant::now();
    while started.elapsed() < wait {
        let mut stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(150));
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        let _ = stream.set_nonblocking(false);
        let Some(path) = read_path(&mut stream) else {
            continue;
        };
        if !path.starts_with("/done?") {
            reply(&mut stream, "404 Not Found", "Shiori RPC", "Nothing here.");
            continue;
        }
        let answer = query(&path);
        let text = |key: &str| answer.get(key).and_then(Value::as_str).unwrap_or("").to_string();
        if text("state") != state || text("code").is_empty() {
            reply(&mut stream, "400 Bad Request", "Login didn't match", "Start again from Shiori RPC.");
            continue;
        }
        return match finish(&text("code")) {
            Ok(done) => {
                send_to(&mut stream, &format!("{}/signed-in", site()));
                Ok(done)
            }
            Err(error) => {
                reply(&mut stream, "400 Bad Request", "Login failed", "Start again from Shiori RPC.");
                Err(error)
            }
        };
    }
    Err("The Discord login took too long. Try again.".into())
}

pub fn status(shared: &Shared) -> Result<Value, String> {
    let Some(token) = token(shared) else {
        return Ok(json!({ "signed_in": false }));
    };
    let reply = client(10)?
        .get(format!("{}/me", api()))
        .bearer_auth(&token)
        .send()
        .map_err(|e| e.to_string())?;
    if reply.status().as_u16() == 401 {
        signed_out(shared);
        return Ok(json!({ "signed_in": false }));
    }
    let me: Value = reply.error_for_status().and_then(|r| r.json()).map_err(|e| e.to_string())?;
    let name = me["name"].as_str().unwrap_or("").to_string();
    if !name.is_empty() {
        remember(shared, &token, &name);
    }
    if let Some(public) = me["profile"]["public"].as_bool() {
        let _ = shared
            .config
            .lock()
            .unwrap()
            .update(Map::from_iter([(PROFILE.to_string(), json!(public))]));
    }
    Ok(json!({
        "signed_in": true,
        "name": name,
        "id": me["id"],
        "avatar": me["avatar"],
        "avatar_url": me["avatar_url"],
        "backup": me["backup"],
        "profile": me["profile"],
    }))
}

fn check(reply: reqwest::blocking::Response, shared: &Shared) -> Result<reqwest::blocking::Response, String> {
    match reply.status().as_u16() {
        401 => {
            signed_out(shared);
            Err("You were signed out. Sign in with Discord again.".into())
        }
        413 => Err("Your backup is too big for the cloud (25 MB max).".into()),
        429 => Err("Too many saves in a short time. Try again in a while.".into()),
        404 => Err("There's no backup in the cloud yet.".into()),
        _ => reply.error_for_status().map_err(|e| e.to_string()),
    }
}

pub fn upload(shared: &Shared, timeout: u64) -> Result<Value, String> {
    let token = token(shared).ok_or("Sign in with Discord first.")?;
    let file = shared.data_dir.join("cache").join("cloud-backup.zip");
    backup::export_for_cloud(&shared.config.lock().unwrap(), &file)?;
    let bytes = std::fs::read(&file).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&file);
    let bytes = bytes?;
    if bytes.len() > MAX_BACKUP {
        return Err("Your backup is too big for the cloud (25 MB max).".into());
    }
    let reply = client(timeout)?
        .put(format!("{}/backup", api()))
        .bearer_auth(&token)
        .header("Content-Type", "application/zip")
        .body(bytes)
        .send()
        .map_err(|e| e.to_string())?;
    check(reply, shared)?.json().map_err(|e| e.to_string())
}

pub fn download(shared: &Shared) -> Result<usize, String> {
    let token = token(shared).ok_or("Sign in with Discord first.")?;
    let reply = client(60)?
        .get(format!("{}/backup", api()))
        .bearer_auth(&token)
        .send()
        .map_err(|e| e.to_string())?;
    let bytes = check(reply, shared)?.bytes().map_err(|e| e.to_string())?;
    let file = shared.data_dir.join("cache").join("cloud-restore.zip");
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&file, &bytes).map_err(|e| e.to_string())?;
    let result = backup::import_from_cloud(&mut shared.config.lock().unwrap(), &file);
    let _ = std::fs::remove_file(&file);
    result
}

pub fn delete(shared: &Shared) -> Result<(), String> {
    let token = token(shared).ok_or("Sign in with Discord first.")?;
    let reply = client(15)?
        .delete(format!("{}/backup", api()))
        .bearer_auth(&token)
        .send()
        .map_err(|e| e.to_string())?;
    if reply.status().as_u16() != 401 {
        reply.error_for_status().map_err(|e| e.to_string())?;
    }
    signed_out(shared);
    Ok(())
}

pub fn logout(shared: &Shared) {
    if let Some(token) = token(shared) {
        if let Ok(client) = client(8) {
            let _ = client.post(format!("{}/logout", api())).bearer_auth(&token).send();
        }
    }
    signed_out(shared);
}

const PROFILE: &str = "profile_public";
const PROFILE_EVERY: Duration = Duration::from_secs(180);
const PROFILE_READING_EVERY: Duration = Duration::from_secs(60);
const PROFILE_TICK: Duration = Duration::from_secs(10);
const PROFILE_GAP: Duration = Duration::from_secs(20);
const PROFILE_HEARTBEAT: Duration = Duration::from_secs(300);
const PROFILE_RETRY_MAX: Duration = Duration::from_secs(300);
const HIDDEN_PRIVACY: [&str; 2] = ["off", "private"];
const PUBLIC_SEXUAL: f64 = 0.5;

fn entry_text(entry: &Map<String, Value>, field: &str) -> String {
    entry.get(field).and_then(Value::as_str).unwrap_or("").trim().to_string()
}

fn short_key(key: &str) -> String {
    sha1_smol::Sha1::from(key).digest().to_string()[..16].to_string()
}

fn hidden(entry: &Map<String, Value>) -> bool {
    match entry_text(entry, "site").as_str() {
        "hidden" => true,
        "shown" => false,
        _ => HIDDEN_PRIVACY.contains(&entry_text(entry, "privacy").to_lowercase().as_str()),
    }
}

fn vndb_cover(url: &str) -> Option<String> {
    let path = url
        .strip_prefix("https://t.vndb.org/cv/")
        .or_else(|| url.strip_prefix("https://t.vndb.org/cv.t/"))?;
    let valid = path.split('/').count() == 2
        && path.chars().all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '.');
    valid.then(|| path.to_string())
}

pub(crate) fn safe_image_scores(sexual: Option<f64>, violence: Option<f64>) -> bool {
    sexual.is_some_and(|score| score < PUBLIC_SEXUAL)
        && violence.is_some_and(|score| score < 1.0)
}

fn safe_image(image: &Value) -> bool {
    safe_image_scores(image["sexual"].as_f64(), image["violence"].as_f64())
}

fn vndb_image(shared: &Shared, id: &str, fetch: bool) -> Option<Value> {
    if let Some(raw) = shared.vndb.cached_raw(id) {
        return Some(raw["image"].clone());
    }
    static TRIED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::LazyLock::new(Default::default);
    if !fetch || !TRIED.lock().unwrap().insert(id.to_string()) {
        return None;
    }
    shared.vndb.get(id).ok()??;
    Some(shared.vndb.cached_raw(id)?["image"].clone())
}

fn cover_path(shared: &Shared, entry: &Map<String, Value>, fetch: bool) -> Option<(String, bool)> {
    let flagged = entry.get("cover_nsfw").and_then(Value::as_bool).unwrap_or(false);
    if entry_text(entry, "cover_source") == "vndb_release" {
        return release_cover_path(entry);
    }
    let image = vndb_id(entry).and_then(|id| vndb_image(shared, &id, fetch));
    let unsafe_image = image.as_ref().is_none_or(|image| !safe_image(image));
    let path = match entry_text(entry, "cover_source").as_str() {
        "url" => vndb_cover(&entry_text(entry, "cover_value"))?,
        "local" | "none" => return None,
        _ => vndb_cover(image.as_ref()?["url"].as_str()?)?,
    };
    Some((path, flagged || unsafe_image))
}

fn release_cover_path(entry: &Map<String, Value>) -> Option<(String, bool)> {
    let path = vndb_cover(&entry_text(entry, "cover_value"))?;
    let safe = entry.get("cover_public_safe").and_then(Value::as_bool) == Some(true);
    let flagged = entry.get("cover_nsfw").and_then(Value::as_bool).unwrap_or(true);
    Some((path, flagged || !safe))
}

fn vndb_id(entry: &Map<String, Value>) -> Option<String> {
    ["vndb_id", "matched_vndb_id"]
        .iter()
        .map(|field| entry_text(entry, field))
        .find(|id| !id.is_empty())
}

fn average_seconds(shared: &Shared, entry: &Map<String, Value>) -> i64 {
    vndb_id(entry)
        .and_then(|id| shared.vndb.cached_raw(&id))
        .and_then(|raw| raw["length_minutes"].as_i64())
        .filter(|minutes| *minutes > 0)
        .map_or(0, |minutes| minutes * 60)
}

fn vn_card(shared: &Shared, key: &str, entry: &Map<String, Value>, fetch: Option<bool>) -> Map<String, Value> {
    let title: String = crate::screenshots::display_name(key, entry).chars().take(80).collect();
    let mut card = Map::from_iter([("k".to_string(), json!(short_key(key))), ("t".to_string(), json!(title))]);
    if let Some(id) = vndb_id(entry) {
        card.insert("id".into(), json!(id));
    }
    if let Some((path, nsfw)) = fetch.and_then(|fetch| cover_path(shared, entry, fetch)) {
        card.insert("cv".into(), json!(path));
        if nsfw {
            card.insert("x".into(), json!(1));
        }
    }
    card
}

pub fn profile_payload(shared: &Shared, fetch: bool) -> Value {
    payload(shared, Some(fetch))
}

fn payload(shared: &Shared, fetch: Option<bool>) -> Value {
    let cfg = shared.config.lock().unwrap();
    let games = crate::commands::library::visible_games(&cfg);
    drop(cfg);
    let mut vns = Vec::new();
    let mut hidden_keys = Vec::new();
    let mut keys = std::collections::HashMap::new();
    for (key, entry) in &games {
        if hidden(entry) {
            hidden_keys.push(short_key(key));
            continue;
        }
        let seconds = entry.get("playtime_seconds").and_then(Value::as_i64).unwrap_or(0);
        let status = entry_text(entry, "status");
        if (seconds < 60 && status.is_empty()) || vndb_id(entry).is_none() {
            continue;
        }
        let mut card = vn_card(shared, key, entry, fetch);
        let average = match status.as_str() {
            "finished" if seconds < 60 => average_seconds(shared, entry),
            _ => 0,
        };
        card.insert("s".into(), json!(status));
        card.insert("p".into(), json!(seconds.max(average).max(0)));
        if average > seconds {
            card.insert("e".into(), json!(1));
        }
        for (field, short) in [("vndb_vote", "u"), ("last_played", "lp"), ("finished_at", "fa"), ("vndb_pulled_at", "q")] {
            if let Some(value) = entry.get(field).and_then(Value::as_i64).filter(|v| *v > 0) {
                card.insert(short.into(), json!(value));
            }
        }
        keys.insert(key.clone(), entry);
        vns.push(Value::Object(card));
    }
    let snap = shared.snapshot();
    let now = keys
        .get(&snap.key)
        .filter(|_| snap.detected && !snap.paused)
        .map(|entry| Value::Object(vn_card(shared, &snap.key, entry, fetch.map(|_| false))))
        .unwrap_or(Value::Null);
    let days = reading_days(keys.values().copied());
    json!({ "public": true, "now": now, "vns": vns, "hidden": hidden_keys, "days": days })
}

const HISTORY_DAYS: i64 = 400;

fn reading_days<'a>(entries: impl Iterator<Item = &'a Map<String, Value>>) -> Map<String, Value> {
    let oldest = (chrono::Local::now().date_naive() - chrono::Duration::days(HISTORY_DAYS)).format("%Y-%m-%d").to_string();
    let mut totals: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    for entry in entries {
        for (day, seconds) in entry.get("daily").and_then(Value::as_object).into_iter().flatten() {
            let seconds = seconds.as_i64().unwrap_or(0);
            if seconds > 0 && day.as_str() >= oldest.as_str() {
                *totals.entry(day.clone()).or_default() += seconds;
            }
        }
    }
    totals.into_iter().map(|(day, seconds)| (day, json!(seconds.min(86_400)))).collect()
}

fn put_profile(shared: &Shared, body: &Value) -> Result<Value, String> {
    put_profile_within(shared, body, 20)
}

fn put_profile_within(shared: &Shared, body: &Value, timeout: u64) -> Result<Value, String> {
    let token = token(shared).ok_or("Sign in with Discord first.")?;
    let reply = client(timeout)?
        .put(format!("{}/profile", api()))
        .bearer_auth(&token)
        .json(body)
        .send()
        .map_err(|e| e.to_string())?;
    check(reply, shared)?.json().map_err(|e| e.to_string())
}

pub fn set_public(shared: &Shared, public: bool) -> Result<Value, String> {
    let body = if public { profile_payload(shared, false) } else { json!({ "public": false }) };
    let answer = put_profile(shared, &body)?;
    let _ = shared
        .config
        .lock()
        .unwrap()
        .update(Map::from_iter([(PROFILE.to_string(), json!(public))]));
    Ok(answer)
}

fn library_shape(body: &Value) -> String {
    let mut vns = body["vns"].clone();
    for card in vns.as_array_mut().into_iter().flatten() {
        if let Some(card) = card.as_object_mut() {
            for field in ["p", "lp", "e"] {
                card.remove(field);
            }
        }
    }
    vns.to_string()
}

fn retry_wait(failures: u32) -> Duration {
    (PROFILE_GAP * 2u32.saturating_pow(failures.min(5))).min(PROFILE_RETRY_MAX)
}

pub fn start_profile_sync(shared: std::sync::Arc<Shared>) {
    std::thread::spawn(move || {
        let mut last_body = String::new();
        let mut last_shape = String::new();
        let mut last_now = Value::Null;
        let mut last_push: Option<Instant> = None;
        let mut failures = 0u32;
        loop {
            std::thread::sleep(PROFILE_TICK);
            let wanted = shared.config.lock().unwrap().bool(PROFILE);
            if !wanted || token(&shared).is_none() {
                last_body.clear();
                last_shape.clear();
                last_now = Value::Null;
                last_push = None;
                failures = 0;
                continue;
            }
            let probe = payload(&shared, None);
            let text = probe.to_string();
            let shape = library_shape(&probe);
            let since = last_push.map_or(Duration::MAX, |at| at.elapsed());
            let wait = if failures > 0 { retry_wait(failures) } else { PROFILE_GAP };
            if since < wait {
                continue;
            }
            let urgent = probe["now"]["k"] != last_now["k"] || shape != last_shape;
            let every = if probe["now"].is_null() { PROFILE_EVERY } else { PROFILE_READING_EVERY };
            let due = text != last_body && since >= every;
            let heartbeat = !probe["now"].is_null() && since >= PROFILE_HEARTBEAT;
            if !(urgent || due || heartbeat) {
                continue;
            }
            last_push = Some(Instant::now());
            let body = profile_payload(&shared, true);
            match put_profile(&shared, &body) {
                Ok(_) => {
                    last_body = text;
                    last_shape = shape;
                    last_now = probe["now"].clone();
                    failures = 0;
                }
                Err(error) => {
                    failures += 1;
                    crate::crash::note(format!("public profile: {error}"));
                }
            }
        }
    });
}

fn stop_reading_on_site(shared: &Shared) {
    if !shared.config.lock().unwrap().bool(PROFILE) || token(shared).is_none() {
        return;
    }
    let mut body = profile_payload(shared, false);
    body["now"] = Value::Null;
    if let Err(error) = put_profile_within(shared, &body, 4) {
        crate::crash::note(format!("public profile on exit: {error}"));
    }
}

pub fn save_on_exit(shared: &Shared) {
    stop_reading_on_site(shared);
    let wanted = shared.config.lock().unwrap().bool(AUTO_SAVE);
    if wanted && token(shared).is_some() {
        if let Err(error) = upload(shared, 10) {
            crate::crash::note(format!("cloud save on exit: {error}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_time_alone_does_not_change_the_library_shape() {
        let a = json!({"vns": [{"k": "a", "s": "playing", "p": 60, "lp": 1}]});
        let b = json!({"vns": [{"k": "a", "s": "playing", "p": 120, "lp": 2}]});
        let c = json!({"vns": [{"k": "a", "s": "finished", "p": 120, "lp": 2}]});
        assert_eq!(library_shape(&a), library_shape(&b));
        assert_ne!(library_shape(&b), library_shape(&c));
    }

    #[test]
    fn retries_slow_down_but_stay_bounded() {
        assert_eq!(retry_wait(1), Duration::from_secs(40));
        assert!(retry_wait(2) > retry_wait(1));
        assert_eq!(retry_wait(30), PROFILE_RETRY_MAX);
    }

    #[test]
    fn only_vndb_covers_and_shared_games_go_public() {
        assert_eq!(vndb_cover("https://t.vndb.org/cv/21/97921.jpg").as_deref(), Some("21/97921.jpg"));
        assert_eq!(vndb_cover("https://t.vndb.org/cv.t/21/97921.jpg").as_deref(), Some("21/97921.jpg"));
        assert_eq!(vndb_cover("https://files.catbox.moe/o9ev2s.png"), None);
        assert_eq!(vndb_cover("https://t.vndb.org/cv/../../x.jpg"), None);
        let entry = |privacy: &str| Map::from_iter([("privacy".to_string(), json!(privacy))]);
        assert!(hidden(&entry("off")));
        assert!(hidden(&entry("Private")));
        assert!(!hidden(&entry("partial")));
        assert!(!hidden(&entry("")));
        let mut shown = entry("off");
        shown.insert("site".into(), json!("shown"));
        assert!(!hidden(&shown));
        let mut kept = entry("full");
        kept.insert("site".into(), json!("hidden"));
        assert!(hidden(&kept));
        assert_eq!(short_key("bgi@eustia").len(), 16);
        assert!(safe_image(&json!({ "sexual": 0.33, "violence": 0 })));
        assert!(!safe_image(&json!({ "sexual": 0.93, "violence": 0 })));
        assert!(!safe_image(&json!({ "sexual": 0, "violence": 1.2 })));
        assert!(!safe_image(&json!({})));
    }

    #[test]
    fn release_cover_uses_its_own_image_and_public_safety() {
        let mut entry = Map::from_iter([
            ("cover_value".into(), json!("https://t.vndb.org/cv/12/123456789.jpg")),
            ("cover_nsfw".into(), json!(false)),
            ("cover_public_safe".into(), json!(true)),
        ]);
        assert_eq!(release_cover_path(&entry), Some(("12/123456789.jpg".into(), false)));
        entry.insert("cover_public_safe".into(), json!(false));
        assert_eq!(release_cover_path(&entry), Some(("12/123456789.jpg".into(), true)));
        entry.remove("cover_public_safe");
        assert_eq!(release_cover_path(&entry), Some(("12/123456789.jpg".into(), true)));
        entry.insert("cover_value".into(), json!("https://example.com/image.jpg"));
        assert_eq!(release_cover_path(&entry), None);
        assert!(!safe_image_scores(Some(0.5), Some(0.0)));
        assert!(!safe_image_scores(Some(0.0), None));
    }

    #[test]
    fn login_answers_are_read() {
        let answer = query("/done?state=ab12&token=x-Y_z&name=Len%20%E3%81%97%2B1");
        assert_eq!(answer["state"], "ab12");
        assert_eq!(answer["token"], "x-Y_z");
        assert_eq!(answer["name"], "Len し+1");
        assert_eq!(nonce().len(), 32);
        assert_ne!(nonce(), nonce());
    }

    fn visit(port: u16, path: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        answer
    }

    #[test]
    fn the_browser_hands_the_token_back() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let browser = std::thread::spawn(move || {
            let lost = visit(port, "/favicon.ico");
            let wrong = visit(port, "/done?state=other&code=evil");
            let right = visit(port, "/done?state=abc123&code=one-time");
            (lost, wrong, right)
        });
        let got = wait_for_login(&listener, "abc123", Duration::from_secs(10), |code| Ok(code.to_string())).unwrap();
        let (lost, wrong, right) = browser.join().unwrap();
        assert_eq!(got, "one-time");
        assert!(lost.starts_with("HTTP/1.1 404"));
        assert!(wrong.starts_with("HTTP/1.1 400"));
        assert!(right.starts_with("HTTP/1.1 302") && right.contains("/signed-in"));
    }
}
